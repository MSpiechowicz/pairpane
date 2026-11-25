//! The pairing of the host under attack and under strain: connections that do nothing, joiners that come at once, the end
//! of a code and what is announced about it. A joiner written by hand stands for a peer that does not follow the rules.

use std::time::Duration;

use iroh::endpoint::Connection;
use pretty_assertions::assert_eq;

use super::tests::{Node, closed_with, wrong_code};
use super::*;
use crate::pairing::{
    Confirmation, Handshake, MAX_FAILURES, MESSAGE_LEN, PAIR_VERSION, PairingEndReason, Role, TAG_LEN,
};
use crate::wire::Channel;

/// A joiner that speaks the pairing protocol step by step, so a test can stop at any of them.
struct ByHand {
    conn: Connection,
    channel: Channel,
    confirmation: Confirmation,
}

impl ByHand {
    /// Connects and sends the first message. It returns when the host has answered with its own, which the host does only
    /// after it has taken the attempt.
    async fn start(joiner: &Node, host: &Node, code: &str) -> Self {
        let code = PairingCode::parse(code).unwrap();
        let conn = joiner
            .link
            .ctx
            .endpoint
            .connect(host.target().into_addr(), PAIR_ALPN)
            .await
            .unwrap();

        let handshake = Handshake::start(Role::Join, &code);
        let mut channel = Channel::open(&conn).await.unwrap();
        channel
            .send(&[&[PAIR_VERSION], handshake.message()].concat())
            .await
            .unwrap();

        let host_message = channel.recv_exact(MESSAGE_LEN).await.unwrap();
        let confirmation = handshake.finish(&host_message, &joiner.id(), &host.id()).unwrap();

        Self {
            conn,
            channel,
            confirmation,
        }
    }

    /// Sends the proof of the code and takes the host's.
    async fn prove(&mut self) {
        self.channel.send(&self.confirmation.tag(Role::Join)).await.unwrap();
        self.channel.recv_exact(TAG_LEN).await.unwrap();
    }
}

/// A connection that is made and then says nothing.
async fn silent(joiner: &Node, host: &Node) -> Connection {
    joiner
        .link
        .ctx
        .endpoint
        .connect(host.target().into_addr(), PAIR_ALPN)
        .await
        .unwrap()
}

async fn next_ending(events: &mut broadcast::Receiver<LinkEvent>) -> PairingEndReason {
    loop {
        let event = timeout(Duration::from_secs(10), events.recv()).await.unwrap().unwrap();
        if let LinkEvent::PairingEnded { reason } = event {
            return reason;
        }
    }
}

async fn next_paired(events: &mut broadcast::Receiver<LinkEvent>) -> Device {
    timeout(Duration::from_secs(20), async {
        loop {
            if let LinkEvent::Paired(device) = events.recv().await.unwrap() {
                return device;
            }
        }
    })
    .await
    .expect("the pairing is announced")
}

fn attempts_left(node: &Node) -> u8 {
    let PairingState::Waiting { attempts_left, .. } = node.link.pairing_state().unwrap() else {
        panic!("the code is still active");
    };

    attempts_left
}

#[tokio::test]
async fn connections_that_send_nothing_do_not_keep_a_real_joiner_out() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let offer = host.link.begin_pairing().unwrap();

    // Each of these waits for a first message that never comes; they may wait together, within the cap of a source.
    let mut idle = Vec::new();
    for _ in 0..ctx::MAX_CONNECTIONS_PER_IP - 1 {
        idle.push(silent(&joiner, &host).await);
    }

    joiner.link.join(&offer.code, &host.target()).await.unwrap();

    assert_eq!(host.link.devices().unwrap().len(), 1);
    assert!(idle.iter().all(|conn| conn.close_reason().is_none()));
}

#[tokio::test]
async fn a_connection_that_sends_nothing_is_closed_and_does_not_use_up_a_try() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.link.begin_pairing().unwrap();
    let idle = silent(&joiner, &host).await;

    // The host gives up on it after the time it allows for a first message; the limit here is only a safety net.
    timeout(Duration::from_secs(30), idle.closed())
        .await
        .expect("the host closes a connection that says nothing");

    assert_eq!(attempts_left(&host), MAX_FAILURES);
}

