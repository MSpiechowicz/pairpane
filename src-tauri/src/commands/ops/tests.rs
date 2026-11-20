//! Tests of the transfer commands. The ones marked "needs the engine" run jobs or renames for real and pass once
//! the core's engine bodies exist; the rest never reach the engine's `submit` or `rename_in`.

use std::time::{Duration, Instant};

use pairpane_core::ops::{AnswerError, MAX_NAME_UNITS, Refused, UnknownJob};
use tauri::async_runtime::block_on;
use tauri::ipc::InvokeResponseBody;

use super::*;

fn root() -> String {
    if cfg!(windows) { "C:\\data" } else { "/data" }.to_owned()
}

/// A second absolute folder.
fn elsewhere() -> String {
    if cfg!(windows) { "C:\\x" } else { "/x" }.to_owned()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// A channel that records what it receives, decoded.
fn collecting_channel() -> (Channel<OpsEvent>, Arc<Mutex<Vec<OpsEvent>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("events are sent as JSON");
        };
        sink.lock()
            .expect("not poisoned")
            .push(serde_json::from_str(&json).expect("an OpsEvent"));

        Ok(())
    });

    (channel, received)
}

fn received(events: &Arc<Mutex<Vec<OpsEvent>>>) -> Vec<OpsEvent> {
    events.lock().expect("not poisoned").clone()
}

fn is_snapshot(event: &OpsEvent) -> bool {
    matches!(event, OpsEvent::Snapshot { .. })
}

fn plain_request(names: Vec<String>) -> CommandResult<Request> {
    request(OpKind::Copy, root(), names, root(), false)
}

#[test]
fn a_valid_request_keeps_its_folders_names_and_flags() {
    let request = request(OpKind::Move, root(), names(&["a.txt", "b"]), elsewhere(), true).expect("valid request");

    assert_eq!(request.kind, OpKind::Move);
    assert_eq!(request.from, Path::new(&root()));
    assert_eq!(request.to, Path::new(&elsewhere()));
    assert_eq!(request.names, names(&["a.txt", "b"]));
    assert!(request.verify);
}

#[test]
fn duplicate_names_are_dropped_and_the_order_of_the_first_ones_is_kept() {
    let request = plain_request(names(&["b", "a", "b", "c", "a"])).expect("valid request");

    assert_eq!(request.names, names(&["b", "a", "c"]));
}

#[test]
fn between_one_and_the_most_names_are_accepted_counted_after_dropping_duplicates() {
    let error = plain_request(Vec::new()).expect_err("no names");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    let most: Vec<_> = (0..MAX_NAMES).map(|n| n.to_string()).collect();
    assert_eq!(plain_request(most.clone()).expect("the most").names.len(), MAX_NAMES);

    let mut too_many = most.clone();
    too_many.push("one more".to_owned());
    let error = plain_request(too_many).expect_err("over the limit");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    // The same name many times is one item.
    let mut repeated = most;
    repeated.extend(std::iter::repeat_n("0".to_owned(), 10));
    assert_eq!(plain_request(repeated).expect("duplicates").names.len(), MAX_NAMES);
}

#[test]
fn a_name_is_at_most_255_bytes_or_units_and_all_names_together_at_most_8_mib() {
    let longest = "a".repeat(MAX_NAME_UNITS);
    assert!(plain_request(names(&[&longest])).is_ok());

    let too_long = "a".repeat(MAX_NAME_UNITS + 1);
    let error = plain_request(names(&["fine", &too_long])).expect_err("a name over the limit");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    // Short enough names, more bytes than the cap together, as different names and as the same one repeated.
    let count = MAX_NAMES_BYTES / MAX_NAME_UNITS + 1;
    let many: Vec<String> = (0..count).map(|index| format!("{index:0>MAX_NAME_UNITS$}")).collect();
    assert!(many.len() <= MAX_NAMES);
    let error = plain_request(many).expect_err("names over the total");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    let repeated = vec![longest; count];
    let error = plain_request(repeated).expect_err("the same name over the total");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
}

#[test]
fn names_must_be_plain_names() {
    for bad in ["..", ".", "a/b", "a/", "/etc", "", "a/../b"] {
        let error = plain_request(names(&["fine", bad])).expect_err(bad);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
    }
    #[cfg(windows)]
    for bad in ["a\\b", "C:", "C:\\x", "con.txt:stream", "a\\..\\b"] {
        assert!(plain_request(names(&[bad])).is_err(), "{bad}");
    }
}

#[test]
fn a_name_that_makes_the_path_too_long_is_refused() {
    let long = "a".repeat(5000);

    let error = plain_request(names(&[&long])).expect_err("too long");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
}

