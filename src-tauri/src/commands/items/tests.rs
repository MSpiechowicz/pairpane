//! Tests of the item commands: what is refused before a thread is taken, the caps, the texts the page gets, the undo
//! record and the changes themselves in a temporary folder. Nothing here moves an item to the real Trash.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pairpane_core::ops::MAX_NAME_UNITS;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::testing::{elsewhere, names, root};

fn path_of(folder: &tempfile::TempDir) -> String {
    folder.path().to_string_lossy().into_owned()
}

/// Takes every place of `cap`.
fn fill(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max).map(|_| cap.acquire(max).expect("a place")).collect()
}

/// The paths an opener was asked to open.
type Asked = Arc<Mutex<Vec<String>>>;

/// An opener that records the path it was asked to open and succeeds.
fn recording_opener() -> (impl FnOnce(String) -> Result<(), String> + Send + 'static, Asked) {
    let asked = Asked::default();
    let sink = Arc::clone(&asked);

    (
        move |path| {
            sink.lock().expect("not poisoned").push(path);

            Ok(())
        },
        asked,
    )
}

fn opened(list: &Asked) -> Vec<String> {
    list.lock().expect("not poisoned").clone()
}

fn never_open(_path: String) -> Result<(), String> {
    panic!("the file must not be opened");
}

