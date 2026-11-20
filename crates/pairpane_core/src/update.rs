//! The in-app updater's rules that do not depend on Tauri: where updates come from, whether this build can
//! verify them, what the UI is told about a release, and how download progress is reported.
//!
//! The shell checks [`UPDATE_ENDPOINT`] with `tauri-plugin-updater`, which verifies every download against the
//! public key built into the app (and, with `requireSignedVersion`, that the signature was made for the announced
//! version) before installing it. A build without a usable key (see [`configured_pubkey`]) never contacts it.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::PublicKey;

use crate::model::{MAX_UPDATE_NOTES_BYTES, UpdateInfo, UpdatePhase, UpdateProgress};

/// The release manifest written by the release workflow's publish job. Only published releases are served
/// here (drafts never are). The only URL the updater requests; the downloads it lists must be HTTPS as well.
pub const UPDATE_ENDPOINT: &str = "https://github.com/MSpiechowicz/pairpane/releases/latest/download/latest.json";

/// What `update_check` and `update_install` report when the build has no usable public key.
pub const UPDATES_NOT_CONFIGURED: &str = "updates are not configured for this build";

/// The configured updater public key when it is one the updater can verify signatures with: the base64-encoded
/// content of a minisign `.pub` file, as `tauri signer generate` writes it.
///
/// `None` for a missing or empty value, a placeholder, or anything else that does not decode to a minisign public
/// key. The updater decodes the key the same way.
pub fn configured_pubkey(value: Option<&str>) -> Option<&str> {
    let value = value?;

    let decoded = STANDARD.decode(value).ok()?;
    let text = std::str::from_utf8(&decoded).ok()?;

    PublicKey::decode(text).ok().map(|_| value)
}

impl UpdateInfo {
    /// The info shown for an announced release. Empty notes become `None`; notes longer than
    /// [`MAX_UPDATE_NOTES_BYTES`] are cut at a character boundary.
    pub fn announced(version: String, current_version: String, date: Option<String>, notes: Option<String>) -> Self {
        let notes = notes
            .filter(|notes| !notes.trim().is_empty())
            .map(|notes| cut_to(notes, MAX_UPDATE_NOTES_BYTES));

        Self {
            version,
            current_version,
            date,
            notes,
        }
    }
}

fn cut_to(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }

    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }

    text.truncate(end);
    text
}

/// How this copy of Pairpane was installed, as far as updating it is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// The Linux AppImage (what install.sh installs); updated in place, no password.
    AppImage,
    /// A Linux .deb or .rpm package: updated by the system package manager, never in-app.
    LinuxPackage,
    /// The macOS app bundle.
    MacApp,
    /// The Windows NSIS installer (what install.ps1 installs).
    Nsis,
    /// The Windows MSI installer: elevates through UAC, so it is updated with a new installer, never in-app.
    Msi,
}

/// The CPU architecture the running build was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

/// Why this installation cannot be updated in-app.
pub const UPDATE_WITH_PACKAGE_MANAGER: &str =
    "this installation was made from a .deb or .rpm package; update it with your package manager";

/// Why an MSI installation cannot be updated in-app.
pub const UPDATE_WITH_INSTALLER: &str =
    "this installation was made with the MSI installer; update it with the installer from the releases page";

/// No updater artifact is published for this platform.
pub const NO_UPDATE_FOR_PLATFORM: &str = "no update builds are published for this platform";

