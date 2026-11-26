use pairpane_core::model::{DeviceStatus, DevicesEvent, IpcErrorCode, PairingEnd};
use pairpane_link::{Device, Error, LinkEvent, PairingEndReason, Status};

use super::convert::*;

/// A valid node id: the public key of the Ed25519 base point.
pub(super) const NODE_ID: &str = "5866666666666666666666666666666666666666666666666666666666666666";

fn device() -> Device {
    Device {
        node_id: NODE_ID.parse().expect("a node id"),
        name: "Laptop".to_owned(),
        paired_at: 1_700_000_000_000,
        addrs: vec!["10.0.0.5:47620".parse().expect("an address")],
    }
}

#[test]
fn errors_become_fixed_messages_without_the_text_of_the_library() {
    let secret = "10.1.2.3:47620 org.freedesktop.secrets";
    let errors = [
        (Error::Keychain(secret.to_owned()), IpcErrorCode::Io),
        (Error::KeyUnavailable(secret.to_owned()), IpcErrorCode::Io),
        (Error::KeychainUnavailable(secret.to_owned()), IpcErrorCode::Unsupported),
        (Error::Bind(secret.to_owned()), IpcErrorCode::Io),
        (Error::Unreachable(secret.to_owned()), IpcErrorCode::Io),
        (Error::Protocol("the secret detail"), IpcErrorCode::Protocol),
        (Error::Io(std::io::Error::other(secret)), IpcErrorCode::Io),
    ];

    for (error, code) in errors {
        let failure = command_error(&error);
        let shown = serde_json::to_string(&failure).expect("serializable");

        assert_eq!(failure.code(), code);
        assert!(!shown.contains("10.1.2.3"), "{shown}");
        assert!(!shown.contains("secrets"), "{shown}");
        assert!(!shown.contains("secret detail"), "{shown}");
    }
}

#[test]
fn errors_are_sorted_into_the_codes_the_page_acts_on() {
    let table = [
        (Error::AlreadyRunning, IpcErrorCode::Busy),
        (Error::InvalidName, IpcErrorCode::InvalidInput),
        (Error::InvalidCode, IpcErrorCode::InvalidInput),
        (Error::InvalidTarget, IpcErrorCode::InvalidInput),
        (Error::TooManyDevices, IpcErrorCode::InvalidInput),
        (Error::PairingRejected, IpcErrorCode::InvalidInput),
        (Error::UnknownDevice, IpcErrorCode::NotFound),
        (Error::PairingTimedOut, IpcErrorCode::Timeout),
        (Error::InsecureKeyFile, IpcErrorCode::Io),
        (Error::CorruptDevices, IpcErrorCode::Io),
        (Error::Closed, IpcErrorCode::Internal),
        (Error::NoRuntime, IpcErrorCode::Internal),
        (Error::Random, IpcErrorCode::Internal),
    ];

    for (error, code) in table {
        assert_eq!(command_error(&error).code(), code, "{error:?}");
    }
}

#[test]
fn a_key_file_that_others_can_read_is_not_called_forbidden() {
    let shown = serde_json::to_string(&command_error(&Error::InsecureKeyFile)).expect("serializable");

    assert!(shown.contains("0600"), "{shown}");
    assert!(!shown.contains("forbidden"), "{shown}");
}

#[test]
fn log_kinds_never_hold_the_error_text() {
    let kind = error_kind(&Error::Unreachable("10.1.2.3:47620".to_owned()));

    assert_eq!(kind, "unreachable");
}

#[test]
fn a_paired_device_is_sent_without_its_addresses() {
    let value = serde_json::to_value(paired(&device())).expect("serializable");

    assert_eq!(
        value,
        serde_json::json!({ "nodeId": NODE_ID, "name": "Laptop", "pairedAt": 1_700_000_000_000_u64 })
    );
}

#[test]
fn link_events_become_events_of_the_screen() {
    assert_eq!(
        event(LinkEvent::Paired(device())),
        Some(DevicesEvent::Paired {
            device: paired(&device())
        })
    );
    assert_eq!(
        event(LinkEvent::Unpaired(NODE_ID.parse().expect("a node id"))),
        Some(DevicesEvent::Unpaired {
            node_id: NODE_ID.to_owned()
        })
    );
}

#[test]
fn the_end_of_a_code_keeps_its_reason() {
    let table = [
        (PairingEndReason::Used, PairingEnd::Used),
        (PairingEndReason::Expired, PairingEnd::Expired),
        (PairingEndReason::Cancelled, PairingEnd::Cancelled),
        (PairingEndReason::TooManyAttempts, PairingEnd::TooManyAttempts),
    ];

    for (reason, end) in table {
        assert_eq!(
            event(LinkEvent::PairingEnded { reason }),
            Some(DevicesEvent::PairingEnded { reason: end })
        );
    }
}

#[test]
fn statuses_keep_the_name_a_device_gives_itself() {
    assert_eq!(
        status(Status::Online {
            name: "Desk".to_owned()
        }),
        DeviceStatus::Online {
            name: "Desk".to_owned()
        }
    );
    assert_eq!(status(Status::Offline), DeviceStatus::Offline);
    assert_eq!(status(Status::Refused), DeviceStatus::Refused);
}

#[test]
fn a_device_id_is_read_with_its_spaces_trimmed_and_nothing_else_is() {
    assert!(parse_node_id(&format!("  {NODE_ID}\n")).is_ok());

    let error = parse_node_id("not an id").expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(
        !serde_json::to_string(&error)
            .expect("serializable")
            .contains("not an id")
    );
}

#[test]
fn the_device_name_is_the_host_name_made_fit() {
    let long = "x".repeat(100);
    let table = [
        (Some("Desk"), "Desk"),
        (Some("  Desk  "), "Desk"),
        (Some("Maciejs-MacBook.local"), "Maciejs-MacBook"),
        (Some(long.as_str()), &long[..64]),
        (Some(""), FALLBACK_NAME),
        (Some(".local"), FALLBACK_NAME),
        (Some("bell\u{7}"), FALLBACK_NAME),
        (None, FALLBACK_NAME),
    ];

    for (host, name) in table {
        assert_eq!(device_name(host), name, "{host:?}");
    }
}
