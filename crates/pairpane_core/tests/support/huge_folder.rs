//! The folder of the listing budget, shared by `tests/budgets.rs` and `benches/listing.rs`: 100 000 entries, one in ten a
//! folder.

use std::fs;

use tempfile::TempDir;

/// Entries of the folder.
pub const ENTRIES: usize = 100_000;

pub fn huge_folder() -> TempDir {
    let temp = TempDir::new().expect("temp dir");
    for index in 0..ENTRIES {
        let name = format!("file-{index:06}.txt");
        if index % 10 == 0 {
            fs::create_dir(temp.path().join(name)).expect("dir");
        } else {
            fs::write(temp.path().join(name), b"x").expect("file");
        }
    }
    temp
}