/// The file name of the updater artifact for `installer` on `arch` at `version`, exactly as the release pipeline
/// publishes it (`.github/workflows/release.yml`).
///
/// `.deb`/`.rpm` and MSI installations are refused: the updater would hand the package to `pkexec dpkg`/`rpm` or an
/// elevated `msiexec` from a temp file other processes of the user could swap before the password/UAC prompt.
pub fn update_asset(installer: Installer, arch: Arch, version: &str) -> Result<String, &'static str> {
    match (installer, arch) {
        (Installer::LinuxPackage, _) => Err(UPDATE_WITH_PACKAGE_MANAGER),
        (Installer::AppImage, Arch::X86_64) => Ok(format!("Pairpane_{version}_amd64.AppImage")),
        (Installer::MacApp, Arch::Aarch64) => Ok(format!("Pairpane_{version}_aarch64.app.tar.gz")),
        (Installer::MacApp, Arch::X86_64) => Ok(format!("Pairpane_{version}_x64.app.tar.gz")),
        (Installer::Nsis, Arch::X86_64) => Ok(format!("Pairpane_{version}_x64-setup.exe")),
        (Installer::Msi, _) => Err(UPDATE_WITH_INSTALLER),
        _ => Err(NO_UPDATE_FOR_PLATFORM),
    }
}

/// Where release `version` publishes `asset`: this repository's GitHub release, over HTTPS.
pub fn release_download_url(version: &str, asset: &str) -> String {
    format!("https://github.com/MSpiechowicz/pairpane/releases/download/v{version}/{asset}")
}

/// Checks that an announced update points at exactly this build's artifact: the download is `asset` of release
/// `version` in this repository, and its signature's trusted comment names that file and version.
///
/// The updater verifies the signature itself (including the trusted comment) before installing; this check makes
/// sure a valid signature for a *different* file of the release (another platform or installer type) is refused,
/// and that nothing is downloaded from anywhere else.
pub fn check_announced(download_url: &str, signature: &str, version: &str, asset: &str) -> Result<(), String> {
    let expected_url = release_download_url(version, asset);
    if download_url != expected_url {
        return Err(format!("the update points at {download_url}, expected {expected_url}"));
    }

    let decoded = STANDARD
        .decode(signature.trim())
        .map_err(|_| "the update signature is not base64".to_owned())?;
    let text = std::str::from_utf8(&decoded).map_err(|_| "the update signature is not text".to_owned())?;
    let parsed =
        minisign_verify::Signature::decode(text).map_err(|_| "the update signature is malformed".to_owned())?;

    let fields: Vec<&str> = parsed.trusted_comment().split('\t').collect();
    let names_file = fields.contains(&format!("file:{asset}").as_str());
    let names_version = fields.contains(&format!("version:{version}").as_str());

    if !names_file || !names_version {
        return Err(format!("the update signature is not for {asset} version {version}"));
    }

    Ok(())
}

/// How many bytes pass between two download reports when the size is unknown.
pub const UNKNOWN_SIZE_REPORT_STEP: u64 = 512 * 1024;

/// Turns the updater's per-chunk download callbacks into [`UpdateProgress`] reports.
///
/// Downloads arrive in many small chunks, so [`chunk`](Self::chunk) reports at most once per percent of a known
/// size (or every [`UNKNOWN_SIZE_REPORT_STEP`] bytes otherwise), plus the first chunk and the last one of a known
/// size.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    downloaded: u64,
    total: Option<u64>,
    reported_at: Option<u64>,
}

impl ProgressTracker {
    /// Records a downloaded chunk of `len` bytes; `total` is the size the server announced. Returns the report to
    /// send, if one is due. A size the download has outgrown is no longer trusted.
    pub fn chunk(&mut self, len: usize, total: Option<u64>) -> Option<UpdateProgress> {
        let len = u64::try_from(len).unwrap_or(u64::MAX);
        self.downloaded = self.downloaded.saturating_add(len);
        self.total = total.filter(|total| *total >= self.downloaded);

        if !self.report_due() {
            return None;
        }

        self.reported_at = Some(self.downloaded);
        Some(self.progress(UpdatePhase::Downloading))
    }

    fn report_due(&self) -> bool {
        let Some(reported_at) = self.reported_at else {
            return true;
        };

        if self.total == Some(self.downloaded) {
            return true;
        }

        let step = match self.total {
            Some(total) => (total / 100).max(1),
            None => UNKNOWN_SIZE_REPORT_STEP,
        };

        self.downloaded - reported_at >= step
    }

