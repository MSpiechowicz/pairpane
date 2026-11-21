//! What stops the page from making the shell run a program of its choice: a settings file that is not saved into, and
//! the user's answer to the question put before a program is started.

use std::fs;

use pairpane_core::model::{ExternalEdit, IpcErrorCode};
use pairpane_core::settings::SETTINGS_FILE;
use tauri::async_runtime::block_on;

use super::tests::{code, code_and_message, copying_to, editor_in, external_asking, fill, open_text, options, yes};
use super::*;
use crate::commands::testing::folder_with;

#[cfg(unix)]
#[test]
fn a_hard_link_to_the_settings_or_the_state_saved_through_is_refused() {
    let config = tempfile::tempdir().expect("folder");
    let elsewhere = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    let planted = r#"{"editor":{"program":"evil"}}"#;
    for kept in [SETTINGS_FILE, "state.json"] {
        fs::write(config.path().join(kept), "{}").expect("write");
        fs::hard_link(config.path().join(kept), elsewhere.path().join(format!("link-{kept}"))).expect("link");
    }
    let path = elsewhere.path().to_string_lossy().into_owned();

    for kept in [SETTINGS_FILE, "state.json"] {
        let name = format!("link-{kept}");
        let revision = open_text(&editor, &path, &name).revision;

        let refused = block_on(editor.save(path.clone(), name, planted.to_owned(), options(&revision)));

        assert_eq!(code(refused), IpcErrorCode::Forbidden, "{kept}");
        assert_eq!(fs::read(config.path().join(kept)).expect("read"), b"{}", "{kept}");
    }

    // A file that is no link to either is saved as before.
    fs::write(elsewhere.path().join("plain.txt"), "x").expect("write");
    let revision = open_text(&editor, &path, "plain.txt").revision;
    block_on(editor.save(path, "plain.txt".to_owned(), "y".to_owned(), options(&revision))).expect("saved");
}

#[test]
fn a_program_the_user_does_not_allow_is_not_started_and_the_page_is_told_so() {
    let (_folder, path) = folder_with(&[("a.txt", b"the text")]);
    let copies = tempfile::tempdir().expect("folder");
    let copy = copies.path().join("copy");

    let refused = external_asking(&copying_to(&copy), &path, "a.txt", |_| false);

    assert_eq!(
        code_and_message(refused),
        (
            IpcErrorCode::InvalidInput,
            "the external editor was not allowed".to_owned()
        )
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!copy.exists(), "the program ran");
}

