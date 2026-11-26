//! Where a host leads, as the real OpenSSH says it (`ssh -G -F` a configuration written by the test): what is applied
//! of it, and which hosts are refused because OpenSSH would send them through a proxy. Tests that need the client are
//! skipped, with a note, on a machine that has none (see [`ssh_program`]).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use pairpane_core::model::ConnectTarget;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::ssh_program;
use crate::auth::KeyFile;
use crate::plan::{Plan, plan};
use crate::resolve::raw_lines;
use crate::verify::NewKey;
use crate::{AgentSource, PoolConfig, Timeouts};

const PROXY: &str = "ssh_config sends this host through a proxy, which Pairpane does not support; connect with ssh";

/// What `ssh -G` says about `host` of the file `config` that a refusal is about (the files that are read, and the keys).
pub(super) fn says(program: Option<&Path>, config: Option<&Path>, host: &str) -> String {
    let (Some(program), Some(config)) = (program, config) else {
        return "no ssh to ask".to_owned();
    };

    format!(
        "ssh -G {host} says:\n{}",
        raw_lines(
            program,
            config,
            host,
            &["userknownhostsfile", "globalknownhostsfile", "identityfile"]
        )
    )
}

/// Files in a temporary folder (the first is the configuration given to `ssh -G -F`) and a pool configuration that asks
/// the real OpenSSH about them. `{dir}` in a file is the folder, in the slash form that `ssh_config` reads on every
/// system.
pub(super) struct Lab {
    pub(super) dir: TempDir,
    pub(super) config: PoolConfig,
}

impl Lab {
    pub(super) fn new(files: &[(&str, &str)]) -> Option<Self> {
        let program = ssh_program()?;
        let dir = tempfile::tempdir().expect("temp dir");
        let slashed = dir.path().to_string_lossy().replace('\\', "/");

        for (name, text) in files {
            let path = dir.path().join(name);

            fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
            fs::write(path, text.replace("{dir}", &slashed)).expect("written");
        }

        let config = PoolConfig {
            home: dir.path().join("home"),
            system_known_hosts: None,
            ssh_program: Some(program),
            search_ssh: false,
            ssh_config_file: Some(dir.path().join(files[0].0)),
            agent: AgentSource::None,
            default_user: Some("me".to_owned()),
            timeouts: Timeouts::default(),
        };

        Some(Self { dir, config })
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub(super) fn plan(&self, host: &str) -> std::io::Result<Plan> {
        plan(&self.config, &typed(host))
    }

    /// The plan for `host`, which is expected to work: where it is refused, the failure says what `ssh -G` printed.
    pub(super) fn planned(&self, host: &str) -> Plan {
        self.plan(host).unwrap_or_else(|error| {
            panic!(
                "{host} is refused: {error}\n{}",
                says(
                    self.config.ssh_program.as_deref(),
                    self.config.ssh_config_file.as_deref(),
                    host
                )
            )
        })
    }

    /// Why `host` is refused as invalid, before OpenSSH is asked.
    fn refused(&self, host: &str) -> String {
        let error = self.plan(host).expect_err(host);

        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{host}");

        error.to_string()
    }

    /// Whether OpenSSH sends `host` through a proxy, which is refused; any other failure is a mistake of the test.
    fn proxied(&self, host: &str) -> bool {
        match self.plan(host) {
            Ok(_) => false,
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::Unsupported, "{host}: {error}");
                assert_eq!(error.to_string(), PROXY);

                true
            }
        }
    }
}

pub(super) fn typed(host: &str) -> ConnectTarget {
    ConnectTarget {
        host: host.to_owned(),
        port: None,
        user: None,
        key_file: None,
    }
}

