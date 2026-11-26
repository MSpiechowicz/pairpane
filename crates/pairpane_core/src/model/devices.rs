use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the secret key of this device is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KeyStorage {
    /// The system keychain (Secret Service, Keychain, Credential Manager).
    Keychain,
    /// A private file (readable only by the user) in the data folder, because the system has no keychain.
    File,
    /// Nowhere: the key is lost when the app ends, so a pairing does not survive a restart.
    Memory,
}

/// This device, as the other devices see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OwnDevice {
    /// The public identity of this device (its node id), as text.
    pub node_id: String,
    /// The name other devices show for this one.
    pub name: String,
    /// Where the secret key is kept.
    pub key_storage: KeyStorage,
}

/// A device this one is paired with. Network addresses are never sent to the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PairedDevice {
    /// The public identity of the device, as text.
    pub node_id: String,
    /// The name given to the device (by the user, or the one it announced).
    pub name: String,
    /// When the pairing was made, in milliseconds since the Unix epoch.
    #[ts(type = "number")]
    pub paired_at: u64,
}

/// The answer of `devices_open`: what the Devices screen shows first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevicesOverview {
    /// This device.
    pub own: OwnDevice,
    /// The paired devices, in the order they were paired.
    pub devices: Vec<PairedDevice>,
}

/// A QR code as one SVG path: modules are `M x y h n v 1 h -n z` runs on a grid of `size` by `size` modules, so the
/// path holds digits and the letters `M`, `h`, `v`, `z` only. The page draws it black on a white square with a quiet
/// zone of 4 modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QrImage {
    /// Width and height of the code in modules.
    pub size: u16,
    /// The `d` attribute of the path.
    pub path: String,
}

/// A pairing code that is being offered, with the way to reach this device. The code is secret: it is never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PairingShown {
    /// The one-time code the other device types.
    pub code: String,
    /// How long the code stays valid, in milliseconds.
    pub expires_in_ms: u32,
    /// The text the other device pastes (or scans) to find this one.
    pub target: String,
    /// The target as a QR code; `null` when it does not fit into one.
    pub qr: Option<QrImage>,
}

impl fmt::Debug for PairingShown {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("PairingShown")
            .field("expires_in_ms", &self.expires_in_ms)
            .field("qr", &self.qr.is_some())
            .finish_non_exhaustive()
    }
}

/// A device found on the local network that offers to pair. Network addresses are never sent to the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FoundDevice {
    /// The public identity of the device, as text.
    pub node_id: String,
    /// The name the device announced; anyone on the network can announce any name. `null` when it gave none.
    pub name: Option<String>,
    /// This device is already paired.
    pub paired: bool,
}

/// Whether a paired device can be reached right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum DeviceStatus {
    /// It answered; `name` is the name it gives itself now.
    Online { name: String },
    /// It did not answer.
    Offline,
    /// It answered and does not know this device any more.
    Refused,
}

/// Why the pairing code stopped being offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PairingEnd {
    /// Another device used it.
    Used,
    /// Its time ran out.
    Expired,
    /// It was cancelled from this side.
    Cancelled,
    /// Too many wrong codes were tried, so it was withdrawn.
    TooManyAttempts,
}

