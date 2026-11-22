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
