//! A server that does not answer, one that goes away, and how many sites there can be.

use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pairpane_core::model::{ConnectOutcome, SecretKind};
use pretty_assertions::assert_eq;

use super::{Harness, assert_fixed, count, path, quick, target_of};
use crate::test_server::Settings;
use crate::{AgentSource, MAX_SITES, Timeouts};

const PASSWORD: &str = "correct horse";

fn timeouts(request_ms: u64, idle_ms: u64) -> Timeouts {
    Timeouts {
        request: Duration::from_millis(request_ms),
        handshake: Duration::from_millis(1_000),
        idle: Duration::from_millis(idle_ms),
        ..quick()
    }
}

#[test]
fn a_server_that_says_nothing_times_out_without_blocking_for_long() {
    let h = Harness::with(Settings::default(), timeouts(300, 600_000), AgentSource::None);
    // Takes the connection and never speaks: the SSH banner does not come.
    let silent = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let target = target_of(silent.local_addr().expect("an address"));
    let started = Instant::now();

    let error = h.connect_to(&target, Some(PASSWORD)).expect_err("timed out");

    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert_fixed(&error, silent.local_addr().expect("an address"));
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn time_spent_on_a_question_is_not_counted_against_the_handshake() {
    use crate::{HostKeyPrompt, UnknownHost};

    struct Slow;

    impl HostKeyPrompt for Slow {
        fn trust_new(&self, _: &UnknownHost) -> bool {
            std::thread::sleep(Duration::from_millis(1_500));

            true
        }
    }

    let h = Harness::with(Settings::default(), timeouts(5_000, 600_000), AgentSource::None);

    h.pool.assume_asked(&h.target(), SecretKind::Password);

    let outcome = h.pool.connect(
        &h.target(),
        Some(Harness::secret(SecretKind::Password, PASSWORD)),
        std::sync::Arc::new(Slow),
    );

    // The handshake limit is 1 s; the question took 1.5 s.
    assert!(matches!(outcome, Ok(ConnectOutcome::Connected { .. })), "{outcome:?}");
}

#[test]
fn a_refused_connection_is_reported_as_such() {
    let h = Harness::new(Settings::default());
    let closed = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let target = target_of(closed.local_addr().expect("an address"));

    drop(closed);

    let error = h.connect_to(&target, None).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::ConnectionRefused);
}

#[test]
fn a_request_the_server_never_answers_times_out() {
    let h = Harness::with(
        Settings {
            stalled: vec!["/stuck".to_owned()],
            ..Settings::default()
        },
        timeouts(300, 600_000),
        AgentSource::None,
    );

    h.trust();

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");
    let started = Instant::now();
    let error = fs
        .read_dir(&path("/stuck"), &AtomicBool::new(false))
        .expect_err("timed out");

    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert_fixed(&error, h.server.addr);
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn a_dropped_connection_is_opened_again_without_a_question() {
    let h = Harness::new(Settings::default());

    h.server.put("kept/file", b"");

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");

    assert_eq!(
        fs.read_dir(&path("/kept"), &AtomicBool::new(false))
            .expect("listed")
            .len(),
        1
    );

    h.server.drop_connections();
    // The client learns of it when it next speaks (or hears the close): either way the call below must succeed.
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(count(&*fs, "/kept"), 1);
    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 2);
    assert_eq!(h.prompt.times(), 1, "only the first connection asked");
}

#[test]
fn an_idle_connection_is_closed_and_comes_back_when_used() {
    let h = Harness::with(Settings::default(), timeouts(5_000, 200), AgentSource::None);

    h.server.put("idle/file", b"");

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");

    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 1);
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(
        fs.read_dir(&path("/idle"), &AtomicBool::new(false))
            .expect("listed")
            .len(),
        1
    );
    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 2);
}

