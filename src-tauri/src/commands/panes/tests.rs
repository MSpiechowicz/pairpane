//! Tests of the panes' slots: what an open keeps from the one before, the size jobs' folder, the listing cap and the
//! path rules.

use super::*;

fn root() -> &'static str {
    if cfg!(windows) { "C:\\data" } else { "/data" }
}

/// Opens `folder` in the left pane of `slots` and returns what it replaced.
fn open(slots: &Slots, generation: u64, folder: &Path) -> Option<Slot> {
    let (earlier, _) = replace_slot(
        &mut lock(slots),
        PaneId::Left,
        generation,
        SlotKey::Folder(folder.to_owned()),
        &Arc::new(AtomicBool::new(false)),
        &Channel::new(|_| Ok(())),
    );

    earlier
}

/// The listing of `folder` opened as `generation` has started.
fn start(slots: &Slots, generation: u64, folder: &Path) {
    claim_sizing(slots, PaneId::Left, generation, &SlotKey::Folder(folder.to_owned()));
}

/// Starts a size job for `name` inside `path` in the left pane.
fn size_job(slots: &Slots, path: &Path, name: &str) -> CommandResult<Arc<AtomicBool>> {
    let slot = lock(slots)
        .get_mut(&PaneId::Left)
        .map(|slot| slot.sizing.begin(path.to_string_lossy().into_owned(), name));

    slot.expect("open pane").map(|job| job.stop)
}

/// A pane whose folder (generation 1) has listed and has a size job running in it.
fn measuring() -> (Slots, PathBuf, Arc<AtomicBool>) {
    let slots = Slots::default();
    let folder = PathBuf::from(root());
    open(&slots, 1, &folder);
    start(&slots, 1, &folder);
    let stop = size_job(&slots, &folder, "photos").expect("job");

    (slots, folder, stop)
}

#[test]
fn a_folder_and_an_archive_at_the_same_path_are_different_places_so_neither_keeps_the_others_watch() {
    let path = PathBuf::from(root());

    assert_eq!(SlotKey::Folder(path.clone()), SlotKey::Folder(path.clone()));
    assert_ne!(SlotKey::Folder(path.clone()), SlotKey::Archive(path.clone()));
    assert_eq!(SlotKey::Archive(path.clone()).measured(), Some(path.as_path()));
}

#[test]
fn a_child_must_be_a_plain_name() {
    let root = if cfg!(windows) { "C:\\data" } else { "/data" };

    let joined = resolve(Some(root.to_owned()), Some("photos")).expect("plain name");
    assert_eq!(joined, Path::new(root).join("photos"));

    for bad in ["..", ".", "a/b", "a/", "/etc", ""] {
        let error = resolve(Some(root.to_owned()), Some(bad)).expect_err(bad);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
    }
    #[cfg(windows)]
    for bad in ["a\\b", "C:", "C:\\x"] {
        assert!(resolve(Some(root.to_owned()), Some(bad)).is_err(), "{bad}");
    }
}

