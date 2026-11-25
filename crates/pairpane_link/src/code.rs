//! The one-time pairing code: 8 characters of Crockford's base32 (the digits and the letters without I, L, O and U, so
//! two characters are not mistaken for each other), shown as `XXXX-XXXX`. That is 40 bits from the system's random
//! generator; it is short-lived (120 seconds), single use and cancelled after three wrong tries, and the key exchange
//! it feeds never lets a guess be checked offline, so 40 bits are enough.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};

/// The number of characters of a code, without the dash.
pub const CODE_LEN: usize = 8;

/// The 32 characters a code is made of.
pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A pairing code, kept as its 8 normalised characters. It is the password of the key exchange; it is never printed by
/// `Debug`, and it is wiped from memory when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode([u8; CODE_LEN]);

impl PairingCode {
    /// A new random code.
    pub fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; CODE_LEN]);
        getrandom::fill(&mut *bytes).map_err(|_| Error::Random)?;

        // 256 is a multiple of 32, so the low five bits of a random byte are uniform over the alphabet.
        Ok(Self(bytes.map(|byte| ALPHABET[usize::from(byte & 31)])))
    }

    /// Reads a code a user typed: case does not matter, dashes and spaces are ignored, and as in Crockford's
    /// encoding `O` reads as `0` and `I` and `L` as `1`. Any other character, or a length other than 8, is refused.
    pub fn parse(input: &str) -> Result<Self> {
        let mut code = Zeroizing::new([0u8; CODE_LEN]);
        let mut length = 0;

        for character in input.chars().filter(|c| *c != '-' && !c.is_whitespace()) {
            let normal = match character.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };

            let slot = code.get_mut(length).ok_or(Error::InvalidCode)?;
            *slot = u8::try_from(normal)
                .ok()
                .filter(|byte| ALPHABET.contains(byte))
                .ok_or(Error::InvalidCode)?;
            length += 1;
        }

        if length != CODE_LEN {
            return Err(Error::InvalidCode);
        }

        Ok(Self(*code))
    }

    /// The normalised characters, which are the password of the key exchange.
    pub fn as_bytes(&self) -> &[u8; CODE_LEN] {
        &self.0
    }
}

/// The code as it is shown: `XXXX-XXXX`.
impl fmt::Display for PairingCode {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, second) = self.0.split_at(CODE_LEN / 2);

        // Every byte is an ASCII character of the alphabet.
        write!(
            out,
            "{}-{}",
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(second)
        )
    }
}

impl Drop for PairingCode {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("PairingCode(..)")
    }
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod tests;