#[test]
fn a_disconnected_site_is_gone() {
    let h = Harness::new(Settings::default());
    let site = h.site(Some(PASSWORD));
    let fs = h.pool.fs(site).expect("a site");

    h.pool.disconnect(site);

    assert_eq!(
        h.pool.fs(site).err().map(|error| error.kind()),
        Some(ErrorKind::NotConnected)
    );
    assert_eq!(
        fs.read_dir(&path("/"), &AtomicBool::new(false))
            .expect_err("closed")
            .kind(),
        ErrorKind::NotConnected
    );
}

#[test]
fn there_is_a_most_to_the_sites() {
    let h = Harness::new(Settings::default());

    h.trust();

    for number in 0..MAX_SITES {
        let mut target = h.target();

        target.user = Some(format!("me{number}"));
        h.connect_to(&target, Some(PASSWORD)).expect("connected");
    }

    let mut one_more = h.target();

    one_more.user = Some("me-last".to_owned());

    assert_eq!(
        h.connect_to(&one_more, Some(PASSWORD)).expect_err("too many").kind(),
        ErrorKind::QuotaExceeded
    );
}

#[test]
fn shutting_down_closes_everything_quickly_and_refuses_new_work() {
    let h = Harness::new(Settings::default());

    h.trust();

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");
    let started = Instant::now();

    h.pool.shut_down();
    h.pool.shut_down();

    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(
        h.connect(Some(PASSWORD)).expect_err("shut down").kind(),
        ErrorKind::NotConnected
    );
    assert!(fs.read_dir(&path("/"), &AtomicBool::new(false)).is_err());
}

#[test]
fn hosts_that_are_not_valid_are_refused_before_anything_is_sent() {
    let h = Harness::new(Settings::default());

    for bad in ["", "a b", "x,y", "*.example", "-oProxyCommand=x", "a@b"] {
        let mut target = h.target();

        target.host = bad.to_owned();
        assert_eq!(
            h.connect_to(&target, None).expect_err("refused").kind(),
            ErrorKind::InvalidInput,
            "{bad:?}"
        );
    }

    assert_eq!(h.server.counts.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn callers_that_find_a_site_dead_share_one_attempt_and_fail_at_once() {
    let h = Harness::new(Settings::default());

    h.trust();

    let fs = h.pool.fs(h.site(Some(PASSWORD))).expect("a site");

    h.server.drop_connections();
    h.server.refuse_new_connections();
    std::thread::sleep(Duration::from_millis(200));

    let started = Instant::now();
    let listers: Vec<_> = (0..4)
        .map(|_| {
            let fs = fs.clone();

            std::thread::spawn(move || fs.read_dir(&path("/"), &AtomicBool::new(false)))
        })
        .collect();

    for lister in listers {
        assert!(lister.join().expect("the thread").is_err());
    }

    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(
        h.server.counts.refused.load(Ordering::Relaxed),
        1,
        "one attempt for all four"
    );

    // Within the next seconds a caller is told the same without another attempt.
    assert!(fs.read_dir(&path("/"), &AtomicBool::new(false)).is_err());
    assert_eq!(h.server.counts.refused.load(Ordering::Relaxed), 1);
}

#[test]
fn connecting_to_a_site_whose_connection_failed_lets_it_try_again() {
    let h = Harness::new(Settings::default());

    h.trust();

    let site = h.site(Some(PASSWORD));
    let fs = h.pool.fs(site).expect("a site");

    h.server.drop_connections();
    h.server.refuse_new_connections();
    std::thread::sleep(Duration::from_millis(200));

    assert!(fs.read_dir(&path("/"), &AtomicBool::new(false)).is_err());

    // The server is back, but the failure is remembered for a while, so the call is told the same without a try.
    h.server.accept_new_connections();
    assert!(fs.read_dir(&path("/"), &AtomicBool::new(false)).is_err());

    // The user connects again: the same site, and its connection is tried again at once.
    assert_eq!(h.site(None), site);
    assert!(fs.read_dir(&path("/"), &AtomicBool::new(false)).is_ok());
}
