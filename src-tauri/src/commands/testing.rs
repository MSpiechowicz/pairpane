//! Helpers shared by the command tests.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use tauri::ipc::{Channel, InvokeResponseBody};

/// How long a test waits for something that must happen before it calls it a hang.
pub(super) const PATIENCE: Duration = Duration::from_secs(20);

/// Waits until `ready` says so, and fails the test when it takes longer than [`PATIENCE`].
pub(super) fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let limit = Instant::now() + PATIENCE;

    while !ready() {
        assert!(Instant::now() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A gate that holds the page's channel: while it is shut, a scan waits in the delivery of an event, so it runs on.
pub(super) struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    pub(super) fn release(&self) {
        *self.open.lock().expect("not poisoned") = true;
        self.changed.notify_all();
    }

    fn pass(&self) {
        let open = self.open.lock().expect("not poisoned");
        let (_open, result) = self
            .changed
            .wait_timeout_while(open, PATIENCE, |open| !*open)
            .expect("not poisoned");

        assert!(!result.timed_out(), "the gate was never opened");
    }
}

/// A channel whose events are recorded and then held at the gate, until it is released.
pub(super) fn gated<T: DeserializeOwned + Send + 'static>() -> (Channel<T>, Arc<Gate>, Arc<Mutex<Vec<T>>>) {
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
    });
    let log = Arc::new(Mutex::new(Vec::new()));
    let (held, recorded) = (Arc::clone(&gate), Arc::clone(&log));

    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(text) = body else {
            panic!("events are sent as JSON");
        };
        recorded
            .lock()
            .expect("not poisoned")
            .push(serde_json::from_str(&text).expect("an event"));
        held.pass();

        Ok(())
    });

    (channel, gate, log)
}

/// An absolute folder that the tests never touch on disk.
pub(super) fn root() -> String {
    if cfg!(windows) { "C:\\data" } else { "/data" }.to_owned()
}

/// A second absolute folder.
pub(super) fn elsewhere() -> String {
    if cfg!(windows) { "C:\\x" } else { "/x" }.to_owned()
}

pub(super) fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// A folder holding the files, and its path as the page would send it.
pub(super) fn folder_with(files: &[(&str, &[u8])]) -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().expect("folder");
    for (name, bytes) in files {
        std::fs::write(folder.path().join(name), bytes).expect("file");
    }
    let path = folder.path().to_string_lossy().into_owned();

    (folder, path)
}