#[test]
fn what_the_configuration_says_is_applied_and_what_is_typed_wins() {
    let Some(lab) = Lab::new(&[(
        "config",
        "Host files\n  HostName files.example\n  User bob\n  Port 2222\n  IdentityFile {dir}/k1\n  \
         IdentityFile ~/k2\n  IdentityFile %d/k3\n  IdentityFile %d/%r-%h-%p-%n-100%%\n  IdentityFile ~/%u\n  \
         IdentityFile none\nHost *\n  User everyone\n",
    )]) else {
        return;
    };
    let home = &lab.config.home;
    let planned = lab.planned("files");

    assert_eq!(planned.target.host, "files.example");
    assert_eq!(planned.target.port, 2222);
    assert_eq!(planned.target.user, "bob");
    assert_eq!(planned.label, "bob@files:2222");
    let named = |path: PathBuf| KeyFile { path, named: true };

    // `none` is no file, the tokens are filled in, and the files the configuration names are all there are: OpenSSH
    // lists the default names only when it names none.
    assert_eq!(
        planned.target.key_files,
        [
            named(lab.path("k1")),
            named(home.join("k2")),
            named(home.join("k3")),
            named(home.join("bob-files.example-2222-files-100%")),
            named(home.join("me")),
        ]
    );

    // What the dialog has typed is what `ssh -l -p` would have said.
    let overridden = plan(
        &lab.config,
        &ConnectTarget {
            host: "files".to_owned(),
            port: Some(2200),
            user: Some("typed".to_owned()),
            key_file: None,
        },
    )
    .expect("planned");

    assert_eq!(overridden.target.port, 2200);
    assert_eq!(overridden.target.user, "typed");
    assert_eq!(overridden.label, "typed@files:2200");

    let other = lab.planned("elsewhere");

    assert_eq!(other.target.host, "elsewhere");
    assert_eq!(other.target.port, 22);
    assert_eq!(other.target.user, "everyone");
    assert_eq!(other.label, "everyone@elsewhere");

    // A host with no `IdentityFile` gets the default names OpenSSH lists, which are defaults and not asked for.
    let defaults = &other.target.key_files;

    assert!(defaults.iter().all(|file| !file.named), "{defaults:?}");
    assert!(
        defaults
            .iter()
            .all(|file| file.path.parent() == Some(home.join(".ssh").as_path()))
    );
    assert!(
        defaults
            .iter()
            .any(|file| file.path == home.join(".ssh").join("id_ed25519"))
    );
}

#[test]
fn the_known_hosts_files_and_the_alias_are_the_ones_ssh_names() {
    let Some(lab) = Lab::new(&[(
        "config",
        "Host files\n  UserKnownHostsFile {dir}/kh1 {dir}/kh2\n  GlobalKnownHostsFile {dir}/global\n  \
         HostKeyAlias Vault.Example\n  StrictHostKeyChecking yes\n\
         Host plain\n  UserKnownHostsFile /dev/null\n  GlobalKnownHostsFile {dir}/global\n  Port 2200\n\
         Host loose\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {dir}/kh1\n\
         Host spaced\n  UserKnownHostsFile {dir}/with space/kh\n  GlobalKnownHostsFile {dir}/global\n",
    )]) else {
        return;
    };
    let files = lab.planned("files");

    assert_eq!(files.known_hosts.user, [lab.path("kh1"), lab.path("kh2")]);
    assert_eq!(files.known_hosts.global, [lab.path("global")]);
    assert_eq!(files.target.known_as.as_str(), "vault.example");
    assert_eq!(files.new_key, NewKey::Strict);

    let plain = lab.planned("plain");

    assert!(plain.known_hosts.user.is_empty(), "no file to write to");
    assert_eq!(plain.target.known_as.as_str(), "[plain]:2200");
    assert_eq!(plain.new_key, NewKey::Ask);

    assert_eq!(
        lab.planned("loose").new_key,
        NewKey::Ask,
        "ask first, as for any other value"
    );

    // The output does not quote a path with a space in it: its words are joined again.
    assert_eq!(lab.planned("spaced").known_hosts.user, [lab.path("with space/kh")]);
}

