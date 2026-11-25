//! Finding devices on the local network with mDNS (the `iroh-mdns-address-lookup` crate, which uses swarm-discovery).
//!
//! A device that announces itself puts its node id, its addresses (only those of the user's own networks, see
//! `is_local_network`) and its name in multicast packets on the local network, and every device on that network can
//! read them, also the ones that are not paired. By default a device announces itself only while a pairing code is active. The name is only a label: nothing
//! in mDNS is authenticated, so a connection to a found device is still checked against its node id, and pairing
//! against the code, and what mDNS says is never written to disk. The service name `pairpane-link` keeps these records apart from other programs that use iroh.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use iroh::address_lookup::AddrFilter;
use iroh::endpoint_info::UserData;
use iroh::{EndpointId, TransportAddr};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use n0_future::StreamExt;
use tokio::sync::watch;

use super::ctx::{Ctx, MAX_REMEMBERED, lock};
use crate::devices::validate_name;

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
    /// The addresses it announced. They are only tried when connecting, never kept, because anyone on the network can
    /// announce any address.
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

/// Whether an address is on a network of the user's own: a private IPv4 range, the shared address space of Tailscale and
/// carrier-grade NAT (100.64.0.0/10), link-local, or an IPv6 unique local (fc00::/7) or link-local (fe80::/10) address.
/// Global addresses are not announced to the local network.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private() || ip.is_link_local() || (ip.octets()[0] == 100 && ip.octets()[1] & 0xC0 == 64)
        }
        IpAddr::V6(ip) => (ip.segments()[0] & 0xFE00) == 0xFC00 || (ip.segments()[0] & 0xFFC0) == 0xFE80,
    }
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
                evict_oldest(ctx, &mut found, ctx.is_paired(&node_id));
            }

            let name = endpoint_info
                .user_data()
                .and_then(|data| validate_name(data.as_ref()).ok());
            let addrs = endpoint_info.ip_addrs().copied().collect();
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

/// Makes room in the full list for a new device: the one heard longest ago goes, and a device that is paired is the last
/// to go, so a flood of made-up devices cannot push out the real ones.
fn evict_oldest(ctx: &Ctx, found: &mut HashMap<EndpointId, FoundEntry>, newcomer_is_paired: bool) {
    let oldest = |paired: bool| {
        found
            .iter()
            .filter(|(id, _)| ctx.is_paired(id) == paired)
            .min_by_key(|(_, entry)| entry.seen)
            .map(|(id, _)| *id)
    };

    // A newcomer that is not paired may not take the place of one that is.
    let victim = oldest(false).or_else(|| newcomer_is_paired.then(|| oldest(true)).flatten());
    if let Some(victim) = victim {
        found.remove(&victim);
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
