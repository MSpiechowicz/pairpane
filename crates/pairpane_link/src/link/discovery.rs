//! Finding devices on the local network with mDNS (the `iroh-mdns-address-lookup` crate, which uses swarm-discovery).
//!
//! A device that announces itself puts its node id, its addresses (only those of the user's own networks, see
//! [`is_local_network`]) and its name in multicast packets on the local network, and every device on that network can
//! read them, also the ones that are not paired. By default a device announces itself only while a pairing code is
//! active. The name is only a label: nothing in mDNS is authenticated, so a connection to a found device is still
//! checked against its node id, and pairing against the code, and what mDNS says is never written to disk. Of what a
//! device reports, only the addresses on the user's own networks are listed, and at most [`MAX_ADDRS`] of them. The
//! service name `pairpane-link` keeps these records apart from other programs that use iroh.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use iroh::address_lookup::AddrFilter;
use iroh::endpoint_info::UserData;
use iroh::{EndpointId, TransportAddr};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use n0_future::StreamExt;
use tokio::sync::watch;

use super::ctx::{Ctx, MAX_REMEMBERED, lock};
use super::local_net::{is_local_network, is_local_peer};
use crate::devices::{MAX_ADDRS, validate_name};

/// The mDNS service name of Pairpane devices.
const SERVICE_NAME: &str = "pairpane-link";

/// What the device does on the local network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discovery {
    /// Nothing: it neither listens nor announces.
    Off,
    /// It lists the devices that announce themselves, and announces itself only while a pairing code is active, so a
    /// device that is joining can find it. This is the default.
    Find,
    /// It lists the devices that announce themselves and always announces itself, which makes it visible on the local
    /// network whenever the app runs.
    FindAndAnnounce,
}

/// A device seen on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The device's node id.
    pub node_id: EndpointId,
    /// The name it announced, if any. Anyone on the network can announce any name.
    pub name: Option<String>,
    /// The addresses it announced that are on the user's own networks, at most eight. They are only tried when
    /// connecting, never kept, because anyone on the network can announce any address. A join by this device (with a
    /// [`Target`](super::Target) made from it) dials them even though the target carries none.
    pub addrs: Vec<SocketAddr>,
    /// Whether the device is paired with this one.
    pub paired: bool,
}

#[derive(Debug, Clone)]
pub struct FoundEntry {
    pub name: Option<String>,
    pub addrs: BTreeSet<SocketAddr>,
    /// When it was last heard, as a stamp of [`Ctx::next_found_stamp`].
    pub seen: u64,
}

/// The name as an mDNS attribute, cut at a character boundary to the length one allows.
pub fn announced_name(name: &str) -> Option<UserData> {
    let mut end = name.len().min(UserData::MAX_LENGTH);
    while !name.is_char_boundary(end) {
        end -= 1;
    }

    name[..end].to_owned().try_into().ok()
}

/// The filter of what is announced: addresses on the user's own networks, and no relay.
fn announce_filter() -> AddrFilter {
    AddrFilter::new(|addrs| {
        Cow::Owned(
            addrs
                .iter()
                .filter(|addr| matches!(addr, TransportAddr::Ip(ip) if is_local_network(ip.ip())))
                .cloned()
                .collect(),
        )
    })
}

/// Listens for devices on the local network, and announces this one when `mode` says so or while a pairing code is
/// active (`announcing` is true). Runs until aborted. mDNS cannot be told to start or stop announcing, so the service is
/// made again when that changes. When the machine has no network that mDNS can use, it ends and the rest of the link
/// works without it.
pub async fn run(ctx: Arc<Ctx>, mode: Discovery, mut announcing: watch::Receiver<bool>) {
    loop {
        let advertise = mode == Discovery::FindAndAnnounce || *announcing.borrow_and_update();

        let built = MdnsAddressLookup::builder()
            .service_name(SERVICE_NAME)
            .advertise(advertise)
            .addr_filter(announce_filter())
            .build(ctx.endpoint.id());
        let mdns = match built {
            Ok(mdns) => mdns,
            Err(error) => {
                tracing::warn!(%error, "mDNS discovery is not available");
                return;
            }
        };

        let Ok(lookups) = ctx.endpoint.address_lookup() else {
            return;
        };
        lookups.clear();
        lookups.add(mdns.clone());
        let mut events = mdns.subscribe().await;

        loop {
            tokio::select! {
                event = events.next() => match event {
                    Some(event) => record(&ctx, event),
                    None => return,
                },
                changed = announcing.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if mode == Discovery::FindAndAnnounce {
                        continue;
                    }
                    break;
                }
            }
        }
    }
}

/// Notes what mDNS reported in the list of found devices. Nothing here is written to disk.
pub(crate) fn record(ctx: &Ctx, event: DiscoveryEvent) {
    let mut found = lock(&ctx.found);

    match event {
        DiscoveryEvent::Discovered { endpoint_info, .. } => {
            let node_id = endpoint_info.endpoint_id;
            if node_id == ctx.endpoint.id() {
                return;
            }

            if found.len() >= MAX_REMEMBERED && !found.contains_key(&node_id) {
                // One look at the device list for the whole event, not one per entry of the found list.
                let paired: HashSet<EndpointId> = ctx.devices().list().iter().map(|device| device.node_id).collect();
                if !make_room(&mut found, &paired, paired.contains(&node_id)) {
                    return;
                }
            }

            let name = endpoint_info
                .user_data()
                .and_then(|data| validate_name(data.as_ref()).ok());
            let addrs = endpoint_info
                .ip_addrs()
                .copied()
                .filter(|addr| is_local_peer(addr.ip()))
                .take(MAX_ADDRS)
                .collect();
            found.insert(
                node_id,
                FoundEntry {
                    name,
                    addrs,
                    seen: ctx.next_found_stamp(),
                },
            );
        }
        DiscoveryEvent::Expired { endpoint_id } => {
            found.remove(&endpoint_id);
        }
        _ => {}
    }
}

/// Makes room in the full list for a new device, and says whether it did. The device heard longest ago that is not in
/// `paired` goes; a device that is paired goes only for a newcomer that is paired too, so a flood of made-up devices
/// cannot push out the real ones. When nothing may go (every entry is paired and the newcomer is not) the newcomer is not
/// to be added, or the list would grow past its limit.
fn make_room(
    found: &mut HashMap<EndpointId, FoundEntry>,
    paired: &HashSet<EndpointId>,
    newcomer_is_paired: bool,
) -> bool {
    let oldest = |is_paired: bool| {
        found
            .iter()
            .filter(|(id, _)| paired.contains(*id) == is_paired)
            .min_by_key(|(_, entry)| entry.seen)
            .map(|(id, _)| *id)
    };

    let victim = oldest(false).or_else(|| newcomer_is_paired.then(|| oldest(true)).flatten());
    match victim {
        Some(victim) => {
            found.remove(&victim);
            true
        }
        None => false,
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
