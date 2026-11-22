//! Traditional zip encryption ("ZipCrypto"): a stream cipher of three 32-bit keys that a 12-byte header, the first thing
//! of the entry's data, checks the password against. It is weak, but it is what most password zips use. The cipher is
//! written here (it needs the CRC-32 step by byte, which the checksum crate does not offer).

use std::io::{self, Read};

use zeroize::Zeroize;

use crate::archive::Password;

/// The bytes of the header that precede the data.
pub const HEADER: usize = 12;

/// The CRC-32 table, built at compile time.
const TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;

    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;

        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
            bit += 1;
        }

        table[index] = value;
        index += 1;
    }

    table
};

/// One step of CRC-32 (no initial or final inversion) over a byte.
fn crc_step(crc: u32, byte: u8) -> u32 {
    TABLE[usize::from((crc as u8) ^ byte)] ^ (crc >> 8)
}

/// The state of the cipher: three keys, overwritten when dropped.
pub struct Keys([u32; 3]);

impl Keys {
    /// The keys after `password` was fed in.
    pub fn new(password: &Password) -> Self {
        let mut keys = Self([0x1234_5678, 0x2345_6789, 0x3456_7890]);

        for byte in password.as_bytes() {
            keys.update(*byte);
        }

        keys
    }

    fn update(&mut self, plain: u8) {
        let [first, second, third] = &mut self.0;

        *first = crc_step(*first, plain);
        *second = second
            .wrapping_add(*first & 0xFF)
            .wrapping_mul(134_775_813)
            .wrapping_add(1);
        *third = crc_step(*third, (*second >> 24) as u8);
    }

    fn keystream(&self) -> u8 {
        let temp = (self.0[2] | 2) as u16;

        (temp.wrapping_mul(temp ^ 1) >> 8) as u8
    }

    /// Decrypts `bytes` in place.
    pub fn decrypt(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            *byte ^= self.keystream();
            self.update(*byte);
        }
    }

    /// Encrypts `bytes` in place (used by the tests, which build encrypted zips).
    #[cfg(test)]
    pub fn encrypt(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            let plain = *byte;
            *byte ^= self.keystream();
            self.update(plain);
        }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// The bytes the last byte of the decrypted header may be: the high byte of the CRC, or, when the sizes and CRC follow the
/// data (flag bit 3), also the high byte of the DOS time, which is what writers use then.
pub fn check_bytes(crc: u32, dos_time: u16, descriptor: bool) -> [u8; 2] {
    let by_crc = (crc >> 24) as u8;

    [by_crc, if descriptor { (dos_time >> 8) as u8 } else { by_crc }]
}

/// Decrypts the 12-byte header read from `source` and tells whether `password` is the right one: its last byte must be one
/// of `accept` (see [`check_bytes`]). The keys are ready for the data that follows when it is.
pub fn open_header<R: Read>(source: &mut R, password: &Password, accept: &[u8]) -> io::Result<Option<Keys>> {
    let mut header = [0; HEADER];
    source.read_exact(&mut header).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => crate::archive::error::unreadable(),
        _ => error,
    })?;

    let mut keys = Keys::new(password);
    keys.decrypt(&mut header);
    let right = accept.contains(&header[HEADER - 1]);
    header.zeroize();

    Ok(right.then_some(keys))
}

/// Decrypts what `inner` yields.
pub struct Decrypting<R> {
    pub inner: R,
    pub keys: Keys,
}

impl<R: Read> Read for Decrypting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.keys.decrypt(&mut buffer[..count]);

        Ok(count)
    }
}