#[test]
fn a_proxy_that_applies_refuses_the_host_and_none_does_not() {
    let Some(lab) = Lab::new(&[(
        "config",
        "Host jump\n  ProxyJump bastion\nHost command\n  ProxyCommand nc %h %p\n\
         Host none\n  ProxyJump none\n  ProxyCommand none\n\
         Host early\n  ProxyJump none\nHost *\n  ProxyJump bastion\n",
    )]) else {
        return;
    };

    assert!(lab.proxied("jump"));
    assert!(lab.proxied("command"));
    assert!(!lab.proxied("none"));
    assert!(!lab.proxied("early"));
    assert!(lab.proxied("elsewhere"));
}

/// A configuration (name, files, the host asked about, whether it is sent through a proxy).
type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str, bool);

/// The configurations of the security review that a parser of our own got wrong: for each, what OpenSSH does with the
/// host (it was asked with `ssh -G` as well).
#[test]
fn the_constructs_that_a_parser_of_our_own_got_wrong_are_openssh_s_to_answer() {
    const BASTION: &str = "Host *\n  ProxyJump bastion\n";

    let cases: [Case; 16] = [
        (
            "an Include in a Host block applies to that host only",
            &[
                ("config", "Host web\n  Include {dir}/inc/p.conf\n"),
                ("inc/p.conf", BASTION),
            ],
            "web",
            true,
        ),
        (
            "an Include in a Host block, for another host",
            &[
                ("config", "Host web\n  Include {dir}/inc/p.conf\n"),
                ("inc/p.conf", BASTION),
            ],
            "other",
            false,
        ),
        (
            "Match final reads the file again for the HostName",
            &[(
                "config",
                "Host web\n  HostName web.internal\nHost *.internal\n  ProxyJump bastion\nMatch final\n  User x\n",
            )],
            "web",
            true,
        ),
        (
            "Match final and a proxy command of the HostName",
            &[(
                "config",
                "Host web\n  HostName web.internal\nHost web.internal\n  ProxyCommand nc %h %p\nMatch final all\n  User x\n",
            )],
            "web",
            true,
        ),
        (
            "Match final with an Include in a Host block",
            &[
                (
                    "config",
                    "Host web\n  HostName web.internal\nHost *.internal\n  Include {dir}/inc/p.conf\nMatch final\n  User x\n",
                ),
                ("inc/p.conf", BASTION),
            ],
            "web",
            true,
        ),
        (
            "an Include by a glob of files",
            &[("config", "Include {dir}/inc/*.conf\n"), ("inc/a.conf", BASTION)],
            "web",
            true,
        ),
        (
            "an Include by a glob of folders",
            &[("config", "Include {dir}/inc/*/p.conf\n"), ("inc/sub/p.conf", BASTION)],
            "web",
            true,
        ),
        (
            "an Include with a bracket",
            &[("config", "Include {dir}/inc/[a].conf\n"), ("inc/a.conf", BASTION)],
            "web",
            true,
        ),
        (
            "a quoted Include with a space in the path",
            &[
                ("config", "Include \"{dir}/inc/sp ace/p.conf\"\n"),
                ("inc/sp ace/p.conf", BASTION),
            ],
            "web",
            true,
        ),
        (
            "the options after an Include belong to the top of the file",
            &[
                ("config", "Include {dir}/inc/b.part\nProxyJump bastion\n"),
                ("inc/b.part", "Host foo\n  User x\n"),
            ],
            "web",
            true,
        ),
        (
            "Host patterns are case sensitive",
            &[("config", "Host Web\n  ProxyJump none\nHost *\n  ProxyJump bastion\n")],
            "web",
            true,
        ),
        (
            "Host patterns are case sensitive, the other way",
            &[("config", "Host Web\n  ProxyJump none\nHost *\n  ProxyJump bastion\n")],
            "Web",
            false,
        ),
        (
            "a negated pattern",
            &[("config", "Host * !Web\n  ProxyJump bastion\n")],
            "Web",
            false,
        ),
        (
            "a comma does not separate Host patterns",
            &[("config", "Host a,web\n  ProxyJump bastion\n")],
            "web",
            false,
        ),
        (
            "a quoted none is a command of that name",
            &[("config", "Host *\n  ProxyCommand \"none\"\n")],
            "web",
            true,
        ),
        (
            "the host of a Match ignores case, unlike a Host line",
            &[("config", "Match host !Web,*\n  ProxyJump bastion\n")],
            "web",
            false,
        ),
    ];

    for (name, files, host, proxied) in cases {
        let Some(lab) = Lab::new(files) else {
            return;
        };

        assert_eq!(lab.proxied(host), proxied, "{name}");
    }
}

