//! Output of a command as text to show: UTF-8 decoded across reads, with terminal escapes and control characters left out.

/// Where the cleaner is within an escape sequence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Text,
    /// After `ESC`.
    Escape,
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
    /// The last character kept was a carriage return turned into a line break.
    after_return: bool,
}

impl Cleaner {
    /// The text of `bytes`, what came before them included.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        let mut joined = std::mem::take(&mut self.partial);
        joined.extend_from_slice(bytes);

        let complete = match std::str::from_utf8(&joined) {
            Ok(_) => joined.len(),
            // Bytes that may still become a character are kept for the next read.
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => joined.len(),
        };
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
                (State::Escape, _) => State::Text,
                (State::Control, '\u{40}'..='\u{7e}') => State::Text,
                (State::Control, _) => State::Control,
                (State::Command, '\u{7}') => State::Text,
                (State::Command, '\u{1b}') => State::CommandEscape,
                (State::Command, _) => State::Command,
                (State::CommandEscape, '\\') => State::Text,
                (State::CommandEscape, _) => State::Command,
            };
            self.after_return = false;
        }

        out
    }
}
