//! Tests of the editor commands: what is refused before a thread is taken, a save that goes through the commands, what
//! the page is told when something fails (never a path), the external editor and the settings file.

use std::fs;
use std::path::Path;

use pairpane_core::model::{
    EditDocument, EditOpened, EditSaveOptions, EditSaved, ExternalEdit, IpcErrorCode, LineEnding,
};
use pairpane_core::settings::SETTINGS_FILE;
use tauri::async_runtime::block_on;

use super::*;
use crate::commands::limit::Permit;
use crate::commands::testing::folder_with;
use crate::commands::testing::root;

/// An editor whose settings file is `SETTINGS_FILE` of a folder of its own (not made yet).
pub(super) fn editor_in(config: &Path) -> Editor {
    Editor::new(Some(config.join(SETTINGS_FILE)))
}

pub(super) fn open_text(editor: &Editor, path: &str, name: &str) -> EditDocument {
    match block_on(editor.open(path.to_owned(), name.to_owned(), None)).expect("opened") {
        EditOpened::Text { document } => document,
        other => panic!("not text: {other:?}"),
    }
}

pub(super) fn options(revision: &str) -> EditSaveOptions {
    EditSaveOptions {
        encoding: "UTF-8".to_owned(),
        bom: false,
        line_ending: LineEnding::Lf,
        revision: revision.to_owned(),
        force: false,
    }
}

/// Takes every place of `cap`.
pub(super) fn fill(cap: &ThreadCap, max: usize) -> Vec<Permit> {
    (0..max).map(|_| cap.acquire(max).expect("a place")).collect()
}

pub(super) fn code<T: std::fmt::Debug>(result: CommandResult<T>) -> IpcErrorCode {
    result.expect_err("refused").code()
}

fn message<T: std::fmt::Debug>(result: CommandResult<T>) -> String {
    code_and_message(result).1
}

#[test]
fn arguments_that_are_not_valid_are_refused_before_a_thread_is_taken() {
    let editor = Editor::new(None);
    let _places = fill(&editor.threads, EDIT_THREADS);

    for name in ["", ".", "..", "a/b", "../x", "x\0y", &"n".repeat(300)] {
        let name = name.to_owned();
        assert_eq!(
            code(block_on(editor.open(root(), name.clone(), None))),
            IpcErrorCode::InvalidInput,
            "{name:?}"
        );
        assert_eq!(
            code(block_on(editor.save(root(), name.clone(), String::new(), options("")))),
            IpcErrorCode::InvalidInput
        );
        assert_eq!(
            code(block_on(editor.external(root(), name, yes))),
            IpcErrorCode::InvalidInput
        );
    }
    assert_eq!(
        code(block_on(editor.open("relative".to_owned(), "a".to_owned(), None))),
        IpcErrorCode::InvalidInput
    );

    // A good request waits for a place when the cap is full.
    assert_eq!(
        code(block_on(editor.open(root(), "a".to_owned(), None))),
        IpcErrorCode::Busy
    );
    assert_eq!(
        code(block_on(editor.save(
            root(),
            "a".to_owned(),
            String::new(),
            options("")
        ))),
        IpcErrorCode::Busy
    );
}

#[test]
fn a_file_is_opened_and_saved_through_the_commands() {
    let (_folder, path) = folder_with(&[("a.txt", "one\r\ntwo\r\n".as_bytes())]);
    let editor = Editor::new(None);

    let document = open_text(&editor, &path, "a.txt");
    assert_eq!(document.text, "one\ntwo\n");
    assert_eq!(document.line_ending, LineEnding::Crlf);

    let chosen = EditSaveOptions {
        line_ending: LineEnding::Crlf,
        ..options(&document.revision)
    };
    let saved =
        block_on(editor.save(path.clone(), "a.txt".to_owned(), "one\ntwo\nthree\n".to_owned(), chosen)).expect("saved");

    let EditSaved::Saved { size, revision } = saved else {
        panic!("not saved: {saved:?}");
    };
    assert_eq!(size, 17);
    assert_eq!(
        fs::read(Path::new(&path).join("a.txt")).expect("read"),
        b"one\r\ntwo\r\nthree\r\n"
    );
    assert_eq!(open_text(&editor, &path, "a.txt").revision, revision);
}

#[test]
fn a_file_that_changed_is_told_and_not_overwritten() {
    let (folder, path) = folder_with(&[("a.txt", b"one")]);
    let editor = Editor::new(None);
    let revision = open_text(&editor, &path, "a.txt").revision;
    fs::write(folder.path().join("a.txt"), "changed elsewhere").expect("write");

    let answer =
        block_on(editor.save(path, "a.txt".to_owned(), "mine".to_owned(), options(&revision))).expect("answer");

    assert_eq!(answer, EditSaved::Changed { gone: false });
    assert_eq!(
        fs::read(folder.path().join("a.txt")).expect("read"),
        b"changed elsewhere"
    );
}