#[test]
fn both_folders_must_be_absolute_local_paths_without_climbing() {
    let climbing = format!(
        "{}{}..{}x",
        root(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    let mut bad = vec!["relative/dir".to_owned(), String::new(), ".".to_owned(), climbing];
    if cfg!(windows) {
        bad.extend(
            [r"\\attacker\x", "//attacker/x", r"\\?\UNC\a\b", r"\\.\pipe\x"]
                .into_iter()
                .map(str::to_owned),
        );
    }

    for folder in bad {
        let from = request(OpKind::Copy, folder.clone(), names(&["a"]), root(), false);
        assert_eq!(
            from.expect_err(&folder).code(),
            IpcErrorCode::InvalidInput,
            "from {folder}"
        );

        let to = request(OpKind::Copy, root(), names(&["a"]), folder.clone(), false);
        assert_eq!(to.expect_err(&folder).code(), IpcErrorCode::InvalidInput, "to {folder}");
    }
}

#[test]
fn an_invalid_start_is_refused_before_the_engine_is_asked() {
    let ops = Ops::default();

    for (from, selection, to) in [
        ("relative".to_owned(), &["a"][..], elsewhere()),
        (root(), &[][..], elsewhere()),
        (root(), &["a/b"][..], elsewhere()),
        (root(), &["a"][..], "../x".to_owned()),
    ] {
        let error = ops
            .start(OpKind::Copy, from.clone(), names(selection), to, false)
            .expect_err(&from);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{from}");
    }
}

#[test]
fn the_engines_refusals_keep_their_codes_and_tell_no_path() {
    let busy = failed(Refused::Busy);
    let invalid = failed(Refused::InvalidNames);

    assert_eq!(busy.code(), IpcErrorCode::Busy);
    assert_eq!(invalid.code(), IpcErrorCode::InvalidInput);
    assert_eq!(failed(Refused::NamesTooLarge).code(), IpcErrorCode::InvalidInput);

    let missing = failed(UnknownJob);
    assert_eq!(missing.code(), IpcErrorCode::NotFound);
    assert_eq!(failed(AnswerError::UnknownJob).code(), IpcErrorCode::NotFound);
    assert_eq!(failed(AnswerError::StaleQuestion).code(), IpcErrorCode::InvalidInput);
}

#[test]
fn controlling_or_answering_a_job_that_is_not_running_is_not_found() {
    let ops = Ops::default();
    let unknown = JobId(u64::MAX);

    for action in [ControlAction::Pause, ControlAction::Resume, ControlAction::Cancel] {
        let error = ops
            .engine
            .control(unknown, action)
            .map_err(failed)
            .expect_err("unknown job");
        assert_eq!(error.code(), IpcErrorCode::NotFound);
    }

    let error = ops
        .engine
        .answer(unknown, 1, Answer::Skip, false)
        .map_err(failed)
        .expect_err("unknown job");
    assert_eq!(error.code(), IpcErrorCode::NotFound);
}

#[test]
fn subscribing_sends_the_snapshot_first_and_a_later_subscribe_replaces_the_channel() {
    let ops = Ops::default();
    let (first, first_events) = collecting_channel();
    let (second, second_events) = collecting_channel();

    ops.subscribe(first);
    assert!(is_snapshot(&received(&first_events)[0]), "the snapshot comes first");

    ops.subscribe(second);
    assert!(
        is_snapshot(&received(&second_events)[0]),
        "a reload gets its own snapshot"
    );

    let event = OpsEvent::Paused {
        id: JobId(1),
        paused: true,
    };
    deliver(&ops.listener, event.clone());

    assert_eq!(
        received(&second_events).last(),
        Some(&event),
        "the latest channel gets events"
    );
    assert_eq!(
        received(&first_events).len(),
        1,
        "the replaced channel gets nothing more"
    );
}

#[test]
fn events_before_anyone_subscribed_and_after_the_page_closed_are_dropped_quietly() {
    let ops = Ops::default();
    let event = OpsEvent::Paused {
        id: JobId(1),
        paused: false,
    };

    deliver(&ops.listener, event.clone());

    let closed = Channel::new(|_| Err(tauri::Error::Io(io::Error::other("the page is gone"))));
    ops.subscribe(closed);
    deliver(&ops.listener, event);
}

#[test]
fn a_rename_needs_a_folder_and_two_plain_names() {
    let ops = Ops::default();
    let climbing = format!(
        "{}{}..{}x",
        root(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );

    for (path, from, to) in [
        ("relative".to_owned(), "a", "b"),
        (climbing, "a", "b"),
        (root(), "..", "b"),
        (root(), "a", "b/c"),
        (root(), "a", ""),
        (root(), "", "b"),
        (root(), "/etc/passwd", "b"),
    ] {
        let error = block_on(ops.rename(path.clone(), from.to_owned(), to.to_owned())).expect_err(&path);

        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{path} {from} {to}");
    }
}

#[test]
fn renames_are_capped_and_a_finished_one_frees_its_place() {
    let ops = Ops::default();

    let held: Vec<_> = (0..RENAME_THREADS)
        .map(|_| ops.begin_rename().expect("a place"))
        .collect();
    let busy = ops.begin_rename().expect_err("all places taken");
    assert_eq!(busy.code(), IpcErrorCode::Busy);

    // A busy rename is refused without any thread, and before it asks the engine anything.
    let error = block_on(ops.rename(root(), "a".to_owned(), "b".to_owned())).expect_err("busy");
    assert_eq!(error.code(), IpcErrorCode::Busy);

    drop(held);
    assert!(ops.begin_rename().is_ok());
}

#[test]
fn rename_errors_say_what_happened_without_a_path() {
    let exists = rename_error(&io::Error::from(io::ErrorKind::AlreadyExists));
    let missing = rename_error(&io::Error::from(io::ErrorKind::NotFound));
    let denied = rename_error(&io::Error::from(io::ErrorKind::PermissionDenied));
    let other = rename_error(&io::Error::other("disk exploded"));

    assert_eq!(exists.code(), IpcErrorCode::InvalidInput);
    assert_eq!(missing.code(), IpcErrorCode::NotFound);
    assert_eq!(denied.code(), IpcErrorCode::Io);
    assert_eq!(other.code(), IpcErrorCode::Io);

    for error in [exists, missing, denied, other] {
        let text = serde_json::to_string(&error).expect("serializable");
        assert!(!text.contains(std::path::MAIN_SEPARATOR), "{text}");
    }
}

// Needs the engine (`rename_in`, `submit`) from here on.

#[test]
fn renaming_moves_the_item_and_never_replaces_another() {
    let folder = tempfile::tempdir().expect("temp folder");
    let path = folder.path().to_string_lossy().into_owned();
    std::fs::write(folder.path().join("a.txt"), "a").expect("file");
    std::fs::write(folder.path().join("taken.txt"), "taken").expect("file");
    let ops = Ops::default();

    block_on(ops.rename(path.clone(), "a.txt".to_owned(), "b.txt".to_owned())).expect("renamed");
    assert_eq!(
        std::fs::read_to_string(folder.path().join("b.txt")).expect("moved"),
        "a"
    );
    assert!(!folder.path().join("a.txt").exists());

    let error = block_on(ops.rename(path.clone(), "b.txt".to_owned(), "taken.txt".to_owned())).expect_err("exists");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert_eq!(
        std::fs::read_to_string(folder.path().join("taken.txt")).expect("kept"),
        "taken"
    );
    assert!(folder.path().join("b.txt").exists());

    let error = block_on(ops.rename(path, "none.txt".to_owned(), "c.txt".to_owned())).expect_err("missing");
    assert_eq!(error.code(), IpcErrorCode::NotFound);
}

/// Two folders, `a.txt` in the first and (when `existing` is given) in the second, an engine that reports to a
/// channel, and a copy of `a.txt` started from the first into the second.
struct Started {
    _from: tempfile::TempDir,
    to: tempfile::TempDir,
    ops: Ops,
    events: Arc<Mutex<Vec<OpsEvent>>>,
    id: JobId,
}

fn start_copy(names_to_copy: &[&str], existing: Option<&str>) -> Started {
    let from = tempfile::tempdir().expect("source folder");
    let to = tempfile::tempdir().expect("target folder");
    std::fs::write(from.path().join("a.txt"), "new").expect("file");
    if let Some(text) = existing {
        std::fs::write(to.path().join("a.txt"), text).expect("file");
    }
    let ops = Ops::default();
    let (channel, events) = collecting_channel();
    ops.subscribe(channel);

    let id = ops
        .start(
            OpKind::Copy,
            from.path().to_string_lossy().into_owned(),
            names(names_to_copy),
            to.path().to_string_lossy().into_owned(),
            false,
        )
        .expect("accepted");

    Started {
        _from: from,
        to,
        ops,
        events,
        id,
    }
}

/// Waits (up to 20 seconds) until an event that `wanted` accepts has been received.
fn wait_for(events: &Arc<Mutex<Vec<OpsEvent>>>, wanted: impl Fn(&OpsEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);

    while !received(events).iter().any(&wanted) {
        assert!(Instant::now() < deadline, "the event came");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_started_copy_reports_added_then_ended_over_the_subscribed_channel() {
    let started = start_copy(&["a.txt", "a.txt"], None);

    wait_for(&started.events, |event| matches!(event, OpsEvent::Ended { .. }));

    let events = received(&started.events);
    assert!(matches!(&events[1], OpsEvent::Added { job } if job.id == started.id));
    assert_eq!(
        std::fs::read_to_string(started.to.path().join("a.txt")).expect("copied"),
        "new"
    );
}

#[test]
fn shutting_down_cancels_a_job_waiting_for_an_answer_and_returns_once_it_has_ended() {
    let started = start_copy(&["a.txt"], Some("old"));
    wait_for(&started.events, |event| matches!(event, OpsEvent::Conflict { .. }));

    let before = Instant::now();
    started.ops.shutdown();

    assert!(before.elapsed() < Duration::from_secs(2));
    assert!(
        received(&started.events)
            .iter()
            .any(|event| matches!(event, OpsEvent::Ended { id, .. } if *id == started.id)),
        "the job had ended by the time shutdown returned"
    );
    assert_eq!(
        std::fs::read_to_string(started.to.path().join("a.txt")).expect("kept"),
        "old"
    );
}
