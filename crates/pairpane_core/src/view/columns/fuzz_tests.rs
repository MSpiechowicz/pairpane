//! A property-style check that no header, cut anywhere or changed anywhere, makes a parser panic or reports a side of
//! zero. The inputs come from a fixed xorshift, so a failure repeats.

use super::HEAD_BYTES;
use super::fixtures::all;
use super::header::from_head;

/// A xorshift64 generator: the same numbers on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;

        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).expect("below a usize")
    }

    /// A byte, often one of the values that headers treat specially.
    fn byte(&mut self) -> u8 {
        match self.below(6) {
            0 => 0,
            1 => 0xFF,
            2 => 0x7F,
            3 => 0x80,
            _ => self.next().to_le_bytes()[0],
        }
    }
}

/// Parses `bytes` and checks the one thing that holds for any input: a size that comes back has no side of zero.
fn parse(bytes: &[u8]) {
    if let Some(found) = from_head(bytes) {
        assert!(found.width != 0 && found.height != 0, "a zero side from {bytes:02x?}");
    }
}

#[test]
fn every_prefix_of_every_format_parses_without_a_panic() {
    for (_, bytes) in all() {
        for end in 0..=bytes.len() {
            parse(&bytes[..end]);
        }
    }
}

#[test]
fn headers_with_bytes_changed_parse_without_a_panic() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);

    for (_, bytes) in all() {
        for _ in 0..4000 {
            let mut changed = bytes.clone();
            for _ in 0..=random.below(4) {
                let at = random.below(changed.len());
                changed[at] = random.byte();
            }
            // Often cut as well, so that a changed length points past the end.
            if random.below(2) == 0 {
                changed.truncate(random.below(changed.len() + 1));
            }

            parse(&changed);
        }
    }
}

#[test]
fn a_header_followed_by_random_bytes_parses_without_a_panic() {
    let mut random = Random(0xD1B5_4A32_D192_ED03);

    for (_, bytes) in all() {
        // The magic number and a little more, then whatever comes: the walks must end wherever they are led.
        let keep = bytes.len().min(16);

        for _ in 0..200 {
            let length = random.below(HEAD_BYTES + 1000);
            let mut input = bytes[..keep].to_vec();
            input.extend((0..length).map(|_| random.byte()));

            parse(&input);
        }
    }
}

#[test]
fn a_whole_window_of_one_byte_value_parses_without_a_panic() {
    for (_, bytes) in all() {
        let keep = bytes.len().min(16);

        for fill in [0, 1, 0x7F, 0x80, 0xD0, 0xE1, 0xFF] {
            let mut input = bytes[..keep].to_vec();
            input.resize(HEAD_BYTES + 100, fill);

            parse(&input);
        }
    }
}
