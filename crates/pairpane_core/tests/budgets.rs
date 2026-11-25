//! The performance budgets of the quality gate (#24), checked in CI on every system (the `Performance budgets` job of
//! `.github/workflows/ci.yml`). They are ignored by a plain `cargo test`, which builds without optimisations; run them with
//! `cargo test --release -p pairpane_core --test budgets -- --ignored --test-threads=1`. See `docs/performance-budgets.md`.
//!
//! Each budget prints what it measured next to its limit and, in CI, adds a row to the job summary
//! (`GITHUB_STEP_SUMMARY`), so the numbers are tracked from run to run.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pairpane_core::listing::list;
use pairpane_core::model::{ListEvent, OpKind, OpsEvent, Outcome};
use pairpane_core::ops::{Engine, Request, Task};
use tempfile::TempDir;

#[path = "support/huge_folder.rs"]
mod huge_folder;

use huge_folder::{ENTRIES, huge_folder};

/// The longest the first batch of that folder may take: what the pane draws first.
const FIRST_BATCH_BUDGET: Duration = Duration::from_millis(100);
/// Size of the file the copy budget copies: well beyond any cache line, short enough for a CI disk.
const COPY_BYTES: usize = 256 * 1024 * 1024;
/// The engine's copy must run at this share of a plain buffered copy on the same disk, or better.
const COPY_SHARE: f64 = 0.8;
/// Runs of each measurement; the median is what counts, so one slow run on a shared runner does not fail it.
const RUNS: usize = 5;
/// How long the copy budget waits for one copy job before it fails instead of hanging.
const PATIENCE: Duration = Duration::from_secs(300);

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn millis(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

/// Prints one measured budget and adds it to the CI job summary when there is one.
fn report(name: &str, measured: &str, budget: &str, met: bool) {
    let verdict = if met { "met" } else { "MISSED" };
    println!("budget {name}: {measured} (budget {budget}) {verdict}");

    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let row = format!(
            "| {} | {name} | {measured} | {budget} | {verdict} |\n",
            std::env::consts::OS
        );
        let written = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut summary| summary.write_all(row.as_bytes()));
        written.expect("the job summary takes a row");
    }
}

/// The time to the first batch of the folder, and to its end.
fn time_listing(folder: &Path) -> (Duration, Duration) {
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let mut first = None;

    list(folder, false, &cancel, |event| {
        if matches!(event, ListEvent::Batch { .. }) && first.is_none() {
            first = Some(start.elapsed());
        }
    })
    .expect("listable");

    (first.expect("a batch came"), start.elapsed())
}

#[test]
#[ignore = "a performance budget: run in release mode with --ignored (see the module docs)"]
fn a_folder_of_100k_entries_shows_its_first_batch_within_budget() {
    let folder = huge_folder();
    // A first read fills the system's caches, as the folder a user opens again would have them.
    time_listing(folder.path());

    let (firsts, wholes): (Vec<_>, Vec<_>) = (0..RUNS).map(|_| time_listing(folder.path())).unzip();
    let first = median(firsts);
    let whole = median(wholes);

    let met = first <= FIRST_BATCH_BUDGET;
    report(
        "listing 100k: first batch",
        &millis(first),
        &format!("≤ {}", millis(FIRST_BATCH_BUDGET)),
        met,
    );
    report("listing 100k: all entries", &millis(whole), "reported only", true);
    assert!(
        met,
        "the first batch of {ENTRIES} entries took {first:?}, over {FIRST_BATCH_BUDGET:?}"
    );
}

/// A plain copy, the way any program copies: a loop over a 1 MiB buffer, no system fast path.
fn plain_copy(source: &Path, target: &Path) -> io::Result<()> {
    let mut from = File::open(source)?;
    let mut to = File::create(target)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let read = from.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        to.write_all(&buffer[..read])?;
    }
}

/// An engine whose events land in a channel.
fn engine() -> (Engine, mpsc::Receiver<OpsEvent>) {
    let engine = Engine::new();
    let (sender, events) = mpsc::channel();
    let sender = Mutex::new(sender);
    engine.set_sink(Arc::new(move |event| {
        let _ = sender.lock().expect("sender").send(event);
    }));
    (engine, events)
}

