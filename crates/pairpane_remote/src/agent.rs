//! Signing in with the keys of an ssh-agent (Unix sockets only; the Windows agents are not supported yet).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use russh::client::Handle;

use crate::auth::Credentials;
use crate::verify::Verifier;

/// Where the ssh-agent is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    /// None: no agent is used.
    None,
    /// The socket named by `SSH_AUTH_SOCK`.
    Env,
    /// This socket.
    Path(PathBuf),
}

/// Where the agent is for a host, as its `IdentityAgent` says (`configured`, as `ssh -G` prints it): `none` is no agent,
/// `SSH_AUTH_SOCK` (what the option is when it is not set) is the app's own agent (`default`), `$NAME` is the socket that
/// the variable `NAME` names (`var` reads the app's environment), and an absolute path (a leading `~/` is the home folder)
/// is that socket. Anything else is no agent: a socket that cannot be placed is not guessed at.
pub(crate) fn source(
    configured: Option<&str>,
    default: &AgentSource,
    home: &Path,
    var: impl Fn(&str) -> Option<OsString>,
) -> AgentSource {
    let absolute = |path: PathBuf| {
        if path.is_absolute() {
            AgentSource::Path(path)
        } else {
            AgentSource::None
        }
    };

    match configured {
        None | Some("SSH_AUTH_SOCK") => default.clone(),
        Some("none") => AgentSource::None,
        Some(name) if name.starts_with('$') => var(&name[1..]).map_or(AgentSource::None, |path| absolute(path.into())),
        Some(path) => match path.strip_prefix("~/") {
            Some(rest) => absolute(home.join(rest)),
            None => absolute(PathBuf::from(path)),
        },
    }
}

/// Offers the agent's keys to the server, at most `max` tries in all (`tries` counts them). Whether one was accepted.
#[cfg(unix)]
pub(crate) async fn sign_in(
    handle: &mut Handle<Verifier>,
    credentials: &Credentials<'_>,
    tries: &mut usize,
    max: usize,
) -> bool {
    use std::time::Duration;

    use russh::keys::agent::AgentIdentity;
    use russh::keys::agent::client::AgentClient;

    use crate::auth::rsa_hash;
    use crate::runtime::limited;

    /// How long the agent gets to answer each request.
    const ANSWER: Duration = Duration::from_secs(5);

    let connected = match &credentials.agent {
        AgentSource::None => return false,
        AgentSource::Env => {
            limited(ANSWER, async {
                AgentClient::connect_env().await.map_err(|_| agent_gone())
            })
            .await
        }
        AgentSource::Path(path) => {
            limited(ANSWER, async {
                AgentClient::connect_uds(path).await.map_err(|_| agent_gone())
            })
            .await
        }
    };
    let Ok(mut client) = connected else {
        return false;
    };
    let Ok(identities) = limited(ANSWER, async {
        client.request_identities().await.map_err(|_| agent_gone())
    })
    .await
    else {
        return false;
    };

    for identity in identities {
        // A certificate is not used: only the plain keys the agent holds.
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };

        if *tries >= max {
            break;
        }

        *tries += 1;

        let hash = rsa_hash(handle, key.algorithm().is_rsa()).await;
        let signed = limited(ANSWER, async {
            handle
                .authenticate_publickey_with(credentials.user, key, hash, &mut client)
                .await
                .map_err(|_| agent_gone())
        })
        .await;

        if signed.is_ok_and(|result| result.success()) {
            return true;
        }
    }

    false
}

#[cfg(unix)]
fn agent_gone() -> std::io::Error {
    crate::error::fail(std::io::ErrorKind::NotConnected, "the ssh-agent did not answer")
}

#[cfg(not(unix))]
pub(crate) async fn sign_in(
    _handle: &mut Handle<Verifier>,
    _credentials: &Credentials<'_>,
    _tries: &mut usize,
    _max: usize,
) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn identity_agent_says_where_the_agent_is() {
        let default = AgentSource::Path(PathBuf::from("/run/default"));
        let var = |name: &str| (name == "AGENT").then(|| OsString::from("/run/from-var"));
        let at = |configured: Option<&str>| source(configured, &default, Path::new("/home/me"), var);

        assert_eq!(at(None), default);
        assert_eq!(at(Some("SSH_AUTH_SOCK")), default);
        assert_eq!(at(Some("none")), AgentSource::None);
        assert_eq!(at(Some("$AGENT")), AgentSource::Path(PathBuf::from("/run/from-var")));
        assert_eq!(at(Some("$MISSING")), AgentSource::None);
        assert_eq!(at(Some("$")), AgentSource::None);
        assert_eq!(
            at(Some("/run/a b/agent")),
            AgentSource::Path(PathBuf::from("/run/a b/agent"))
        );
        assert_eq!(
            at(Some("~/agent.sock")),
            AgentSource::Path(PathBuf::from("/home/me/agent.sock"))
        );
        assert_eq!(at(Some("relative/agent")), AgentSource::None);
        assert_eq!(at(Some("~other/agent")), AgentSource::None);
    }
}
