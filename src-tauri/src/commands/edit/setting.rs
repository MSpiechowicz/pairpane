//! `settings_editor`: the external editor the settings file names, for the settings page to show. Reading it changes
//! nothing; the page never writes the file (see the module above).

use pairpane_core::model::{EditorSetting, IpcErrorCode};
use pairpane_core::settings;
use pairpane_core::view::is_refusal;
use tauri::State;

use super::{EDIT_THREADS, Editor};
use crate::commands::items::{begin, blocking};
use crate::error::{CommandError, CommandResult};

impl Editor {
    /// The external editor of the settings file, `None` when it sets none (the built-in editor) or there is no place for
    /// settings on this system. A file that is not valid is refused with its fixed text, as F4 would be.
    async fn editor_setting(&self) -> CommandResult<Option<EditorSetting>> {
        let Some(file) = self.settings.clone() else {
            return Ok(None);
        };
        let permit = begin(&self.threads, EDIT_THREADS)?;

        blocking(permit, move || {
            let loaded = settings::load(&file).map_err(|error| {
                if is_refusal(&error) {
                    CommandError::new(IpcErrorCode::InvalidInput, error.to_string())
                } else {
                    tracing::debug!(%error, "cannot read the settings file");
                    CommandError::new(IpcErrorCode::Io, "the settings file cannot be read")
                }
            })?;

            Ok(loaded.editor().map(|editor| EditorSetting {
                program: editor.program.trim().to_owned(),
                args: editor.args.clone(),
            }))
        })
        .await?
    }
}

/// The external editor the user's settings file names (`null`: the built-in editor), for the settings page to show.
#[tauri::command]
pub async fn settings_editor(editor: State<'_, Editor>) -> CommandResult<Option<EditorSetting>> {
    editor.editor_setting().await
}
