//! Making the endpoint: relays off, no address lookup but mDNS (added later), the ports of the configuration in order.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use iroh::endpoint::{BindOpts, presets};
use iroh::{Endpoint, SecretKey};

use super::discovery::{self, Discovery};
use super::{LINK_ALPN, LinkConfig};
use crate::error::{Error, Result};
use crate::pairing::PAIR_ALPN;

/// Makes the endpoint on the first port of the configuration that can be bound. Relays are off (the endpoint is built
/// from the empty relay setting) and no address lookup is added here, so it never contacts a server. A port that cannot
/// be bound is logged and the next one is tried.
pub async fn bind(config: &LinkConfig, key: SecretKey, name: &str) -> Result<Endpoint> {
    let mut last_error = Error::Bind("no port to try".to_owned());

    for (index, port) in config.ports.iter().enumerate() {
        match bind_on(config, key.clone(), name, *port).await {
            Ok(endpoint) => return Ok(endpoint),
            Err(error) => {
                let next = config.ports.get(index + 1);
                tracing::warn!(port, %error, ?next, "the port could not be bound");
                last_error = error;
            }
        }
    }

    Err(last_error)
}

async fn bind_on(config: &LinkConfig, key: SecretKey, name: &str, port: u16) -> Result<Endpoint> {
    let failed = |error: &dyn std::fmt::Display| Error::Bind(error.to_string());

    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![PAIR_ALPN.to_vec(), LINK_ALPN.to_vec()])
        .clear_ip_transports();

    if config.loopback_only {
        builder = builder
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .map_err(|e| failed(&e))?;
    } else {
        builder = builder
            .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
            .map_err(|e| failed(&e))?
            // A machine without IPv6 (or one whose IPv4 socket already covers it) still works.
            .bind_addr_with_opts(
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
                BindOpts::default().set_is_required(false),
            )
            .map_err(|e| failed(&e))?;
    }

    // The name goes out with the announcement, which is made only while a code is active (or always, if asked for).
    if config.discovery != Discovery::Off
        && let Some(data) = discovery::announced_name(name)
    {
        builder = builder.user_data_for_address_lookup(data);
    }

    builder.bind().await.map_err(|e| failed(&e))
}