/// What the devices channel of `devices_open` delivers while the screen is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum DevicesEvent {
    /// Another device paired with this one.
    Paired { device: PairedDevice },
    /// A device was removed.
    Unpaired { node_id: String },
    /// The pairing code is no longer offered.
    PairingEnded { reason: PairingEnd },
    /// Events were missed: the listener replaces its list with `devices`.
    Resync { devices: Vec<PairedDevice> },
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    fn device() -> PairedDevice {
        PairedDevice {
            node_id: "abc".to_owned(),
            name: "Laptop".to_owned(),
            paired_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn key_storage_and_pairing_end_serialize_in_camel_case() {
        let storage = [
            (KeyStorage::Keychain, "keychain"),
            (KeyStorage::File, "file"),
            (KeyStorage::Memory, "memory"),
        ];
        let ends = [
            (PairingEnd::Used, "used"),
            (PairingEnd::Expired, "expired"),
            (PairingEnd::Cancelled, "cancelled"),
            (PairingEnd::TooManyAttempts, "tooManyAttempts"),
        ];

        for (value, wire) in storage {
            assert_eq!(serde_json::to_value(value).expect("serializable"), json!(wire));
        }
        for (value, wire) in ends {
            assert_eq!(serde_json::to_value(value).expect("serializable"), json!(wire));
        }
    }

    #[test]
    fn overview_serializes_in_camel_case() {
        let overview = DevicesOverview {
            own: OwnDevice {
                node_id: "me".to_owned(),
                name: "Desk".to_owned(),
                key_storage: KeyStorage::File,
            },
            devices: vec![device()],
        };

        let value = serde_json::to_value(overview).expect("serializable");

        assert_eq!(
            value,
            json!({
                "own": { "nodeId": "me", "name": "Desk", "keyStorage": "file" },
                "devices": [{ "nodeId": "abc", "name": "Laptop", "pairedAt": 1_700_000_000_000_u64 }],
            })
        );
    }

    #[test]
    fn pairing_shown_serializes_with_and_without_a_qr_code() {
        let mut shown = PairingShown {
            code: "123456".to_owned(),
            expires_in_ms: 120_000,
            target: "abc".to_owned(),
            qr: Some(QrImage {
                size: 21,
                path: "M1 1h2v1h-2z".to_owned(),
            }),
        };

        let with_qr = serde_json::to_value(&shown).expect("serializable");
        shown.qr = None;
        let without_qr = serde_json::to_value(&shown).expect("serializable");

        assert_eq!(
            with_qr,
            json!({
                "code": "123456",
                "expiresInMs": 120_000,
                "target": "abc",
                "qr": { "size": 21, "path": "M1 1h2v1h-2z" },
            })
        );
        assert_eq!(without_qr["qr"], json!(null));
    }

    #[test]
    fn pairing_shown_debug_hides_the_code_and_target() {
        let shown = PairingShown {
            code: "123456".to_owned(),
            expires_in_ms: 120_000,
            target: "secret-node-id".to_owned(),
            qr: None,
        };

        let text = format!("{shown:?}");

        assert!(!text.contains("123456"), "{text}");
        assert!(!text.contains("secret-node-id"), "{text}");
    }

    #[test]
    fn found_device_serializes_a_missing_name_as_null() {
        let found = FoundDevice {
            node_id: "abc".to_owned(),
            name: None,
            paired: true,
        };

        let value = serde_json::to_value(found).expect("serializable");

        assert_eq!(value, json!({ "nodeId": "abc", "name": null, "paired": true }));
    }

    #[test]
    fn statuses_are_tagged_by_kind() {
        let statuses = [
            (
                DeviceStatus::Online {
                    name: "Laptop".to_owned(),
                },
                json!({ "kind": "online", "name": "Laptop" }),
            ),
            (DeviceStatus::Offline, json!({ "kind": "offline" })),
            (DeviceStatus::Refused, json!({ "kind": "refused" })),
        ];

        for (status, wire) in statuses {
            assert_eq!(serde_json::to_value(status).expect("serializable"), wire);
        }
    }

    #[test]
    fn events_are_tagged_by_kind_with_camel_case_fields() {
        let events = [
            (
                DevicesEvent::Paired { device: device() },
                json!({
                    "kind": "paired",
                    "device": { "nodeId": "abc", "name": "Laptop", "pairedAt": 1_700_000_000_000_u64 },
                }),
            ),
            (
                DevicesEvent::Unpaired {
                    node_id: "abc".to_owned(),
                },
                json!({ "kind": "unpaired", "nodeId": "abc" }),
            ),
            (
                DevicesEvent::PairingEnded {
                    reason: PairingEnd::TooManyAttempts,
                },
                json!({ "kind": "pairingEnded", "reason": "tooManyAttempts" }),
            ),
            (
                DevicesEvent::Resync { devices: vec![] },
                json!({ "kind": "resync", "devices": [] }),
            ),
        ];

        for (event, wire) in events {
            assert_eq!(serde_json::to_value(event).expect("serializable"), wire);
        }
    }
}
