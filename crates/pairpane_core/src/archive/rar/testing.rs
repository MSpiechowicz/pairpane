//! RAR 5 archives written by hand for the tests: stored (not compressed) entries, so `unrar` can read them. The tests that
//! run `unrar` skip themselves where it is not installed.

/// What an entry of a test archive is.
pub(super) enum Item<'a> {
    File(&'a str, &'a [u8]),
    /// A file with the Unix mode given.
    Moded(&'a str, &'a [u8], u32),
    Dir(&'a str),
    Symlink(&'a str, &'a str),
    Hardlink(&'a str, &'a str),
}

/// 2023-11-14 22:13:20 UTC.
pub(super) const MOMENT: u32 = 1_700_000_000;

/// A variable-length integer: seven bits per byte, low first.
fn vint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();

    loop {
        let low = (value & 0x7F) as u8;
        value >>= 7;

        if value == 0 {
            bytes.push(low);

            return bytes;
        }

        bytes.push(low | 0x80);
    }
}

/// A block of the archive: its checksum, its size, then `kind`, the flags, the `extra` area and the size of the data that
/// follows, and the `body`.
fn block(kind: u64, body: &[u8], extra: &[u8], data: Option<usize>) -> Vec<u8> {
    let flags = u64::from(!extra.is_empty()) | (u64::from(data.is_some()) << 1);
    let mut head = vint(kind);
    head.extend(vint(flags));

    if !extra.is_empty() {
        head.extend(vint(extra.len() as u64));
    }

    if let Some(size) = data {
        head.extend(vint(size as u64));
    }

    head.extend_from_slice(body);
    head.extend_from_slice(extra);

    let mut sized = vint(head.len() as u64);
    sized.extend(head);

    let mut bytes = crc32fast::hash(&sized).to_le_bytes().to_vec();
    bytes.extend(sized);

    bytes
}

/// The file header of `name` with the content `data`, and the redirection (`link` is its kind and target) that makes it a
/// link.
fn entry(name: &str, data: &[u8], directory: bool, mode: u32, link: Option<(u64, &str)>) -> Vec<u8> {
    let mut body = vint(u64::from(directory) | 0x2 | if directory || link.is_some() { 0 } else { 0x4 });
    body.extend(vint(data.len() as u64));
    body.extend(vint(u64::from(mode)));
    body.extend_from_slice(&MOMENT.to_le_bytes());

    if !directory && link.is_none() {
        body.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
    }

    // Stored (method 0), made on Unix.
    body.extend(vint(0));
    body.extend(vint(1));
    body.extend(vint(name.len() as u64));
    body.extend_from_slice(name.as_bytes());

    let extra = link.map_or_else(Vec::new, |(kind, target)| {
        let mut record = vint(5);
        record.extend(vint(kind));
        record.extend(vint(0));
        record.extend(vint(target.len() as u64));
        record.extend_from_slice(target.as_bytes());

        let mut sized = vint(record.len() as u64);
        sized.extend(record);

        sized
    });

    let mut bytes = block(2, &body, &extra, Some(data.len()));
    bytes.extend_from_slice(data);

    bytes
}

/// The archive of `items`.
pub(super) fn rar_of(items: &[Item<'_>]) -> Vec<u8> {
    rar_with(items, 0)
}

/// The archive of `items` with the flags of the main header given (1: one volume of a set).
pub(super) fn rar_with(items: &[Item<'_>], flags: u64) -> Vec<u8> {
    let mut bytes = super::SIGNATURE_5.to_vec();
    let mut main = vint(flags);

    if flags & 1 != 0 {
        // The number of the volume.
        main.extend(vint(1));
    }
    bytes.extend(block(1, &main, &[], None));

    for item in items {
        bytes.extend(match item {
            Item::File(name, data) => entry(name, data, false, 0o644, None),
            Item::Moded(name, data, mode) => entry(name, data, false, *mode, None),
            Item::Dir(name) => entry(name, &[], true, 0o40755, None),
            Item::Symlink(name, target) => entry(name, &[], false, 0o120_777, Some((1, target))),
            Item::Hardlink(name, target) => entry(name, &[], false, 0o120_777, Some((4, target))),
        });
    }

    bytes.extend(block(5, &vint(0), &[], None));

    bytes
}
