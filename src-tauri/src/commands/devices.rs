//! The Devices screen: pairing this device with the user's others and the list of the paired ones, over `pairpane_link`.
//!
//! The link starts when the screen is first opened (or at launch, when devices are paired already; see
//! `service.rs`), and every command is async because the pairing timer is a task of the async runtime. The pairing code,
//! the address of a device and the ids it is called with are never logged, and no message the page gets carries
//! text of the library or of the system (see `convert.rs`).

use std::time::Duration;

use pairpane_core::model::{
    DeviceStatus, DevicesEvent, DevicesOverview, FoundDevice, OwnDevice, PairedDevice, PairingShown,
};
use pairpane_link::{Error, Link, Target};
use tauri::State;
use tauri::ipc::Channel;

use crate::error::CommandResult;

mod convert;
mod qr;
mod service;

pub use self::convert::{device_name, host_name};
pub use self::service::{LinkService, paired_on_disk};

/// Logs what failed (by its kind) and answers the page with the fixed message.
fn failed(error: &Error) -> crate::error::CommandError {
    tracing::debug!(kind = convert::error_kind(error), "a device link call failed");

    convert::command_error(error)
}

/// What the screen shows first: this device and the paired ones.
fn overview(link: &Link) -> CommandResult<DevicesOverview> {
    let this = link.this_device().map_err(|error| failed(&error))?;
    let devices = link.devices().map_err(|error| failed(&error))?;

    Ok(DevicesOverview {
        own: OwnDevice {
            node_id: this.node_id.to_string(),
            name: this.name,
            key_storage: convert::key_storage(this.key_store),
        },
        devices: convert::paired_list(&devices),
    })
}

fn paired_devices(link: &Link) -> CommandResult<Vec<PairedDevice>> {
    let devices = link.devices().map_err(|error| failed(&error))?;

    Ok(convert::paired_list(&devices))
}

/// Opens the screen: starts the link when it does not run, ends any pairing code that is offered (a code does not
/// outlive the screen that showed it), makes `on_event` the channel of what happens next, and answers with this device and
/// the paired ones. A later call replaces the earlier channel (after a reload of the page), by its `session`: a number
/// that grows with every open of the page, so an open that was slow and comes after a newer one does not take the channel
/// (it still gets the overview).
#[tauri::command]
pub async fn devices_open(
    service: State<'_, LinkService>,
    on_event: Channel<DevicesEvent>,
    session: u64,
) -> CommandResult<DevicesOverview> {
    let link = service.ensure_started().await.map_err(|error| {
        tracing::warn!(kind = error.kind(), "the device link did not start");

        error.into_command_error()
    })?;

    // The channel is taken first, in one step: an open that is older than the page that has it neither takes it nor
    // ends that page's code.
    if service.subscribe(on_event, session) {
        service.cancel_code();
    }

    overview(&link)
}

/// Offers a pairing code (and the address to pair with) for another device to use. A code that is offered is replaced.
#[tauri::command]
pub async fn devices_pair_begin(service: State<'_, LinkService>) -> CommandResult<PairingShown> {
    let link = service.link()?;

    let offer = link.begin_pairing().map_err(|error| failed(&error))?;
    let target = link.this_device().map_err(|error| failed(&error))?.target().to_string();

    Ok(PairingShown {
        code: offer.code,
        expires_in_ms: u32::try_from(offer.expires_in.as_millis()).unwrap_or(u32::MAX),
        qr: qr::qr_image(&target),
        target,
    })
}

/// Stops offering the pairing code. `true` when a code was offered: only then does a `pairingEnded` event follow.
#[tauri::command]
pub async fn devices_pair_cancel(service: State<'_, LinkService>) -> CommandResult<bool> {
    service.link()?.cancel_pairing().map_err(|error| failed(&error))
}

/// Pairs with the device at `target` (a device id, or the text the other device shows) by the code its user sees.
/// It waits for the other device for up to about 40 seconds.
#[tauri::command]
pub async fn devices_join(
    service: State<'_, LinkService>,
    code: String,
    target: String,
) -> CommandResult<PairedDevice> {
    let link = service.link()?;
    let target: Target = target.parse().map_err(|error| failed(&error))?;

    let device = link.join(&code, &target).await.map_err(|error| failed(&error))?;
    service.refresh_paired(&link);

    Ok(convert::paired(&device))
}

/// The devices found on the local network that offer to pair.
#[tauri::command]
pub async fn devices_discovered(service: State<'_, LinkService>) -> CommandResult<Vec<FoundDevice>> {
    let found = service.link()?.discovered().map_err(|error| failed(&error))?;

    Ok(found.into_iter().map(convert::found).collect())
}

/// Whether a paired device answers now.
#[tauri::command]
pub async fn devices_status(service: State<'_, LinkService>, node_id: String) -> CommandResult<DeviceStatus> {
    let link = service.link()?;
    let node_id = convert::parse_node_id(&node_id)?;

    let status = link.status(&node_id).await.map_err(|error| failed(&error))?;

    Ok(convert::status(status))
}

/// Gives a paired device a new name and answers with the paired devices.
#[tauri::command]
pub async fn devices_rename(
    service: State<'_, LinkService>,
    node_id: String,
    name: String,
) -> CommandResult<Vec<PairedDevice>> {
    let link = service.link()?;
    let node_id = convert::parse_node_id(&node_id)?;

    link.rename_device(&node_id, &name).map_err(|error| failed(&error))?;
    service.refresh_paired(&link);

    paired_devices(&link)
}

/// Unpairs a device and answers with the paired devices.
#[tauri::command]
pub async fn devices_unpair(service: State<'_, LinkService>, node_id: String) -> CommandResult<Vec<PairedDevice>> {
    let link = service.link()?;
    let node_id = convert::parse_node_id(&node_id)?;

    link.unpair(&node_id).map_err(|error| failed(&error))?;
    service.refresh_paired(&link);

    paired_devices(&link)
}

/// How long quitting waits for the link to close its connections.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

#[cfg(test)]
mod convert_tests;
#[cfg(test)]
mod qr_tests;
#[cfg(test)]
mod tests;
