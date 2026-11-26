//! What `ssh -G` prints: one `key value` line per option, the keys in lower case. Only the few that browsing needs are
//! read, the rest are ignored.

use std::io;

use crate::allowed::AuthOptions;
use crate::error::fail;

/// What `ssh -G` says about a host, as printed (nothing is expanded or checked yet).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Answer {
    pub(super) host_name: Option<String>,
    pub(super) user: Option<String>,
    pub(super) port: Option<u16>,
    /// The `IdentityFile` values: `~` and `%` tokens are still in them.
    pub(super) identity_files: Vec<String>,
    /// The words of `UserKnownHostsFile` and `GlobalKnownHostsFile` (a path with a space in it is two words here: the
    /// output does not quote).
    pub(super) user_known_hosts: Vec<String>,
    pub(super) global_known_hosts: Vec<String>,
    pub(super) host_key_alias: Option<String>,
    /// The value of `StrictHostKeyChecking`, in lower case.
    pub(super) strict: Option<String>,
    /// `ProxyJump` or `ProxyCommand` is set to something but `none`.
    pub(super) proxied: bool,
    /// `RevokedHostKeys` or `KnownHostsCommand` is set to something but `none`: host keys are judged by a file or a
    /// program that is not read here.
    pub(super) keys_by_others: bool,
    /// `HashKnownHosts yes`: a key that is saved is saved under a hashed name.
    pub(super) hash_known_hosts: bool,
    /// How the user may sign in, as the configuration says.
    pub(super) auth: AuthOptions,
}

fn unusable() -> io::Error {
    fail(
        io::ErrorKind::InvalidInput,
        "ssh gave a value that Pairpane cannot use for this host",
    )
}

/// A yes/no option as `ssh -G` prints it (`yes` and `true`; anything else is no).
fn is_yes(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "yes" | "true")
}

pub(super) fn parse(output: &[u8]) -> io::Result<Answer> {
    let text = std::str::from_utf8(output)
        .map_err(|_| fail(io::ErrorKind::InvalidData, "ssh gave an answer that cannot be read"))?;
    let mut answer = Answer::default();

    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };

        // The keys that are not read may hold anything.
        let read = matches!(
            key,
            "hostname"
                | "user"
                | "port"
                | "identityfile"
                | "userknownhostsfile"
                | "globalknownhostsfile"
                | "hostkeyalias"
                | "stricthostkeychecking"
                | "proxyjump"
                | "proxycommand"
                | "revokedhostkeys"
                | "knownhostscommand"
                | "hashknownhosts"
                | "identitiesonly"
                | "identityagent"
                | "pubkeyauthentication"
                | "passwordauthentication"
                | "kbdinteractiveauthentication"
                | "preferredauthentications"
                | "batchmode"
                | "numberofpasswordprompts"
        );

        if !read {
            continue;
        }

        if value.chars().any(char::is_control) {
            return Err(unusable());
        }

        match key {
            "hostname" => answer.host_name = Some(value.to_owned()),
            "user" => answer.user = Some(value.to_owned()),
            "port" => answer.port = Some(value.parse().map_err(|_| unusable())?),
            "identityfile" => answer.identity_files.push(value.to_owned()),
            "userknownhostsfile" => answer.user_known_hosts = words(value),
            "globalknownhostsfile" => answer.global_known_hosts = words(value),
            "hostkeyalias" => answer.host_key_alias = Some(value.to_owned()),
            "stricthostkeychecking" => answer.strict = Some(value.to_ascii_lowercase()),
            // A quoted `"none"` is a command of that name, as it is for OpenSSH: only the bare word is no proxy.
            "proxyjump" | "proxycommand" => answer.proxied |= value != "none",
            "revokedhostkeys" | "knownhostscommand" => answer.keys_by_others |= value != "none",
            "hashknownhosts" => answer.hash_known_hosts = is_yes(value),
            "identitiesonly" => answer.auth.identities_only = is_yes(value),
            "identityagent" => answer.auth.identity_agent = Some(value.to_owned()),
            "pubkeyauthentication" => answer.auth.public_key = Some(value.to_ascii_lowercase()),
            "passwordauthentication" => answer.auth.password = Some(value.to_ascii_lowercase()),
            "kbdinteractiveauthentication" => answer.auth.keyboard_interactive = Some(value.to_ascii_lowercase()),
            "batchmode" => answer.auth.batch_mode = is_yes(value),
            "numberofpasswordprompts" => answer.auth.password_prompts = Some(value.parse().map_err(|_| unusable())?),
            _ => {
                answer.auth.preferred = Some(
                    value
                        .split(',')
                        .map(|method| method.trim().to_ascii_lowercase())
                        .collect(),
                );
            }
        }
    }

    Ok(answer)
}

