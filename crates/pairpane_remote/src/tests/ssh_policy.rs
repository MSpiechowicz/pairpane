//! What the real OpenSSH says besides where a host leads (`ssh -G -F` a configuration of the test): the host keys that
//! only others can judge, the ways of signing in that are switched off, `HashKnownHosts`, where a key is saved, a known
//! hosts path with a space in it, and a machine with no OpenSSH. Skipped, with a note, on a machine with no OpenSSH
//! client (see [`ssh_program`](super::ssh_program)).

use std::fs;
use std::io::ErrorKind;
use std::sync::atomic::Ordering;

use pairpane_core::model::{ConnectOutcome, SecretKind};
use pretty_assertions::assert_eq;

use super::ssh_config::{Lab, typed};
use super::ssh_connect::{PASSWORD, alias, connected, files_target, refused_with, signs_in};
use super::{Harness, assert_fixed, needed};
use crate::known_hosts::{KnownHosts, Name, Verdict, append};
use crate::plan::plan;
use crate::test_server::{Settings, generate_key, write_key};

const KEYS_BY_OTHERS: &str =
    "ssh_config names revoked host keys or a known-hosts command, which Pairpane does not support; connect with ssh";

const NO_SSH: &str = "OpenSSH was not found, so ~/.ssh/config cannot be read; install OpenSSH (ssh) to connect";

const NO_WAY_IN: &str = "ssh_config allows no way of signing in that Pairpane supports";

const NO_WAY: &str = "the server did not accept any way of signing in";

/// A server that takes a password and a key.
fn both(key: &russh::keys::PrivateKey) -> Settings {
    Settings {
        authorized_key: Some(key.public_key().clone()),
        ..Settings::default()
    }
}

#[test]
fn host_keys_that_a_file_or_a_program_judge_refuse_the_host_before_anything_is_sent() {
    for extra in [
        "  RevokedHostKeys {home}/revoked\n",
        "  KnownHostsCommand /bin/echo %H\n",
    ] {
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };

        // The revoked file lists the very key the server has, and the key is known as well.
        fs::write(
            h.home.path().join("revoked"),
            format!("{}\n", h.server.host_key.to_openssh().expect("key text")),
        )
        .expect("written");
        h.trust();

        let error = h.connect_to(&files_target(), Some(PASSWORD)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::Unsupported, "{extra}");
        assert_eq!(error.to_string(), KEYS_BY_OTHERS, "{extra}");
        assert_fixed(&error, h.server.addr);
        assert_eq!(h.prompt.times(), 0, "{extra}");
        assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0, "{extra}");
        assert_eq!(h.server.counts.offered(), Vec::<String>::new(), "{extra}");
    }

    // `none` is what the defaults say.
    let Some(h) = alias(
        Settings::default(),
        "  RevokedHostKeys none\n  KnownHostsCommand none\n",
    ) else {
        return;
    };

    h.trust();
    connected(&h);
}

#[test]
fn a_known_hosts_path_with_a_space_in_it_is_found_and_written() {
    let Some(h) = alias(
        Settings::default(),
        "  UserKnownHostsFile \"{home}/my home/.ssh/known_hosts\"\n",
    ) else {
        return;
    };
    let spaced = h.home.path().join("my home").join(".ssh").join("known_hosts");
    let key = &h.server.host_key;
    let name = Name::of("127.0.0.1", h.server.addr.port()).expect("a name");

    // Known there: no question.
    append(
        &spaced,
        &name,
        key.algorithm().as_str(),
        &key.to_bytes().expect("bytes"),
        false,
    )
    .expect("trusted");
    connected(&h);
    assert_eq!(h.prompt.times(), 0);

    // A key that is not there is asked about and written to that file, where its folder is there.
    let h = alias(
        Settings::default(),
        "  UserKnownHostsFile \"{home}/my home/.ssh/known_hosts\"\n",
    )
    .expect("ssh is there");
    let folder = h.home.path().join("my home").join(".ssh");

    fs::create_dir_all(&folder).expect("the folder");
    connected(&h);
    assert_eq!(h.prompt.times(), 1);
    assert!(h.prompt.asked.lock().expect("asked")[0].saved);
    assert!(
        fs::read_to_string(folder.join("known_hosts"))
            .expect("written")
            .contains("ssh-ed25519 ")
    );
    assert!(!h.known_hosts().exists());
}

