//! The accept loop: every incoming connection is looked at after its handshake, when the peer's node id is known, and
//! is served only if its protocol is allowed now. The pairing ALPN is served only while a code is active; the link ALPN
//! only for a node id that is pinned. Anything else is closed at once with an error code.
//!
//! Before the handshake a connection from an address that has not proved it can receive packets is sent a retry, so a
//! forged source address cannot start handshakes, and one source address may have only a few connections at once.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, Incoming, IncomingAddr};
use tokio::sync::OwnedSemaphorePermit;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::ctx::{Ctx, IpGuard, LinkEvent, now_ms};
use super::hello::{self, LINK_ALPN};
use super::{CLOSE_NOT_ALLOWED, CLOSE_REFUSED};
use crate::pairing::{PAIR_ALPN, host_session};

/// How long a peer may take to finish the QUIC handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Accepts connections until the endpoint is closed. The tasks that serve them belong to this one: when it is aborted
/// or ends, they are aborted with it.
pub async fn run(ctx: Arc<Ctx>) {
    let mut serving = JoinSet::new();

    loop {
        tokio::select! {
            incoming = ctx.endpoint.accept() => {
                let Some(incoming) = incoming else {
                    break;
                };
                if let Some((incoming, permit, guard)) = admit(&ctx, incoming) {
                    serving.spawn(serve(Arc::clone(&ctx), incoming, permit, guard));
                }
            }
            // Finished tasks are collected as they end.
            Some(_) = serving.join_next(), if !serving.is_empty() => {}
        }
    }
}

/// Decides whether to take an incoming connection now. One that is not taken has been dealt with: it was sent a retry
/// or refused.
fn admit(ctx: &Arc<Ctx>, incoming: Incoming) -> Option<(Incoming, OwnedSemaphorePermit, Option<IpGuard>)> {
    if !incoming.remote_addr_validated() {
        // The client sends its first packet again with the token it is given, from the address that it claims.
        if let Err(error) = incoming.retry() {
            tracing::debug!(%error, "a retry could not be sent");
        }
        return None;
    }

    let guard = match incoming.remote_addr() {
        IncomingAddr::Ip(addr) => match ctx.admit(addr.ip()) {
            Some(guard) => Some(guard),
            None => {
                incoming.refuse();
                return None;
            }
        },
        _ => None,
    };

    match Arc::clone(&ctx.connections).try_acquire_owned() {
        Ok(permit) => Some((incoming, permit, guard)),
        Err(_) => {
            incoming.refuse();
            None
        }
    }
}

async fn serve(ctx: Arc<Ctx>, incoming: Incoming, _permit: OwnedSemaphorePermit, _guard: Option<IpGuard>) {
    let remote = match incoming.remote_addr() {
        IncomingAddr::Ip(addr) => Some(addr),
        _ => None,
    };

    let Ok(accepting) = incoming.accept() else {
        return;
    };
    let Ok(Ok(conn)) = timeout(HANDSHAKE_TIMEOUT, accepting).await else {
        return;
    };

    match conn.alpn() {
        PAIR_ALPN => serve_pairing(&ctx, &conn, remote).await,
        LINK_ALPN => serve_link(&ctx, &conn, remote).await,
        _ => conn.close(CLOSE_NOT_ALLOWED, b"unknown protocol"),
    }
}

async fn serve_pairing(ctx: &Ctx, conn: &Connection, remote: Option<SocketAddr>) {
    let Some(attempt) = ctx.host.start_attempt() else {
        conn.close(CLOSE_NOT_ALLOWED, b"not pairing");
        return;
    };

    // The source of the pairing connection is authenticated by the key exchange: the joiner proved the code on it.
    let paired = host_session(conn, &ctx.endpoint.id(), attempt, &ctx.name, |node_id, name| {
        ctx.devices().add_with_addrs(node_id, name, now_ms(), remote.as_slice())
    })
    .await;

    match paired {
        Ok(device) => {
            let _ = ctx.events.send(LinkEvent::Paired(device));
            conn.close(super::CLOSE_DONE, b"paired");
        }
        Err(error) => {
            tracing::debug!(%error, "a pairing attempt failed");
            conn.close(CLOSE_REFUSED, b"pairing failed");
        }
    }
}

async fn serve_link(ctx: &Ctx, conn: &Connection, remote: Option<SocketAddr>) {
    let node_id = conn.remote_id();
    if !ctx.is_paired(&node_id) {
        conn.close(CLOSE_NOT_ALLOWED, b"not paired");
        return;
    }

    // The peer's own socket is the one it listens on, so it is where to dial it next time. The connection is
    // authenticated, so the address it came from is a real address of that device.
    ctx.remember(node_id, remote);

    if let Err(error) = hello::answer(ctx, conn).await {
        tracing::debug!(%error, "a hello failed");
    }

    conn.close(super::CLOSE_DONE, b"bye");
}
