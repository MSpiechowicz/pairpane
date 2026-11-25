//! The framing both protocols (pairing and the hello of a link) speak over one bidirectional QUIC stream: every message
//! is a frame of a 2-byte big-endian length and that many bytes, never more than [`MAX_FRAME`], and every read has a
//! time limit. A peer can therefore neither make this side allocate much nor keep it waiting.

use std::future::Future;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use tokio::time::timeout;

use crate::error::{Error, Result};

/// The application error code of a connection that ended well.
pub const CLOSE_DONE: VarInt = VarInt::from_u32(0);
/// The application error code of a connection that was turned down: the protocol is not allowed now, or the device is
/// not paired.
pub const CLOSE_NOT_ALLOWED: VarInt = VarInt::from_u32(1);
/// The application error code of a pairing that failed.
pub const CLOSE_REFUSED: VarInt = VarInt::from_u32(2);

/// The most bytes in one frame.
pub const MAX_FRAME: usize = 4096;

/// How long a read waits for the next frame.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// The two halves of a stream, read and written in frames.
pub struct Channel {
    send: SendStream,
    recv: RecvStream,
}

impl Channel {
    /// Opens the stream on a connection this side made. The peer does not see a stream until something is written to
    /// it, so the caller sends first.
    pub async fn open(conn: &Connection) -> Result<Self> {
        let (send, recv) = step(conn.open_bi()).await?;

        Ok(Self { send, recv })
    }

    /// Takes the stream the peer opened on a connection this side accepted.
    pub async fn accept(conn: &Connection) -> Result<Self> {
        let (send, recv) = step(conn.accept_bi()).await?;

        Ok(Self { send, recv })
    }

    /// Sends one frame.
    pub async fn send(&mut self, payload: &[u8]) -> Result<()> {
        let length = u16::try_from(payload.len())
            .ok()
            .filter(|length| usize::from(*length) <= MAX_FRAME)
            .ok_or(Error::Protocol("a message is too long"))?;

        step(self.send.write_all(&length.to_be_bytes())).await?;
        step(self.send.write_all(payload)).await
    }

    /// Reads one frame. A stream that ends or is reset (which is how a peer that turned this side down looks) is
    /// [`Error::PairingRejected`], a peer that stays silent is [`Error::PairingTimedOut`], and a length over
    /// [`MAX_FRAME`] is a [`Error::Protocol`] error that is raised before any of the payload is read.
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut prefix = [0u8; 2];
        step(self.recv.read_exact(&mut prefix)).await?;

        let length = usize::from(u16::from_be_bytes(prefix));
        if length > MAX_FRAME {
            return Err(Error::Protocol("a message is too long"));
        }

        let mut payload = vec![0u8; length];
        step(self.recv.read_exact(&mut payload)).await?;

        Ok(payload)
    }

    /// Reads one frame that must be exactly `length` bytes.
    pub async fn recv_exact(&mut self, length: usize) -> Result<Vec<u8>> {
        let payload = self.recv().await?;

        if payload.len() != length {
            return Err(Error::Protocol("a message has the wrong size"));
        }

        Ok(payload)
    }

    /// Tells the peer to stop sending on this stream, so its next write fails. For tests of a peer that goes away.
    #[cfg(test)]
    pub fn stop_receiving(&mut self) {
        let _ = self.recv.stop(VarInt::from_u32(0));
    }

    /// Ends the sending half, so the peer reads what was sent and then the end of the stream.
    pub fn finish(&mut self) {
        let _ = self.send.finish();
    }
}

/// Runs one step of the stream within [`STEP_TIMEOUT`]. A step that fails means the peer is gone or has turned this side
/// down.
async fn step<T, E>(future: impl Future<Output = std::result::Result<T, E>>) -> Result<T> {
    timeout(STEP_TIMEOUT, future)
        .await
        .map_err(|_| Error::PairingTimedOut)?
        .map_err(|_| Error::PairingRejected)
}
