//! An archive password: kept in memory only, overwritten when it is dropped, and never shown by `Debug`.

use std::fmt;

use zeroize::Zeroizing;

/// The password of an archive, as the user typed it. It has no `Display`, no `Serialize` and a `Debug` that hides it,
/// so no log line, event or saved state can hold it by accident.
#[derive(Clone)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The password's bytes, for a key derivation. The caller must not keep them longer than it needs them.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("Password(..)")
    }
}

/// Whether `password` opens the archive of `index` in `file`, judged cheaply (nothing is decoded): a zip by the check
/// byte or verifier of its first encrypted entry, a 7z by its headers and the start of its first encrypted entry. A format with nothing to check ahead accepts any password; its later
/// failure asks again. A zip without an encrypted entry accepts too. Errors are those of reading the archive.
pub fn check_password(
    index: &super::index::ArchiveIndex,
    file: &dyn super::file::ReadAt,
    password: &Password,
) -> std::io::Result<bool> {
    match index.format() {
        crate::model::ArchiveFormat::Zip => super::zip::check_password(index, file, password),
        crate::model::ArchiveFormat::SevenZ => super::sevenz::check_password(file, password),
        _ => Ok(true),
    }
}

/// [`check_password`] for what is about to seal new content with the password (a rewrite of an archive): a zip is judged by
/// the whole of the encrypted entry that declares the least (see [`super::zip::confirm_password`]) and a 7z by the whole of
/// an encrypted entry that starts a block (see [`super::sevenz::confirm_password`]), so the checksum that ends it is
/// checked. Only that one entry is read: an archive whose entries are sealed under different passwords is not fully
/// verified. Errors are those of reading the archive, a raised `cancel` included; a 7z with encrypted content that cannot
/// be checked at all is `Unsupported`.
pub fn confirm_password(
    index: &super::index::ArchiveIndex,
    file: &dyn super::file::ReadAt,
    cancel: &std::sync::atomic::AtomicBool,
    password: &Password,
) -> std::io::Result<bool> {
    match index.format() {
        crate::model::ArchiveFormat::Zip => super::zip::confirm_password(index, file, cancel, password),
        crate::model::ArchiveFormat::SevenZ => super::sevenz::confirm_password(file, cancel, password),
        _ => check_password(index, file, password),
    }
}