#[test]
fn the_page_is_told_what_failed_and_never_where() {
    let (folder, path) = folder_with(&[("secret-file.txt", b"x")]);
    fs::create_dir(folder.path().join("secret-folder")).expect("folder");
    let editor = Editor::new(None);
    let held = path.clone();
    // A save is only for a file that was opened as text; a folder cannot be, so it is put among them to see what the save
    // of one says.
    open_text(&editor, &path, "secret-file.txt");
    editor
        .opened
        .remember(opened::key(folder.path(), "secret-folder").expect("key"));

    let cases = [
        (
            block_on(editor.open(path.clone(), "secret-folder".to_owned(), None)),
            "the editor opens files, not folders",
        ),
        (
            block_on(editor.open(path.clone(), "secret-missing".to_owned(), None)),
            "the file no longer exists",
        ),
        (
            block_on(editor.open(path.clone(), "secret-file.txt".to_owned(), Some("klingon".to_owned()))),
            "the encoding is not known",
        ),
    ];
    for (result, text) in cases {
        let shown = message(result);

        assert_eq!(shown, text);
        assert!(
            !shown.contains(&held) && !shown.contains('/') && !shown.contains("secret"),
            "{shown}"
        );
    }

    let saves = [
        (
            block_on(editor.save(path.clone(), "secret-folder".to_owned(), "x".to_owned(), options(""))),
            "the editor saves files, not folders",
        ),
        (
            block_on(editor.save(
                path.clone(),
                "secret-file.txt".to_owned(),
                "日本語".to_owned(),
                EditSaveOptions {
                    encoding: "windows-1250".to_owned(),
                    ..options("")
                },
            )),
            "some characters cannot be saved in this encoding",
        ),
        (
            block_on(editor.save(
                path,
                "secret-file.txt".to_owned(),
                "x".repeat(25 * 1024 * 1024 + 1),
                options(""),
            )),
            "the text is too large to save",
        ),
    ];
    for (result, text) in saves {
        let shown = message(result);

        assert_eq!(shown, text);
        assert!(!shown.contains('/') && !shown.contains("secret"), "{shown}");
    }
}

#[test]
fn nothing_is_saved_into_the_config_folder() {
    let config = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    fs::write(config.path().join("state.json"), "{}").expect("write");
    fs::create_dir(config.path().join("sub")).expect("folder");
    fs::write(config.path().join("sub").join("a.txt"), "x").expect("write");
    let path = config.path().to_string_lossy().into_owned();
    let inner = config.path().join("sub").to_string_lossy().into_owned();
    // Opened, so that it is the config folder that refuses the save.
    let first_revision = open_text(&editor, &path, "state.json").revision;
    let second_revision = open_text(&editor, &inner, "a.txt").revision;

    let first = block_on(editor.save(
        path,
        "state.json".to_owned(),
        "{\"hacked\":true}".to_owned(),
        options(&first_revision),
    ));
    let second = block_on(editor.save(
        inner,
        "a.txt".to_owned(),
        "hacked".to_owned(),
        options(&second_revision),
    ));

    assert_eq!(code(first), IpcErrorCode::Forbidden);
    assert_eq!(code(second), IpcErrorCode::Forbidden);
    assert_eq!(fs::read(config.path().join("state.json")).expect("read"), b"{}");
    assert_eq!(fs::read(config.path().join("sub").join("a.txt")).expect("read"), b"x");
}

#[cfg(unix)]
#[test]
fn a_link_into_the_config_folder_does_not_get_round_it() {
    let config = tempfile::tempdir().expect("folder");
    let elsewhere = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    fs::write(config.path().join("state.json"), "{}").expect("write");
    std::os::unix::fs::symlink(config.path(), elsewhere.path().join("shortcut")).expect("link");
    let through = elsewhere.path().join("shortcut").to_string_lossy().into_owned();
    let revision = open_text(&editor, &through, "state.json").revision;

    let refused = block_on(editor.save(
        through,
        "state.json".to_owned(),
        "hacked".to_owned(),
        options(&revision),
    ));

    assert_eq!(code(refused), IpcErrorCode::Forbidden);
    assert_eq!(fs::read(config.path().join("state.json")).expect("read"), b"{}");
}

/// The settings of an editor that is `cp`, which gives a copy of the file it is started on at `copy`.
pub(super) fn copying_to(copy: &Path) -> String {
    serde_json::json!({ "editor": { "program": "cp", "args": ["{file}", copy] } }).to_string()
}

/// The user allows what is asked.
pub(super) fn yes(_question: String) -> bool {
    true
}

/// What F4 answers for the file `name` of the folder `path` when `settings.json` holds `json` and the user allows the
/// program.
fn external_with(json: &str, path: &str, name: &str) -> CommandResult<ExternalEdit> {
    external_asking(json, path, name, yes)
}

pub(super) fn external_asking(
    json: &str,
    path: &str,
    name: &str,
    ask: impl FnOnce(String) -> bool + Send + 'static,
) -> CommandResult<ExternalEdit> {
    let config = tempfile::tempdir().expect("folder");
    fs::write(config.path().join(SETTINGS_FILE), json).expect("write");

    block_on(editor_in(config.path()).external(path.to_owned(), name.to_owned(), ask))
}

