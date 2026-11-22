//! Pack jobs on real folders: what the archives hold read back through the readers of the crate, the links and special
//! files that are left out, the names that fail, the question about a taken name, a cancel, a pause and the limits.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::archive::{EntryType, Flow, Input, Password, build_index, open_source};
use crate::model::{ArchiveFormat, ControlAction, JobPhase, OpsEvent, Outcome};
use crate::model::{PackFormat, PackLevel};

use super::hooks::Hooks;
use super::testing::{Ended, Rig, has_no_partial_files, pattern, phases, put, read, set_times, task, temp, tree};
use super::{PackJob, Request, Task};

/// The formats a pack job writes, with the archive each is named by.
pub(super) const FORMATS: [(PackFormat, ArchiveFormat, &str); 3] = [
    (PackFormat::Zip, ArchiveFormat::Zip, "out.zip"),
    (PackFormat::TarZst, ArchiveFormat::TarZst, "out.tar.zst"),
    (PackFormat::SevenZ, ArchiveFormat::SevenZ, "out.7z"),
];

/// The items of the scene.
pub(super) const ITEMS: [&str; 4] = ["a.txt", "docs", "empty", "z.bin"];

/// An entry as the readers show it: path, whether it is a folder, mode, time and content.
pub(super) type Seen = (String, bool, Option<u32>, Option<i64>, Vec<u8>);

pub(super) fn options(format: PackFormat, level: PackLevel) -> PackJob {
    PackJob {
        format,
        level,
        password: None,
        encrypt_names: false,
    }
}

fn pack_task(name: &str, options: PackJob) -> Task {
    Task::Pack {
        name: name.to_owned(),
        options,
    }
}

/// The job that packs `names` of `root/src` into the archive `name` in `root/dst`.
pub(super) fn pack(root: &Path, names: &[&str], name: &str, options: PackJob) -> Request {
    task(&root.join("src"), names, &root.join("dst"), pack_task(name, options))
}

pub(super) fn zip(root: &Path) -> Request {
    pack(root, &ITEMS, "out.zip", options(PackFormat::Zip, PackLevel::Normal))
}