fn code<T: std::fmt::Debug>(result: CommandResult<T>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

fn trashed(names: &[&str]) -> Trashed {
    let mut record = Trashed::default();
    record.folder = root().into();
    record.names = self::names(names);
    record.from = 1;
    record.to = 2;

    record
}

#[test]
fn bad_requests_are_refused_before_a_thread_is_taken_and_good_ones_wait_for_a_place() {
    let items = Items::default();
    let _quick = fill(&items.quick, QUICK_THREADS);
    let _deletes = fill(&items.deletes, DELETE_THREADS);
    let too_long = "n".repeat(MAX_NAME_UNITS + 1);

    // Refused whatever the caps say.
    for bad in [
        vec![],
        names(&["a/b"]),
        names(&[".."]),
        names(&[""]),
        names(&[&too_long]),
    ] {
        assert_eq!(
            code(block_on(items.trash(root(), bad.clone()))),
            IpcErrorCode::InvalidInput
        );
        assert_eq!(code(block_on(items.delete(root(), bad))), IpcErrorCode::InvalidInput);
    }
    assert_eq!(
        code(block_on(items.trash("rel".to_owned(), names(&["a"])))),
        IpcErrorCode::InvalidInput
    );
    for bad in ["", "a//b", "/a", "a/", "..", "a/../b", &too_long] {
        assert_eq!(
            code(block_on(items.mkdir(root(), bad.to_owned()))),
            IpcErrorCode::InvalidInput,
            "{bad}"
        );
    }
    for bad in ["", "a/b", "..", &too_long] {
        let name = bad.to_owned();
        assert_eq!(
            code(block_on(items.mkfile(root(), name, never_open))),
            IpcErrorCode::InvalidInput,
            "{bad}"
        );
    }
    for (name, link_name) in [
        ("a/b", "l"),
        ("a", "l/m"),
        ("a", ""),
        ("a", too_long.as_str()),
        (too_long.as_str(), "l"),
    ] {
        let linked = items.link(
            LinkKind::Symbolic,
            root(),
            name.to_owned(),
            elsewhere(),
            link_name.to_owned(),
        );
        assert_eq!(code(block_on(linked)), IpcErrorCode::InvalidInput, "{name} {link_name}");
    }

    // Accepted by the checks, so only the caps say no.
    assert_eq!(code(block_on(items.trash(root(), names(&["a"])))), IpcErrorCode::Busy);
    assert_eq!(code(block_on(items.delete(root(), names(&["a"])))), IpcErrorCode::Busy);
    assert_eq!(
        code(block_on(items.mkdir(root(), "a/b".to_owned()))),
        IpcErrorCode::Busy
    );
    assert_eq!(
        code(block_on(items.mkfile(root(), "a.txt".to_owned(), never_open))),
        IpcErrorCode::Busy
    );
    let linked = items.link(LinkKind::Hard, root(), "a".to_owned(), elsewhere(), "b".to_owned());
    assert_eq!(code(block_on(linked)), IpcErrorCode::Busy);
}

#[test]
fn a_finished_change_frees_its_place_and_deletes_have_places_of_their_own() {
    let folder = tempfile::tempdir().expect("temp folder");
    std::fs::write(folder.path().join("a.txt"), "a").expect("file");
    let items = Items::default();

    let quick = fill(&items.quick, QUICK_THREADS);
    block_on(items.delete(path_of(&folder), names(&["a.txt"]))).expect("deleted while the quick places are taken");
    assert!(!folder.path().join("a.txt").exists());

    drop(quick);
    block_on(items.mkdir(path_of(&folder), "made".to_owned())).expect("a place was free again");
    assert!(items.quick.acquire(QUICK_THREADS).is_some());
}

#[test]
fn undoing_with_no_record_or_the_wrong_token_finds_nothing_to_undo() {
    let items = Items::default();
    let expected = if CAN_UNTRASH {
        IpcErrorCode::NotFound
    } else {
        IpcErrorCode::Unsupported
    };

    assert_eq!(code(block_on(items.untrash(1))), expected);

    if CAN_UNTRASH {
        let token = items.remember(trashed(&["a"])).expect("a token");
        assert_eq!(code(block_on(items.untrash(token + 1))), IpcErrorCode::NotFound);
        assert!(items.take(token).is_some(), "the wrong token left the record alone");

        let error = block_on(items.untrash(token + 1)).expect_err("nothing");
        assert_eq!(
            serde_json::to_value(&error).expect("serializable")["message"],
            "There is nothing to undo"
        );
    }
}

#[test]
fn a_move_that_trashed_nothing_gets_no_token_and_a_newer_move_replaces_the_record() {
    let items = Items::default();

    assert_eq!(items.remember(trashed(&[])), None);

    if CAN_UNTRASH {
        let first = items.remember(trashed(&["a"])).expect("token");
        assert_eq!(items.remember(trashed(&[])), None);
        let second = items.remember(trashed(&["b"])).expect("token");

        assert_ne!(first, second);
        assert!(items.take(first).is_none(), "the old token is stale");
        assert_eq!(items.take(second).expect("the newer record").names, names(&["b"]));
    } else {
        assert_eq!(items.remember(trashed(&["a"])), None, "macOS gets no token");
    }
}

#[test]
fn an_undo_blocked_by_an_item_keeps_its_record_and_a_finished_one_uses_it_up() {
    if !CAN_UNTRASH {
        return;
    }
    let items = Items::default();
    let token = items.remember(trashed(&["a"])).expect("token");

    let record = items.take(token).expect("record");
    assert!(items.take(token).is_none(), "a second undo meanwhile finds nothing");
    items.settle(
        token,
        record.clone(),
        &Err(UntrashFailure::InTheWay { name: "a".to_owned() }),
    );
    assert!(items.take(token).is_some(), "kept for a retry");

    items.settle(token, record.clone(), &Err(UntrashFailure::FolderChanged));
    assert!(items.take(token).is_some(), "kept for a retry once the folder is back");

    items.settle(token, record.clone(), &Err(UntrashFailure::NothingFound));
    assert!(items.take(token).is_none(), "nothing found uses it up");
    items.settle(token, record.clone(), &Ok(1));
    assert!(items.take(token).is_none(), "a full restore uses it up");

    // A move made while the undo ran keeps its place.
    let newer = items.remember(trashed(&["b"])).expect("token");
    items.settle(token, record, &Err(UntrashFailure::Other));
    assert!(items.take(token).is_none());
    assert!(items.take(newer).is_some());
}

#[test]
fn trashing_an_item_that_is_gone_reports_the_failure_and_gives_no_token() {
    let folder = tempfile::tempdir().expect("temp folder");
    let items = Items::default();

    let outcome = block_on(items.trash(path_of(&folder), names(&["missing.txt"]))).expect("an outcome");

    assert_eq!(outcome.undo, None);
    let failure = outcome.failure.expect("the failure");
    assert_eq!(failure.code, IpcErrorCode::NotFound);
    assert!(failure.message.starts_with("missing.txt: "), "{}", failure.message);
}

#[test]
fn texts_for_the_page_say_what_happened_and_never_hold_a_path() {
    let own = |kind, text| io::Error::new(kind, text);
    let cases = [
        (
            io::Error::from(io::ErrorKind::AlreadyExists),
            IpcErrorCode::InvalidInput,
        ),
        (io::Error::from(io::ErrorKind::NotFound), IpcErrorCode::NotFound),
        (io::Error::from(io::ErrorKind::PermissionDenied), IpcErrorCode::Io),
        (
            own(io::ErrorKind::InvalidInput, "the folder path has too many levels"),
            IpcErrorCode::InvalidInput,
        ),
        (
            own(
                io::ErrorKind::CrossesDevices,
                "a hard link cannot be made to a file on another disk",
            ),
            IpcErrorCode::Io,
        ),
        (
            own(
                io::ErrorKind::Other,
                "Windows allows symbolic links only with Developer Mode on",
            ),
            IpcErrorCode::Io,
        ),
        (io::Error::from(io::ErrorKind::Other), IpcErrorCode::Io),
        (io::Error::from(io::ErrorKind::InvalidInput), IpcErrorCode::Io),
        (io::Error::from_raw_os_error(39), IpcErrorCode::Io),
    ];

    for (error, expected) in cases {
        let told = item_error(DELETE, &error);

        assert_eq!(told.code, expected, "{error:?}");
        assert!(!told.message.contains(std::path::MAIN_SEPARATOR), "{}", told.message);
        assert!(!told.message.is_empty());
    }

    // The words of our own errors reach the page as they are.
    let told = item_error(
        MAKE_LINK,
        &own(io::ErrorKind::InvalidInput, "a hard link can be made only to a file"),
    );
    assert_eq!(told.message, "a hard link can be made only to a file");

    // A Trash full of one name is told as it is, not as a clash of names in the folder of the user.
    let told = item_error(
        TRASH,
        &own(io::ErrorKind::Other, "the Trash holds too many items with this name"),
    );
    assert_eq!(told.code, IpcErrorCode::Io);
    assert_eq!(told.message, "the Trash holds too many items with this name");

    let failure = ItemFailure {
        name: "a.txt".to_owned(),
        error: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    assert_eq!(
        item_failure(DELETE, &failure).message,
        "a.txt: You do not have permission to delete this item."
    );

    for failure in [
        UntrashFailure::Unsupported,
        UntrashFailure::NothingFound,
        UntrashFailure::InTheWay {
            name: "a.txt".to_owned(),
        },
        UntrashFailure::FolderChanged,
        UntrashFailure::Other,
    ] {
        let told = untrash_error(&failure);

        assert!(!told.message.contains(std::path::MAIN_SEPARATOR), "{}", told.message);
    }
    assert_eq!(
        untrash_error(&UntrashFailure::Unsupported).code,
        IpcErrorCode::Unsupported
    );
    assert_eq!(
        untrash_error(&UntrashFailure::NothingFound).code,
        IpcErrorCode::NotFound
    );
    assert_eq!(
        untrash_error(&UntrashFailure::InTheWay { name: "a".to_owned() }).code,
        IpcErrorCode::InvalidInput
    );
    assert_eq!(
        untrash_error(&UntrashFailure::FolderChanged).code,
        IpcErrorCode::InvalidInput
    );
}

#[test]
fn a_new_folder_path_is_made_inside_the_folder_and_its_first_part_returned() {
    let folder = tempfile::tempdir().expect("temp folder");
    let items = Items::default();

    let first = block_on(items.mkdir(path_of(&folder), "a/b/c".to_owned())).expect("made");
    assert_eq!(first, "a");
    assert!(folder.path().join("a").join("b").join("c").is_dir());

    // An existing folder on the way is used; the last one must be new.
    let first = block_on(items.mkdir(path_of(&folder), "a/b/d".to_owned())).expect("reused");
    assert_eq!(first, "a");
    let error = block_on(items.mkdir(path_of(&folder), "a/b/c".to_owned())).expect_err("exists");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);

    let error = block_on(items.mkdir(
        folder.path().join("none").to_string_lossy().into_owned(),
        "x".to_owned(),
    ))
    .expect_err("gone");
    assert_eq!(error.code(), IpcErrorCode::NotFound);
    let text = serde_json::to_string(&error).expect("serializable");
    assert!(!text.contains(&path_of(&folder)), "{text}");
}

#[test]
fn a_new_file_is_created_empty_and_opened_with_its_path() {
    let folder = tempfile::tempdir().expect("temp folder");
    let items = Items::default();
    let (opener, asked) = recording_opener();

    let was_opened = block_on(items.mkfile(path_of(&folder), "notes.txt".to_owned(), opener)).expect("created");

    assert!(was_opened);
    assert_eq!(std::fs::read(folder.path().join("notes.txt")).expect("the file"), b"");
    assert_eq!(
        opened(&asked),
        vec![folder.path().join("notes.txt").to_string_lossy().into_owned()]
    );

    let (opener, asked) = recording_opener();
    let error = block_on(items.mkfile(path_of(&folder), "notes.txt".to_owned(), opener)).expect_err("exists");
    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(opened(&asked).is_empty(), "an existing file is not opened");
}

#[test]
fn a_file_the_system_cannot_open_is_kept_and_the_error_says_so_without_the_cause() {
    let folder = tempfile::tempdir().expect("temp folder");
    let items = Items::default();
    let secret = path_of(&folder);

    let error = block_on(items.mkfile(path_of(&folder), "notes.txt".to_owned(), {
        let detail = format!("no handler for {secret}");
        move |_| Err(detail)
    }))
    .expect_err("not opened");

    assert_eq!(error.code(), IpcErrorCode::Process);
    assert!(folder.path().join("notes.txt").exists());
    let text = serde_json::to_string(&error).expect("serializable");
    assert!(!text.contains(&secret), "{text}");
}

#[test]
fn a_program_or_script_is_created_but_not_opened() {
    let folder = tempfile::tempdir().expect("temp folder");
    let items = Items::default();
    let called = Arc::new(AtomicBool::new(false));

    for name in ["run.sh", "SETUP.EXE", "tool.py"] {
        let seen = Arc::clone(&called);
        let was_opened = block_on(items.mkfile(path_of(&folder), name.to_owned(), move |_| {
            seen.store(true, Ordering::SeqCst);

            Ok(())
        }))
        .expect("created");

        assert!(!was_opened, "{name}");
        assert!(folder.path().join(name).exists(), "{name}");
    }
    assert!(!called.load(Ordering::SeqCst), "the opener was never called");
}

#[test]
fn a_symbolic_link_leads_where_the_pane_showed_the_item_and_a_hard_link_shares_the_file() {
    let from = tempfile::tempdir().expect("source folder");
    let to = tempfile::tempdir().expect("link folder");
    std::fs::write(from.path().join("a.txt"), "data").expect("file");
    let items = Items::default();

    let made = block_on(items.link(
        LinkKind::Symbolic,
        path_of(&from),
        "a.txt".to_owned(),
        path_of(&to),
        "soft.txt".to_owned(),
    ));
    match made {
        Ok(()) => assert_eq!(
            std::fs::read_link(to.path().join("soft.txt")).expect("a link"),
            from.path().join("a.txt")
        ),
        // Windows without the right to make symbolic links.
        Err(error) if cfg!(windows) => assert_eq!(error.code(), IpcErrorCode::Io),
        Err(error) => panic!("{error:?}"),
    }

    block_on(items.link(
        LinkKind::Hard,
        path_of(&from),
        "a.txt".to_owned(),
        path_of(&to),
        "hard.txt".to_owned(),
    ))
    .expect("hard link");
    assert_eq!(
        std::fs::read_to_string(to.path().join("hard.txt")).expect("shared"),
        "data"
    );

    let again = items.link(
        LinkKind::Hard,
        path_of(&from),
        "a.txt".to_owned(),
        path_of(&to),
        "hard.txt".to_owned(),
    );
    assert_eq!(code(block_on(again)), IpcErrorCode::InvalidInput);

    let missing_target = items.link(
        LinkKind::Hard,
        path_of(&to),
        "none".to_owned(),
        path_of(&from),
        "x".to_owned(),
    );
    assert_eq!(code(block_on(missing_target)), IpcErrorCode::NotFound);
}

#[test]
fn a_permanent_delete_removes_files_and_folders_and_stops_at_the_first_failure() {
    let folder = tempfile::tempdir().expect("temp folder");
    std::fs::write(folder.path().join("a.txt"), "a").expect("file");
    std::fs::create_dir_all(folder.path().join("tree").join("deep")).expect("folders");
    std::fs::write(folder.path().join("tree").join("deep").join("b.txt"), "b").expect("file");
    std::fs::write(folder.path().join("last.txt"), "last").expect("file");
    let items = Items::default();

    let error =
        block_on(items.delete(path_of(&folder), names(&["a.txt", "missing", "last.txt"]))).expect_err("stopped");
    assert_eq!(error.code(), IpcErrorCode::NotFound);
    let text = serde_json::to_string(&error).expect("serializable");
    assert!(text.contains("missing: "), "{text}");
    assert!(!folder.path().join("a.txt").exists());
    assert!(
        folder.path().join("last.txt").exists(),
        "the items after the failure are untouched"
    );

    block_on(items.delete(path_of(&folder), names(&["tree", "last.txt"]))).expect("deleted");
    assert!(!folder.path().join("tree").exists());
    assert!(!folder.path().join("last.txt").exists());
}