#[test]
fn without_settings_or_an_editor_the_external_editor_is_not_configured() {
    let (_folder, path) = folder_with(&[("a.txt", b"x")]);
    let config = tempfile::tempdir().expect("folder");

    for editor in [Editor::new(None), editor_in(config.path())] {
        let answer = block_on(editor.external(path.clone(), "a.txt".to_owned(), yes)).expect("answer");

        assert_eq!(answer, ExternalEdit::NotConfigured);
    }

    let empty = external_with(r#"{"editor":{"program":"","args":["{file}"]}}"#, &path, "a.txt");
    assert_eq!(empty.expect("answer"), ExternalEdit::NotConfigured);
}

#[cfg(unix)]
#[test]
fn the_external_editor_is_started_on_the_file_without_a_shell() {
    use std::time::{Duration, Instant};

    // `cp` is given the file and a copy to make: it can only make it when the name, with its space and the characters
    // a shell would act on, came as one argument.
    let (_folder, path) = folder_with(&[("a b; $(x).txt", b"the text")]);
    let copies = tempfile::tempdir().expect("folder");
    let copy = copies.path().join("copy");
    let answer = external_with(&copying_to(&copy), &path, "a b; $(x).txt").expect("answer");

    assert_eq!(answer, ExternalEdit::Launched);
    let until = Instant::now() + Duration::from_secs(10);
    while fs::read(&copy).ok().as_deref() != Some(&b"the text"[..]) {
        assert!(Instant::now() < until, "the program did not copy the file");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_program_that_is_not_there_is_a_process_error_without_a_path() {
    let (_folder, path) = folder_with(&[("a.txt", b"x")]);

    let refused = external_with(r#"{"editor":{"program":"pairpane-no-such-editor"}}"#, &path, "a.txt");

    assert_eq!(
        code_and_message(refused),
        (
            IpcErrorCode::Process,
            "the external editor could not be started".to_owned()
        )
    );
}

#[test]
fn settings_that_are_not_valid_and_files_an_editor_cannot_take_are_refused() {
    let (folder, path) = folder_with(&[("a.txt", b"x")]);
    fs::create_dir(folder.path().join("inner")).expect("folder");

    let cases = [
        ("not json", "a.txt", "the settings file is not valid"),
        (
            r#"{"editor":{"program":"bin/ed"}}"#,
            "a.txt",
            "the editor program must be a name or an absolute path",
        ),
        (
            r#"{"editor":{"program":"true"}}"#,
            "inner",
            "the editor opens files, not folders",
        ),
    ];
    for (json, name, text) in cases {
        let refused = external_with(json, &path, name);

        assert_eq!(code_and_message(refused), (IpcErrorCode::InvalidInput, text.to_owned()));
    }
}

pub(super) fn code_and_message<T: std::fmt::Debug>(result: CommandResult<T>) -> (IpcErrorCode, String) {
    let error = result.expect_err("refused");
    let value = serde_json::to_value(&error).expect("serializable");
    let message = value["message"].as_str().expect("a message").to_owned();

    (error.code(), message)
}

#[test]
fn the_settings_file_is_made_with_the_defaults_and_handed_to_the_system() {
    let config = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path().join("pairpane").as_path());
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = std::sync::Arc::clone(&opened);

    block_on(editor.settings(move |file| {
        record.lock().expect("lock").push(file);

        Ok(())
    }))
    .expect("opened");

    let file = config.path().join("pairpane").join(SETTINGS_FILE);
    // The file it leads to is handed over (the canonical path; on macOS a temporary folder is under /private).
    let real = fs::canonicalize(&file).expect("canonical");
    assert_eq!(*opened.lock().expect("lock"), [real.to_string_lossy().into_owned()]);
    assert_eq!(pairpane_core::settings::load(&file).expect("valid").editor(), None);

    // A file that is there is not touched.
    fs::write(&file, r#"{"editor":{"program":"vim"}}"#).expect("write");
    block_on(editor.settings(|_| Ok(()))).expect("opened");
    assert_eq!(
        fs::read_to_string(&file).expect("read"),
        r#"{"editor":{"program":"vim"}}"#
    );
}

#[test]
fn the_settings_that_cannot_be_made_or_opened_are_told_without_a_path() {
    let nowhere = block_on(Editor::new(None).settings(|_| Ok(())));
    assert_eq!(
        code_and_message(nowhere),
        (
            IpcErrorCode::Unsupported,
            "there is no place to keep settings on this system".to_owned()
        )
    );

    // The "folder" of the file is a file.
    let config = tempfile::tempdir().expect("folder");
    fs::write(config.path().join("pairpane"), "x").expect("write");
    let blocked = block_on(editor_in(&config.path().join("pairpane")).settings(|_| Ok(())));
    assert_eq!(
        code_and_message(blocked),
        (IpcErrorCode::Io, "the settings file cannot be created".to_owned())
    );

    let refused = block_on(editor_in(config.path()).settings(|_| Err("/secret/path: no handler".to_owned())));
    assert_eq!(
        code_and_message(refused),
        (
            IpcErrorCode::Process,
            "the settings file could not be opened".to_owned()
        )
    );
}
