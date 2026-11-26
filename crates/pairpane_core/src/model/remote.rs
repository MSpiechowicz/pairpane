use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies one connection to a server (a site). Monotonic within a run and never reused; a `number` in TypeScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SiteId(#[ts(type = "number")] pub u64);

/// Where to connect, as the Connect dialog collects it. Only what the user typed: a secret is sent apart from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectTarget {
    /// A host name, an address, or an alias of the user's `~/.ssh/config`.
    pub host: String,
    /// The port; `null` for the one the configuration gives, or 22.
    pub port: Option<u16>,
    /// The user name; `null` for the one the configuration gives, or the name of the local user.
    pub user: Option<String>,
    /// The private key file to try first; `null` for the configured and the default keys.
    pub key_file: Option<String>,
}

/// What kind of secret a server asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SecretKind {
    /// The password of the account.
    Password,
    /// The passphrase of an encrypted private key.
    Passphrase,
}

/// Why a kind of secret is asked for when it is not the one that was just answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SecretReason {
    /// The passphrase opened the key file, but the server did not accept the key: the password is asked for instead.
    KeyNotAccepted,
}

/// The answer of `remote_connect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ConnectOutcome {
    /// The connection is open.
    Connected {
        /// The connection, for the commands that follow.
        site: SiteId,
        /// What the path bar shows for the server (`user@host`).
        label: String,
        /// The folder the server starts in (the home folder), an absolute path.
        home: String,
    },
    /// A secret is needed to go on: ask for it and connect again with it.
    SecretNeeded {
        /// What to ask for.
        secret: SecretKind,
        /// A secret was given and the server did not accept it.
        wrong: bool,
        /// The other kind of secret that would also let the user in (a locked key and a password method are both
        /// possible), for a button that switches the question; `null` when only `secret` can.
        other: Option<SecretKind>,
        /// Why this kind is asked for, when the user just answered another one; `null` otherwise.
        reason: Option<SecretReason>,
    },
}

/// A host the user's `~/.ssh/config` names, for the suggestions of the Connect dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshHost {
    /// The name after `Host` that the user types.
    pub alias: String,
    /// The `HostName` it stands for; `null` when the alias is the name.
    pub host_name: Option<String>,
    /// The `User` it sets, if any.
    pub user: Option<String>,
    /// The `Port` it sets, if any.
    pub port: Option<u16>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn site_id_is_a_bare_number() {
        assert_eq!(serde_json::to_value(SiteId(7)).expect("serializable"), json!(7));
    }

    #[test]
    fn connect_target_serializes_in_camel_case() {
        let target = ConnectTarget {
            host: "files.example".to_owned(),
            port: Some(2222),
            user: None,
            key_file: Some("/home/me/.ssh/id_work".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(&target).expect("serializable"),
            json!({ "host": "files.example", "port": 2222, "user": null, "keyFile": "/home/me/.ssh/id_work" })
        );
        assert_eq!(
            serde_json::from_value::<ConnectTarget>(json!({ "host": "h", "port": null, "user": "u", "keyFile": null }))
                .expect("deserializable"),
            ConnectTarget {
                host: "h".to_owned(),
                port: None,
                user: Some("u".to_owned()),
                key_file: None,
            }
        );
    }

    #[test]
    fn connect_outcome_is_tagged_by_kind() {
        let connected = ConnectOutcome::Connected {
            site: SiteId(3),
            label: "me@files.example".to_owned(),
            home: "/home/me".to_owned(),
        };
        let needed = [
            (
                SecretKind::Password,
                "password",
                false,
                None,
                json!(null),
                None,
                json!(null),
            ),
            (
                SecretKind::Passphrase,
                "passphrase",
                true,
                Some(SecretKind::Password),
                json!("password"),
                None,
                json!(null),
            ),
            (
                SecretKind::Password,
                "password",
                false,
                None,
                json!(null),
                Some(SecretReason::KeyNotAccepted),
                json!("keyNotAccepted"),
            ),
        ];

        assert_eq!(
            serde_json::to_value(&connected).expect("serializable"),
            json!({ "kind": "connected", "site": 3, "label": "me@files.example", "home": "/home/me" })
        );

        for (secret, wire, wrong, other, other_wire, reason, reason_wire) in needed {
            assert_eq!(
                serde_json::to_value(ConnectOutcome::SecretNeeded {
                    secret,
                    wrong,
                    other,
                    reason
                })
                .expect("serializable"),
                json!({
                    "kind": "secretNeeded", "secret": wire, "wrong": wrong, "other": other_wire, "reason": reason_wire
                })
            );
        }
    }

    #[test]
    fn ssh_host_serializes_in_camel_case() {
        let host = SshHost {
            alias: "work".to_owned(),
            host_name: Some("work.example".to_owned()),
            user: None,
            port: Some(22),
        };

        assert_eq!(
            serde_json::to_value(&host).expect("serializable"),
            json!({ "alias": "work", "hostName": "work.example", "user": null, "port": 22 })
        );
    }
}