/// Copies `name` of `from` into `to` with the engine and waits for the job to end.
fn engine_copy(engine: &Engine, events: &mpsc::Receiver<OpsEvent>, from: &Path, name: &str, to: &Path) {
    let request = Request::task(
        from.to_path_buf(),
        vec![name.to_owned()],
        to.to_path_buf(),
        Task::Transfer,
    );
    assert_eq!(request.kind, OpKind::Copy);
    let job = engine.submit(request).expect("the copy is accepted");

    loop {
        match events.recv_timeout(PATIENCE).expect("the copy ends in time") {
            OpsEvent::Ended {
                id, outcome, errors, ..
            } if id == job => {
                assert!(
                    matches!(outcome, Outcome::Done) && errors.is_empty(),
                    "{outcome:?} {errors:?}"
                );
                return;
            }
            _ => {}
        }
    }
}

/// Makes `dir` an empty folder. Removing what an earlier run wrote is tried a few times: on Windows a virus scanner or the
/// indexer may still hold a file it was just written, and the removal fails until it lets go.
fn fresh_folder(dir: &Path) {
    let mut tries = 0;
    while dir.exists() {
        match fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(error) if tries < 10 => {
                tries += 1;
                eprintln!("removing {} failed ({error}); trying again", dir.display());
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => panic!("cannot remove {}: {error}", dir.display()),
        }
    }
    fs::create_dir(dir).expect("target dir");
}

#[test]
#[ignore = "a performance budget: run in release mode with --ignored (see the module docs)"]
fn the_engine_copies_a_large_file_near_the_speed_of_a_plain_copy() {
    let temp = TempDir::new().expect("temp dir");
    let source_dir = temp.path().join("source");
    let plain_dir = temp.path().join("plain");
    let engine_dir = temp.path().join("engine");
    fs::create_dir(&source_dir).expect("source dir");

    // Bytes that do not compress or deduplicate, so neither copy gets a shortcut the other does not.
    let mut bytes = vec![0_u8; COPY_BYTES];
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    for chunk in bytes.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
    let source = source_dir.join("big.bin");
    fs::write(&source, &bytes).expect("source file");
    drop(bytes);

    let (engine, events) = engine();
    let mut plain = Vec::new();
    let mut ours = Vec::new();

    // The two take turns, and which goes first alternates: a runner that slows down mid-run slows both alike, and the
    // second copy, which may run while the first one's bytes are still written back, is each of them as often.
    for run in 0..RUNS {
        for dir in [&plain_dir, &engine_dir] {
            fresh_folder(dir);
        }

        let mut time_plain = || {
            let start = Instant::now();
            plain_copy(&source, &plain_dir.join("big.bin")).expect("plain copy");
            plain.push(start.elapsed());
        };
        let mut time_ours = || {
            let start = Instant::now();
            engine_copy(&engine, &events, &source_dir, "big.bin", &engine_dir);
            ours.push(start.elapsed());
        };
        if run % 2 == 0 {
            time_plain();
            time_ours();
        } else {
            time_ours();
            time_plain();
        }

        assert_eq!(
            fs::metadata(engine_dir.join("big.bin")).expect("copied").len(),
            COPY_BYTES as u64
        );
    }

    let plain = median(plain);
    let ours = median(ours);
    let mib = COPY_BYTES as f64 / (1024.0 * 1024.0);
    let speed = |time: Duration| mib / time.as_secs_f64();
    let share = speed(ours) / speed(plain);

    let met = share >= COPY_SHARE;
    report(
        "copy 256 MiB: engine vs plain copy",
        &format!(
            "{:.0} MiB/s vs {:.0} MiB/s ({:.0} %)",
            speed(ours),
            speed(plain),
            share * 100.0
        ),
        &format!("≥ {:.0} %", COPY_SHARE * 100.0),
        met,
    );
    assert!(engine.shutdown(Duration::from_secs(10)), "the engine stops");
    assert!(
        met,
        "the engine copied at {:.0} % of a plain copy, under {:.0} %",
        share * 100.0,
        COPY_SHARE * 100.0
    );
}
