//! The properties of one item, for the Properties dialog.

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::listing::platform;
use crate::model::{EntryKind, ItemProperties, WindowsAttributes};

use super::report::refusal;

/// The properties of the item `name` of `folder` (see [`super::properties`]).
pub(super) fn properties(folder: &Path, name: &str) -> io::Result<ItemProperties> {
    if !platform::is_plain_name(name) || !super::name_fits(name) {
        return Err(refusal(
            io::ErrorKind::InvalidInput,
            "the name is not a plain file name",
        ));
    }

    let path = folder.join(name);
    let own = fs::symlink_metadata(&path)?;
    let link = own
        .file_type()
        .is_symlink()
        .then(|| fs::read_link(&path).ok())
        .flatten();

    // Like the panes: a link shows what it leads to, unless that cannot be followed.
    let followed = if link.is_some() {
        platform::follow(&path)
    } else {
        Some(own.clone())
    };
    let shown = followed.as_ref().unwrap_or(&own);

    let kind = match &followed {
        Some(metadata) if metadata.is_dir() => EntryKind::Dir,
        Some(metadata) if metadata.is_file() => EntryKind::File,
        _ => EntryKind::Other,
    };
    let (owner, group) = owner_and_group(shown);

    Ok(ItemProperties {
        name: name.to_owned(),
        kind,
        size: if kind == EntryKind::File { shown.len() } else { 0 },
        created: millis(shown.created()),
        modified: millis(shown.modified()),
        accessed: millis(shown.accessed()),
        mode: platform::mode(shown),
        owner,
        group,
        windows: windows_attributes(shown),
        link_target: link.map(|target| target.to_string_lossy().into_owned()),
    })
}

/// Milliseconds since the Unix epoch (negative before it); `None` when the filesystem does not say.
fn millis(time: io::Result<SystemTime>) -> Option<i64> {
    let time = time.ok()?;

    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}

/// The owner and the group by name, or by number where the system knows no name for it.
#[cfg(unix)]
fn owner_and_group(metadata: &Metadata) -> (Option<String>, Option<String>) {
    use std::os::unix::fs::MetadataExt;

    let owner = uzers::get_user_by_uid(metadata.uid()).map_or_else(
        || metadata.uid().to_string(),
        |user| user.name().to_string_lossy().into_owned(),
    );
    let group = uzers::get_group_by_gid(metadata.gid()).map_or_else(
        || metadata.gid().to_string(),
        |group| group.name().to_string_lossy().into_owned(),
    );

    (Some(owner), Some(group))
}

#[cfg(not(unix))]
fn owner_and_group(_metadata: &Metadata) -> (Option<String>, Option<String>) {
    (None, None)
}

#[cfg(windows)]
fn windows_attributes(metadata: &Metadata) -> Option<WindowsAttributes> {
    use std::os::windows::fs::MetadataExt;

    const READ_ONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const ARCHIVE: u32 = 0x20;

    let attributes = metadata.file_attributes();

    Some(WindowsAttributes {
        read_only: attributes & READ_ONLY != 0,
        hidden: attributes & HIDDEN != 0,
        system: attributes & SYSTEM != 0,
        archive: attributes & ARCHIVE != 0,
    })
}

#[cfg(not(windows))]
fn windows_attributes(_metadata: &Metadata) -> Option<WindowsAttributes> {
    None
}
