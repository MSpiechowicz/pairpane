//! Finding devices on the local network with mDNS (the `iroh-mdns-address-lookup` crate, which uses swarm-discovery).
//!
//! A device that announces itself puts its node id, its addresses and its name in multicast packets on the local network,
//! and every device on that network can read them, also the ones that are not paired. The name is only a label: nothing
//! in mDNS is authenticated, so a connection to a found device is still checked against its node id, and pairing
//! against the code. The service name `pairpane-link` keeps these records apart from other programs that use iroh.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use iroh::EndpointId;
use iroh::endpoint_info::UserData;
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use n0_future::StreamExt;
use tokio::task::JoinHandle;

use super::ctx::{Ctx, MAX_REMEMBERED, lock};
use crate::devices::validate_name;

/// The mDNS service name of Pairpane devices.
const SERVICE_NAME: &str = "pairpane-link";

/// What the device does on the local network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discovery {
    /// Nothing: it neither listens nor announces.
    Off,
    /// It lists the devices that announce themselves but does not announce itself.
    Find,
    /// It lists the devices that announce themselves and announces itself.
    FindAndAnnounce,
}

/// A device seen on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The device's node id.
    pub node_id: EndpointId,
    /// The name it announced, if any. Anyone on the network can announce any name.
    pub name: Option<String>,
    /// The addresses it announced.
    pub addrs: Vec<SocketAddr>,
    /// Whether the device is paired with this one.
    pub paired: bool,
}

#[derive(Debug, Clone)]
pub struct FoundEntry {
    pub name: Option<String>,
    pub addrs: BTreeSet<SocketAddr>,
}

/// The name as an mDNS attribute, cut at a character boundary to the length one allows.
pub fn announced_name(name: &str) -> Option<UserData> {
    let mut end = name.len().min(UserData::MAX_LENGTH);
    while !name.is_char_boundary(end) {
        end -= 1;
    }

    name[..end].to_owned().try_into().ok()
}

/// Starts listening, and announcing if `mode` says so, on the endpoint of `ctx`. Returns the task that keeps the list of
/// found devices, or `None` when `mode` is off or the machine has no network that mDNS can use (the rest of the link
/// works without it).
pub async fn start(ctx: &Arc<Ctx>, mode: Discovery) -> Option<JoinHandle<()>> {
    if mode == Discovery::Off {
        return None;
    }

    let built = MdnsAddressLookup::builder()
        .service_name(SERVICE_NAME)
        .advertise(mode == Discovery::FindAndAnnounce)
        .build(ctx.endpoint.id());
    let mdns = match built {
        Ok(mdns) => mdns,
        Err(error) => {
            tracing::warn!(%error, "mDNS discovery is not available");
            return None;
        }
    };

    ctx.endpoint.address_lookup().ok()?.add(mdns.clone());
    let mut events = mdns.subscribe().await;

    let ctx = Arc::clone(ctx);
    Some(tokio::spawn(async move {
        // Keeps the lookup alive as long as this task runs; dropping the last copy stops it.
        let _mdns = mdns;

        while let Some(event) = events.next().await {
            record(&ctx, event);
        }
    }))
}

fn record(ctx: &Ctx, event: DiscoveryEvent) {
    let mut found = lock(&ctx.found);

    match event {
        DiscoveryEvent::Discovered { endpoint_info, .. } => {
            let node_id = endpoint_info.endpoint_id;
            if node_id == ctx.endpoint.id() || (found.len() >= MAX_REMEMBERED && !found.contains_key(&node_id)) {
                return;
            }

            let name = endpoint_info
                .user_data()
                .and_then(|data| validate_name(data.as_ref()).ok());
            let addrs: BTreeSet<SocketAddr> = endpoint_info.ip_addrs().copied().collect();
            found.insert(
                node_id,
                FoundEntry {
                    name,
                    addrs: addrs.clone(),
                },
            );
            drop(found);

            // A paired device that is on the local network is worth remembering for when it is not.
            ctx.keep(&node_id, &addrs.into_iter().collect::<Vec<_>>());
        }
        DiscoveryEvent::Expired { endpoint_id } => {
            found.remove(&endpoint_id);
        }
        _ => {}
    }
}