    /// The report sent once the download is verified and installing starts.
    pub fn installing(&self) -> UpdateProgress {
        self.progress(UpdatePhase::Installing)
    }

    /// The report sent once the update is installed.
    pub fn done(&self) -> UpdateProgress {
        self.progress(UpdatePhase::Done)
    }

    fn progress(&self, phase: UpdatePhase) -> UpdateProgress {
        UpdateProgress {
            phase,
            downloaded: self.downloaded,
            total: self.total,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// The real updater public key (base64 of the `.pub` file, as `tauri signer generate` writes it).
    const VALID_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU2NEFEQzdGMkJFODhGOEIKUldTTGorZ3JmOXhLNW5JZ3g3NGMrcGtQSWJmOStMUGZ5Qk1wTXhyaHVhSkdjcExvU3VEM0tPVjIK";

    #[test]
    fn a_minisign_public_key_is_usable() {
        assert_eq!(configured_pubkey(Some(VALID_KEY)), Some(VALID_KEY));
    }

    #[test]
    fn missing_placeholder_or_garbage_keys_turn_updates_off() {
        for value in [None, Some(""), Some("NOT_CONFIGURED: see docs"), Some("bm90IGEga2V5")] {
            assert_eq!(configured_pubkey(value), None, "{value:?}");
        }
    }

    #[test]
    fn the_endpoint_is_the_published_https_manifest() {
        assert!(UPDATE_ENDPOINT.starts_with("https://github.com/MSpiechowicz/pairpane/releases/latest/download/"));
        assert!(UPDATE_ENDPOINT.ends_with("/latest.json"));
    }

    #[test]
    fn empty_notes_are_dropped_and_long_notes_cut_at_a_char_boundary() {
        let empty = UpdateInfo::announced("0.2.0".into(), "0.1.1".into(), None, Some("  \n".into()));
        assert_eq!(empty.notes, None);

        let long = "é".repeat(MAX_UPDATE_NOTES_BYTES);
        let cut = UpdateInfo::announced("0.2.0".into(), "0.1.1".into(), None, Some(long))
            .notes
            .expect("kept");
        assert!(cut.len() <= MAX_UPDATE_NOTES_BYTES);
        assert!(cut.chars().all(|c| c == 'é'));
    }

    #[test]
    fn progress_is_reported_first_then_per_percent_then_at_the_end() {
        let mut tracker = ProgressTracker::default();

        let first = tracker.chunk(10, Some(1000)).expect("first chunk is reported");
        assert_eq!((first.downloaded, first.total), (10, Some(1000)));
        assert_eq!(first.phase, UpdatePhase::Downloading);

        assert_eq!(
            tracker.chunk(5, Some(1000)),
            None,
            "less than a percent since the last report"
        );
        assert!(
            tracker.chunk(5, Some(1000)).is_some(),
            "a full percent since the last report"
        );

        let last = tracker.chunk(980, Some(1000)).expect("the last chunk is reported");
        assert_eq!(last.downloaded, 1000);

        assert_eq!(tracker.installing().phase, UpdatePhase::Installing);
        assert_eq!(tracker.done().phase, UpdatePhase::Done);
    }

    #[test]
    fn each_installation_gets_exactly_its_own_artifact() {
        let cases = [
            (Installer::AppImage, Arch::X86_64, "Pairpane_0.2.0_amd64.AppImage"),
            (Installer::MacApp, Arch::Aarch64, "Pairpane_0.2.0_aarch64.app.tar.gz"),
            (Installer::MacApp, Arch::X86_64, "Pairpane_0.2.0_x64.app.tar.gz"),
            (Installer::Nsis, Arch::X86_64, "Pairpane_0.2.0_x64-setup.exe"),
        ];

        for (installer, arch, asset) in cases {
            assert_eq!(update_asset(installer, arch, "0.2.0").as_deref(), Ok(asset));
        }
    }

    #[test]
    fn linux_packages_and_unbuilt_platforms_are_not_updated_in_app() {
        assert_eq!(
            update_asset(Installer::LinuxPackage, Arch::X86_64, "0.2.0"),
            Err(UPDATE_WITH_PACKAGE_MANAGER)
        );
        assert_eq!(
            update_asset(Installer::Msi, Arch::X86_64, "0.2.0"),
            Err(UPDATE_WITH_INSTALLER)
        );
        assert_eq!(
            update_asset(Installer::AppImage, Arch::Aarch64, "0.2.0"),
            Err(NO_UPDATE_FOR_PLATFORM)
        );
        assert_eq!(
            update_asset(Installer::Nsis, Arch::Aarch64, "0.2.0"),
            Err(NO_UPDATE_FOR_PLATFORM)
        );
    }

    /// The real signature of `Pairpane_0.1.1_amd64.deb` from the published v0.1.1 release (`latest.json`).
    const DEB_011_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTTGorZ3JmOXhLNXYyYSt4bzQ1S0UxWDlIWlNyQ2RGcFJzMEdBRU5DOFlPTm1Mc2VqTVhaQTNLMnl4NGdEVUdXb1pKUjN3ODFXaWo1YjVHSlV6Y3Q2eUJLRUVBay9peHd3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1MTcyNzQ5CWZpbGU6UGFpcnBhbmVfMC4xLjFfYW1kNjQuZGViCXZlcnNpb246MC4xLjEKZlNaUGh6Z2hkTDZJN3JWNE9JRzFQUjBIMUFUVC9OS2xxMWRtWDcvdmhERVMzUWx4L1h3UWxwMzVzbVhWVFpXV2xEdS9za1J1VGtNM05yNWNQSEw1QUE9PQo=";

    fn deb_url() -> String {
        release_download_url("0.1.1", "Pairpane_0.1.1_amd64.deb")
    }

    #[test]
    fn an_update_for_exactly_this_artifact_is_accepted() {
        let result = check_announced(&deb_url(), DEB_011_SIGNATURE, "0.1.1", "Pairpane_0.1.1_amd64.deb");

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn another_host_another_release_or_another_file_is_refused() {
        let asset = "Pairpane_0.1.1_amd64.deb";
        let elsewhere = "https://example.com/Pairpane_0.1.1_amd64.deb";
        let other_release = release_download_url("0.1.0", asset);

        assert!(check_announced(elsewhere, DEB_011_SIGNATURE, "0.1.1", asset).is_err());
        assert!(check_announced(&other_release, DEB_011_SIGNATURE, "0.1.1", asset).is_err());

        // A genuine signature, but for the .deb: refused for the AppImage at the AppImage's own URL.
        let appimage = "Pairpane_0.1.1_amd64.AppImage";
        let appimage_url = release_download_url("0.1.1", appimage);
        assert!(check_announced(&appimage_url, DEB_011_SIGNATURE, "0.1.1", appimage).is_err());

        // The right file, announced under another version.
        let as_newer = release_download_url("0.1.2", asset);
        assert!(check_announced(&as_newer, DEB_011_SIGNATURE, "0.1.2", asset).is_err());
    }

    #[test]
    fn a_malformed_signature_is_refused() {
        for signature in ["", "not base64!", "bm90IGEgc2lnbmF0dXJl"] {
            assert!(check_announced(&deb_url(), signature, "0.1.1", "Pairpane_0.1.1_amd64.deb").is_err());
        }
    }

    #[test]
    fn a_size_the_download_outgrew_is_dropped() {
        let mut tracker = ProgressTracker::default();

        tracker.chunk(100, Some(50));

        assert_eq!(tracker.done().total, None);
    }
}
