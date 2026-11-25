//! The key exchange and the confirmation, without a network: two handshakes in memory.

use iroh::{EndpointId, SecretKey};

use super::{Confirmation, Handshake, MESSAGE_LEN, Role, TAG_LEN, tags_match};
use crate::code::PairingCode;

fn id() -> EndpointId {
    SecretKey::generate().public()
}

fn code(text: &str) -> PairingCode {
    PairingCode::parse(text).unwrap()
}

/// Both sides run the exchange with the codes and ids they each have.
struct Run {
    joiner: Confirmation,
    host: Confirmation,
}

/// What each side takes the two node ids to be, as `(joiner, host)`.
type Ids<'a> = (&'a EndpointId, &'a EndpointId);

fn run(join_code: &str, host_code: &str, joiner_sees: Ids, host_sees: Ids) -> Run {
    let joiner = Handshake::start(Role::Join, &code(join_code));
    let host = Handshake::start(Role::Host, &code(host_code));
    let (join_message, host_message) = (joiner.message().to_vec(), host.message().to_vec());

    Run {
        joiner: joiner.finish(&host_message, joiner_sees.0, joiner_sees.1).unwrap(),
        host: host.finish(&join_message, host_sees.0, host_sees.1).unwrap(),
    }
}

/// A run in which both sides see the same two node ids, as they do without a man in the middle.
fn honest_run(join_code: &str, host_code: &str) -> Run {
    let ids = (id(), id());

    run(join_code, host_code, (&ids.0, &ids.1), (&ids.0, &ids.1))
}

fn assert_no_tag_matches(run: &Run) {
    assert!(!run.host.verify(Role::Join, &run.joiner.tag(Role::Join)));
    assert!(!run.joiner.verify(Role::Host, &run.host.tag(Role::Host)));
}

#[test]
fn messages_have_the_size_the_protocol_expects() {
    let handshake = Handshake::start(Role::Join, &code("7K2M-9QXZ"));

    assert_eq!(handshake.message().len(), MESSAGE_LEN);
}

#[test]
fn the_same_code_and_ids_make_matching_tags_in_both_directions() {
    let run = honest_run("7K2M-9QXZ", "7k2m9qxz");

    assert!(run.host.verify(Role::Join, &run.joiner.tag(Role::Join)));
    assert!(run.joiner.verify(Role::Host, &run.host.tag(Role::Host)));
}

#[test]
fn a_wrong_code_makes_no_tag_match() {
    let run = honest_run("7K2M-9QXZ", "7K2M-9QXY");

    assert_no_tag_matches(&run);
}

#[test]
fn a_relay_between_two_connections_is_caught_by_the_node_ids() {
    // A man in the middle holds one connection to each device: the joiner sees the attacker's id as the host's, and the
    // host sees the attacker's id as the joiner's. Even a relay of the key exchange messages cannot make the tags match.
    let (joiner_id, host_id, attacker_id) = (id(), id(), id());

    let run = run(
        "7K2M-9QXZ",
        "7K2M-9QXZ",
        (&joiner_id, &attacker_id),
        (&attacker_id, &host_id),
    );

    assert_no_tag_matches(&run);
}

#[test]
fn a_tag_made_for_one_role_is_not_the_tag_of_the_other() {
    let run = honest_run("7K2M-9QXZ", "7K2M-9QXZ");

    assert!(run.joiner.tag(Role::Join) != run.joiner.tag(Role::Host));
    assert!(
        !run.joiner.verify(Role::Host, &run.joiner.tag(Role::Join)),
        "a reflected tag is no proof"
    );
}

#[test]
fn two_sessions_never_share_tags() {
    let first = honest_run("7K2M-9QXZ", "7K2M-9QXZ");
    let second = honest_run("7K2M-9QXZ", "7K2M-9QXZ");

    assert!(first.joiner.tag(Role::Join) != second.joiner.tag(Role::Join));
    assert!(
        !second.host.verify(Role::Join, &first.joiner.tag(Role::Join)),
        "a recorded tag cannot be replayed"
    );
}

#[test]
fn a_bad_message_is_an_error() {
    let own = id();

    for bad in [&[][..], &[0x42; 5], &[0x42; MESSAGE_LEN + 1]] {
        let handshake = Handshake::start(Role::Join, &code("7K2M-9QXZ"));
        assert!(handshake.finish(bad, &own, &own).is_err(), "{} bytes", bad.len());
    }

    // The joiner's own side byte (A) is not what a joiner may receive.
    let echo = Handshake::start(Role::Join, &code("7K2M-9QXZ"));
    let message = echo.message().to_vec();
    assert!(echo.finish(&message, &own, &own).is_err());
}

#[test]
fn tags_are_compared_whole_and_by_length() {
    let tag = [7u8; TAG_LEN];

    assert!(tags_match(&tag, &tag));
    for position in [0, TAG_LEN / 2, TAG_LEN - 1] {
        let mut other = tag;
        other[position] ^= 1;
        assert!(!tags_match(&tag, &other), "differs at {position}");
    }
    assert!(!tags_match(&tag, &tag[..TAG_LEN - 1]));
    assert!(!tags_match(&tag, &[tag.as_slice(), &[0]].concat()));
    assert!(!tags_match(&tag, &[]));
}
