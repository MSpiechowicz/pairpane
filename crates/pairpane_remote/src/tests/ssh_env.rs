//! What `ssh -G` is told and what it says back about files and the environment: the answer is the one the user's own
//! `ssh` gets (the app's environment, a `known_hosts` list that cannot be read in two ways at once). Skipped, with a note,
//! on a machine with no OpenSSH client (see [`ssh_program`](super::ssh_program)).

use std::io::ErrorKind;
use std::sync::atomic::Ordering;

use pretty_assertions::assert_eq;

use super::ssh_connect::{PASSWORD, alias, files_target};
use crate::known_hosts::{Name, append};
use crate::test_server::{Settings, generate_key};

const AMBIGUOUS: &str = "ssh_config names a known_hosts file Pairpane cannot place (a relative path or a path with spaces next to another file); connect with ssh";

#[test]
fn a_known_hosts_list_that_could_name_two_files_is_refused_and_not_guessed() {
    let Some(h) = alias(Settings::default(), "  UserKnownHostsFile {home}/kh rel_kh\n") else {
        return;
    };
    let other = generate_key();
    let name = Name::of("127.0.0.1", h.server.addr.port()).expect("a name");

    // `ssh` reads two files here, `kh` and `rel_kh`; and `kh` holds another key for the server, which stops the
    // connection. Joined into one path no file is found, and the key would be asked about.
    append(
        &h.home.path().join("kh"),
        &name,
        other.algorithm().as_str(),
        &other.public_key().to_bytes().expect("key bytes"),
        false,
    )
    .expect("another key is known");

    let error = h.connect_to(&files_target(), Some(PASSWORD)).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), AMBIGUOUS);
    assert_eq!(h.prompt.times(), 0);
    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0);
}

/// What `ssh -G -F config host` prints when the test's own process runs it (the user's own `ssh`).
#[cfg(unix)]
fn plain_ssh(config: &std::path::Path, host: &str) -> String {
    let output = std::process::Command::new(super::ssh_program().expect("ssh"))
        .args(["-G", "-F"])
        .arg(config)
        .args(["--", host])
        .output()
        .expect("ssh runs");

    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(unix)]
#[test]
fn the_configuration_is_read_with_the_environment_of_the_app_as_the_users_own_ssh_does() {
    use super::ssh_config::Lab;

    // Cargo sets this for a test, and not in the few variables a stripped environment would keep; a command that tests
    // a variable (as `Match exec` lines do for a VPN or a project) must see it, as it does for `ssh`.
    if std::env::var_os("CARGO_MANIFEST_DIR").is_none() {
        return;
    }

    for (variable, proxied) in [("CARGO_MANIFEST_DIR", true), ("PAIRPANE_NO_SUCH_VARIABLE", false)] {
        let Some(lab) = Lab::new(&[(
            "config",
            &format!("Match exec \"test -n \\\"${variable}\\\"\"\n  ProxyJump bastion\n"),
        )]) else {
            return;
        };

        assert_eq!(
            plain_ssh(&lab.path("config"), "web").contains("proxyjump bastion"),
            proxied,
            "{variable}: what ssh says"
        );
        assert_eq!(
            lab.plan("web").is_err(),
            proxied,
            "{variable}: what Pairpane makes of it"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_variable_of_the_environment_is_expanded_in_a_path_as_ssh_does() {
    use super::ssh_config::Lab;

    let Some(folder) = std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let Some(lab) = Lab::new(&[(
        "config",
        "Host files\n  HostName files.example\n  UserKnownHostsFile ${CARGO_MANIFEST_DIR}/kh_probe\n  \
         GlobalKnownHostsFile /dev/null\n",
    )]) else {
        return;
    };

    assert_eq!(lab.planned("files").known_hosts.user, [folder.join("kh_probe")]);
}
