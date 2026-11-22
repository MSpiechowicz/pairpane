//! WinZip AES encryption (AE-1 and AE-2): the data of an entry is `salt | verifier (2 bytes) | ciphertext | tag (10
//! bytes)`. PBKDF2-HMAC-SHA1 (1000 rounds) over the password and the salt gives the AES key, the HMAC key and the
//! verifier; the cipher is AES in CTR mode with a little-endian counter that starts at 1; the tag is HMAC-SHA1 of the
//! ciphertext, cut to 10 bytes.

use std::io::{self, Read};

use aes::{Aes128, Aes192, Aes256};
use ctr::Ctr128LE;
use ctr::cipher::{KeyIvInit, StreamCipher};
use hmac::digest::KeyInit as _;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use zeroize::Zeroizing;

use crate::archive::Password;
use crate::archive::error::unreadable;
use crate::view::refusal;

/// The bytes of the verifier that follows the salt.
pub const VERIFIER: usize = 2;

/// The bytes of the authentication tag at the end.
pub const TAG: usize = 10;

const ROUNDS: u32 = 1000;

/// The key length in bytes of an AES strength of the extra field (1, 2 or 3), `None` for any other.
pub fn key_length(strength: u8) -> Option<usize> {
    match strength {
        1 => Some(16),
        2 => Some(24),
        3 => Some(32),
        _ => None,
    }
}

/// The salt length of a key of `key` bytes.
pub fn salt_length(key: usize) -> usize {
    key / 2
}

enum Cipher {
    Aes128(Ctr128LE<Aes128>),
    Aes192(Ctr128LE<Aes192>),
    Aes256(Ctr128LE<Aes256>),
}

impl Cipher {
    fn new(key: &[u8]) -> Self {
        // The counter block is little-endian and starts at 1.
        let mut counter = [0; 16];
        counter[0] = 1;

        match key.len() {
            16 => Self::Aes128(Ctr128LE::new_from_slices(key, &counter).expect("a 128-bit key and counter")),
            24 => Self::Aes192(Ctr128LE::new_from_slices(key, &counter).expect("a 192-bit key and counter")),
            _ => Self::Aes256(Ctr128LE::new_from_slices(key, &counter).expect("a 256-bit key and counter")),
        }
    }

    fn apply(&mut self, bytes: &mut [u8]) {
        match self {
            Self::Aes128(cipher) => cipher.apply_keystream(bytes),
            Self::Aes192(cipher) => cipher.apply_keystream(bytes),
            Self::Aes256(cipher) => cipher.apply_keystream(bytes),
        }
    }
}

/// What a password and a salt make: the cipher, the HMAC and the verifier to compare with the one in the file.
pub struct Keys {
    cipher: Cipher,
    mac: Hmac<Sha1>,
    verifier: [u8; VERIFIER],
}

impl Keys {
    /// Derives the keys for an AES key of `key` bytes (16, 24 or 32).
    pub fn derive(password: &Password, salt: &[u8], key: usize) -> Self {
        let mut derived = Zeroizing::new(vec![0; 2 * key + VERIFIER]);
        pbkdf2::pbkdf2_hmac::<Sha1>(password.as_bytes(), salt, ROUNDS, &mut derived);

        Self {
            cipher: Cipher::new(&derived[..key]),
            mac: Hmac::<Sha1>::new_from_slice(&derived[key..2 * key]).expect("HMAC takes a key of any length"),
            verifier: [derived[2 * key], derived[2 * key + 1]],
        }
    }

    /// Whether `stored` is the verifier of this password.
    pub fn accepts(&self, stored: [u8; VERIFIER]) -> bool {
        self.verifier == stored
    }

    /// The verifier these keys write (used by the tests).
    #[cfg(test)]
    pub fn verifier(&self) -> [u8; VERIFIER] {
        self.verifier
    }

    /// Encrypts `bytes` in place and returns the tag to put after the last of them (used by the tests, which build
    /// encrypted zips); `bytes` is the whole ciphertext.
    #[cfg(test)]
    pub fn seal(mut self, bytes: &mut [u8]) -> [u8; TAG] {
        self.cipher.apply(bytes);
        self.mac.update(bytes);
        let full = self.mac.finalize().into_bytes();

        full[..TAG].try_into().expect("a tag")
    }
}

/// Decrypts `length` bytes of ciphertext from `inner`, and checks the tag that follows them once they are all read.
pub struct Opening<R> {
    inner: R,
    keys: Keys,
    left: u64,
    checked: bool,
}

impl<R: Read> Opening<R> {
    /// `inner` is positioned at the ciphertext, which is `length` bytes long and is followed by the tag.
    pub fn new(inner: R, keys: Keys, length: u64) -> Self {
        Self {
            inner,
            keys,
            left: length,
            checked: false,
        }
    }

    /// Reads what is left of the ciphertext (a decoder stops at the end of its own data and may not have read it all) and
    /// checks the tag. The tag failing after the verifier was right means the data is damaged, not that the password is
    /// wrong (the verifier is wrong for 65,535 of 65,536 wrong passwords).
    pub fn finish(&mut self) -> io::Result<()> {
        let mut sink = [0; 4096];

        while self.left > 0 {
            if self.read(&mut sink)? == 0 {
                return Err(unreadable());
            }
        }

        Ok(())
    }
}

impl<R: Read> Read for Opening<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        if self.left == 0 {
            return self.check_tag().map(|()| 0);
        }

        let room = usize::try_from(self.left).map_or(buffer.len(), |left| left.min(buffer.len()));
        let count = self.inner.read(&mut buffer[..room])?;

        if count == 0 {
            return Err(unreadable());
        }

        self.left -= count as u64;
        self.keys.mac.update(&buffer[..count]);
        self.keys.cipher.apply(&mut buffer[..count]);

        Ok(count)
    }
}

impl<R: Read> Opening<R> {
    fn check_tag(&mut self) -> io::Result<()> {
        if self.checked {
            return Ok(());
        }

        let mut tag = [0; TAG];
        self.inner.read_exact(&mut tag).map_err(|_| unreadable())?;
        self.keys
            .mac
            .clone()
            .verify_truncated_left(&tag)
            .map_err(|_| refusal(io::ErrorKind::InvalidData, "an entry's data is damaged"))?;
        self.checked = true;

        Ok(())
    }
}
