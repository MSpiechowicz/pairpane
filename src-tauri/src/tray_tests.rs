use super::*;

#[test]
fn closing_hides_only_while_a_device_is_paired_and_the_tray_is_not_missing() {
    let table = [
        (true, TrayHealth::Untried, true),
        (true, TrayHealth::Ready, true),
        (true, TrayHealth::Unavailable, false),
        (false, TrayHealth::Untried, false),
        (false, TrayHealth::Ready, false),
        (false, TrayHealth::Unavailable, false),
    ];

    for (keeps_alive, health, hides) in table {
        assert_eq!(should_hide(keeps_alive, health), hides, "{keeps_alive} {health:?}");
    }
}

#[test]
fn only_an_exit_without_a_code_of_a_window_that_would_hide_is_prevented() {
    let table = [
        (None, true, true, ExitAction::Prevent),
        // Every window is gone: the app ends.
        (None, false, true, ExitAction::Allow),
        // The window would not have been hidden.
        (None, true, false, ExitAction::Allow),
        // A quit (the tray's, the system's) and a restart always go on.
        (Some(0), true, true, ExitAction::Allow),
        (Some(tauri::RESTART_EXIT_CODE), true, true, ExitAction::Allow),
    ];

    for (code, window_alive, hide, action) in table {
        assert_eq!(
            exit_action(code, window_alive, hide),
            action,
            "{code:?} {window_alive} {hide}"
        );
    }
}

#[test]
fn only_a_hidden_window_is_shown_before_a_question() {
    assert!(must_reveal(false));
    assert!(!must_reveal(true));
}
