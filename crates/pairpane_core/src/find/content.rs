//! Searching the text inside files with ripgrep's engine (`grep-searcher`, `grep-regex`).
//!
//! The file is read as a stream through a reader that stops once the search is cancelled, never memory-mapped and
//! never whole: the searcher holds at most one line (and [`HEAP_LIMIT`]) at a time. A binary file (a NUL byte in what
//! it reads) is left out, so are the matches before the NUL.

use std::fs::File;
use std::io::{self, Read};

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Encoding, Searcher, SearcherBuilder, Sink, SinkFinish, SinkMatch};

use crate::archive::names::is_hostile;
use crate::model::FindText;
use crate::view::refusal;

use super::{HEAP_LIMIT, NEST_LIMIT, REGEX_LIMIT, Run, SNIPPET_CHARS};

/// The first matching line of a file and how many lines match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) matches: u32,
    pub(crate) line: String,
    pub(crate) number: u64,
}

/// What searching one file came to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scan {
    Found(Found),
    /// No match, a binary file, or the search was stopped.
    Nothing,
    /// The file could not be read to its end (it vanished, it cannot be read, a line is longer than the heap limit).
    Unread,
}

/// The text to look for, compiled once and shared by the threads of a search.
pub(crate) struct Content {
    matcher: RegexMatcher,
    encoding: Option<Encoding>,
    any_size: bool,
}

impl Content {
    /// Compiles `text`: a literal or a regular expression, with the case and word rules it asks for, within the limits of
    /// a search. A refusal has a fixed text.
    pub(crate) fn new(text: &FindText) -> io::Result<Self> {
        let matcher = RegexMatcherBuilder::new()
            .fixed_strings(!text.regex)
            .case_insensitive(!text.case_sensitive)
            .word(text.whole_word)
            .line_terminator(Some(b'\n'))
            .size_limit(REGEX_LIMIT)
            .dfa_size_limit(REGEX_LIMIT)
            .nest_limit(NEST_LIMIT)
            .build(&text.pattern)
            .map_err(|_| refusal(io::ErrorKind::InvalidInput, "the text is not valid or is too complex"))?;

        let encoding = text
            .encoding
            .as_deref()
            .map(|label| Encoding::new(label.trim()))
            .transpose()
            .map_err(|_| refusal(io::ErrorKind::InvalidInput, "the encoding is not known"))?;

        Ok(Self {
            matcher,
            encoding,
            any_size: text.any_size,
        })
    }

    /// Whether a file of `size` bytes is too big to search.
    pub(crate) fn too_big(&self, size: u64) -> bool {
        !self.any_size && size > super::MAX_TEXT_FILE
    }

    /// A searcher for one thread: lines, line numbers, binary files quit on their first NUL, a bounded buffer, the UTF-8
    /// and UTF-16 byte order marks understood and the encoding the search names (when it names one) read.
    pub(crate) fn searcher(&self) -> Searcher {
        SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(0))
            .heap_limit(Some(HEAP_LIMIT))
            .bom_sniffing(true)
            .encoding(self.encoding.clone())
            .build()
    }

    /// Searches `file` (opened by the caller without following a link) to its end, a match or the stop of `gate`.
    pub(crate) fn search(&self, searcher: &mut Searcher, file: File, gate: &Run<'_>) -> Scan {
        let mut tally = Tally {
            gate,
            matches: 0,
            first: None,
            binary: false,
        };
        let outcome = searcher.search_reader(&self.matcher, Gated { inner: file, gate }, &mut tally);

        if outcome.is_err() {
            return if gate.raised() { Scan::Nothing } else { Scan::Unread };
        }

        match tally.first {
            Some((number, line)) if !tally.binary => Scan::Found(Found {
                matches: tally.matches,
                line,
                number,
            }),
            _ => Scan::Nothing,
        }
    }
}

/// A reader that fails once the search stops. The error is not `Interrupted`, which readers retry for good.
struct Gated<'g, R> {
    inner: R,
    gate: &'g Run<'g>,
}

impl<R: Read> Read for Gated<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.gate.raised() {
            return Err(io::Error::other("the search was stopped"));
        }

        self.inner.read(buffer)
    }
}

/// Counts the matching lines and keeps the first.
struct Tally<'g> {
    gate: &'g Run<'g>,
    matches: u32,
    first: Option<(u64, String)>,
    binary: bool,
}

impl Sink for Tally<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        self.matches = self.matches.saturating_add(1);

        if self.first.is_none() {
            self.first = Some((found.line_number().unwrap_or(0), snippet(found.bytes())));
        }

        Ok(!self.gate.raised())
    }

    fn finish(&mut self, _searcher: &Searcher, finish: &SinkFinish) -> Result<(), io::Error> {
        self.binary = finish.binary_byte_offset().is_some();

        Ok(())
    }
}

/// The text of a matching line as a result shows it: trimmed, cut to [`SNIPPET_CHARS`] characters, with every control
/// character (and every bidirectional control, which could reorder what is shown) replaced by a space or `�`.
pub(crate) fn snippet(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim()
        .chars()
        .take(SNIPPET_CHARS)
        .map(|character| match character {
            '\t' => ' ',
            other if is_hostile(other) => '\u{FFFD}',
            other => other,
        })
        .collect()
}
