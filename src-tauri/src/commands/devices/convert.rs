//! What the link crate says, as what the page is told: every message is a fixed text, because the library's own texts
//! (keychain, bind and connection errors) carry system details that the page must not show.

use pairpane_core::model::{
    DeviceStatus, DevicesEvent, FoundDevice, IpcErrorCode, KeyStorage, PairedDevice, PairingEnd,
};
use pairpane_link::{Device, Error, Found, LinkEvent, MAX_NAME_CHARS, NodeId, PairingEndReason, Status, StoreKind};

use crate::error::{CommandError, CommandResult};

/// The name of a device whose host name is not usable.
pub const FALLBACK_NAME: &str = "Pairpane device";

/// The page's error for a failed link call.
pub fn command_error(error: &Error) -> CommandError {
    let (code, message) = match error {
        Error::Io(_) => (IpcErrorCode::Io, "The device files could not be read or written."),
        Error::Keychain(_) => (
            IpcErrorCode::Io,
            "The system keychain could not be used. Unlock your keyring and try again.",
        ),
        Error::KeyUnavailable(_) => (
            IpcErrorCode::Io,
            "The device key cannot be read. Unlock your keyring and try again.",
        ),
        Error::KeychainUnavailable(_) => (
            IpcErrorCode::Unsupported,
            "This system has no keychain to keep the device key in.",
        ),
        Error::InsecureKeyFile => (
            IpcErrorCode::Io,
            "The device key file can be read by other users. Make it readable only by you (mode 0600) and try again.",
        ),
        Error::CorruptKey | Error::KeyMismatch => (
            IpcErrorCode::Io,
            "The stored device key is damaged or is not the key of this device.",
        ),
        Error::CorruptIdentityMarker | Error::CorruptDevices => {
            (IpcErrorCode::Io, "The files of the paired devices are damaged.")
        }
        Error::AlreadyRunning => (IpcErrorCode::Busy, "Pairpane is already using the device files."),
        Error::InvalidName => (
            IpcErrorCode::InvalidInput,
            "A device name must be 1 to 64 characters without control characters.",
        ),
        Error::InvalidCode => (IpcErrorCode::InvalidInput, "That is not a pairing code."),
        Error::InvalidTarget => (IpcErrorCode::InvalidInput, "That is not the address of a device."),
        Error::TooManyDevices => (IpcErrorCode::InvalidInput, "Too many devices are paired."),
        Error::PairingRejected => (
            IpcErrorCode::InvalidInput,
            "The code is wrong, used, expired or cancelled.",
        ),
        Error::UnknownDevice => (IpcErrorCode::NotFound, "There is no such paired device."),
        Error::Bind(_) => (
            IpcErrorCode::Io,
            "The device link could not start listening on the network.",
        ),
        Error::Unreachable(_) => (IpcErrorCode::Io, "The other device could not be reached."),
        Error::PairingTimedOut => (IpcErrorCode::Timeout, "The other device did not answer in time."),
        Error::Protocol(_) => (IpcErrorCode::Protocol, "The other device did not follow the protocol."),
        Error::Random => (IpcErrorCode::Internal, "The system's random generator failed."),
        _ => (IpcErrorCode::Internal, "The device link is not running."),
    };

    CommandError::new(code, message)
}

/// A fixed word for the kind of an error, for the log: the error's own text can hold system details.
pub fn error_kind(error: &Error) -> &'static str {
    match error {
        Error::Io(_) => "io",
        Error::Keychain(_) => "keychain",
        Error::KeychainUnavailable(_) => "no keychain",
        Error::KeyUnavailable(_) => "key unavailable",
        Error::InsecureKeyFile => "insecure key file",
        Error::CorruptKey | Error::KeyMismatch => "key",
        Error::CorruptIdentityMarker | Error::CorruptDevices => "damaged files",
        Error::AlreadyRunning => "already running",
        Error::Bind(_) => "bind",
        Error::Unreachable(_) => "unreachable",
        Error::Closed | Error::NoRuntime => "not running",
        _ => "other",
    }
}

pub fn key_storage(kind: StoreKind) -> KeyStorage {
    match kind {
        StoreKind::Keychain => KeyStorage::Keychain,
        StoreKind::File => KeyStorage::File,
        StoreKind::Memory => KeyStorage::Memory,
    }
}

/// A paired device as the page sees it: without the addresses it was reached at.
pub fn paired(device: &Device) -> PairedDevice {
    PairedDevice {
        node_id: device.node_id.to_string(),
        name: device.name.clone(),
        paired_at: device.paired_at,
    }
}

pub fn paired_list(devices: &[Device]) -> Vec<PairedDevice> {
    devices.iter().map(paired).collect()
}

/// A device found on the network, without the addresses it announced.
pub fn found(device: Found) -> FoundDevice {
    FoundDevice {
        node_id: device.node_id.to_string(),
        name: device.name,
        paired: device.paired,
    }
}

pub fn status(status: Status) -> DeviceStatus {
    match status {
        Status::Online { name } => DeviceStatus::Online { name },
        Status::Offline => DeviceStatus::Offline,
        Status::Refused => DeviceStatus::Refused,
    }
}

/// Why the code ended; a reason a later version of the crate adds is read as a cancel.
pub fn end(reason: PairingEndReason) -> PairingEnd {
    match reason {
        PairingEndReason::Used => PairingEnd::Used,
        PairingEndReason::Expired => PairingEnd::Expired,
        PairingEndReason::TooManyAttempts => PairingEnd::TooManyAttempts,
        _ => PairingEnd::Cancelled,
    }
}

/// What the page is told of a link event; `None` for an event of a kind it does not know.
pub fn event(event: LinkEvent) -> Option<DevicesEvent> {
    match event {
        LinkEvent::Paired(device) => Some(DevicesEvent::Paired {
            device: paired(&device),
        }),
        LinkEvent::Unpaired(node_id) => Some(DevicesEvent::Unpaired {
            node_id: node_id.to_string(),
        }),
        LinkEvent::PairingEnded { reason } => Some(DevicesEvent::PairingEnded { reason: end(reason) }),
        _ => None,
    }
}

/// The id of a device as the page sends it.
pub fn parse_node_id(text: &str) -> CommandResult<NodeId> {
    text.trim()
        .parse()
        .map_err(|_| CommandError::new(IpcErrorCode::InvalidInput, "That is not a device id."))
}

/// The name this device is announced by: the host name without a trailing `.local` (macOS adds it), cut to the length a
/// name may have; [`FALLBACK_NAME`] when there is none or it is not a valid name.
pub fn device_name(host: Option<&str>) -> String {
    let host = host.map_or("", str::trim);
    let host = host
        .strip_suffix(".local")
        .or_else(|| host.strip_suffix(".LOCAL"))
        .unwrap_or(host);
    let cut: String = host.chars().take(MAX_NAME_CHARS).collect();

    pairpane_link::devices::validate_name(&cut).unwrap_or_else(|_| FALLBACK_NAME.to_owned())
}

/// The name of this computer, as the system gives it.
#[cfg(unix)]
pub fn host_name() -> Option<String> {
    let nodename = rustix::system::uname().nodename().to_string_lossy().into_owned();

    Some(nodename)
}

#[cfg(windows)]
pub fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(not(any(unix, windows)))]
pub fn host_name() -> Option<String> {
    None
}
