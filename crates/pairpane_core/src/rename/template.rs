//! The templates of a rename: placeholders for the name and the extension (whole, or a range of their characters), the
//! counter and the date, and the brackets that stand for themselves.
//!
//! `[N]` is the name of the item without its extension and `[E]` the extension; `[N3]` is its third character,
//! `[N2-5]` the second to the fifth, `[N2-]` the second to the last, `[N-3]` the third from the end and `[N-3-]` the
//! last three. Positions count Unicode scalar values from 1, and one outside the text gives nothing (not an error).
//! `[C]` is the counter, `[Y]` the year, `[M]` the month, `[D]` the day, `[h]` the hour, `[m]` the minute and `[s]` the
//! second of the modification time, `[YMD]` is `2024-05-17` and `[hms]` is `13-05-09`. `[[` and `]]` are the brackets
//! themselves. Letters are case-sensitive; anything else in brackets is refused.

use jiff::Timestamp;
use jiff::tz::TimeZone;

/// The longest part of a placeholder quoted in a message.
const QUOTED_CHARS: usize = 16;

/// Which characters of a name or an extension a placeholder takes; positions count from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Span {
    /// `[N]`
    Whole,
    /// `[N3]`
    At(usize),
    /// `[N2-5]`; nothing when the end is before the start.
    Between(usize, usize),
    /// `[N2-]`
    From(usize),
    /// `[N-3]`: the third character from the end.
    FromEnd(usize),
    /// `[N-3-]`: the last three characters.
    Last(usize),
}

/// The part of the local date or time that a placeholder writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Date,
    Time,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Name(Span),
    Ext(Span),
    Counter,
    When(Part),
}

/// A template, parsed once and rendered for every item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Template {
    pieces: Vec<Piece>,
}

/// The local date and time of a modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Local {
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
    second: i8,
}

impl Local {
    /// The date and time of `millis` since the Unix epoch in `zone`; `None` for a time that cannot be shown.
    pub(super) fn of(millis: i64, zone: &TimeZone) -> Option<Self> {
        let time = Timestamp::from_millisecond(millis)
            .ok()?
            .to_zoned_datetime(zone.clone());

        Some(Self {
            year: time.year(),
            month: time.month(),
            day: time.day(),
            hour: time.hour(),
            minute: time.minute(),
            second: time.second(),
        })
    }

    fn write(&self, part: Part, out: &mut String) {
        let Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } = *self;

        out.push_str(&match part {
            Part::Year => format!("{year:04}"),
            Part::Month => format!("{month:02}"),
            Part::Day => format!("{day:02}"),
            Part::Hour => format!("{hour:02}"),
            Part::Minute => format!("{minute:02}"),
            Part::Second => format!("{second:02}"),
            Part::Date => format!("{year:04}-{month:02}-{day:02}"),
            Part::Time => format!("{hour:02}-{minute:02}-{second:02}"),
        });
    }
}

/// What a template is rendered for: the name and the extension of the item, the counter and the date.
pub(super) struct Context<'a> {
    pub(super) name: &'a str,
    pub(super) ext: &'a str,
    pub(super) counter: &'a str,
    pub(super) local: Option<Local>,
}

impl Template {
    /// Parses `text`; the error is a message about it for the user.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();

        while let Some(char) = chars.next() {
            match char {
                '[' if chars.next_if_eq(&'[').is_some() => literal.push('['),
                ']' if chars.next_if_eq(&']').is_some() => literal.push(']'),
                ']' => return Err("a ] has no [ before it (write ]] for the bracket itself)".to_owned()),
                '[' => {
                    let mut inside = String::new();
                    loop {
                        match chars.next() {
                            Some(']') => break,
                            Some(next) => inside.push(next),
                            None => return Err("a [ is not closed (write [[ for the bracket itself)".to_owned()),
                        }
                    }

                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(placeholder(&inside)?);
                }
                other => literal.push(other),
            }
        }

        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }

        Ok(Self { pieces })
    }

    /// Whether the template writes a date or a time.
    pub(super) fn uses_date(&self) -> bool {
        self.pieces.iter().any(|piece| matches!(piece, Piece::When(_)))
    }

    pub(super) fn render(&self, context: &Context<'_>) -> String {
        let mut out = String::new();

        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Name(span) => span.take(context.name, &mut out),
                Piece::Ext(span) => span.take(context.ext, &mut out),
                Piece::Counter => out.push_str(context.counter),
                Piece::When(part) => {
                    if let Some(local) = &context.local {
                        local.write(*part, &mut out);
                    }
                }
            }
        }

        out
    }
}

