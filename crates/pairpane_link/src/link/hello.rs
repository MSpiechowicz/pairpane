//! The hello of a link, over the ALPN [`LINK_ALPN`]: the dialling device sends the protocol version and its name, the
//! other answers with the same, and both close. It is all a device needs to know that a paired device is online (a
//! connection that gets this far was accepted for its pinned node id), and it is the base later features build on.

use std::net::SocketAddr;
use std::time::Duration;

use iroh::EndpointAddr;
use iroh::endpoint::{Connection, ConnectionError};
use tokio::time::timeout;

use super::ctx::Ctx;
use crate::devices::validate_name;
use crate::error::{Error, Result};
use crate::wire::Channel;

/// The ALPN of a link between paired devices.
pub const LINK_ALPN: &[u8] = b"pairpane/link/1";

/// The version a hello starts with.
pub const HELLO_VERSION: u8 = 1;

/// How long a probe may take, from dialling to the answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the answering side waits for the other to take the answer before it closes the connection.
const LINGER: Duration = Duration::from_secs(2);

/// What a device that was reached says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// The protocol version.
    pub version: u8,
    /// The device's name.
    pub name: String,
}

fn encode(name: &str) -> Vec<u8> {
    let mut bytes = vec![HELLO_VERSION];
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> Result<Hello> {
    let (version, name) = bytes.split_first().ok_or(Error::Protocol("a hello is empty"))?;
    let name = std::str::from_utf8(name).map_err(|_| Error::Protocol("a device name is not text"))?;

    Ok(Hello {
        version: *version,
        name: validate_name(name).map_err(|_| Error::Protocol("a device name is not allowed"))?,
    })
}

/// How a probe ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// The device answered with a hello of the version this one speaks; `addr` is the address the connection used.
    Answered { hello: Hello, addr: Option<SocketAddr> },
    /// The device closed the connection because it does not have this one paired.
    Refused,
    /// The device could not be reached in time, or did not speak the protocol.
    Unreachable,
}

/// Dials `addr` and says hello. At most [`MAX_PROBES`](super::ctx::MAX_PROBES) probes run at once, and the time limit
/// [`PROBE_TIMEOUT`] counts from the moment one starts.
pub async fn probe(ctx: &Ctx, addr: EndpointAddr) -> Probe {
    let Ok(_permit) = ctx.probes.acquire().await else {
        return Probe::Unreachable;
    };

    timeout(PROBE_TIMEOUT, exchange(ctx, addr))
        .await
        .unwrap_or(Probe::Unreachable)
}

async fn exchange(ctx: &Ctx, addr: EndpointAddr) -> Probe {
    let Ok(conn) = ctx.endpoint.connect(addr, LINK_ALPN).await else {
        return Probe::Unreachable;
    };

    let answer = async {
        let mut channel = Channel::open(&conn).await?;
        channel.send(&encode(&ctx.name)).await?;
        decode(&channel.recv().await?)
    }
    .await;

    let probe = match answer {
        Ok(hello) if hello.version == HELLO_VERSION => Probe::Answered {
            hello,
            addr: super::used_addr(&conn),
        },
        Ok(_) => Probe::Unreachable,
        Err(_) => refusal_or_unreachable(&conn).await,
    };

    conn.close(super::CLOSE_DONE, b"bye");
    probe
}

/// Why a connection that failed ended: when the device closed it with the code of a refusal, it has no pairing with this
/// one.
async fn refusal_or_unreachable(conn: &Connection) -> Probe {
    // The close usually arrives together with the failure, but not always before it is noticed.
    let reason = match conn.close_reason() {
        Some(reason) => Some(reason),
        None => timeout(Duration::from_millis(500), conn.closed()).await.ok(),
    };

    match reason {
        Some(ConnectionError::ApplicationClosed(close)) if close.error_code == super::CLOSE_NOT_ALLOWED => {
            Probe::Refused
        }
        _ => Probe::Unreachable,
    }
}

/// Answers the hello of a paired device on `conn`.
pub async fn answer(ctx: &Ctx, conn: &Connection) -> Result<Hello> {
    let mut channel = Channel::accept(conn).await?;
    let hello = decode(&channel.recv().await?)?;
    if hello.version != HELLO_VERSION {
        return Err(Error::Protocol("the hello version is not supported"));
    }

    channel.send(&encode(&ctx.name)).await?;
    channel.finish();
    // The dialling side closes once it has the answer; closing first could drop the answer.
    let _ = timeout(LINGER, conn.closed()).await;

    Ok(hello)
}
