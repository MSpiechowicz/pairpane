//! Searching a file for text or bytes: forward or backward from an offset, in chunks of [`SEARCH_CHUNK`] bytes that
//! overlap by the length of a match, so a match across two chunks is found and a huge file is never held.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use memchr::memchr3_iter;
use memchr::memmem::{Finder, FinderRev};

use crate::model::{SearchHit, SearchNeedle, SearchQuery};

use super::file::ViewFile;
use super::{MAX_NEEDLE, SEARCH_CHUNK, encoding, refusal};

/// A progress report goes out each time this many more bytes were scanned.
const PROGRESS_STEP: u64 = 64 * 1024 * 1024;

/// The bytes a needle is looked for as.
enum Pattern {
    /// One spelling: found with a substring search.
    Exact {
        forward: Box<Finder<'static>>,
        backward: Box<FinderRev<'static>>,
        len: usize,
    },
    /// Each character may be spelled several ways (its case variants): `choices[i]` are the byte spellings of
    /// character `i`. No spelling is the start of another one of the same character, so the first that fits is the one.
    Folded { choices: Vec<Vec<Vec<u8>>>, first: [u8; 3] },
}

impl Pattern {
    fn exact(bytes: &[u8]) -> Self {
        Self::Exact {
            forward: Box::new(Finder::new(bytes).into_owned()),
            backward: Box::new(FinderRev::new(bytes).into_owned()),
            len: bytes.len(),
        }
    }

    /// The most bytes a match is long.
    fn longest(&self) -> usize {
        match self {
            Self::Exact { len, .. } => *len,
            Self::Folded { choices, .. } => choices
                .iter()
                .map(|spellings| spellings.iter().map(Vec::len).max().unwrap_or(0))
                .sum(),
        }
    }

    /// The length of the match that starts at `at`, if there is one.
    fn match_at(&self, buf: &[u8], at: usize) -> Option<usize> {
        match self {
            Self::Exact { forward, len, .. } => buf[at..].starts_with(forward.needle()).then_some(*len),
            Self::Folded { choices, .. } => {
                let mut end = at;

                for spellings in choices {
                    let spelling = spellings.iter().find(|spelling| buf[end..].starts_with(spelling))?;
                    end += spelling.len();
                }

                Some(end - at)
            }
        }
    }

    /// The starts of the matches that start before `limit` and at a multiple of `align` (counted from `base`, the
    /// file offset of `buf[0]`), first to last, or last to first with `reverse`.
    fn first_or_last(&self, buf: &[u8], base: u64, limit: usize, align: u64, reverse: bool) -> Option<(usize, usize)> {
        let aligned = |at: usize| (base + at as u64).is_multiple_of(align);

        match self {
            Self::Exact { forward, backward, len } => {
                if reverse {
                    let mut end = buf.len().min(limit.saturating_sub(1) + len);
                    while let Some(at) = backward.rfind(&buf[..end]) {
                        if aligned(at) {
                            return Some((at, *len));
                        }

                        end = at + len - 1;
                    }

                    return None;
                }

                let mut from = 0;
                while let Some(found) = forward.find(&buf[from..]) {
                    let at = from + found;
                    if at >= limit {
                        return None;
                    }

                    if aligned(at) {
                        return Some((at, *len));
                    }

                    from = at + 1;
                }

                None
            }
            Self::Folded { first, .. } => {
                let mut starts = memchr3_iter(first[0], first[1], first[2], &buf[..limit.min(buf.len())])
                    .filter(|at| aligned(*at))
                    .filter_map(|at| self.match_at(buf, at).map(|len| (at, len)));

                if reverse { starts.next_back() } else { starts.next() }
            }
        }
    }
}

