//! The accept loop: every incoming connection is looked at after its handshake, when the peer's node id is known, and
//! is served only if its protocol is allowed now. The pairing ALPN is served only while a code is active; the link ALPN
//! only for a node id that is pinned. Anything else is closed at once with an error code.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, Incoming, IncomingAddr};
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

use super::ctx::{Ctx, LinkEvent};
use super::hello::{self, LINK_ALPN};
use super::{CLOSE_NOT_ALLOWED, CLOSE_REFUSED};
use crate::error::Result;
use crate::pairing::{PAIR_ALPN, host_session};

/// How long a peer may take to finish the QUIC handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Accepts connections until the endpoint is closed.
pub async fn run(ctx: Arc<Ctx>) {
    while let Some(incoming) = ctx.endpoint.accept().await {
        let Ok(permit) = Arc::clone(&ctx.connections).try_acquire_owned() else {
            incoming.refuse();
            continue;
        };

        tokio::spawn(serve(Arc::clone(&ctx), incoming, permit));
    }
}

async fn serve(ctx: Arc<Ctx>, incoming: Incoming, _permit: OwnedSemaphorePermit) {
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

    let paired = host_session(conn, &ctx.endpoint.id(), attempt, &ctx.name, |node_id, name| {
        ctx.devices()
            .add_with_addrs(node_id, name, super::ctx::now_ms(), remote.as_slice())
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

    // The peer's own socket is the one it listens on, so it is where to dial it next time.
    ctx.remember(node_id, remote);

    let answered: Result<_> = hello::answer(ctx, conn).await;
    if let Err(error) = answered {
        tracing::debug!(%error, "a hello failed");
    }

    conn.close(super::CLOSE_DONE, b"bye");
}