#[test]
fn a_guessed_known_hosts_path_is_not_made_for_a_key_that_is_trusted_for_the_session_only() {
    let Some(h) = alias(
        Settings::default(),
        "  UserKnownHostsFile \"{home}/my home/.ssh/known_hosts\"\n",
    ) else {
        return;
    };

    // `<home>/my` is not there, so `my home` is a guess: no folder is made for it, and the question says so.
    connected(&h);
    assert_eq!(h.prompt.times(), 1);
    assert!(!h.prompt.asked.lock().expect("asked")[0].saved);
    assert!(!h.home.path().join("my home").exists());
    assert!(!h.home.path().join("my").exists());
}

#[test]
fn the_default_known_hosts_files_are_absolute_whatever_form_ssh_prints_them_in() {
    let Some(lab) = Lab::new(&[("config", "Host files\n  HostName files.example\n")]) else {
        return;
    };
    let planned = lab.plan("files").expect("planned");

    assert!(planned.known_hosts.user[0].is_absolute(), "{:?}", planned.known_hosts);
    assert!(planned.known_hosts.user[0].ends_with(".ssh/known_hosts"));
    assert!(planned.known_hosts.global.iter().all(|path| path.is_absolute()));
}

#[test]
fn more_known_hosts_files_than_the_limit_refuse_the_host() {
    let files: String = (0..9).map(|n| format!("{{dir}}/kh{n} ")).collect();
    let Some(lab) = Lab::new(&[("config", &format!("Host files\n  UserKnownHostsFile {files}\n"))]) else {
        return;
    };
    let error = lab.plan("files").expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(
        error.to_string(),
        "ssh_config names too many known_hosts files; connect with ssh"
    );
}

#[test]
fn a_host_name_that_cannot_be_used_is_refused_with_or_without_a_host_key_alias() {
    for text in [
        "Host files\n  HostName a/b\n  HostKeyAlias vault\n",
        "Host files\n  HostName a/b\n",
        "Host files\n  HostName a*b\n  HostKeyAlias vault\n",
    ] {
        let Some(lab) = Lab::new(&[("config", text)]) else {
            return;
        };
        let error = lab.plan("files").expect_err(text);

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{text}");
        assert_eq!(error.to_string(), "the host name is not valid", "{text}");
    }
}

#[test]
fn a_user_name_that_openssh_would_not_take_is_refused_from_the_dialog_and_from_the_configuration() {
    let Some(lab) = Lab::new(&[("config", "Host files\n  HostName files.example\nHost bad\n  User a$b\n")]) else {
        return;
    };

    for user in [
        "a b", "-oProxy", "a'b", "a`b", "a\"b", "a$b", "a\\b", "a;b", "a&b", "a<b", "a>b", "a|b", "a(b", "a)b", "a{b",
        "a}b", "a\tb", "a\u{1}b", "a@b",
    ] {
        let mut target = typed("files");

        target.user = Some(user.to_owned());

        let error = plan(&lab.config, &target).expect_err(user);

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{user:?}");
        assert_eq!(error.to_string(), "the user name is not valid", "{user:?}");
    }

    let mut fine = typed("files");

    fine.user = Some("alice.b-c_d".to_owned());

    assert_eq!(plan(&lab.config, &fine).expect("planned").target.user, "alice.b-c_d");
    assert_eq!(
        lab.plan("bad").expect_err("configured").to_string(),
        "the user name is not valid"
    );
}

