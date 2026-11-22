//! Zip archives. [`directory`] reads the central directory (and nothing else of the file) into [`Record`]s; the data of
//! an entry is read through the [`ZipPlace`] each record carries.
//!
//! [`Record`]: super::entries::Record

pub mod directory;

/// Everything the central directory says about where and how an entry's data is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipPlace {
    /// The compression method as the directory writes it (0 stored, 8 deflate, 12 bzip2, 14 LZMA, 93 zstd, 99 a WinZip AES
    /// entry, whose real method is in [`AesExtra::method`]).
    pub method: u16,
    /// The general purpose bit flags: bit 0 encrypted, bit 3 sizes and CRC follow the data, bit 11 UTF-8 names.
    pub flags: u16,
    pub crc: u32,
    /// The size of the data in the archive: after compression, and with the encryption header or the AES salt and
    /// tag included.
    pub compressed: u64,
    /// Where the local header of the entry is, counted from the start of the file (data that precedes the zip, as in a
    /// self-extracting program, is already added). The local header is followed by the data after its name and extra
    /// field, whose lengths are in the local header itself and not always the directory's.
    pub offset: u64,
    /// The DOS time of the entry: with flag bit 3 the check byte of ZipCrypto is its high byte.
    pub dos_time: u16,
    /// The version made by: the host system in the high byte (3 is Unix), the zip version in the low one.
    pub made_by: u16,
    pub external: u32,
    /// The WinZip AES field of the entry, when it has one.
    pub aes: Option<AesExtra>,
}

/// The WinZip AES extra field (id `0x9901`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AesExtra {
    /// 1 is AE-1 (the CRC is kept), 2 is AE-2 (the CRC is zero and not checked).
    pub version: u16,
    /// 1 is AES-128, 2 is AES-192, 3 is AES-256.
    pub strength: u8,
    /// The method of the data before it was encrypted.
    pub method: u16,
}