/// The first match at or after `query.from` (or the last one before it, going backward), as the byte range it
/// occupies. A text needle is looked for as the bytes of the query's encoding; a case-insensitive one matches the
/// characters whose lower or upper case is one character as well. `cancel` is looked at before each chunk, and a
/// cancelled search ends with an `Interrupted` error; `progress` gets the number of bytes scanned so far, each 64 MiB.
/// A needle longer than [`MAX_NEEDLE`] or empty is refused, one that its encoding cannot spell finds nothing.
pub fn search(
    file: &ViewFile,
    query: &SearchQuery,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> io::Result<Option<SearchHit>> {
    let Some((pattern, align)) = compile(query)? else {
        return Ok(None);
    };

    let size = file.size()?;
    let overlap = pattern.longest();
    let mut position = query.from.min(size);
    let mut scanned = 0;
    let mut next_report = PROGRESS_STEP;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(refusal(io::ErrorKind::Interrupted, "the search was cancelled"));
        }

        // The chunk owns the starts in `owned_from..owned_to`; it is read a match longer so a match that starts in it can end outside.
        let (owned_from, owned_to) = if query.backward {
            if position == 0 {
                return Ok(None);
            }

            (position.saturating_sub(SEARCH_CHUNK as u64), position)
        } else {
            if position >= size {
                return Ok(None);
            }

            (position, position.saturating_add(SEARCH_CHUNK as u64).min(size))
        };

        let buf = file.read_at(owned_from, (owned_to - owned_from) as usize + overlap)?;
        let limit = (owned_to - owned_from) as usize;

        if let Some((at, len)) = pattern.first_or_last(&buf, owned_from, limit, align, query.backward) {
            return Ok(Some(SearchHit {
                offset: owned_from + at as u64,
                len: len as u64,
            }));
        }

        scanned += owned_to - owned_from;
        if scanned >= next_report {
            progress(scanned);
            next_report += PROGRESS_STEP;
        }

        position = if query.backward { owned_from } else { owned_to };
    }
}

/// The pattern of a query and the alignment of its matches; `None` for a needle that cannot occur.
fn compile(query: &SearchQuery) -> io::Result<Option<(Pattern, u64)>> {
    match &query.needle {
        SearchNeedle::Hex { bytes } => {
            if bytes.is_empty() {
                return Err(refusal(io::ErrorKind::InvalidInput, "the search text is empty"));
            }

            if bytes.len() > MAX_NEEDLE {
                return Err(refusal(io::ErrorKind::InvalidInput, "the search text is too long"));
            }

            Ok(Some((Pattern::exact(bytes), 1)))
        }
        SearchNeedle::Text { text, case_sensitive } => {
            let encoding = encoding::by_name(&query.encoding)?;
            let align = if encoding::is_utf16(encoding) { 2 } else { 1 };

            Ok(text_pattern(text, *case_sensitive, encoding)?.map(|pattern| (pattern, align)))
        }
    }
}

fn text_pattern(text: &str, case_sensitive: bool, encoding: &'static Encoding) -> io::Result<Option<Pattern>> {
    if text.is_empty() {
        return Err(refusal(io::ErrorKind::InvalidInput, "the search text is empty"));
    }

    if text.chars().count() > MAX_NEEDLE {
        return Err(refusal(io::ErrorKind::InvalidInput, "the search text is too long"));
    }

    let mut choices = Vec::new();
    for character in text.chars() {
        let spellings: Vec<Vec<u8>> = variants(character, case_sensitive)
            .into_iter()
            .filter_map(|variant| spell(variant, encoding))
            .collect();

        if spellings.is_empty() {
            return Ok(None);
        }

        choices.push(spellings);
    }

    if choices.iter().all(|spellings| spellings.len() == 1) {
        return Ok(Some(Pattern::exact(&choices.concat().concat())));
    }

    // Only a character with another case has several spellings, so the first bytes are few: at most three.
    let mut first = [0; 3];
    let starts: Vec<u8> = choices[0].iter().map(|spelling| spelling[0]).collect();
    for (slot, start) in first.iter_mut().zip(starts.iter().cycle()) {
        *slot = *start;
    }

    Ok(Some(Pattern::Folded { choices, first }))
}

/// The character and, unless `case_sensitive`, its lower and upper case when each is one character (a case that
/// is several, like the `SS` of `ß`, is not a variant).
fn variants(character: char, case_sensitive: bool) -> Vec<char> {
    let mut all = vec![character];
    if case_sensitive {
        return all;
    }

    let mut lower = character.to_lowercase();
    if let (Some(one), None) = (lower.next(), lower.next()) {
        all.push(one);
    }

    let mut upper = character.to_uppercase();
    if let (Some(one), None) = (upper.next(), upper.next()) {
        all.push(one);
    }

    all.sort_unstable();
    all.dedup();

    all
}

/// The bytes of `character` in `encoding`; `None` where the encoding has no such character. UTF-16 is spelled
/// by hand (the encoder of the web encodes to UTF-8 for it).
fn spell(character: char, encoding: &'static Encoding) -> Option<Vec<u8>> {
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let mut units = [0; 2];
        let units = character.encode_utf16(&mut units);

        let bytes = units
            .iter()
            .flat_map(|unit| {
                if encoding == UTF_16LE {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            })
            .collect();

        return Some(bytes);
    }

    let mut utf8 = [0; 4];
    let (bytes, _, unmappable) = encoding.encode(character.encode_utf8(&mut utf8));

    (!unmappable).then(|| bytes.into_owned())
}