/// The words of a list: `ssh -G` joins the entries with a space and does not quote them, so an entry with a space in it
/// is several words (see `known::files`). A run of spaces is kept as empty words, so that it can be joined again.
fn words(value: &str) -> Vec<String> {
    value.split(' ').map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn parsed(text: &str) -> Answer {
        parse(text.as_bytes()).expect("parsed")
    }

    #[test]
    fn the_keys_that_are_used_are_read_and_the_others_ignored() {
        let answer = parsed(
            "host files\nuser bob\nhostname 10.0.0.1\nport 2200\ncompression no\n\
             identityfile ~/.ssh/one\nidentityfile /keys/with space/two\n\
             userknownhostsfile /h/kh1 /h/kh2\nglobalknownhostsfile /etc/ssh/ssh_known_hosts\n\
             hostkeyalias vault\nstricthostkeychecking TRUE\nsendenv LANG\u{7}\n",
        );

        assert_eq!(
            answer,
            Answer {
                host_name: Some("10.0.0.1".to_owned()),
                user: Some("bob".to_owned()),
                port: Some(2200),
                identity_files: vec!["~/.ssh/one".to_owned(), "/keys/with space/two".to_owned()],
                user_known_hosts: vec!["/h/kh1".to_owned(), "/h/kh2".to_owned()],
                global_known_hosts: vec!["/etc/ssh/ssh_known_hosts".to_owned()],
                host_key_alias: Some("vault".to_owned()),
                strict: Some("true".to_owned()),
                ..Answer::default()
            }
        );
    }

    #[test]
    fn any_proxy_but_none_is_a_proxy() {
        assert!(!parsed("hostname h\n").proxied);
        assert!(!parsed("proxyjump none\nproxycommand none\n").proxied);

        for line in [
            "proxyjump bastion",
            "proxyjump NONE",
            "proxyjump none,bastion",
            "proxycommand nc %h %p",
            "proxycommand \"none\"",
            "proxycommand none ",
        ] {
            assert!(parsed(&format!("{line}\n")).proxied, "{line}");
        }
    }

    #[test]
    fn a_value_that_cannot_be_used_refuses_the_answer() {
        for text in [
            "hostname a\u{1b}[31mb\n",
            "user bob\u{0}x\n",
            "port 99999\n",
            "port http\n",
            "numberofpasswordprompts many\n",
            "identityfile /k\ty\n",
        ] {
            let error = parse(text.as_bytes()).expect_err(text);

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{text:?}");
            assert_eq!(
                error.to_string(),
                "ssh gave a value that Pairpane cannot use for this host"
            );
        }

        assert_eq!(
            parse(&[b'u', b's', b'e', b'r', b' ', 0xff, b'\n'])
                .expect_err("not text")
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_crlf_ending_is_not_part_of_the_value() {
        assert_eq!(parsed("hostname h\r\nport 22\r\n").host_name.as_deref(), Some("h"));
    }

    #[test]
    fn the_options_that_judge_host_keys_or_limit_the_ways_in_are_read() {
        let answer = parsed(
            "revokedhostkeys none\nknownhostscommand none\nhashknownhosts yes\nidentitiesonly yes\nidentityagent none\n\
             pubkeyauthentication true\npasswordauthentication no\nkbdinteractiveauthentication yes\n\
             preferredauthentications publickey, Password\nbatchmode yes\nnumberofpasswordprompts 0\n",
        );

        assert!(!answer.keys_by_others);
        assert!(answer.hash_known_hosts);
        assert_eq!(
            answer.auth,
            AuthOptions {
                identities_only: true,
                identity_agent: Some("none".to_owned()),
                public_key: Some("true".to_owned()),
                password: Some("no".to_owned()),
                keyboard_interactive: Some("yes".to_owned()),
                preferred: Some(vec!["publickey".to_owned(), "password".to_owned()]),
                batch_mode: true,
                password_prompts: Some(0),
            }
        );

        for line in ["revokedhostkeys /h/krl", "knownhostscommand /bin/echo %H"] {
            assert!(parsed(&format!("{line}\n")).keys_by_others, "{line}");
        }
    }
}
