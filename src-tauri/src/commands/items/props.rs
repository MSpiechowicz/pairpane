//! The Properties of one item: `ops_properties`, a quick read of the item's metadata on a blocking thread of the same
//! cap as the other single-item commands (a read on a hung mount never returns and must not use up the thread pool).

use pairpane_core::listing::platform;
use pairpane_core::model::{IpcErrorCode, ItemProperties};
use pairpane_core::ops::{self, name_fits};
use tauri::State;

use super::{Items, QUICK_THREADS, begin, in_folder, item_error};
use crate::commands::ops::{check_name, failed, names_too_large};
use crate::commands::panes::resolve;
use crate::error::{CommandError, CommandResult};

/// What a failed read is called in "You do not have permission to ...".
const READ: &str = "read this item's properties";

impl Items {
    /// Reads the properties of the item `name` in the folder `path`, on a blocking thread.
    async fn properties(&self, path: String, name: String) -> CommandResult<ItemProperties> {
        let folder = resolve(Some(path), None)?;
        if !platform::is_plain_name(&name) {
            return Err(CommandError::new(
                IpcErrorCode::InvalidInput,
                "an item name must be a single plain name",
            ));
        }
        if !name_fits(&name) {
            return Err(names_too_large());
        }
        check_name(&folder, &name)?;
        let permit = begin(&self.quick, QUICK_THREADS)?;

        in_folder(permit, folder, move |actual| ops::properties(actual, &name))
            .await?
            .map_err(|error| failed(item_error(READ, &error)))
    }
}

/// Reads what the Properties dialog shows about the item `name` of the folder `path`: its kind, size, times, permissions,
/// owner and group, Windows attributes and, for a link, where it points. A folder's size is 0 (the pane measures it).
#[tauri::command]
pub async fn ops_properties(items: State<'_, Items>, path: String, name: String) -> CommandResult<ItemProperties> {
    items.properties(path, name).await
}

#[cfg(test)]
mod tests {
    use pairpane_core::model::EntryKind;
    use tauri::async_runtime::block_on;

    use super::*;
    use crate::commands::testing::root;

    fn code<T: std::fmt::Debug>(result: CommandResult<T>) -> IpcErrorCode {
        result.expect_err("refused").code()
    }

    #[test]
    fn a_name_that_is_not_plain_is_refused_before_a_thread_is_taken() {
        let items = Items::default();
        let _held: Vec<_> = (0..QUICK_THREADS)
            .map(|_| items.quick.acquire(QUICK_THREADS).expect("a place"))
            .collect();

        for name in ["", ".", "..", "a/b", "../x", "x\0y"] {
            assert_eq!(
                code(block_on(items.properties(root(), name.to_owned()))),
                IpcErrorCode::InvalidInput,
                "{name:?}"
            );
        }
        assert_eq!(
            code(block_on(items.properties(root(), "n".repeat(300)))),
            IpcErrorCode::InvalidInput
        );
        assert_eq!(
            code(block_on(items.properties("relative".to_owned(), "a".to_owned()))),
            IpcErrorCode::InvalidInput
        );
        // A good request waits for a place when the caps are full.
        assert_eq!(
            code(block_on(items.properties(root(), "a".to_owned()))),
            IpcErrorCode::Busy
        );
    }

    #[test]
    fn the_properties_of_an_item_are_read_and_a_missing_item_has_no_path_in_its_error() {
        let folder = tempfile::tempdir().expect("folder");
        std::fs::write(folder.path().join("a.txt"), "hello").expect("file");
        let path = folder.path().to_string_lossy().into_owned();
        let items = Items::default();

        let shown = block_on(items.properties(path.clone(), "a.txt".to_owned())).expect("properties");
        assert_eq!(
            (shown.name.as_str(), shown.kind, shown.size),
            ("a.txt", EntryKind::File, 5)
        );

        let error = block_on(items.properties(path.clone(), "gone".to_owned())).expect_err("missing");
        assert_eq!(error.code(), IpcErrorCode::NotFound);
        assert!(!serde_json::to_string(&error).expect("json").contains(&path));
    }
}
