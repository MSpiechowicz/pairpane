//! The settings file: what is read, what is refused, and the command line it makes.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;

use super::*;

fn loaded(json: &str) -> io::Result<Settings> {
    let folder = tempfile::tempdir().expect("folder");
    let path = folder.path().join(SETTINGS_FILE);
    fs::write(&path, json).expect("write");

    load(&path)
}

fn refused(json: &str) -> String {
    let error = loaded(json).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::InvalidInput, "{json}");
    assert!(crate::view::is_refusal(&error));

    error.to_string()
}

fn editor(program: &str, args: &[&str]) -> ExternalEditor {
    ExternalEditor {
        program: program.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

fn file() -> PathBuf {
    if cfg!(windows) {
        "C:\\data\\my file.txt"
    } else {
        "/data/my file.txt"
    }
    .into()
}

fn os(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

#[test]
fn a_missing_file_is_the_defaults() {
    let folder = tempfile::tempdir().expect("folder");

    let settings = load(&folder.path().join(SETTINGS_FILE)).expect("load");

    assert_eq!(settings, Settings::default());
    assert_eq!(settings.editor(), None);
}

#[test]
fn an_editor_is_read_with_its_arguments() {
    let settings = loaded(r#"{"version":1,"editor":{"program":"code","args":["--wait","{file}"]}}"#).expect("load");

    assert_eq!(settings.editor(), Some(&editor("code", &["--wait", "{file}"])));
}

#[test]
fn no_editor_an_empty_program_and_a_blank_one_mean_the_built_in_editor() {
    for json in [
        "{}",
        r#"{"version":1}"#,
        r#"{"version":1,"editor":{"program":"","args":["{file}"]}}"#,
        r#"{"editor":{"program":"  "}}"#,
        r#"{"editor":{"program":""}}"#,
    ] {
        assert_eq!(loaded(json).expect("load").editor(), None, "{json}");
    }
}

#[test]
fn a_comment_is_allowed_and_the_args_are_optional() {
    let settings = loaded(r#"{"$comment":"mine","editor":{"program":"vim"}}"#).expect("load");

    assert_eq!(settings.editor(), Some(&editor("vim", &[])));
}

#[test]
fn an_unknown_top_level_key_is_refused_so_a_typo_is_not_a_silent_built_in_editor() {
    assert_eq!(
        refused(r#"{"editer":{"program":"vim"}}"#),
        "the settings file is not valid"
    );
    assert_eq!(
        refused(r#"{"$comment":"mine","other":1}"#),
        "the settings file is not valid"
    );
}

#[test]
fn a_file_that_is_not_valid_is_refused_with_a_fixed_text() {
    for json in [
        "",
        "not json",
        "[]",
        r#"{"version":2,"editor":{"program":"code"}}"#,
        r#"{"version":"1"}"#,
        r#"{"editor":"code"}"#,
        r#"{"editor":{"program":"code","shell":true}}"#,
        r#"{"editor":{"program":1}}"#,
        r#"{"editor":{"program":"code","args":[1]}}"#,
    ] {
        assert_eq!(refused(json), "the settings file is not valid", "{json}");
    }
}

#[test]
fn a_program_is_a_name_or_an_absolute_path() {
    let absolute = if cfg!(windows) {
        r"C:\\tools\\ed.exe"
    } else {
        "/usr/bin/ed"
    };
    loaded(&format!(r#"{{"editor":{{"program":"{absolute}"}}}}"#)).expect("an absolute path");
    loaded(r#"{"editor":{"program":"my editor"}}"#).expect("a name with a space");

    for program in ["bin/ed", "./ed", "../ed", "a/b/c", "ed\\u0000"] {
        let json = format!(r#"{{"editor":{{"program":"{program}"}}}}"#);

        assert_eq!(
            refused(&json),
            "the editor program must be a name or an absolute path",
            "{program}"
        );
    }
}

#[test]
fn a_bare_name_is_one_plain_name_and_on_windows_has_no_drive_or_stream() {
    for windows in [false, true] {
        for name in ["code", "my editor", "ed.exe", "ed-2"] {
            assert!(is_bare_name(name, windows), "{name} {windows}");
        }

        for name in ["", ".", "..", "bin/ed", "./ed", "../ed", "a/b"] {
            assert!(!is_bare_name(name, windows), "{name:?} {windows}");
        }
    }

    // A backslash is a separator on Windows only; a colon is refused there only.
    assert!(is_bare_name("a\\b", false));
    assert!(is_bare_name("a:b", false));
    for name in ["a\\b", "C:evil.exe", "C:", "c:..\\evil", "ed.exe:stream", ":x"] {
        assert!(!is_bare_name(name, true), "{name}");
    }
}

#[test]
fn the_revision_follows_the_bytes_of_the_file() {
    let folder = tempfile::tempdir().expect("folder");
    let path = folder.path().join(SETTINGS_FILE);
    assert_eq!(load_with_revision(&path).expect("missing").1, 0);

    fs::write(&path, r#"{"editor":{"program":"a"}}"#).expect("write");
    let first = load_with_revision(&path).expect("first").1;
    assert_ne!(first, 0);
    assert_eq!(load_with_revision(&path).expect("again").1, first);

    fs::write(&path, r#"{"editor":{"program":"b"}}"#).expect("write");
    assert_ne!(load_with_revision(&path).expect("second").1, first);
}

#[test]
fn a_file_over_the_cap_is_refused() {
    let padding = " ".repeat(usize::try_from(MAX_SETTINGS).expect("fits"));

    assert_eq!(refused(&format!("{{}}{padding}")), "the settings file is not valid");
}

#[cfg(unix)]
#[test]
fn something_that_is_not_a_file_is_refused_and_a_pipe_is_not_waited_on() {
    let folder = tempfile::tempdir().expect("folder");
    crate::view::testing::fifo(folder.path(), "pipe");
    let pipe = folder.path().join("pipe");

    for path in [folder.path(), pipe.as_path()] {
        let error = load(path).expect_err("refused");

        assert_eq!(error.to_string(), "the settings file is not valid");
    }
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_read_has_its_own_text() {
    use std::os::unix::fs::PermissionsExt;

    // Root reads anything.
    if rustix::process::geteuid().is_root() {
        return;
    }

    let folder = tempfile::tempdir().expect("folder");
    let path = folder.path().join(SETTINGS_FILE);
    fs::write(&path, "{}").expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");

    let error = load(&path).expect_err("refused");

    assert_eq!(error.to_string(), "the settings file cannot be read");
    assert!(crate::view::is_refusal(&error));
}

#[test]
fn ensure_creates_the_defaults_with_their_folder_and_leaves_a_file_alone() {
    let folder = tempfile::tempdir().expect("folder");
    let path = folder.path().join("config").join(SETTINGS_FILE);

    ensure(&path).expect("ensure");

    // The file made is valid and has no editor yet.
    assert_eq!(load(&path).expect("load").editor(), None);
    let text = fs::read_to_string(&path).expect("read");
    assert!(
        text.contains("\"version\": 1") && text.contains("\"program\": \"\""),
        "{text}"
    );

    fs::write(&path, r#"{"editor":{"program":"vim"}}"#).expect("edit");
    ensure(&path).expect("ensure again");

    assert_eq!(load(&path).expect("load").editor(), Some(&editor("vim", &[])));
}

#[test]
fn the_file_replaces_only_an_argument_that_is_exactly_the_placeholder() {
    let (program, args) = argv(
        &editor("code", &["--wait", "{file}", "--goto={file}", "{file} "]),
        &file(),
    );

    assert_eq!(program, OsString::from("code"));
    assert_eq!(args[0], OsString::from("--wait"));
    assert_eq!(args[1], file().into_os_string());
    assert_eq!(args[2], OsString::from("--goto={file}"));
    assert_eq!(args[3], OsString::from("{file} "));
    assert_eq!(args.len(), 4);
}

#[test]
fn the_file_is_added_last_when_no_argument_is_the_placeholder() {
    let (_, args) = argv(&editor("kitty", &["vim", "-n"]), &file());
    assert_eq!(args, [os(&["vim", "-n"]), vec![file().into_os_string()]].concat());

    let (_, none) = argv(&editor("gedit", &[]), &file());
    assert_eq!(none, vec![file().into_os_string()]);
}

#[test]
fn a_name_with_spaces_and_quotes_is_one_argument() {
    let tricky = Path::new(if cfg!(windows) {
        "C:\\a b\\$(x) \"q\" ; y.txt"
    } else {
        "/a b/$(x) \"q\" ; y.txt"
    });

    let (_, args) = argv(&editor("ed", &["{file}"]), tricky);

    assert_eq!(args, vec![tricky.as_os_str().to_owned()]);
}