#[test]
fn hash_known_hosts_saves_a_hashed_name_that_is_then_known() {
    let Some(h) = alias(Settings::default(), "  HashKnownHosts yes\n") else {
        return;
    };
    let port = h.server.addr.port();

    connected(&h);

    let saved = fs::read_to_string(h.known_hosts()).expect("file");
    let line = saved.lines().next().expect("a line");

    assert_eq!(saved.lines().count(), 1);
    assert!(line.starts_with("|1|"), "{line}");
    assert!(!saved.contains("127.0.0.1"), "{saved}");

    // Found again by its hash: no second question.
    let key = &h.server.host_key;
    let known = KnownHosts::load(&h.known_hosts()).expect("read");
    let name = Name::of("127.0.0.1", port).expect("a name");

    assert_eq!(
        known.check(&name, key.algorithm().as_str(), &key.to_bytes().expect("bytes")),
        Verdict::Known
    );
    connected(&h);
    assert_eq!(h.prompt.times(), 1);
}

#[test]
fn the_question_says_whether_the_key_is_saved() {
    // Saved to the file of the configuration.
    let Some(h) = alias(Settings::default(), "") else {
        return;
    };

    connected(&h);
    assert!(h.prompt.asked.lock().expect("asked")[0].saved);

    // No file to save to: it is trusted for this run, and said so. It is not asked about again.
    for extra in ["  UserKnownHostsFile none\n", "  UserKnownHostsFile /dev/null\n"] {
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };

        connected(&h);
        assert!(!h.prompt.asked.lock().expect("asked")[0].saved, "{extra}");
        assert!(!h.known_hosts().exists(), "{extra}");

        connected(&h);
        assert_eq!(h.prompt.times(), 1, "{extra}");
    }
}

#[test]
fn a_way_of_signing_in_that_the_configuration_switches_off_is_not_tried_or_asked_for() {
    // Control: a server that takes a password asks for one.
    let Some(h) = alias(Settings::default(), "") else {
        return;
    };

    h.trust();
    assert_eq!(
        h.connect_to(&files_target(), None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );

    // Keys are all that is left, and the server has none that fits: no password is offered.
    for extra in [
        "  PasswordAuthentication no\n  KbdInteractiveAuthentication no\n",
        "  PreferredAuthentications publickey\n",
    ] {
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };

        refused_with(&h, ErrorKind::PermissionDenied, NO_WAY, extra);
        assert_eq!(h.server.counts.offered(), Vec::<String>::new(), "{extra}");
    }
}

#[test]
fn a_configuration_with_no_way_of_signing_in_that_is_supported_says_so_before_connecting() {
    for extra in [
        "  PreferredAuthentications gssapi-with-mic,hostbased\n",
        "  PubkeyAuthentication no\n  PasswordAuthentication no\n  KbdInteractiveAuthentication no\n",
        "  PreferredAuthentications publickey\n  PubkeyAuthentication no\n",
    ] {
        let Some(h) = alias(Settings::default(), extra) else {
            return;
        };

        refused_with(&h, ErrorKind::Unsupported, NO_WAY_IN, extra);
        assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0, "{extra}");
    }
}

