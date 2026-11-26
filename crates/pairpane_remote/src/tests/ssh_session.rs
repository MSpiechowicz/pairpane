//! The configuration an `sftp` session gets: `ssh -G` is asked as `sftp` asks `ssh` (`-s <host> sftp`), so `Match
//! sessiontype subsystem` and `Match command sftp` blocks apply to a connection (a proxy, a stricter host key check,
//! other `known_hosts` files), and a `RemoteCommand` of the host does not get in the way. A block a shell session or
//! another command would get is not applied. Skipped, with a note, on a machine with no OpenSSH client, and for a
//! keyword an older OpenSSH does not have (it fails to read a file that has it, as it does for the user's own `ssh`).

use std::fs;
use std::io::ErrorKind;
use std::sync::atomic::Ordering;

use base64::Engine;
use pretty_assertions::assert_eq;

use super::ssh_config::Lab;
use super::ssh_connect::{alias, connected, files_target, slashed};
use super::{Harness, assert_fixed};
use crate::known_hosts::{Name, append};
use crate::test_server::{Settings, USER, generate_key};

const PROXY: &str = "ssh_config sends this host through a proxy, which Pairpane does not support; connect with ssh";

/// The ways a block can name an `sftp` session.
const SFTP: [&str; 2] = ["sessiontype subsystem", "command sftp"];

/// Whether the OpenSSH on this machine reads `Match <criteria>`; where it is one that names an `sftp` session, that it
/// applies it. An OpenSSH that does not know the keyword fails on the file (`InvalidInput`), and the test is left; one that
/// reads it and does not apply it to the session is a failure.
fn supported(criteria: &str) -> bool {
    let text = format!("Match {criteria}\n  Port 2200\nHost *\n  HostName files.example\n");
    let Some(lab) = Lab::new(&[("config", &text)]) else {
        return false;
    };

    match lab.plan("files") {
        Ok(planned) => {
            assert_eq!(planned.target.port == 2200, SFTP.contains(&criteria), "{criteria}");

            true
        }
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidInput, "{criteria}: {error}");
            eprintln!("skipped: this OpenSSH does not read `Match {criteria}`");

            false
        }
    }
}

/// A harness whose `files` host leads to the server, with `block` (lines of `ssh_config`, which may name `{home}`)
/// before the host's own, where it wins.
fn session(block: &str) -> Option<Harness> {
    Harness::via_ssh(Settings::default(), |home, port| {
        format!(
            "{block}Host files\n  HostName 127.0.0.1\n  Port {port}\n  User {USER}\n  GlobalKnownHostsFile {}\n  \
             UserKnownHostsFile {}\n",
            slashed(&home.join("system_known_hosts")),
            slashed(&home.join(".ssh").join("known_hosts"))
        )
        .replace("{home}", &slashed(home))
    })
}

#[test]
fn a_proxy_that_only_an_sftp_session_gets_refuses_the_host_and_one_for_other_sessions_does_not() {
    for criteria in SFTP {
        if !supported(criteria) {
            continue;
        }

        let Some(h) = session(&format!("Match {criteria}\n  ProxyJump bastion.invalid\n")) else {
            return;
        };

        h.trust();

        let error = h.connect_to(&files_target(), None).expect_err(criteria);

        assert_eq!(error.kind(), ErrorKind::Unsupported, "{criteria}");
        assert_eq!(error.to_string(), PROXY, "{criteria}");
        assert_fixed(&error, h.server.addr);
        assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0, "{criteria}");
    }

    // A shell, a command, another one: not this session.
    for criteria in ["sessiontype shell", "sessiontype exec", "command tmux"] {
        if !supported(criteria) {
            continue;
        }

        let Some(h) = session(&format!("Match {criteria}\n  ProxyJump bastion.invalid\n")) else {
            return;
        };

        h.trust();
        connected(&h);
    }
}

#[test]
fn a_strict_host_key_check_that_only_an_sftp_session_gets_refuses_a_key_that_is_not_known() {
    for criteria in SFTP {
        if !supported(criteria) {
            continue;
        }

        let Some(h) = session(&format!("Match {criteria}\n  StrictHostKeyChecking yes\n")) else {
            return;
        };
        let error = h.connect_to(&files_target(), None).expect_err(criteria);

        assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{criteria}");
        assert!(
            error.to_string().contains("StrictHostKeyChecking yes"),
            "{criteria}: {error}"
        );
        assert_fixed(&error, h.server.addr);
        assert_eq!(h.prompt.times(), 0, "{criteria}");
        assert!(!h.known_hosts().exists(), "{criteria}");
    }
}

#[test]
fn known_hosts_files_that_only_an_sftp_session_gets_are_the_ones_that_judge_the_key() {
    for criteria in SFTP {
        if !supported(criteria) {
            continue;
        }

        let Some(h) = session(&format!(
            "Match {criteria}\n  UserKnownHostsFile {{home}}/sftp_known_hosts\n"
        )) else {
            return;
        };
        let port = h.server.addr.port();
        let other = generate_key().public_key().to_bytes().expect("bytes");
        let there = h.home.path().join("sftp_known_hosts");

        // The default file has the key, and the file of the block has another one: refused, and not asked.
        h.trust();
        fs::write(
            &there,
            format!(
                "[127.0.0.1]:{port} ssh-ed25519 {}\n",
                base64::engine::general_purpose::STANDARD.encode(other)
            ),
        )
        .expect("written");

        let error = h.connect_to(&files_target(), None).expect_err(criteria);

        assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{criteria}");
        assert!(error.to_string().contains("known_hosts line 1"), "{criteria}: {error}");
        assert_fixed(&error, h.server.addr);
        assert_eq!(h.prompt.times(), 0, "{criteria}");

        // With the right key there, it connects without asking, and the default file is not the one that is written.
        let key = &h.server.host_key;

        fs::remove_file(&there).expect("removed");
        fs::remove_file(h.known_hosts()).expect("removed");
        append(
            &there,
            &Name::of("127.0.0.1", port).expect("a name"),
            key.algorithm().as_str(),
            &key.to_bytes().expect("bytes"),
            false,
        )
        .expect("trusted");
        connected(&h);
        assert_eq!(h.prompt.times(), 0, "{criteria}");
        assert!(!h.known_hosts().exists(), "{criteria}");
    }
}

#[test]
fn a_remote_command_in_the_configuration_does_not_stop_the_host_from_resolving() {
    let Some(h) = alias(Settings::default(), "  RemoteCommand tmux\n  RequestTTY yes\n") else {
        return;
    };

    connected(&h);
}
