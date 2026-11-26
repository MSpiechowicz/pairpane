//! Which agent `IdentityAgent` names, and how a server that is open under one `ssh_config` alias is reused by another.
//! Skipped, with a note, on a machine with no OpenSSH client (see [`ssh_program`](super::ssh_program)).

use std::io::ErrorKind;

use pairpane_core::model::ConnectOutcome;
use pretty_assertions::assert_eq;

use super::Harness;
use super::ssh_config::typed;
use super::ssh_connect::{PASSWORD, slashed};
use crate::test_server::{Settings, USER};

#[cfg(unix)]
#[test]
fn identity_agent_names_the_agent_and_the_one_of_the_environment_is_left_alone() {
    use super::connect::agent_with;
    use super::key_only;
    use super::ssh_connect::{alias_agent, signs_in};
    use crate::AgentSource;
    use crate::test_server::generate_key;

    let key = generate_key();
    let (_wanted_folder, wanted, _wanted_agent) = agent_with(&key);
    let (_other_folder, other, _other_agent) = agent_with(&generate_key());
    let (wanted_text, other_text) = (slashed(&wanted), slashed(&other));

    // The server takes `key`, which only the `wanted` agent holds. The pool's own agent (`SSH_AUTH_SOCK` of the app) is
    // the first of each pair.
    for (default, extra, expected) in [
        (&other, format!("  IdentityAgent {wanted_text}\n"), true),
        (&wanted, format!("  IdentityAgent {other_text}\n"), false),
        (&wanted, "  IdentityAgent SSH_AUTH_SOCK\n".to_owned(), true),
        (&wanted, String::new(), true),
        (&wanted, "  IdentityAgent none\n".to_owned(), false),
        (
            &wanted,
            "  IdentityAgent /nonexistent/pairpane/agent\n".to_owned(),
            false,
        ),
        (
            &wanted,
            "  IdentityAgent $PAIRPANE_NO_SUCH_VARIABLE\n".to_owned(),
            false,
        ),
    ] {
        let Some(h) = alias_agent(key_only(&key), AgentSource::Path(default.clone()), &extra) else {
            return;
        };

        assert_eq!(signs_in(&h, &extra), expected, "{extra}");
    }
}

const OTHER_NAME: &str =
    "this server is already open under another name with different ssh_config settings; disconnect it first";

/// A harness with an alias for each of `hosts` (name, `known_hosts` file, more lines), all leading to the one server.
fn aliases(hosts: &'static [(&'static str, &'static str, &'static str)]) -> Option<Harness> {
    Harness::via_ssh(Settings::default(), move |home, port| {
        hosts
            .iter()
            .map(|(name, known_hosts, more)| {
                format!(
                    "Host {name}\n  HostName 127.0.0.1\n  Port {port}\n  User {USER}\n  UserKnownHostsFile {}\n  \
                     GlobalKnownHostsFile {}\n  {more}\n",
                    slashed(&home.join(known_hosts)),
                    slashed(&home.join("system_known_hosts"))
                )
            })
            .collect()
    })
}

fn connect_as(h: &Harness, alias: &str) -> std::io::Result<ConnectOutcome> {
    h.connect_to(&typed(alias), Some(PASSWORD))
}

#[test]
fn a_second_alias_reuses_the_site_when_its_settings_are_the_same_or_looser_and_is_refused_when_stricter() {
    const HOSTS: &[(&str, &str, &str)] = &[
        ("files", ".ssh/known_hosts", ""),
        ("same", ".ssh/known_hosts", ""),
        ("strict", ".ssh/known_hosts", "StrictHostKeyChecking yes"),
        ("elsewhere", ".ssh/other_known_hosts", ""),
        ("batch", ".ssh/known_hosts", "BatchMode yes"),
        (
            "nopassword",
            ".ssh/known_hosts",
            "PasswordAuthentication no\n  KbdInteractiveAuthentication no",
        ),
        ("keysonly", ".ssh/known_hosts", "IdentitiesOnly yes"),
    ];
    let Some(h) = aliases(HOSTS) else {
        return;
    };
    let site = |outcome: ConnectOutcome| match outcome {
        ConnectOutcome::Connected { site, .. } => site,
        other => panic!("expected a connection, got {other:?}"),
    };

    h.trust();

    let first = site(connect_as(&h, "files").expect("connected"));

    // The same settings: the same site.
    assert_eq!(site(connect_as(&h, "same").expect("reused")), first);

    // Stricter or different ones are not answered with a site that was opened under the first alias's.
    for alias in ["strict", "elsewhere", "batch", "nopassword", "keysonly"] {
        let error = connect_as(&h, alias).expect_err(alias);

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{alias}");
        assert_eq!(error.to_string(), OTHER_NAME, "{alias}");
    }

    // The other way round: opened under the strict settings, a looser alias is served by it.
    let h = aliases(HOSTS).expect("ssh is there");

    h.trust();

    let strict = site(connect_as(&h, "strict").expect("connected"));

    assert_eq!(site(connect_as(&h, "files").expect("reused")), strict);
}
