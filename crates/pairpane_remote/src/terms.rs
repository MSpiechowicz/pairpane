//! What a site was opened under, as far as it is about trust and signing in: a second connect to the same user, host and
//! port that comes with other settings (an `ssh_config` alias for the same server) is not allowed to ride on the first
//! one's if its own would have been stricter. See [`Terms::stricter_than`].

use crate::allowed::Allowed;
use crate::known_hosts::{Files, Name};
use crate::plan::Plan;
use crate::verify::NewKey;

/// The settings of an open site that make a connection easier or harder to trust.
#[derive(Debug, Clone)]
pub(crate) struct Terms {
    new_key: NewKey,
    known_hosts: Files,
    known_as: Name,
    allowed: Allowed,
}

impl Terms {
    pub(crate) fn of(plan: &Plan) -> Self {
        Self {
            new_key: plan.new_key,
            known_hosts: plan.known_hosts.clone(),
            known_as: plan.target.known_as.clone(),
            allowed: plan.target.allowed.clone(),
        }
    }

    /// Whether `self` asks for more than `open` was opened with, or for something that cannot be compared: a key that is
    /// refused where it was asked about (`StrictHostKeyChecking yes`, `BatchMode yes`), other `known_hosts` files (or
    /// another name in them), fewer ways of signing in or no more questions. Equal settings are not stricter, and a looser
    /// one is no reason to refuse.
    pub(crate) fn stricter_than(&self, open: &Self) -> bool {
        (self.new_key != NewKey::Ask && open.new_key == NewKey::Ask)
            || self.known_hosts != open.known_hosts
            || self.known_as != open.known_as
            || self.allowed.stricter_than(&open.allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> Terms {
        Terms {
            new_key: NewKey::Ask,
            known_hosts: Files {
                user: vec!["/h/kh".into()],
                joined: Vec::new(),
                global: Vec::new(),
            },
            known_as: Name::of("host", 22).expect("a name"),
            allowed: Allowed::default(),
        }
    }

    #[test]
    fn equal_terms_and_looser_ones_are_not_stricter_and_every_difference_in_trust_is() {
        let open = terms();

        assert!(!terms().stricter_than(&open));

        let mut strict = terms();

        strict.new_key = NewKey::Strict;
        assert!(strict.stricter_than(&open));
        // The other way round, the open one was the strict one.
        assert!(!open.stricter_than(&strict));

        let mut batch = terms();

        batch.new_key = NewKey::Batch;
        assert!(batch.stricter_than(&open));

        let mut files = terms();

        files.known_hosts.user.push("/h/other".into());
        assert!(files.stricter_than(&open));

        let mut alias = terms();

        alias.known_as = Name::alias("vault").expect("an alias");
        assert!(alias.stricter_than(&open));
    }
}