fn placeholder(inside: &str) -> Result<Piece, String> {
    let piece = match inside {
        "C" => Some(Piece::Counter),
        "Y" => Some(Piece::When(Part::Year)),
        "M" => Some(Piece::When(Part::Month)),
        "D" => Some(Piece::When(Part::Day)),
        "h" => Some(Piece::When(Part::Hour)),
        "m" => Some(Piece::When(Part::Minute)),
        "s" => Some(Piece::When(Part::Second)),
        "YMD" => Some(Piece::When(Part::Date)),
        "hms" => Some(Piece::When(Part::Time)),
        _ => ranged(inside),
    };

    piece.ok_or_else(|| {
        let shown: String = inside.chars().take(QUOTED_CHARS).collect();
        let more = if inside.chars().count() > QUOTED_CHARS {
            "…"
        } else {
            ""
        };

        format!("[{shown}{more}] is not a placeholder")
    })
}

/// `N` or `E` with its optional range.
fn ranged(inside: &str) -> Option<Piece> {
    let mut chars = inside.chars();
    let letter = chars.next()?;
    let span = Span::parse(chars.as_str())?;

    match letter {
        'N' => Some(Piece::Name(span)),
        'E' => Some(Piece::Ext(span)),
        _ => None,
    }
}

/// A position: digits only, from 1. A number too big for a name is as good as any other.
fn position(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    match text.parse::<usize>().unwrap_or(usize::MAX) {
        0 => None,
        number => Some(number),
    }
}

impl Span {
    /// The range after the letter of a placeholder (`""`, `3`, `2-5`, `2-`, `-3`, `-3-`).
    fn parse(text: &str) -> Option<Self> {
        if text.is_empty() {
            return Some(Self::Whole);
        }

        if let Some(tail) = text.strip_prefix('-') {
            return match tail.strip_suffix('-') {
                Some(count) => position(count).map(Self::Last),
                None => position(tail).map(Self::FromEnd),
            };
        }

        match text.split_once('-') {
            None => position(text).map(Self::At),
            Some((first, "")) => position(first).map(Self::From),
            Some((first, last)) => Some(Self::Between(position(first)?, position(last)?)),
        }
    }

    /// Writes the characters of `text` that this range takes.
    fn take(self, text: &str, out: &mut String) {
        match self {
            Self::Whole => out.push_str(text),
            Self::At(first) => out.extend(text.chars().nth(first - 1)),
            Self::Between(first, last) => out.extend(text.chars().skip(first - 1).take(last.saturating_sub(first - 1))),
            Self::From(first) => out.extend(text.chars().skip(first - 1)),
            Self::FromEnd(count) => {
                let total = text.chars().count();

                if count <= total {
                    out.extend(text.chars().nth(total - count));
                }
            }
            Self::Last(count) => {
                let total = text.chars().count();

                out.extend(text.chars().skip(total.saturating_sub(count)));
            }
        }
    }
}

/// The name and the extension of an item, as the columns of a pane show them: the extension is what follows the last
/// dot; a folder, a name that starts with a dot and one that ends with a dot have none.
pub(super) fn split_name(name: &str, folder: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if !folder && dot > 0 && dot + 1 < name.len() => (&name[..dot], &name[dot + 1..]),
        _ => (name, ""),
    }
}