#[tokio::test]
async fn a_joiner_that_sends_while_an_attempt_runs_is_turned_away_and_not_counted() {
    let host = Node::start("host").await;
    let first = Node::start("first").await;
    let second = Node::start("second").await;
    let mut events = host.link.subscribe().unwrap();
    let offer = host.link.begin_pairing().unwrap();
    let mut running = ByHand::start(&first, &host, &offer.code).await;

    let turned_away = second.link.join(&offer.code, &host.target()).await;

    assert!(matches!(turned_away, Err(Error::PairingRejected)), "{turned_away:?}");
    assert_eq!(
        attempts_left(&host),
        MAX_FAILURES,
        "the one that was turned away is not a failure"
    );
    // The attempt that was running is not harmed by it.
    running.prove().await;
    running.channel.send(b"first").await.unwrap();
    let device = next_paired(&mut events).await;
    assert_eq!((device.node_id, device.name.as_str()), (first.id(), "first"));
    assert!(second.link.devices().unwrap().is_empty());
}

#[tokio::test]
async fn the_device_is_announced_at_once_and_not_after_the_joiner_has_taken_the_hosts_name() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let mut events = host.link.subscribe().unwrap();
    let offer = host.link.begin_pairing().unwrap();
    let mut by_hand = ByHand::start(&joiner, &host, &offer.code).await;
    by_hand.prove().await;

    // The joiner sends its name and then neither reads the host's nor closes the connection. The host waits for that for
    // a long time in the tests, so an announcement that waits for it too does not come.
    by_hand.channel.send(b"joiner").await.unwrap();

    let device = next_paired(&mut events).await;
    assert_eq!((device.node_id, device.name.as_str()), (joiner.id(), "joiner"));
    assert!(by_hand.conn.close_reason().is_none());
}

#[tokio::test]
async fn the_device_is_saved_even_when_the_joiner_does_not_take_the_hosts_name() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let mut events = host.link.subscribe().unwrap();
    let offer = host.link.begin_pairing().unwrap();

    // A joiner that does the whole exchange and then tells the host to stop sending to it.
    let mut by_hand = ByHand::start(&joiner, &host, &offer.code).await;
    by_hand.prove().await;
    by_hand.channel.stop_receiving();
    tokio::time::sleep(Duration::from_millis(300)).await;
    by_hand.channel.send(b"joiner").await.unwrap();

    let device = next_paired(&mut events).await;
    assert_eq!((device.node_id, device.name.as_str()), (joiner.id(), "joiner"));
    assert_eq!(host.link.devices().unwrap(), vec![device]);
}

#[tokio::test]
async fn an_attempt_that_sent_the_key_exchange_message_still_counts() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    host.link.begin_pairing().unwrap();

    let conn = silent(&joiner, &host).await;
    let mut channel = Channel::open(&conn).await.unwrap();
    channel.send(&[PAIR_VERSION; 1 + MESSAGE_LEN]).await.unwrap();
    closed_with(&conn).await;

    assert_eq!(attempts_left(&host), MAX_FAILURES - 1);
}

#[tokio::test]
async fn the_end_of_a_code_is_reported_with_its_reason() {
    let host = Node::start("host").await;
    let joiner = Node::start("joiner").await;
    let mut events = host.link.subscribe().unwrap();

    // Used up.
    host.pair_with(&joiner).await;
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Used);

    // Cancelled, and replaced.
    host.link.begin_pairing().unwrap();
    host.link.begin_pairing().unwrap();
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Cancelled);
    host.link.cancel_pairing().unwrap();
    assert_eq!(next_ending(&mut events).await, PairingEndReason::Cancelled);

    // Failed too often.
    let offer = host.link.begin_pairing().unwrap();
    let wrong = wrong_code(&offer);
    for _ in 0..MAX_FAILURES {
        joiner.link.join(wrong, &host.target()).await.unwrap_err();
    }
    assert_eq!(next_ending(&mut events).await, PairingEndReason::TooManyAttempts);
}

#[tokio::test]
async fn a_code_that_runs_out_is_reported_as_expired_with_nobody_calling_anything() {
    let host = Node::start_with("host", |config| config.pairing_ttl = Duration::from_millis(300)).await;
    let mut events = host.link.subscribe().unwrap();

    host.link.begin_pairing().unwrap();

    assert_eq!(next_ending(&mut events).await, PairingEndReason::Expired);
}

#[tokio::test]
async fn a_pairing_code_cannot_be_made_where_no_runtime_runs() {
    let node = Node::start("node").await;

    // A thread of its own has no runtime, as a plain thread of the app would not.
    let made = std::thread::scope(|scope| scope.spawn(|| node.link.begin_pairing()).join().unwrap());

    assert!(matches!(made, Err(Error::NoRuntime)), "{made:?}");
    assert_eq!(
        node.link.pairing_state().unwrap(),
        PairingState::Idle,
        "no code was made"
    );
}
