//! Helpers shared by the command tests.

/// An absolute folder that the tests never touch on disk.
pub(super) fn root() -> String {
    if cfg!(windows) { "C:\\data" } else { "/data" }.to_owned()
}

/// A second absolute folder.
pub(super) fn elsewhere() -> String {
    if cfg!(windows) { "C:\\x" } else { "/x" }.to_owned()
}

pub(super) fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}
