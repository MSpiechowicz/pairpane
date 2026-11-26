use std::sync::Arc;

use pretty_assertions::assert_eq;
use russh::client::Handler;
use russh::keys::{Algorithm, PublicKeyOrCertificate};

use crate::known_hosts::{KnownHosts, Name};
use crate::test_server::generate_key;
use crate::verify::{HostKeyPrompt, NewKey, Outcome, Policy, Refusal, UnknownHost, Verifier};

fn host() -> Name {
    Name::of("host", 22).expect("a valid host")
}

struct Never;

impl HostKeyPrompt for Never {
    fn trust_new(&self, _: &UnknownHost) -> bool {
        panic!("a pinned key is never asked about");
    }
}

#[tokio::test]
async fn a_reopened_connection_accepts_only_the_key_it_had() {
    let (first, second) = (generate_key().public_key().clone(), generate_key().public_key().clone());

    for (presented, accepted) in [(first.clone(), true), (second, false)] {
        let outcome = Outcome::default();
        let mut verifier = Verifier::new("host", 22, host(), Policy::Pinned(first.clone()), outcome.clone());

        assert_eq!(
            verifier
                .check_server_key(&PublicKeyOrCertificate::from(presented))
                .await
                .expect("checked"),
            accepted
        );
        assert_eq!(outcome.refusal(), (!accepted).then_some(Refusal::NotPinned));
    }
}

#[test]
fn the_server_is_asked_for_the_kinds_of_key_already_held() {
    let key = generate_key().public_key().clone();
    let held = |line: &str| Policy::Known {
        hosts: KnownHosts::parse(line),
        path: Some("known_hosts".into()),
        hash: false,
        new_key: NewKey::Ask,
        prompt: Arc::new(Never),
        accepted: Arc::default(),
    };

    assert_eq!(
        held("host ssh-ed25519 AAAA").preferred(&host()).key.to_vec(),
        [Algorithm::Ed25519]
    );
    assert_eq!(held("host ssh-rsa AAAA").preferred(&host()).key.len(), 3);
    // Nothing held, or only kinds that are not used: the default list.
    assert_eq!(
        held("other ssh-ed25519 AAAA").preferred(&host()).key,
        russh::Preferred::default().key
    );
    assert_eq!(
        Policy::Pinned(key).preferred(&host()).key.to_vec(),
        [Algorithm::Ed25519]
    );
}
