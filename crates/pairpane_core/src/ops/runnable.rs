//! Which file names the system would run, or act on like a program, when they are opened.

/// File extensions (lowercase) of programs, scripts, shortcuts and installers that the system would run, or follow
/// to something that runs, when opened: a new file with one of these is created but never opened for the user.
/// One list for every system, so the answer does not depend on where the app runs; a file made on one system can be
/// opened on another through a shared folder.
const RUNNABLE_EXTENSIONS: &[&str] = &[
    // Windows programs, scripts and installers.
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msc",
    "scr",
    "pif",
    "cpl",
    "hta",
    "lnk",
    "reg",
    "jar",
    "msp",
    "mst",
    "ps1xml",
    "psc1",
    "psd1",
    "vb",
    "ws",
    "wsc",
    "sct",
    "shb",
    "shs",
    "chm",
    "hlp",
    "inf",
    "appx",
    "msix",
    "appinstaller",
    "diagcab",
    "gadget",
    "xll",
    "jnlp",
    // Windows shortcuts and other files that point to a program, a page or a search.
    "url",
    "website",
    "appref-ms",
    "application",
    "settingcontent-ms",
    "library-ms",
    "search-ms",
    "scf",
    // macOS apps, scripts, installers and shortcuts.
    "app",
    "command",
    "terminal",
    "workflow",
    "action",
    "scpt",
    "scptd",
    "applescript",
    "pkg",
    "mpkg",
    "prefpane",
    "fileloc",
    "inetloc",
    "webloc",
    "tool",
    // Linux and other Unix systems.
    "sh",
    "py",
    "desktop",
    "appimage",
    "run",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "pl",
    "rb",
    "php",
    "mjs",
    "cjs",
];

/// Whether the system would run a file called `name` when it is opened (see [`super::is_runnable`]).
///
/// The extension is the text after the last dot. A name that is only a dot and an extension (`.bat`) counts too:
/// `Path::extension` sees no extension in it, but a file with such a name is still a script to run. A name with no
/// dot, or with nothing after the last one, has no extension.
pub(super) fn is_runnable(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };

    RUNNABLE_EXTENSIONS
        .iter()
        .any(|runnable| runnable.eq_ignore_ascii_case(extension))
}
