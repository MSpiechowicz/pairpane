//! Update commands: check the release manifest, install the update it announces, relaunch.
//!
//! The UI checks at startup and from the native menu, and installs only after the user confirms. Every request
//! goes to [`UPDATE_ENDPOINT`] or the download it lists, which must be exactly this installation's artifact of that
//! release on GitHub (`check_announced`). A download is installed only when its signature matches the public key
//! built into the app, names that file and was made for the announced version (`requireSignedVersion`), and that
//! version is newer than the running one (downgrades are off). `.deb`/`.rpm` installations are updated by the
//! package manager, never in-app.

use std::time::Duration;

use pairpane_core::model::{IpcErrorCode, UpdateInfo, UpdateProgress};
use pairpane_core::update::{Arch, Installer, ProgressTracker, UPDATE_ENDPOINT, check_announced, update_asset};
use tauri::ipc::Channel;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, State, Url};
use tauri_plugin_updater::{Error as UpdaterError, Update, Updater, UpdaterExt};
use time::format_description::well_known::Rfc3339;

use crate::error::{CommandError, CommandResult};
use crate::updates::Updates;

/// Longest wait for a connection to the update server.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Longest wait for the next bytes of a response; a stalled check or download fails after it.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The newer release the manifest announces, or `null` when this version is current. `unsupported` when this
/// build has no updater public key or is a development build (nothing is requested then).
#[tauri::command]
pub async fn update_check(app: AppHandle, updates: State<'_, Updates>) -> CommandResult<Option<UpdateInfo>> {
    // A dev build must not update itself: the updater would overwrite `target/debug` with the released app.
    if cfg!(debug_assertions) {
        return Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "updates are off in development builds",
        ));
    }

    check(&updates, this_installation(), |pubkey| updater(&app, pubkey)).await
}

/// Downloads, verifies and installs the update the last `update_check` found, which must be `version` (the one the
/// user confirmed), reporting progress on `on_progress`. On Windows the installer takes over and the app exits;
/// elsewhere the new version runs after `app_relaunch`. One install runs at a time.
#[tauri::command]
pub async fn update_install(
    updates: State<'_, Updates>,
    version: String,
    on_progress: Channel<UpdateProgress>,
) -> CommandResult<()> {
    install(&updates, &version, &on_progress).await
}

/// Restarts the app after an update was installed.
#[tauri::command]
pub fn app_relaunch(app: AppHandle) {
    tracing::info!("relaunching after an update");
    app.request_restart();
}

/// The updater for this build: the fixed endpoint, `pubkey`, and a client that speaks HTTPS only (for the check
/// and the download alike).
fn updater(app: &AppHandle, pubkey: &str) -> Result<Updater, UpdaterError> {
    let endpoint = Url::parse(UPDATE_ENDPOINT)?;

    app.updater_builder()
        .pubkey(pubkey)
        .endpoints(vec![endpoint])?
        .configure_client(|client| {
            client
                .https_only(true)
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
        })
        .build()
}

/// How this copy was installed and for which CPU, from the bundle marker the bundler patches into each binary.
fn this_installation() -> Option<(Installer, Arch)> {
    let installer = match tauri::utils::platform::bundle_type()? {
        BundleType::AppImage => Installer::AppImage,
        BundleType::Deb | BundleType::Rpm => Installer::LinuxPackage,
        BundleType::App | BundleType::Dmg => Installer::MacApp,
        BundleType::Nsis => Installer::Nsis,
        BundleType::Msi => Installer::Msi,
    };

    let arch = if cfg!(target_arch = "aarch64") {
        Arch::Aarch64
    } else {
        Arch::X86_64
    };

    Some((installer, arch))
}