#[test]
fn relative_climbing_and_network_paths_are_refused_and_no_path_means_home() {
    let root = if cfg!(windows) { "C:\\data" } else { "/data" };
    let climbing = format!("{root}{}..{}x", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
    for bad in ["relative/dir", climbing.as_str()] {
        let error = resolve(Some(bad.to_owned()), None).expect_err(bad);
        assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{bad}");
    }
    #[cfg(windows)]
    for bad in [r"\\attacker\x", "//attacker/x", r"\\?\UNC\a\b", r"\\.\pipe\x"] {
        assert!(resolve(Some(bad.to_owned()), None).is_err(), "{bad}");
    }

    if let Some(home) = std::env::home_dir() {
        assert_eq!(resolve(None, None).expect("home"), home);
    }
}

#[test]
fn a_pane_showing_results_has_no_folder_to_measure_and_listing_them_stops_the_size_job() {
    let (slots, folder, stop) = measuring();

    assert_eq!(SlotKey::Folder(folder.clone()).measured(), Some(folder.as_path()));
    assert_eq!(
        SlotKey::Archive(folder.join("pack.zip")).measured(),
        Some(folder.join("pack.zip").as_path())
    );
    assert_eq!(SlotKey::Results(FindId(7)).measured(), None);
    assert_ne!(SlotKey::Results(FindId(7)), SlotKey::Results(FindId(8)));

    let results = SlotKey::Results(FindId(7));
    replace_slot(
        &mut lock(&slots),
        PaneId::Left,
        2,
        results.clone(),
        &Arc::new(AtomicBool::new(false)),
        &Channel::new(|_| Ok(())),
    );
    claim_sizing(&slots, PaneId::Left, 2, &results);

    assert!(stop.load(Ordering::Relaxed), "listing the results stops the job");
    assert!(
        size_job(&slots, &folder, "photos").is_err(),
        "nothing is measured in results"
    );
}

#[test]
fn listing_errors_tell_the_user_what_happened() {
    let not_found = list_error(&io::Error::from(io::ErrorKind::NotFound));
    let denied = list_error(&io::Error::from(io::ErrorKind::PermissionDenied));

    assert_eq!(not_found.code, IpcErrorCode::NotFound);
    assert_eq!(denied.code, IpcErrorCode::Io);
    assert!(denied.message.contains("permission"));
}

#[test]
fn reopening_the_same_folder_keeps_its_size_job_and_listing_another_stops_it() {
    let (slots, folder, stop) = measuring();
    let folder = folder.as_path();

    let earlier = open(&slots, 2, folder).expect("replaced");
    start(&slots, 2, folder);
    assert!(earlier.cancel.load(Ordering::Relaxed), "the earlier listing stops");
    assert!(!stop.load(Ordering::Relaxed), "a reload leaves the size job running");

    let photos = folder.join("photos");
    open(&slots, 3, &photos);
    start(&slots, 3, &photos);
    assert!(stop.load(Ordering::Relaxed), "a listed folder stops the job");
    assert!(size_job(&slots, folder, "photos").is_err());
    assert!(size_job(&slots, &photos, "2024").is_ok());
}

#[test]
fn a_failed_open_leaves_the_size_job_and_target_of_the_shown_folder() {
    let (slots, folder, stop) = measuring();
    let folder = folder.as_path();

    // The open of `locked` fails: its listing never starts.
    let locked = folder.join("locked");
    let earlier = open(&slots, 2, &locked).expect("replaced");

    assert!(earlier.cancel.load(Ordering::Relaxed), "the earlier listing stops");
    assert!(!stop.load(Ordering::Relaxed), "the shown folder's job keeps running");
    assert!(
        size_job(&slots, folder, "other").is_ok(),
        "its folders can still be measured"
    );
    assert!(
        size_job(&slots, &locked, "x").is_err(),
        "the folder that failed is not the target"
    );
}

#[test]
fn a_late_listing_never_takes_the_size_jobs_from_a_newer_open() {
    let (slots, folder, stop) = measuring();
    let folder = folder.as_path();

    let photos = folder.join("photos");
    let other = folder.join("other");
    open(&slots, 2, &photos);
    open(&slots, 3, &other);
    start(&slots, 2, &photos);

    assert!(
        !stop.load(Ordering::Relaxed),
        "the superseded open does not move the jobs"
    );

    start(&slots, 3, &other);
    assert!(stop.load(Ordering::Relaxed), "the latest open does");
    assert!(size_job(&slots, folder, "photos").is_err());
    assert!(size_job(&slots, &other, "x").is_ok());
}

#[test]
fn each_pane_has_its_own_listing_cap_and_a_finished_listing_frees_a_place() {
    let panes = Panes::default();

    let mut running: Vec<_> = (0..LISTING_THREADS)
        .map(|_| panes.listings(PaneId::Left).acquire(LISTING_THREADS).expect("place"))
        .collect();
    assert!(
        panes.listings(PaneId::Left).acquire(LISTING_THREADS).is_none(),
        "the left pane is full"
    );
    assert!(
        panes.listings(PaneId::Right).acquire(LISTING_THREADS).is_some(),
        "the right pane is not"
    );

    running.pop();
    assert!(panes.listings(PaneId::Left).acquire(LISTING_THREADS).is_some());
}
