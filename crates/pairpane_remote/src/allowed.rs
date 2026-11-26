//! Which ways of signing in the user's `ssh_config` allows, as `ssh -G` prints it: `PubkeyAuthentication`,
//! `PasswordAuthentication`, `KbdInteractiveAuthentication`, `PreferredAuthentications`, `IdentitiesOnly` and
//! `IdentityAgent`, and the two that decide whether the user may be asked for a secret at all (`BatchMode`,
//! `NumberOfPasswordPrompts`). A way that is not allowed is never tried, and never asked about (no password question
//! where the password methods are off).
//!
//! The ways Pairpane has are the public keys (the agent's, then the key files), the password, and the one hidden prompt of
//! keyboard-interactive; the others `PreferredAuthentications` may name (`gssapi-with-mic`, `hostbased`) are not
//! supported and are left out. They are tried in the order of `PreferredAuthentications` when it is set, else in the
//! usual order.

/// The options of the configuration that say which ways of signing in are allowed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AuthOptions {
    pub(crate) identities_only: bool,
    /// `IdentityAgent` as printed (`none`, a path, `SSH_AUTH_SOCK`).
    pub(crate) identity_agent: Option<String>,
    pub(crate) public_key: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) keyboard_interactive: Option<String>,
    /// The comma-separated `PreferredAuthentications`, split.
    pub(crate) preferred: Option<Vec<String>>,
    /// `BatchMode yes`: nothing is asked of the user.
    pub(crate) batch_mode: bool,
    /// `NumberOfPasswordPrompts`: how many times a password or a passphrase may be asked for.
    pub(crate) password_prompts: Option<u32>,
}

/// One way of signing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The agent's keys and the key files.
    PublicKey,
    /// The password, as a password.
    Password,
    /// The password, as the answer to one hidden keyboard-interactive prompt.
    Keyboard,
}

impl Phase {
    fn named(method: &str) -> Option<Self> {
        match method {
            "publickey" => Some(Self::PublicKey),
            "password" => Some(Self::Password),
            "keyboard-interactive" => Some(Self::Keyboard),
            _ => None,
        }
    }
}

/// The ways in that are allowed, in the order they are tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Allowed {
    /// The ssh-agent's keys may be offered.
    pub(crate) agent: bool,
    /// The user may be asked for a password or a passphrase (neither `BatchMode yes` nor `NumberOfPasswordPrompts 0`).
    pub(crate) ask: bool,
    phases: Vec<Phase>,
}

impl Default for Allowed {
    /// Everything, in the usual order: where there is no configuration to say otherwise.
    fn default() -> Self {
        Self {
            agent: true,
            ask: true,
            phases: vec![Phase::PublicKey, Phase::Password, Phase::Keyboard],
        }
    }
}

/// An option that `ssh -G` prints as `yes`/`no` (or `true`/`false`; `PubkeyAuthentication` also has `unbound` and
/// `host-bound`, which are yes). A value that is none of them is taken as no.
fn enabled(value: Option<&str>) -> bool {
    value.is_none_or(|value| matches!(value, "yes" | "true" | "unbound" | "host-bound"))
}

impl Allowed {
    pub(crate) fn of(options: &AuthOptions) -> Self {
        let preferred: Vec<Phase> = match &options.preferred {
            Some(methods) => methods.iter().filter_map(|method| Phase::named(method)).collect(),
            None => Self::default().phases,
        };
        let mut phases: Vec<Phase> = Vec::new();

        for phase in preferred {
            let wanted = match phase {
                Phase::PublicKey => enabled(options.public_key.as_deref()),
                Phase::Password => enabled(options.password.as_deref()),
                Phase::Keyboard => enabled(options.keyboard_interactive.as_deref()),
            };

            if wanted && !phases.contains(&phase) {
                phases.push(phase);
            }
        }

        // With `IdentitiesOnly` only the identity files of the configuration are used: an agent's keys are not looked at.
        let agent = phases.contains(&Phase::PublicKey)
            && !options.identities_only
            && options.identity_agent.as_deref() != Some("none");

        Self {
            agent,
            ask: !options.batch_mode && options.password_prompts != Some(0),
            phases,
        }
    }

    /// Whether this is stricter than `open`: a way, the agent or a question that `open` has is not here. A server that is
    /// open under `open` must not be reused for a configuration that forbids what it was opened with.
    pub(crate) fn stricter_than(&self, open: &Self) -> bool {
        open.phases.iter().any(|phase| !self.phases.contains(phase))
            || (open.agent && !self.agent)
            || (open.ask && !self.ask)
    }

    pub(crate) fn phases(&self) -> &[Phase] {
        &self.phases
    }