async fn check(
    updates: &Updates,
    installation: Option<(Installer, Arch)>,
    build: impl FnOnce(&str) -> Result<Updater, UpdaterError>,
) -> CommandResult<Option<UpdateInfo>> {
    let pubkey = updates.pubkey()?;

    // Refused before any network access: a binary run straight from a build folder, or a .deb/.rpm install.
    let Some((installer, arch)) = installation else {
        return Err(CommandError::new(
            IpcErrorCode::Unsupported,
            "this copy of Pairpane was not installed from a release, so it cannot update itself",
        ));
    };
    update_asset(installer, arch, "0.0.0").map_err(|reason| CommandError::new(IpcErrorCode::Unsupported, reason))?;

    // Whatever an earlier check found is forgotten: if this one fails or is refused, nothing stays installable.
    updates.set_pending(None);

    let updater = build(pubkey).map_err(|error| updater_error("cannot check for updates", error))?;

    let found = updater
        .check()
        .await
        .map_err(|error| updater_error("cannot check for updates", error))?;

    if let Some(update) = &found {
        let asset = update_asset(installer, arch, &update.version)
            .map_err(|reason| CommandError::new(IpcErrorCode::Unsupported, reason))?;

        if let Err(reason) = check_announced(update.download_url.as_str(), &update.signature, &update.version, &asset) {
            tracing::warn!(%reason, "refused an announced update");
            return Err(CommandError::new(IpcErrorCode::Forbidden, reason));
        }
    }

    let info = found.as_ref().map(announced);
    tracing::info!(available = ?info.as_ref().map(|info| &info.version), "update check finished");

    updates.set_pending(found);
    Ok(info)
}

fn announced(update: &Update) -> UpdateInfo {
    let date = update.date.and_then(|date| date.format(&Rfc3339).ok());

    UpdateInfo::announced(
        update.version.clone(),
        update.current_version.clone(),
        date,
        update.body.clone(),
    )
}

async fn install(updates: &Updates, version: &str, on_progress: &Channel<UpdateProgress>) -> CommandResult<()> {
    updates.pubkey()?;

    let Ok(_installing) = updates.installing.try_lock() else {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "an update is already being installed",
        ));
    };

    let update = updates
        .pending()
        .ok_or_else(|| CommandError::new(IpcErrorCode::NotFound, "no update to install; check for updates first"))?;

    // A later check may have found a newer release than the one the user confirmed.
    if update.version != version {
        return Err(CommandError::new(
            IpcErrorCode::InvalidInput,
            "the available update changed; check for updates again",
        ));
    }

    let mut tracker = ProgressTracker::default();
    let report = |progress: UpdateProgress| {
        // A closed channel only means the page went away; the install goes on.
        on_progress.send(progress).ok();
    };

    // `download` verifies the signature (and, with requireSignedVersion, its version) before returning the bytes.
    let bytes = update
        .download(
            |len, total| {
                if let Some(progress) = tracker.chunk(len, total) {
                    report(progress);
                }
            },
            || {},
        )
        .await
        .map_err(|error| updater_error("cannot download the update", error))?;

    report(tracker.installing());

    let installer = update.clone();
    tauri::async_runtime::spawn_blocking(move || installer.install(bytes))
        .await
        .map_err(|error| CommandError::new(IpcErrorCode::Internal, format!("the install task failed: {error}")))?
        .map_err(|error| updater_error("cannot install the update", error))?;

    tracing::info!(version = %update.version, "update installed");
    updates.set_pending(None);
    report(tracker.done());

    Ok(())
}