pub(super) fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// `src` with the files `a.txt`, `docs/b.txt`, `docs/deep/c.txt` and `z.bin` and the empty folder `empty`, each with a mode
/// and a time (whole seconds, which every format keeps); and the empty `dst`.
pub(super) fn scene() -> TempDir {
    let root = temp();
    let src = root.path().join("src");
    put(root.path(), "src/a.txt", b"alpha");
    put(root.path(), "src/docs/b.txt", b"beta");
    put(root.path(), "src/docs/deep/c.txt", b"gamma");
    put(root.path(), "src/z.bin", &pattern(70_000));
    fs::create_dir_all(src.join("empty")).expect("folder");
    fs::create_dir_all(root.path().join("dst")).expect("folder");

    // The times of the folders go last: what is put into a folder changes its time.
    let files = [("a.txt", 0o644, 1_000_000_001), ("docs/b.txt", 0o644, 1_000_000_002)];
    let more = [
        ("docs/deep/c.txt", 0o600, 1_000_000_003),
        ("z.bin", 0o755, 1_000_000_004),
    ];
    let folders = [
        ("docs/deep", 0o750, 1_000_000_005),
        ("docs", 0o755, 1_000_000_006),
        ("empty", 0o700, 1_000_000_007),
    ];

    for (path, mode, seconds) in files.into_iter().chain(more).chain(folders) {
        set_mode(&src.join(path), mode);
        set_times(&src.join(path), at(seconds));
    }

    root
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// What a read-back of the scene's items shows, in the archive's order.
pub(super) fn expected() -> Vec<Seen> {
    let item = |path: &str, mode: u32, seconds: i64, content: Option<&[u8]>| -> Seen {
        (
            path.to_owned(),
            content.is_none(),
            cfg!(unix).then_some(mode),
            Some(seconds * 1000),
            content.unwrap_or_default().to_vec(),
        )
    };

    vec![
        item("a.txt", 0o644, 1_000_000_001, Some(b"alpha")),
        item("docs", 0o755, 1_000_000_006, None),
        item("docs/b.txt", 0o644, 1_000_000_002, Some(b"beta")),
        item("docs/deep", 0o750, 1_000_000_005, None),
        item("docs/deep/c.txt", 0o600, 1_000_000_003, Some(b"gamma")),
        item("empty", 0o700, 1_000_000_007, None),
        item("z.bin", 0o755, 1_000_000_004, Some(&pattern(70_000))),
    ]
}

/// The bytes of the files of the scene's items.
pub(super) fn expected_bytes() -> u64 {
    expected().iter().map(|seen| seen.4.len() as u64).sum()
}

/// Every entry of the archive at `path`, listed and extracted through the readers of the crate.
pub(super) fn read_back(path: &Path, format: ArchiveFormat, password: Option<&Password>) -> std::io::Result<Vec<Seen>> {
    let bytes = read(path);
    let cancel = AtomicBool::new(false);
    let input = Input {
        file: &bytes,
        path: None,
    };
    let index = build_index(format, &input, &cancel, password)?;
    let wanted: Vec<u32> = (0..index.entries().len() as u32).collect();
    let mut contents = Vec::new();

    open_source(&index, &input, &cancel, password)?.extract(&wanted, &mut |_, data| {
        let mut content = Vec::new();
        data.read_to_end(&mut content)?;
        contents.push(content);

        Ok(Flow::Continue)
    })?;

    Ok(index
        .entries()
        .iter()
        .zip(contents)
        .map(|(entry, content)| {
            (
                entry.path.clone(),
                entry.kind == EntryType::Dir,
                entry.mode.filter(|_| cfg!(unix)),
                entry.modified,
                content,
            )
        })
        .collect())
}

pub(super) fn archived(root: &TempDir, name: &str, format: ArchiveFormat) -> Vec<Seen> {
    read_back(&root.path().join("dst").join(name), format, None).expect("an archive that reads back")
}

pub(super) fn dst(root: &TempDir) -> Vec<String> {
    tree(&root.path().join("dst"))
}

pub(super) fn the_paths(entries: &[Seen]) -> Vec<&str> {
    entries.iter().map(|seen| seen.0.as_str()).collect()
}

#[test]
fn every_format_and_level_reads_back_as_it_was_packed() {
    for (format, kind, name) in FORMATS {
        for level in [PackLevel::Store, PackLevel::Normal, PackLevel::Maximum] {
            let root = scene();

            let ended = Rig::new().run(pack(root.path(), &ITEMS, name, options(format, level)));

            assert_eq!(ended.outcome, Outcome::Done, "{format:?} at {level:?}");
            assert_eq!((ended.skipped, ended.errors.len()), (0, 0), "{format:?} at {level:?}");
            assert_eq!(archived(&root, name, kind), expected(), "{format:?} at {level:?}");
            assert_eq!(dst(&root), [name], "no temporary file is left");
        }
    }
}

#[test]
fn the_progress_counts_the_bytes_of_the_files_through_scanning_and_packing() {
    for (format, _, name) in FORMATS {
        let root = scene();
        let mut rig = Rig::new();

        let job = rig.start(pack(root.path(), &ITEMS, name, options(format, PackLevel::Fast)));
        rig.wait_ended(job);

        let Some(OpsEvent::Progress {
            done_bytes,
            total_bytes,
            done_files,
            total_files,
            ..
        }) = rig
            .seen
            .iter()
            .rev()
            .find(|event| matches!(event, OpsEvent::Progress { .. }))
        else {
            panic!("a progress report");
        };
        let last = (*done_bytes, *total_bytes, *done_files, *total_files);

        assert_eq!(last, (expected_bytes(), expected_bytes(), 4, 4), "{format:?}");
        assert!(
            phases(&rig).ends_with(&[JobPhase::Scanning, JobPhase::Packing]),
            "{format:?}"
        );
    }
}

#[test]
fn the_items_are_packed_in_name_order_whatever_order_they_were_selected_in() {
    let root = scene();

    let ended = Rig::new().run(pack(
        root.path(),
        &["z.bin", "empty", "docs", "a.txt"],
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    ));

    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(archived(&root, "out.zip", ArchiveFormat::Zip), expected());
}

#[test]
fn a_chosen_folder_alone_is_packed_with_what_is_below_it() {
    let root = scene();

    Rig::new().run(pack(
        root.path(),
        &["docs"],
        "out.7z",
        options(PackFormat::SevenZ, PackLevel::Fast),
    ));

    assert_eq!(
        the_paths(&archived(&root, "out.7z", ArchiveFormat::SevenZ)),
        ["docs", "docs/b.txt", "docs/deep", "docs/deep/c.txt"]
    );
}

#[test]
fn a_password_encrypts_the_files_and_for_a_7z_the_names() {
    let root = scene();
    let password = Password::new("open sesame".to_owned());
    let locked = |format, encrypt_names| PackJob {
        password: Some(std::sync::Arc::new(password.clone())),
        encrypt_names,
        ..options(format, PackLevel::Normal)
    };
    let mut rig = Rig::new();

    rig.run(pack(root.path(), &ITEMS, "out.zip", locked(PackFormat::Zip, false)));
    rig.run(pack(root.path(), &ITEMS, "out.7z", locked(PackFormat::SevenZ, true)));

    let (zip, sevenz) = (root.path().join("dst/out.zip"), root.path().join("dst/out.7z"));
    assert_eq!(
        read_back(&zip, ArchiveFormat::Zip, Some(&password)).expect("reads"),
        expected()
    );
    assert!(read_back(&zip, ArchiveFormat::Zip, None).is_err());
    assert_eq!(
        read_back(&sevenz, ArchiveFormat::SevenZ, Some(&password)).expect("reads"),
        expected()
    );
    assert!(read_back(&sevenz, ArchiveFormat::SevenZ, None).is_err());
}

#[test]
fn a_job_with_nothing_to_pack_fails_and_writes_nothing() {
    let root = scene();

    let ended = Rig::new().run(pack(
        root.path(),
        &["gone"],
        "out.zip",
        options(PackFormat::Zip, PackLevel::Normal),
    ));

    assert!(matches!(ended.outcome, Outcome::Failed { .. }), "{:?}", ended.outcome);
    assert_eq!(ended.errors.len(), 1, "the missing item is reported");
    assert_eq!(dst(&root), Vec::<String>::new());
}

/// Runs `request` with the limits `limits` and gives how it ended.
fn limited(root: &TempDir, limits: (usize, usize)) -> Ended {
    let hooks = Hooks {
        pack_limits: Some(limits),
        ..Hooks::default()
    };

    Rig::hooked(|set| *set = hooks).run(zip(root.path()))
}

#[test]
fn more_entries_or_bytes_of_names_than_the_readers_index_fail_the_job_before_anything_is_written() {
    let root = scene();

    for limits in [(6, usize::MAX), (usize::MAX, 20)] {
        let ended = limited(&root, limits);

        let Outcome::Failed { error } = ended.outcome else {
            panic!("the job must fail: {:?}", ended.outcome);
        };
        assert_eq!(error.message, "there are too many files to pack in one archive");
        assert!(ended.errors.is_empty(), "it is not an item that failed");
        assert_eq!(dst(&root), Vec::<String>::new());
    }

    // Exactly as many as the limit takes are packed: seven entries, and 7 + 4 + 10 + 9 + 15 + 5 + 5 bytes of names.
    let ended = limited(&root, (7, 55));
    assert_eq!(ended.outcome, Outcome::Done);
    assert_eq!(dst(&root), ["out.zip"]);
}

#[test]
fn a_cancel_in_the_middle_of_a_file_leaves_no_archive_and_no_temporary_file() {
    for (format, _, name) in FORMATS {
        let root = scene();
        put(root.path(), "src/big.bin", &pattern(16 * 1024 * 1024));
        let hooks = Hooks {
            every_chunk: true,
            ..Hooks::default()
        };
        // The listener cancels at the first progress that shows bytes read, so the cancel lands mid-file.
        let mut rig = Rig::with(hooks, |engine, event| {
            if let OpsEvent::Progress {
                id, done_bytes: 2.., ..
            } = event
            {
                let _ = engine.control(*id, ControlAction::Cancel);
            }
        });

        let ended = rig.run(pack(root.path(), &["big.bin"], name, options(format, PackLevel::Store)));

        assert_eq!(ended.outcome, Outcome::Cancelled, "{format:?}");
        assert_eq!(dst(&root), Vec::<String>::new(), "{format:?}");
        assert!(has_no_partial_files(&root.path().join("src")));
    }
}

#[test]
fn a_disk_with_less_room_than_the_reserve_fails_the_job_and_removes_the_temporary_file() {
    let root = scene();
    let hooks = Hooks {
        free_reserve: Some(u64::MAX / 2),
        ..Hooks::default()
    };

    let ended = Rig::hooked(|set| *set = hooks).run(zip(root.path()));

    let Outcome::Failed { error } = ended.outcome else {
        panic!("the job must fail: {:?}", ended.outcome);
    };
    assert_eq!(error.message, "there is not enough free space left on the disk");
    assert_eq!(dst(&root), Vec::<String>::new());
}

#[test]
fn a_paused_pack_waits_and_goes_on_when_resumed() {
    let root = scene();
    let mut rig = Rig::paused_in(JobPhase::Packing);

    let job = rig.start(zip(root.path()));
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    rig.drain();

    assert_eq!(rig.ended_count(job), 0, "it waits");
    assert!(
        !dst(&root).contains(&"out.zip".to_owned()),
        "the archive is not in place yet"
    );

    rig.engine.control(job, ControlAction::Resume).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Done);
    assert_eq!(archived(&root, "out.zip", ArchiveFormat::Zip), expected());
}

#[test]
fn the_temporary_file_of_the_job_itself_is_not_packed() {
    let root = scene();
    let mut rig = Rig::paused_at_start();

    let job = rig.start(zip(root.path()));
    rig.wait(|event| matches!(event, OpsEvent::Paused { paused: true, .. }));
    // What a pack into a folder that is packed itself would have there: the job's own file, and another run's.
    let own = format!(".pairpane-{}-{}-1.part", crate::ops::run_token(), job.0);
    put(root.path(), &format!("src/docs/{own}"), b"half");
    put(root.path(), "src/docs/.pairpane-other-1-1.part", b"another");
    rig.engine.control(job, ControlAction::Resume).expect("known job");

    assert_eq!(rig.wait_ended(job).outcome, Outcome::Done);
    let paths = archived(&root, "out.zip", ArchiveFormat::Zip);
    assert!(!the_paths(&paths).iter().any(|path| path.ends_with(&own)));
    assert!(the_paths(&paths).contains(&"docs/.pairpane-other-1-1.part"));
}
