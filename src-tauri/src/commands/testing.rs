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

/// A folder holding the files, and its path as the page would send it.
pub(super) fn folder_with(files: &[(&str, &[u8])]) -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().expect("folder");
    for (name, bytes) in files {
        std::fs::write(folder.path().join(name), bytes).expect("file");
    }
    let path = folder.path().to_string_lossy().into_owned();

    (folder, path)
}
