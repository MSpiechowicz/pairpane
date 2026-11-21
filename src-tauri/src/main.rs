// Release builds on Windows have no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    // The app starts itself with this argument to decode a HEIC picture in a process of its own (so a crash of the
    // system's codec cannot take the app down). That must be settled before anything else starts: no window, no plugin
    // and no single-instance check, and nothing but the answer is written to the standard output.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == pairpane_core::view::HEIC_CHILD_ARG)
    {
        let code = pairpane_core::view::heic_child_main();

        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }

    pairpane_app_lib::run()
}
