use super::*;

#[test]
fn a_line_that_is_empty_too_long_or_of_several_lines_is_refused() {
    assert!(check_line("  ").is_err());
    assert!(check_line(&"x".repeat(MAX_LINE + 1)).is_err());
    assert!(check_line("ls\nrm x").is_err());
    assert!(check_line("ls\0").is_err());
    assert!(check_line("ls -la | grep x").is_ok());
}

#[test]
fn escapes_and_control_characters_are_left_out_and_a_character_cut_between_reads_is_joined() {
    let mut cleaner = Cleaner::default();

    assert_eq!(cleaner.push(b"\x1b[1;31mred\x1b[0m\tx\x07\r\n"), "red\tx\n");
    assert_eq!(cleaner.push(b"\x1b]0;title\x07a\x1b]8;;u\x1b\\b"), "ab");
    assert_eq!(cleaner.push(b"50%\r60%\n"), "50%\n60%\n");
    assert_eq!(cleaner.push(&[0xc5]), "");
    assert_eq!(cleaner.push(&[0x82, b'!']), "ł!");
    assert_eq!(cleaner.push(&[0xff, b'x']), "\u{fffd}x");
    assert_eq!(cleaner.push(&[0xe2, 0x82]), "");
    assert_eq!(cleaner.finish(), "\u{fffd}");
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn the_terminal_is_the_one_terminal_names_then_the_first_known_one_found() {
    let found = |name: &str| (name == "konsole" || name == "kitty").then(|| PathBuf::from(format!("/usr/bin/{name}")));
    let folder = Path::new("/home");

    let chosen = |variable: Option<&str>| {
        terminal_from(folder, variable.map(OsString::from), &found).map(|(program, _)| program)
    };

    assert_eq!(chosen(Some("kitty")), Some(OsString::from("/usr/bin/kitty")));
    assert_eq!(chosen(None), Some(OsString::from("/usr/bin/konsole")));
    assert_eq!(chosen(Some("nothing")), Some(OsString::from("/usr/bin/konsole")));
    assert_eq!(chosen(Some("../kitty")), Some(OsString::from("/usr/bin/konsole")));
    assert_eq!(terminal_from(folder, None, &|_: &str| None), None);
}

/// The runs of a command, with `/bin/sh`.
#[cfg(unix)]
mod on_unix {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::super::*;

    /// Runs `line` with `/bin/sh` in `folder`, collecting what it sends; `stop` is raised after `stop_after`.
    fn run_sh(line: &str, folder: &Path, stop_after: Option<Duration>) -> (io::Result<()>, Vec<ShellEvent>) {
        let shell = Shell {
            program: PathBuf::from("/bin/sh"),
        };
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(after) = stop_after {
            let flag = Arc::clone(&stop);
            thread::spawn(move || {
                thread::sleep(after);
                flag.store(true, Ordering::Relaxed);
            });
        }

        let mut events = Vec::new();
        let result = run(&shell, line, folder, &stop, &mut |event| {
            events.push(event);
            true
        });

        (result, events)
    }

    /// All the output text, joined.
    fn output(events: &[ShellEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                ShellEvent::Output { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary folder")
    }

    #[test]
    fn a_command_runs_in_the_folder_and_sends_its_output_and_errors_then_its_code() {
        let folder = temp();
        let real = std::fs::canonicalize(folder.path()).expect("the folder");

        let (result, events) = run_sh("pwd; echo oops >&2; exit 3", folder.path(), None);

        result.expect("ran");
        let text = output(&events);
        assert!(text.contains(&format!("{}\n", real.display())), "{text:?}");
        assert!(text.contains("oops\n"), "{text:?}");
        assert_eq!(
            events.last(),
            Some(&ShellEvent::Done {
                code: Some(3),
                stopped: false,
                truncated: false
            })
        );
    }

    #[test]
    fn a_stop_ends_the_command_and_what_it_started() {
        let folder = temp();
        let started = Instant::now();

        let (result, events) = run_sh(
            "sleep 30 & sleep 30; wait",
            folder.path(),
            Some(Duration::from_millis(200)),
        );

        result.expect("ran");
        assert!(started.elapsed() < Duration::from_secs(10), "stopped, not waited for");
        assert!(matches!(events.last(), Some(ShellEvent::Done { stopped: true, .. })));
    }

    #[test]
    fn a_command_that_reads_its_input_ends_instead_of_waiting() {
        let folder = temp();

        let (result, events) = run_sh("cat; echo after", folder.path(), None);

        result.expect("ran");
        assert_eq!(output(&events), "after\n");
    }

    #[test]
    fn a_program_left_running_in_the_background_does_not_hold_up_the_end() {
        let folder = temp();
        let started = Instant::now();

        let (result, events) = run_sh("sleep 20 & echo started", folder.path(), None);

        result.expect("ran");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        assert_eq!(output(&events), "started\n");
        assert!(matches!(
            events.last(),
            Some(ShellEvent::Done {
                code: Some(0),
                stopped: false,
                ..
            })
        ));
    }

    #[test]
    fn output_past_the_cap_is_read_and_left_out() {
        let folder = temp();

        let (result, events) = run_sh(
            "head -c 3000000 /dev/zero | tr '\\0' a; echo; echo end",
            folder.path(),
            None,
        );

        result.expect("ran");
        assert_eq!(output(&events).len(), MAX_OUTPUT);
        assert_eq!(
            events.last(),
            Some(&ShellEvent::Done {
                code: Some(0),
                stopped: false,
                truncated: true
            })
        );
    }

    #[test]
    fn a_page_that_no_longer_listens_stops_the_command() {
        let folder = temp();
        let shell = Shell {
            program: PathBuf::from("/bin/sh"),
        };
        let started = Instant::now();
        let mut last = None;

        run(
            &shell,
            "echo one; sleep 30",
            folder.path(),
            &AtomicBool::new(false),
            &mut |event| {
                last = Some(event);
                false
            },
        )
        .expect("ran");

        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(matches!(last, Some(ShellEvent::Done { stopped: true, .. })));
    }
}