/// Maps an updater failure onto the IPC error the UI branches on. Signature failures are `forbidden` (shown with
/// a generic message); the details go to the log only.
fn updater_error(context: &str, error: UpdaterError) -> CommandError {
    tracing::warn!(%error, "{context}");

    let code = match error {
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion => IpcErrorCode::Forbidden,

        UpdaterError::ReleaseNotFound
        | UpdaterError::Serialization(_)
        | UpdaterError::Semver(_)
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_)
        | UpdaterError::InvalidUpdaterFormat
        | UpdaterError::BinaryNotFoundInArchive => IpcErrorCode::Protocol,

        UpdaterError::UnsupportedArch
        | UpdaterError::UnsupportedOs
        | UpdaterError::FailedToDetermineExtractPath
        | UpdaterError::EmptyEndpoints
        | UpdaterError::InsecureTransportProtocol => IpcErrorCode::Unsupported,

        UpdaterError::AuthenticationFailed | UpdaterError::DebInstallFailed | UpdaterError::PackageInstallFailed => {
            IpcErrorCode::Process
        }

        UpdaterError::Reqwest(ref request) if request.is_timeout() => IpcErrorCode::Timeout,

        _ => IpcErrorCode::Io,
    };

    CommandError::new(code, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tauri::async_runtime::block_on;
    use tauri::ipc::InvokeResponseBody;

    use super::*;

    fn never_built(_pubkey: &str) -> Result<Updater, UpdaterError> {
        panic!("an updater was built, so the network could have been used");
    }

    fn configured() -> Updates {
        Updates::new(Some("a key the config check accepted".to_owned()))
    }

    /// A channel that records what it receives.
    fn collecting_channel() -> (Channel<UpdateProgress>, Arc<Mutex<Vec<InvokeResponseBody>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let channel = Channel::new(move |body| {
            sink.lock().expect("not poisoned").push(body);
            Ok(())
        });

        (channel, received)
    }

    const APPIMAGE: Option<(Installer, Arch)> = Some((Installer::AppImage, Arch::X86_64));

    #[test]
    fn without_a_key_checking_is_unsupported_and_requests_nothing() {
        let error = block_on(check(&Updates::new(None), APPIMAGE, never_built)).expect_err("updates are off");

        assert_eq!(error.code(), IpcErrorCode::Unsupported);
    }

    #[test]
    fn packages_msi_and_unreleased_binaries_are_refused_before_any_request() {
        let package = Some((Installer::LinuxPackage, Arch::X86_64));
        let msi = Some((Installer::Msi, Arch::X86_64));

        for installation in [package, msi, None] {
            let error = block_on(check(&configured(), installation, never_built)).expect_err("not updated in-app");

            assert_eq!(error.code(), IpcErrorCode::Unsupported);
        }
    }

    #[test]
    fn without_a_key_installing_is_unsupported_and_reports_nothing() {
        let (channel, received) = collecting_channel();

        let error = block_on(install(&Updates::new(None), "0.2.0", &channel)).expect_err("updates are off");

        assert_eq!(error.code(), IpcErrorCode::Unsupported);
        assert!(received.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn installing_needs_an_update_found_by_a_check() {
        let (channel, received) = collecting_channel();

        let error = block_on(install(&configured(), "0.2.0", &channel)).expect_err("nothing to install");

        assert_eq!(error.code(), IpcErrorCode::NotFound);
        assert!(received.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn a_second_install_is_refused_while_one_runs() {
        let updates = configured();
        let (channel, _received) = collecting_channel();
        let _running = updates.installing.try_lock().expect("no install yet");

        let error = block_on(install(&updates, "0.2.0", &channel)).expect_err("one install at a time");

        assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    }

    #[test]
    fn updater_failures_map_to_the_codes_the_ui_branches_on() {
        let mismatch = UpdaterError::SignedVersionMismatch {
            signed: "0.1.0".to_owned(),
            announced: "0.2.0".to_owned(),
        };

        let cases = [
            (mismatch, IpcErrorCode::Forbidden),
            (UpdaterError::MissingSignedVersion, IpcErrorCode::Forbidden),
            (UpdaterError::ReleaseNotFound, IpcErrorCode::Protocol),
            (
                UpdaterError::TargetNotFound("linux-x86_64".to_owned()),
                IpcErrorCode::Protocol,
            ),
            (UpdaterError::UnsupportedOs, IpcErrorCode::Unsupported),
            (UpdaterError::InsecureTransportProtocol, IpcErrorCode::Unsupported),
            (UpdaterError::PackageInstallFailed, IpcErrorCode::Process),
            (UpdaterError::Network("status 503".to_owned()), IpcErrorCode::Io),
        ];

        for (error, code) in cases {
            let description = error.to_string();

            assert_eq!(updater_error("checking", error).code(), code, "{description}");
        }
    }
}