#[test]
fn a_password_that_is_switched_off_is_never_sent_but_the_other_password_method_is() {
    let Some(h) = alias(
        Settings {
            keyboard_interactive: true,
            ..Settings::default()
        },
        "  PasswordAuthentication no\n",
    ) else {
        return;
    };

    h.trust();

    // The password is only offered as the answer to the hidden prompt, and still asked for.
    assert_eq!(
        h.connect_to(&files_target(), None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
    connected(&h);
}

#[test]
fn public_keys_that_are_switched_off_are_not_offered_and_their_passphrase_is_not_asked() {
    let key = generate_key();
    let Some(h) = alias(both(&key), "  PubkeyAuthentication no\n") else {
        return;
    };

    h.trust();
    write_key(&key, Some("open sesame"), &h.ssh("id_ed25519"));

    // The locked key is not looked at: the password is what is asked for, with nothing else on offer.
    assert_eq!(
        h.connect_to(&files_target(), None).expect("answered"),
        needed(SecretKind::Password, false, None)
    );
    connected(&h);
}

#[test]
fn a_locked_key_is_asked_for_alone_where_the_password_methods_are_off() {
    let key = generate_key();
    let Some(h) = alias(
        both(&key),
        "  PasswordAuthentication no\n  KbdInteractiveAuthentication no\n",
    ) else {
        return;
    };

    h.trust();
    write_key(&key, Some("open sesame"), &h.ssh("id_ed25519"));

    assert_eq!(
        h.connect_to(&files_target(), None).expect("answered"),
        needed(SecretKind::Passphrase, false, None)
    );
}

#[test]
fn the_preferred_methods_are_tried_in_their_order() {
    let key = generate_key();

    // The password first: it is offered although the key would have signed in.
    let Some(h) = alias(
        both(&key),
        "  PreferredAuthentications password,publickey\n  IdentityFile {home}/k\n",
    ) else {
        return;
    };

    h.trust();
    write_key(&key, None, &h.home.path().join("k"));
    connected(&h);
    assert_eq!(h.server.counts.offered(), [PASSWORD]);

    // The usual order: the key first, and no password is offered.
    let Some(h) = alias(both(&key), "  IdentityFile {home}/k\n") else {
        return;
    };

    write_key(&key, None, &h.home.path().join("k"));
    assert!(signs_in(&h, "the usual order"));
    assert_eq!(h.server.counts.offered(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn the_agent_is_left_out_with_identities_only_or_no_identity_agent() {
    use super::key_only;
    use super::ssh_connect::alias_agent;
    use crate::AgentSource;
    use crate::tests::connect::agent_with;

    let key = generate_key();
    let (_folder, socket, _agent) = agent_with(&key);

    for (extra, expected) in [
        ("", true),
        ("  IdentitiesOnly yes\n", false),
        ("  IdentityAgent none\n", false),
    ] {
        let Some(h) = alias_agent(key_only(&key), AgentSource::Path(socket.clone()), extra) else {
            return;
        };

        assert_eq!(signs_in(&h, extra), expected, "{extra}");
    }
}

#[test]
fn without_openssh_a_user_configuration_refuses_every_connect_and_a_comment_does_not() {
    let h = Harness::new(Settings::default());

    h.trust();

    // No file: as typed, as before.
    assert!(matches!(
        h.connect(Some(PASSWORD)).expect("answered"),
        ConnectOutcome::Connected { .. }
    ));

    for text in ["Host x\n  User y\n", "# note\n\nInclude other\n", "Host *\n"] {
        let h = Harness::new(Settings::default());

        fs::write(h.ssh("config"), text).expect("config");
        h.trust();

        let error = h.connect(Some(PASSWORD)).expect_err("refused");

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{text:?}");
        assert_eq!(error.to_string(), NO_SSH, "{text:?}");
        assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0);
    }

    for text in ["", "\n\n  \n", "# only a comment\n   # another\n"] {
        let h = Harness::new(Settings::default());

        fs::write(h.ssh("config"), text).expect("config");
        h.trust();
        assert!(
            matches!(
                h.connect(Some(PASSWORD)).expect("answered"),
                ConnectOutcome::Connected { .. }
            ),
            "{text:?}"
        );
    }

    // A configuration that cannot be read is not taken for none.
    let h = Harness::new(Settings::default());

    fs::create_dir(h.ssh("config")).expect("a folder in its place");
    assert_eq!(h.connect(Some(PASSWORD)).expect_err("refused").to_string(), NO_SSH);
}

#[test]
fn an_openssh_that_is_installed_later_is_found_at_the_next_connect() {
    let Some(mut lab) = Lab::new(&[("config", "Host files\n  HostName files.example\n  Port 2200\n")]) else {
        return;
    };

    // No client yet, and the pool is to look again: it finds the one that is there.
    lab.config.ssh_program = None;
    lab.config.search_ssh = true;

    let planned = plan(&lab.config, &typed("files")).expect("planned");

    assert_eq!(
        (planned.target.host.as_str(), planned.target.port),
        ("files.example", 2200)
    );

    // Not to look: used as typed.
    lab.config.search_ssh = false;

    let planned = plan(&lab.config, &typed("files")).expect("planned");

    assert_eq!((planned.target.host.as_str(), planned.target.port), ("files", 22));
}
