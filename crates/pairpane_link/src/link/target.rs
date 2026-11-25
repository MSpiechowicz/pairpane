//! A device to connect to: its node id, which iroh needs to dial anyone (the connection is checked against it), and the
//! socket addresses to try.
//!
//! A device found on the local network is a node id alone, its addresses come from mDNS. A device that is reached over a
//! VPN such as Tailscale is typed as `<node id>@<ip:port>[,<ip:port>...]`, which is what [`Target`]'s `Display` makes, so
//! the device that shows the code can show this text next to it.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use iroh::EndpointId;

use crate::error::Error;

/// The most addresses a target holds.
pub const MAX_TARGET_ADDRS: usize = 16;

/// A device to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The device's node id (its public key, 64 hexadecimal digits).
    pub node_id: EndpointId,
    /// The addresses to try. Empty for a device that mDNS found.
    pub addrs: Vec<SocketAddr>,
}

impl Target {
    /// The address iroh dials.
    #[cfg(test)]
    pub(crate) fn into_addr(self) -> iroh::EndpointAddr {
        iroh::EndpointAddr::from_parts(self.node_id, self.addrs.into_iter().map(iroh::TransportAddr::Ip))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.node_id)?;

        for (index, addr) in self.addrs.iter().enumerate() {
            out.write_str(if index == 0 { "@" } else { "," })?;
            write!(out, "{addr}")?;
        }

        Ok(())
    }
}

impl FromStr for Target {
    type Err = Error;

    /// Reads `<node id>` or `<node id>@<ip:port>[,<ip:port>...]`, with spaces around the parts ignored.
    fn from_str(text: &str) -> Result<Self, Error> {
        let (id, addrs) = match text.trim().split_once('@') {
            Some((id, addrs)) => (id, Some(addrs)),
            None => (text.trim(), None),
        };

        let node_id = id.trim().parse().map_err(|_| Error::InvalidTarget)?;

        let mut parsed = Vec::new();
        for addr in addrs.into_iter().flat_map(|addrs| addrs.split(',')) {
            parsed.push(addr.trim().parse().map_err(|_| Error::InvalidTarget)?);
        }

        if parsed.len() > MAX_TARGET_ADDRS {
            return Err(Error::InvalidTarget);
        }

        Ok(Self { node_id, addrs: parsed })
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
