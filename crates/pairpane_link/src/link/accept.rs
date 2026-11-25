//! The accept loop: every incoming connection is looked at after its handshake, when the peer's node id is known, and
//! is served only if its protocol is allowed now. The pairing ALPN is served only while a code is active; the link ALPN
//! only for a node id that is pinned. Anything else is closed at once with an error code.
//!
//! A connection is taken only from a source on the user's own networks (or this machine); any other is refused before
//! anything is done for it. Before the handshake a connection from an address that has not proved it can receive packets
//! is sent a retry, so a forged source address cannot start handshakes, and one source address may have only a few
//! connections at once.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, Incoming, IncomingAddr};
use tokio::sync::OwnedSemaphorePermit;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::ctx::{Ctx, IpGuard, LinkEvent};
use super::hello::{self, LINK_ALPN};
use super::local_net::is_local_peer;
use crate::pairing::{PAIR_ALPN, PairingState, host_session};
use crate::wire::{CLOSE_DONE, CLOSE_NOT_ALLOWED, CLOSE_REFUSED};

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
fn admit(ctx: &Arc<Ctx>, incoming: Incoming) -> Option<(Incoming, OwnedSemaphorePermit, IpGuard)> {
    // Before the retry, which costs a packet: a source that cannot be a device of the user gets nothing.
    if !matches!(incoming.remote_addr(), IncomingAddr::Ip(addr) if is_local_peer(addr.ip())) {
        incoming.refuse();
        return None;
    }

    if !incoming.remote_addr_validated() {
        // The client sends its first packet again with the token it is given, from the address that it claims.
        if let Err(error) = incoming.retry() {
            tracing::debug!(%error, "a retry could not be sent");
        }
        return None;
    }

    // The source is an address, as checked above.
    let IncomingAddr::Ip(addr) = incoming.remote_addr() else {
        incoming.refuse();
        return None;
    };
    let Some(guard) = ctx.admit(addr.ip()) else {
        incoming.refuse();
        return None;
    };

    match Arc::clone(&ctx.connections).try_acquire_owned() {
        Ok(permit) => Some((incoming, permit, guard)),
        Err(_) => {
            incoming.refuse();
            None
        }
    }
}

async fn serve(ctx: Arc<Ctx>, incoming: Incoming, _permit: OwnedSemaphorePermit, _guard: IpGuard) {
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

async fn serve_pairing(ctx: &Arc<Ctx>, conn: &Connection, remote: Option<SocketAddr>) {
    // The attempt is taken only when the first message has come, but a connection with no code to wait for is turned
    // away now.
    if ctx.host.state() == PairingState::Idle {
        conn.close(CLOSE_NOT_ALLOWED, b"not pairing");
        return;
    }

    // The source of the pairing connection is authenticated by the key exchange: the joiner proved the code on it.
    let paired = host_session(conn, &ctx.endpoint.id(), &ctx.host, &ctx.name, async |node_id, name| {
        ctx.pin(node_id, name, remote.into_iter().collect()).await
    })
    .await;

    match paired {
        Ok(device) => {
            // The connection is closed by the end of the pairing, which waits for the joiner to take the host's name.
            let _ = ctx.events.send(LinkEvent::Paired(device));
        }
        Err(error) => {
            tracing::debug!(%error, "a pairing attempt failed");
            conn.close(CLOSE_REFUSED, b"pairing failed");
        }
    }
}

async fn serve_link(ctx: &Arc<Ctx>, conn: &Connection, remote: Option<SocketAddr>) {
    let node_id = conn.remote_id();
    if !ctx.is_paired(&node_id) {
        conn.close(CLOSE_NOT_ALLOWED, b"not paired");
        return;
    }

    // The peer's own socket is the one it listens on, so it is where to dial it next time. The connection is
    // authenticated, so the address it came from is a real address of that device.
    ctx.remember_in_background(node_id, remote);

    if let Err(error) = hello::answer(ctx, conn).await {
        tracing::debug!(%error, "a hello failed");
    }

    conn.close(CLOSE_DONE, b"bye");
}