#[test]
fn the_user_is_asked_with_the_program_and_its_arguments_and_only_once_for_the_same_pair() {
    let (_folder, path) = folder_with(&[("a.txt", b"x")]);
    let config = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let write = |json: &str| fs::write(config.path().join(SETTINGS_FILE), json).expect("write");
    let start = |ask_for: &std::sync::Arc<std::sync::Mutex<Vec<String>>>| {
        let record = std::sync::Arc::clone(ask_for);

        block_on(editor.external(path.clone(), "a.txt".to_owned(), move |question| {
            record.lock().expect("lock").push(question);

            true
        }))
    };

    write(r#"{"editor":{"program":"true","args":["--flag","{file}"]}}"#);
    assert_eq!(start(&asked).expect("started"), ExternalEdit::Launched);
    assert_eq!(start(&asked).expect("started"), ExternalEdit::Launched);
    assert_eq!(asked.lock().expect("lock").len(), 1, "asked once");
    let question = asked.lock().expect("lock")[0].clone();
    assert!(question.contains(r#""true" "--flag" "{file}""#), "{question}");

    // The same program with other arguments is another pair: it is asked about again.
    write(r#"{"editor":{"program":"true","args":["--other","{file}"]}}"#);
    assert_eq!(start(&asked).expect("started"), ExternalEdit::Launched);
    assert_eq!(asked.lock().expect("lock").len(), 2);
}

#[test]
fn nobody_is_asked_when_there_is_no_editor_or_the_file_is_not_one() {
    let (folder, path) = folder_with(&[("a.txt", b"x")]);
    fs::create_dir(folder.path().join("inner")).expect("folder");

    let none = external_asking(r#"{"editor":{"program":""}}"#, &path, "a.txt", |_| panic!("not asked"));
    assert_eq!(none.expect("answer"), ExternalEdit::NotConfigured);

    let dir = external_asking(r#"{"editor":{"program":"true"}}"#, &path, "inner", |_| {
        panic!("not asked")
    });
    assert_eq!(code(dir), IpcErrorCode::InvalidInput);
}

#[test]
fn too_many_running_editors_are_busy_and_a_place_is_given_back_when_one_ends() {
    let (_folder, path) = folder_with(&[("a.txt", b"x")]);
    let config = tempfile::tempdir().expect("folder");
    fs::write(config.path().join(SETTINGS_FILE), r#"{"editor":{"program":"true"}}"#).expect("write");
    let editor = editor_in(config.path());
    let places = fill(&editor.running, MAX_RUNNING);

    let refused = block_on(editor.external(path.clone(), "a.txt".to_owned(), |_| panic!("not asked")));
    assert_eq!(code(refused), IpcErrorCode::Busy);

    drop(places);
    let started = block_on(editor.external(path, "a.txt".to_owned(), yes));
    assert_eq!(started.expect("started"), ExternalEdit::Launched);
}

#[test]
fn a_refused_program_is_not_asked_about_again_until_the_settings_file_changes() {
    let (_folder, path) = folder_with(&[("a.txt", b"x")]);
    let config = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let refuse = || {
        let count = std::sync::Arc::clone(&asked);

        block_on(editor.external(path.clone(), "a.txt".to_owned(), move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            false
        }))
    };
    let write = |json: &str| fs::write(config.path().join(SETTINGS_FILE), json).expect("write");
    let not_allowed = || {
        (
            IpcErrorCode::InvalidInput,
            "the external editor was not allowed".to_owned(),
        )
    };

    write(r#"{"editor":{"program":"true"}}"#);
    assert_eq!(code_and_message(refuse()), not_allowed());
    assert_eq!(code_and_message(refuse()), not_allowed());
    assert_eq!(code_and_message(refuse()), not_allowed());
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1, "asked once");

    // The file is changed (the pair is the same): the user is asked once more.
    write(r#"{"$comment":"edited","editor":{"program":"true"}}"#);
    assert_eq!(code_and_message(refuse()), not_allowed());
    assert_eq!(code_and_message(refuse()), not_allowed());
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn a_question_nobody_answers_holds_up_neither_opening_nor_saving_and_no_second_question() {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let (folder, path) = folder_with(&[("a.txt", b"one")]);
    let config = tempfile::tempdir().expect("folder");
    fs::write(config.path().join(SETTINGS_FILE), r#"{"editor":{"program":"true"}}"#).expect("write");
    let editor = editor_in(config.path());
    // Both places of the open and save threads are free of the question; the question has its own.
    let (shown, wait_shown) = channel();
    let (answer, wait_answer) = channel::<bool>();

    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            block_on(editor.external(path.clone(), "a.txt".to_owned(), move |_| {
                shown.send(()).expect("send");

                wait_answer.recv().expect("an answer")
            }))
        });
        wait_shown
            .recv_timeout(Duration::from_secs(10))
            .expect("the question is put");

        // Opening and saving go on, with every place of their own free.
        let revision = open_text(&editor, &path, "a.txt").revision;
        block_on(editor.save(path.clone(), "a.txt".to_owned(), "two".to_owned(), options(&revision))).expect("saved");
        assert_eq!(fs::read(folder.path().join("a.txt")).expect("read"), b"two");

        // Another start is told to wait, and is not asked.
        let second = block_on(editor.external(path.clone(), "a.txt".to_owned(), |_| panic!("not asked")));
        assert_eq!(code(second), IpcErrorCode::Busy);

        answer.send(true).expect("send");
        assert_eq!(first.join().expect("thread").expect("started"), ExternalEdit::Launched);
    });

    // The place is given back when the start is over.
    assert_eq!(
        block_on(editor.external(path, "a.txt".to_owned(), |_| panic!("allowed already"))).expect("started"),
        ExternalEdit::Launched
    );
}

#[cfg(unix)]
#[test]
fn a_settings_file_that_leads_to_something_else_than_a_json_file_is_not_opened() {
    use std::os::unix::fs::symlink;

    let config = tempfile::tempdir().expect("folder");
    let elsewhere = tempfile::tempdir().expect("folder");
    let editor = editor_in(config.path());
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let open = |editor: &Editor| {
        let record = std::sync::Arc::clone(&opened);

        block_on(editor.settings(move |file| {
            record.lock().expect("lock").push(file);

            Ok(())
        }))
    };
    let refusal = (IpcErrorCode::Process, "the settings file cannot be opened".to_owned());

    // A link to a file that is not .json (a shell start-up file, say).
    fs::write(elsewhere.path().join("profile"), "run").expect("write");
    symlink(elsewhere.path().join("profile"), config.path().join(SETTINGS_FILE)).expect("link");
    assert_eq!(code_and_message(open(&editor)), refusal);

    // A link to a folder, and one that leads nowhere.
    fs::remove_file(config.path().join(SETTINGS_FILE)).expect("remove");
    fs::create_dir(elsewhere.path().join("dir.json")).expect("folder");
    symlink(elsewhere.path().join("dir.json"), config.path().join(SETTINGS_FILE)).expect("link");
    assert_eq!(code_and_message(open(&editor)), refusal);
    fs::remove_file(config.path().join(SETTINGS_FILE)).expect("remove");
    symlink(elsewhere.path().join("missing.json"), config.path().join(SETTINGS_FILE)).expect("link");
    assert!(open(&editor).is_err());
    assert!(
        opened.lock().expect("lock").is_empty(),
        "nothing was handed to the system"
    );

    // A link to a .json file is followed, as a dotfiles folder needs.
    fs::remove_file(config.path().join(SETTINGS_FILE)).expect("remove");
    fs::write(elsewhere.path().join("mine.json"), "{}").expect("write");
    symlink(elsewhere.path().join("mine.json"), config.path().join(SETTINGS_FILE)).expect("link");
    open(&editor).expect("opened");
    let real = fs::canonicalize(elsewhere.path().join("mine.json")).expect("canonical");
    assert_eq!(*opened.lock().expect("lock"), [real.to_string_lossy().into_owned()]);
}

#[test]
fn only_a_file_opened_as_text_in_this_run_is_saved() {
    let (folder, path) = folder_with(&[("a.txt", b"one"), ("b.txt", b"two")]);
    let editor = Editor::new(None);
    let save = |name: &str, force: bool| {
        let options = EditSaveOptions {
            force,
            ..options("1-2.3-4.5-6-7")
        };

        block_on(editor.save(path.clone(), name.to_owned(), "mine".to_owned(), options))
    };

    // Never opened here: refused, with nothing written, forced or not.
    assert_eq!(code(save("a.txt", false)), IpcErrorCode::Forbidden);
    assert_eq!(code(save("a.txt", true)), IpcErrorCode::Forbidden);
    assert_eq!(fs::read(folder.path().join("a.txt")).expect("read"), b"one");

    // Opened: saved. Another file of the folder is still not.
    let revision = open_text(&editor, &path, "a.txt").revision;
    let saved = block_on(editor.save(path.clone(), "a.txt".to_owned(), "mine".to_owned(), options(&revision)));
    assert!(matches!(saved.expect("saved"), EditSaved::Saved { .. }));
    assert_eq!(fs::read(folder.path().join("a.txt")).expect("read"), b"mine");
    assert_eq!(code(save("b.txt", true)), IpcErrorCode::Forbidden);
    assert_eq!(fs::read(folder.path().join("b.txt")).expect("read"), b"two");
}

#[test]
fn a_file_that_is_not_text_is_not_one_to_save() {
    let (folder, path) = folder_with(&[("a.txt", b"one")]);
    let editor = Editor::new(None);
    fs::write(folder.path().join("nul.bin"), b"abc\0\0def\0ghi").expect("write");

    let opened = block_on(editor.open(path.clone(), "nul.bin".to_owned(), None)).expect("answer");
    assert_eq!(opened, EditOpened::Binary);

    let refused = block_on(editor.save(path, "nul.bin".to_owned(), "mine".to_owned(), options("")));

    assert_eq!(code(refused), IpcErrorCode::Forbidden);
}

#[test]
fn a_file_deleted_after_it_was_opened_is_made_again_with_force_only() {
    let (folder, path) = folder_with(&[("a.txt", b"one")]);
    let editor = Editor::new(None);
    let revision = open_text(&editor, &path, "a.txt").revision;
    fs::remove_file(folder.path().join("a.txt")).expect("remove");

    let gone = block_on(editor.save(path.clone(), "a.txt".to_owned(), "mine".to_owned(), options(&revision)));
    assert_eq!(gone.expect("answer"), EditSaved::Changed { gone: true });
    assert!(!folder.path().join("a.txt").exists());

    let forced = EditSaveOptions {
        force: true,
        ..options(&revision)
    };
    let made = block_on(editor.save(path, "a.txt".to_owned(), "mine".to_owned(), forced));

    assert!(matches!(made.expect("made"), EditSaved::Saved { .. }));
    assert_eq!(fs::read(folder.path().join("a.txt")).expect("read"), b"mine");
}

#[test]
fn a_file_that_was_never_there_is_not_made_by_a_save_even_with_force() {
    let (folder, path) = folder_with(&[]);
    let editor = Editor::new(None);
    let forced = EditSaveOptions {
        force: true,
        ..options("")
    };

    let refused = block_on(editor.save(path, "new.txt".to_owned(), "mine".to_owned(), forced));

    assert_eq!(code(refused), IpcErrorCode::Forbidden);
    assert!(!folder.path().join("new.txt").exists());
}
