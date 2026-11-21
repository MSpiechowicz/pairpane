//! The `Range` header of a media request, read as strictly as it is useful: one `bytes=` range, nothing else.

/// The longest `Range` header that is read; a longer one is ignored.
pub const MAX_HEADER: usize = 200;

/// What a `Range` header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// No usable range: serve the whole file (or the first part of it). A missing header, a header over
    /// [`MAX_HEADER`] bytes, another unit, several ranges and a malformed range all come to this.
    Whole,
    /// These bytes, both ends included, the end already clamped to the end of the file and to the cap.
    Bytes { start: u64, end: u64 },
    /// The range starts at or after the end of the file (or the file is empty): answer 416.
    Unsatisfiable,
}

/// Parses `header` (the value of a `Range` header, if any) for a file of `size` bytes, handing out at most `cap`
/// bytes (`cap` is at least 1). The forms are `bytes=a-b`, `bytes=a-` and `bytes=-n` (the last `n` bytes).
#[must_use]
pub fn parse(header: Option<&str>, size: u64, cap: u64) -> Requested {
    let Some(header) = header else {
        return Requested::Whole;
    };

    if header.len() > MAX_HEADER {
        return Requested::Whole;
    }

    let Some((unit, spec)) = header.split_once('=') else {
        return Requested::Whole;
    };

    if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return Requested::Whole;
    }

    let Some((first, last)) = spec.trim().split_once('-') else {
        return Requested::Whole;
    };

    let (start, end) = match (number(first), number(last)) {
        (Some(Some(start)), Some(Some(end))) if start <= end => (start, Some(end)),
        (Some(Some(start)), Some(None)) => (start, None),
        (Some(None), Some(Some(0))) => return Requested::Unsatisfiable,
        (Some(None), Some(Some(count))) => (size.saturating_sub(count), None),
        _ => return Requested::Whole,
    };

    if size == 0 || start >= size {
        return Requested::Unsatisfiable;
    }

    let last_byte = size - 1;
    let end = end.map_or(last_byte, |end| end.min(last_byte));
    let capped = start.saturating_add(cap.max(1) - 1);

    Requested::Bytes {
        start,
        end: end.min(capped),
    }
}

/// `Some(Some(n))` for digits, `Some(None)` for nothing, `None` for anything else (or a number that does not fit).
fn number(text: &str) -> Option<Option<u64>> {
    let text = text.trim();

    if text.is_empty() {
        return Some(None);
    }

    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    text.parse().ok().map(Some)
}