    pub(crate) fn allows(&self, phase: Phase) -> bool {
        self.phases.contains(&phase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(lines: &[(&str, &str)]) -> AuthOptions {
        let mut options = AuthOptions::default();

        for (key, value) in lines {
            let value = (*value).to_owned();

            match *key {
                "identitiesonly" => options.identities_only = value == "yes",
                "identityagent" => options.identity_agent = Some(value),
                "pubkey" => options.public_key = Some(value),
                "password" => options.password = Some(value),
                "kbd" => options.keyboard_interactive = Some(value),
                "batch" => options.batch_mode = value == "yes",
                "prompts" => options.password_prompts = value.parse().ok(),
                "preferred" => options.preferred = Some(value.split(',').map(str::to_owned).collect()),
                other => panic!("{other}"),
            }
        }

        options
    }

    #[test]
    fn with_no_option_everything_is_allowed_in_the_usual_order() {
        assert_eq!(Allowed::of(&AuthOptions::default()), Allowed::default());
        assert_eq!(
            Allowed::default().phases(),
            [Phase::PublicKey, Phase::Password, Phase::Keyboard]
        );
        assert!(Allowed::default().agent);
    }

    #[test]
    fn a_way_that_is_off_is_not_allowed() {
        let no_keys = Allowed::of(&options(&[("pubkey", "false")]));

        assert!(!no_keys.allows(Phase::PublicKey) && !no_keys.agent);
        assert!(no_keys.allows(Phase::Password) && no_keys.allows(Phase::Keyboard));

        let no_password = Allowed::of(&options(&[("password", "no")]));

        assert!(!no_password.allows(Phase::Password));
        assert!(no_password.allows(Phase::Keyboard) && no_password.allows(Phase::PublicKey));

        let no_prompt = Allowed::of(&options(&[("kbd", "no")]));

        assert!(!no_prompt.allows(Phase::Keyboard) && no_prompt.allows(Phase::Password));
        assert!(Allowed::of(&options(&[("pubkey", "host-bound")])).allows(Phase::PublicKey));
        assert!(Allowed::of(&options(&[("pubkey", "unbound")])).allows(Phase::PublicKey));
        assert!(!Allowed::of(&options(&[("pubkey", "nonsense")])).allows(Phase::PublicKey));
    }

    #[test]
    fn identities_only_or_no_agent_leaves_the_agent_out_and_the_key_files_in() {
        for lines in [&[("identitiesonly", "yes")][..], &[("identityagent", "none")]] {
            let allowed = Allowed::of(&options(lines));

            assert!(!allowed.agent, "{lines:?}");
            assert!(allowed.allows(Phase::PublicKey), "{lines:?}");
        }

        assert!(Allowed::of(&options(&[("identityagent", "/run/agent")])).agent);
    }

    #[test]
    fn the_preferred_methods_are_the_only_ones_and_in_that_order() {
        let allowed = Allowed::of(&options(&[(
            "preferred",
            "password,gssapi-with-mic,publickey,password",
        )]));

        assert_eq!(allowed.phases(), [Phase::Password, Phase::PublicKey]);

        let only_password = Allowed::of(&options(&[("preferred", "keyboard-interactive")]));

        assert_eq!(only_password.phases(), [Phase::Keyboard]);
        assert!(!only_password.agent);

        // What is preferred but switched off stays off; and a list of methods that are not supported allows none.
        assert_eq!(
            Allowed::of(&options(&[("preferred", "publickey,password"), ("pubkey", "no")])).phases(),
            [Phase::Password]
        );
        assert!(
            Allowed::of(&options(&[("preferred", "gssapi-with-mic,hostbased")]))
                .phases()
                .is_empty()
        );
    }

    #[test]
    fn batch_mode_and_no_prompts_allow_no_question() {
        assert!(Allowed::of(&AuthOptions::default()).ask);
        assert!(Allowed::of(&options(&[("prompts", "3")])).ask);
        assert!(!Allowed::of(&options(&[("batch", "yes")])).ask);
        assert!(!Allowed::of(&options(&[("prompts", "0")])).ask);
    }

    #[test]
    fn a_configuration_is_stricter_when_it_lacks_something_the_other_has() {
        let open = Allowed::default();

        assert!(!open.stricter_than(&open));
        assert!(Allowed::of(&options(&[("password", "no")])).stricter_than(&open));
        assert!(Allowed::of(&options(&[("identityagent", "none")])).stricter_than(&open));
        assert!(Allowed::of(&options(&[("batch", "yes")])).stricter_than(&open));
        assert!(Allowed::of(&options(&[("preferred", "publickey")])).stricter_than(&open));
        // A looser or a differently ordered one is not.
        assert!(!open.stricter_than(&Allowed::of(&options(&[("password", "no")]))));
        assert!(
            !Allowed::of(&options(&[("preferred", "password,publickey,keyboard-interactive")])).stricter_than(&open)
        );
    }
}
