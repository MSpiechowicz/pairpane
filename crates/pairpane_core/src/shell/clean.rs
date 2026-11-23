//! Output of a command as text to show: UTF-8 decoded across reads, with terminal escapes and control characters left out.

/// Where the cleaner is within an escape sequence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Text,
    /// After `ESC`.
    Escape,
    /// After `ESC` and an intermediate character (`ESC ( B` chooses a character set): one more character ends it.
    EscapeOne,
    /// In a control sequence (`ESC [`), up to its final byte.
    Control,
    /// In an operating system command (`ESC ]`), up to `BEL` or `ESC \`.
    Command,
    /// After an `ESC` within an operating system command.
    CommandEscape,
}

/// Turns the bytes a command writes into text: invalid UTF-8 is replaced, a character cut between two reads is joined,
/// escape sequences (colours, cursor moves, titles) and control characters but the tab and the line break are left
/// out, and a carriage return before a line break is dropped (a lone one is a line break).
#[derive(Debug, Default)]
pub struct Cleaner {
    /// The start of a character the last read cut.
    partial: Vec<u8>,
    state: State,
    /// Characters within the escape sequence so far: one that does not end in time is given up, and what follows
    /// shows again (an unfinished sequence never hides the rest of the output).
    within: usize,
    /// The last character kept was a carriage return turned into a line break.
    after_return: bool,
}

impl Cleaner {
    /// The text of `bytes`, what came before them included.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        let mut joined = std::mem::take(&mut self.partial);
        joined.extend_from_slice(bytes);

        // The start of a character cut at the end is kept for the next read, whatever comes before it.
        let complete = joined.len() - cut_tail(&joined);
        self.partial = joined.split_off(complete);

        let text = String::from_utf8_lossy(&joined).into_owned();
        self.filter(&text)
    }

    /// The text of what is left from the last read.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.partial);
        let text = String::from_utf8_lossy(&rest).into_owned();

        self.filter(&text)
    }

    fn filter(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for character in text.chars() {
            self.state = match (self.state, character) {
                (State::Text, '\u{1b}') => State::Escape,
                (State::Text, '\r') => {
                    out.push('\n');
                    self.after_return = true;
                    continue;
                }
                (State::Text, '\n') if self.after_return => {
                    self.after_return = false;
                    continue;
                }
                (State::Text, character) => {
                    if character == '\n' || character == '\t' || !character.is_control() {
                        out.push(character);
                    }
                    State::Text
                }
                (State::Escape, '[') => State::Control,
                (State::Escape, ']') => State::Command,
                (State::Escape, '\u{20}'..='\u{2f}') => State::EscapeOne,
                (State::Escape, _) | (State::EscapeOne, _) => State::Text,
                (State::Control, '\u{40}'..='\u{7e}') => State::Text,
                (State::Control, _) => State::Control,
                (State::Command, '\u{7}') => State::Text,
                (State::Command, '\u{1b}') => State::CommandEscape,
                (State::Command, _) => State::Command,
                (State::CommandEscape, '\\') => State::Text,
                (State::CommandEscape, _) => State::Command,
            };
            self.after_return = false;

            self.within = if self.state == State::Text { 0 } else { self.within + 1 };
            let limit = if matches!(self.state, State::Command | State::CommandEscape) {
                MAX_COMMAND
            } else {
                MAX_CONTROL
            };
            if self.within > limit {
                self.state = State::Text;
                self.within = 0;
            }
        }

        out
    }
}

/// The longest control sequence (`ESC [`…) waited for to end, in characters.
const MAX_CONTROL: usize = 64;

/// The longest operating system command (`ESC ]`…, a title or a link) waited for to end, in characters.
const MAX_COMMAND: usize = 4096;

/// How many bytes at the end of `bytes` are the start of a character that the next read may finish: a lead byte and
/// fewer continuation bytes than it needs. 0 when the end is whole (or not UTF-8 at all, which is replaced as it is).
fn cut_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        let needs = match byte {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            // A continuation byte: the lead may be further back.
            0x80..=0xBF => continue,
            _ => return 0,
        };

        return if back < needs { back } else { 0 };
    }

    0
}