/// The system's `ssh_config` on a systemd machine (`20-systemd-ssh-proxy.conf`), as `-F` gets it.
const SYSTEMD: &str = "Host .host machine/.host\n  ProxyCommand /usr/lib/systemd/systemd-ssh-proxy unix/run/ssh-unix-local/socket %p\n  ProxyUseFdpass yes\n  CheckHostIP no\n\
Host unix/* unix%* vsock/* vsock%* vsock-mux/* vsock-mux%* machine/* machine%*\n  User root\n  ProxyCommand /usr/lib/systemd/systemd-ssh-proxy %h %p\n  ProxyUseFdpass yes\n  CheckHostIP no\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n";

#[test]
fn an_ordinary_host_resolves_and_the_names_of_the_systemd_proxy_and_options_are_refused() {
    let Some(lab) = Lab::new(&[("config", SYSTEMD)]) else {
        return;
    };

    assert_eq!(lab.planned("files.example").target.host, "files.example");

    // These go through the proxy command of the file, and are refused for it.
    for host in [".host", "unix%run", "machine%web", "vsock%3"] {
        assert!(lab.proxied(host), "{host}");
    }

    // A slash is not a character of a host name here (so `machine/foo` never gets as far as the proxy), and a name that
    // starts with `-` could be taken for an option.
    for host in [
        "machine/foo",
        "unix/run/x",
        "vsock/3",
        "-oProxyCommand=x",
        "-G",
        "--",
        "-",
    ] {
        assert_eq!(lab.refused(host), "the host name is not valid", "{host}");
    }
}

#[test]
fn a_configuration_that_ssh_cannot_read_refuses_the_host() {
    let Some(lab) = Lab::new(&[("config", "Host *\n  User x\n")]) else {
        return;
    };
    let mut config = lab.config.clone();

    config.ssh_config_file = Some(Path::new("/nonexistent/pairpane/ssh_config").to_owned());

    let error = plan(&config, &typed("files")).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "ssh could not read its configuration for this host");

    let mut broken = lab.config.clone();

    fs::write(lab.path("bad"), "Port nonsense\n").expect("written");
    broken.ssh_config_file = Some(lab.path("bad"));

    assert_eq!(
        plan(&broken, &typed("files")).expect_err("refused").to_string(),
        "ssh could not read its configuration for this host"
    );
}

#[test]
fn without_a_client_the_host_is_used_as_typed_with_the_default_files() {
    let home = tempfile::tempdir().expect("temp dir");
    let config = PoolConfig {
        home: home.path().to_owned(),
        system_known_hosts: Some(home.path().join("system_known_hosts")),
        ssh_program: None,
        search_ssh: false,
        ssh_config_file: None,
        agent: AgentSource::None,
        default_user: Some("me".to_owned()),
        timeouts: Timeouts::default(),
    };
    let planned = plan(&config, &typed("files.example")).expect("planned");

    assert_eq!(planned.target.host, "files.example");
    assert_eq!(planned.target.user, "me");
    assert_eq!(planned.target.port, 22);
    assert_eq!(planned.known_hosts.user, [home.path().join(".ssh").join("known_hosts")]);
    assert_eq!(planned.known_hosts.global, [home.path().join("system_known_hosts")]);
    assert_eq!(planned.new_key, NewKey::Ask);
    assert_eq!(planned.target.known_as.as_str(), "files.example");

    let broken = PoolConfig {
        ssh_program: Some(home.path().join("no-such-ssh")),
        ..config
    };

    assert_eq!(
        plan(&broken, &typed("files.example"))
            .expect_err("not started")
            .to_string(),
        "ssh could not be started"
    );
}
