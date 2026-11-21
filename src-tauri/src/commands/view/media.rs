//! The tokens of the media scheme. A token stands for the open file of one session, not for a path: the scheme
//! (`serve.rs`) serves what the token holds and never opens anything by name.
//!
//! A token is 128 random bits as 32 lowercase hex characters. A session has at most one: minting another revokes the
//! earlier one. At most [`MAX_TOKENS`] tokens live at once (the least recently used one is revoked to make room), a
//! token not used for [`IDLE`] stops working, and closing or evicting the session revokes it.

use std::fmt::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcErrorCode, ViewKind};
use pairpane_core::view::ViewFile;
use tauri::State;

use super::{Released, Sessions, Viewer, invalid};
use crate::error::{CommandError, CommandResult};

/// The tokens that live at once.
pub(super) const MAX_TOKENS: usize = 4;

/// How long a token that nobody uses stays valid.
pub(super) const IDLE: Duration = Duration::from_secs(30 * 60);

/// What a token serves: the file of its session and the type to tell the webview.
pub(super) struct Token {
    pub(super) session: u32,
    pub(super) file: Arc<ViewFile>,
    pub(super) mime: String,
    last_used: Instant,
}

impl Viewer {
    /// A token for the media file of `session`, valid from `now`. Audio and video only.
    pub(super) fn mint(&self, session: u32, now: Instant) -> CommandResult<String> {
        let handle = self.handle(session)?;
        let mime = match handle.kind {
            ViewKind::Audio { mime } | ViewKind::Video { mime } => mime,
            _ => return Err(invalid("the file is not audio or video")),
        };
        let text = random_token()?;

        let mut state = self.lock();
        let mut released = state.purge(now);
        released.extend(state.revoke_session(session));
        while state.tokens.len() >= MAX_TOKENS {
            released.extend(state.revoke_least_used());
        }
        // The session may have been closed since it was looked up; then nothing is left to serve.
        if !state.sessions.iter().any(|found| found.id == session) {
            drop(state);
            drop(released);

            return Err(super::no_session());
        }
        state.tokens.insert(
            text.clone(),
            Token {
                session,
                file: handle.file,
                mime,
                last_used: now,
            },
        );
        drop(state);
        drop(released);

        Ok(text)
    }

    /// The file and type behind `text`, counting this as a use at `now`; `None` for an unknown, revoked or idle token
    /// (an idle one is dropped here).
    pub(super) fn token(&self, text: &str, now: Instant) -> Option<(Arc<ViewFile>, String)> {
        let mut state = self.lock();
        let released = state.purge(now);

        let found = state.tokens.get_mut(text).map(|token| {
            token.last_used = now;

            (Arc::clone(&token.file), token.mime.clone())
        });
        drop(state);
        drop(released);

        found
    }
}

impl Sessions {
    /// Drops the tokens that nobody used for [`IDLE`] before `now`.
    fn purge(&mut self, now: Instant) -> Released {
        self.tokens
            .extract_if(|_, token| now.saturating_duration_since(token.last_used) >= IDLE)
            .map(|(_, token)| token.file)
            .collect()
    }

    fn revoke_session(&mut self, session: u32) -> Released {
        self.tokens
            .extract_if(|_, token| token.session == session)
            .map(|(_, token)| token.file)
            .collect()
    }

    fn revoke_least_used(&mut self) -> Released {
        let oldest = self
            .tokens
            .iter()
            .min_by_key(|(_, token)| token.last_used)
            .map(|(text, _)| text.clone());

        oldest
            .and_then(|text| self.tokens.remove(&text))
            .map(|token| token.file)
            .into_iter()
            .collect()
    }
}

/// 128 bits from the system's generator, as 32 lowercase hex characters.
fn random_token() -> CommandResult<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        tracing::warn!(%error, "no random bytes for a media token");

        CommandError::new(IpcErrorCode::Internal, "no media token could be made")
    })?;

    let mut text = String::with_capacity(32);
    for byte in bytes {
        // Writing to a string cannot fail.
        let _ = write!(text, "{byte:02x}");
    }

    Ok(text)
}

/// A token for the session's audio or video file. The page puts `pairpane-media://localhost/<token>` (on Windows
/// `http://pairpane-media.localhost/<token>`) in an `<audio>` or `<video>` element. It replaces the session's earlier
/// token, and it stops working when the session ends or after 30 idle minutes.
#[tauri::command]
pub async fn view_media(viewer: State<'_, Viewer>, session: u32) -> CommandResult<String> {
    viewer.mint(session, Instant::now())
}
