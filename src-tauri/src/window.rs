//! The main window, created from its `tauri.conf.json` entry (`"create": false`) so that
//! navigation and pop-up guards are attached before any page loads.

use tauri::webview::NewWindowResponse;
use tauri::{App, Manager, Runtime, Url, WebviewWindow, WebviewWindowBuilder};

/// Label of the only window.
pub const MAIN_WINDOW: &str = "main";

/// Release builds suppress the webview's default context menu (Reload, Back, Inspect, ...) except
/// over editable fields and selected text, where Copy/Paste stay useful. Tauri has no setting
/// for this; initialization scripts are injected by the webview itself, outside the page CSP.
const CONTEXT_MENU_SCRIPT: &str = r#"
window.addEventListener('contextmenu', (event) => {
  const target = event.target instanceof Element ? event.target : null;
  const editable = target?.closest('input, textarea, [contenteditable]:not([contenteditable="false"])');
  const selected = (window.getSelection()?.toString() ?? '') !== '';
  if (!editable && !selected) {
    event.preventDefault();
  }
}, { capture: true });
"#;

/// Creates the main window with:
/// - a navigation guard that only allows the app's own origin (the bundled frontend, or the
///   Vite dev server in `tauri dev`); external links must go through an allowlisted command;
/// - a pop-up guard that denies every `window.open` / `target="_blank"` request;
/// - the context-menu script in release builds.
pub fn create_main_window<R: Runtime>(app: &App<R>) -> tauri::Result<WebviewWindow<R>> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;

    let dev_url = if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    };

    let mut builder = WebviewWindowBuilder::from_config(app.handle(), &config)?
        .on_navigation(move |url| allow_navigation(url, dev_url.as_ref()))
        .on_new_window(|url, _features| {
            log_blocked("pop-up", &url);
            NewWindowResponse::Deny
        });

    if !cfg!(debug_assertions) {
        builder = builder.initialization_script(CONTEXT_MENU_SCRIPT);
    }

    builder.build()
}

/// Brings the main window to the front (used when a second instance is launched).
pub fn focus_main_window<R: Runtime, M: Manager<R>>(manager: &M) {
    let Some(window) = manager.get_webview_window(MAIN_WINDOW) else {
        return;
    };

    let raised = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());

    if let Err(error) = raised {
        tracing::warn!(%error, "cannot focus the main window");
    }
}

fn allow_navigation(url: &Url, dev_url: Option<&Url>) -> bool {
    let allowed = is_app_url(url, dev_url);

    if !allowed {
        log_blocked("navigation", url);
    }

    allowed
}

/// True for the bundled frontend and, in development only, for the dev server's origin.
fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    is_bundled_origin(url) || dev_url.is_some_and(|dev| dev.origin() == url.origin())
}

/// The bundled frontend is served as `tauri://localhost` on Linux/macOS and as
/// `http://tauri.localhost` on Windows (WebView2; `useHttpsScheme` is not set in `tauri.conf.json` —
/// change this check if it ever is). Only the exact form counts: any other scheme, host or an
/// explicit port would reach a real server, not the bundle.
fn is_bundled_origin(url: &Url) -> bool {
    if url.port().is_some() {
        return false;
    }

    if cfg!(windows) {
        url.scheme() == "http" && url.host_str() == Some("tauri.localhost")
    } else {
        url.scheme() == "tauri" && url.host_str() == Some("localhost")
    }
}

/// Logs only scheme and host: paths and queries may carry user content.
fn log_blocked(kind: &str, url: &Url) {
    tracing::warn!(
        kind,
        scheme = url.scheme(),
        host = url.host_str().unwrap_or_default(),
        "blocked a request to leave the app origin"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("valid test URL")
    }

    #[test]
    #[cfg(not(windows))]
    fn bundled_frontend_origin_is_allowed() {
        assert!(is_app_url(&url("tauri://localhost/index.html"), None));
    }

    #[test]
    #[cfg(windows)]
    fn bundled_frontend_origin_is_allowed() {
        assert!(is_app_url(&url("http://tauri.localhost/settings"), None));
        // Not the bundle while `useHttpsScheme` is off: WebView2 would send it to a real server.
        assert!(!is_app_url(&url("https://tauri.localhost/"), None));
        assert!(!is_app_url(&url("tauri://localhost/"), None));
    }

    #[test]
    #[cfg(not(windows))]
    fn windows_bundle_origin_is_blocked_elsewhere() {
        // Here it would reach whatever server answers for tauri.localhost, not the bundle.
        assert!(!is_app_url(&url("http://tauri.localhost/"), None));
        assert!(!is_app_url(&url("https://tauri.localhost/settings"), None));
    }

    #[test]
    fn bundle_origin_with_a_port_is_blocked() {
        assert!(!is_app_url(&url("http://tauri.localhost:8080/"), None));
        assert!(!is_app_url(&url("tauri://localhost:8080/"), None));
    }

    #[test]
    fn dev_server_is_allowed_only_when_configured() {
        let dev = url("http://localhost:1420");

        assert!(is_app_url(&url("http://localhost:1420/src/main.ts"), Some(&dev)));
        assert!(!is_app_url(&url("http://localhost:1420/"), None));
        assert!(!is_app_url(&url("http://localhost:9999/"), Some(&dev)));
    }

    #[test]
    fn foreign_origins_are_blocked() {
        let dev = url("http://localhost:1420");

        for raw in [
            "https://example.com/",
            "https://tauri.localhost.evil.com/",
            "tauri://evil/",
            "file:///etc/passwd",
            "data:text/html,hi",
            "javascript:alert(1)",
        ] {
            assert!(!is_app_url(&url(raw), Some(&dev)), "{raw} must be blocked");
        }
    }
}
