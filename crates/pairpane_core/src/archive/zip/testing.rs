//! Encoders and encryptors for the tests: the application only decodes, so the zips the tests read are made here.

use std::io::{self, Write};
use std::sync::atomic::AtomicBool;

use flate2::Compression;
use flate2::write::DeflateEncoder;
use lzma_rust2::{LzmaOptions, LzmaWriter};

use crate::archive::testing::{DOS_TIME, Member, field};
use crate::archive::{ArchiveIndex, Flow, Input, Password, build_index, open_source};
use crate::model::ArchiveFormat;

use super::winzip_aes::{self, Keys, Sealing};
use super::zipcrypto;

pub(super) const STORED: u16 = 0;
pub(super) const DEFLATE: u16 = 8;
pub(super) const BZIP2: u16 = 12;
pub(super) const LZMA: u16 = 14;
pub(super) const ZSTD: u16 = 93;

/// `data` compressed with the method `code`, as a zip stores it.
pub(super) fn compress(code: u16, data: &[u8]) -> Vec<u8> {
    match code {
        STORED => data.to_vec(),
        DEFLATE => {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(data).expect("compress");

            encoder.finish().expect("finish")
        }
        BZIP2 => {
            let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
            encoder.write_all(data).expect("compress");

            encoder.finish().expect("finish")
        }
        ZSTD => ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest),
        LZMA => {
            let options = LzmaOptions::with_preset(1);
            // The zip header: a version, the length of the properties, then the properties; the stream ends with a marker.
            let mut out = vec![9, 20, 5, 0];
            let mut encoder = LzmaWriter::new_no_header(Vec::new(), &options, true).expect("an encoder");
            out.push(encoder.props());
            out.extend_from_slice(&options.dict_size.to_le_bytes());
            encoder.write_all(data).expect("compress");
            out.extend_from_slice(&encoder.finish().expect("finish"));

            out
        }
        _ => unreachable!("a method the tests encode"),
    }
}

/// An entry of `content` compressed with `method`.
pub(super) fn member<'a>(name: &'a str, content: &[u8], method: u16) -> Member<'a> {
    Member {
        method,
        data: compress(method, content),
        ..Member::stored(name, content)
    }
}

/// An entry of `content` (compressed with `method`) encrypted with ZipCrypto. With `descriptor` the entry says its sizes
/// follow the data and the check byte is the DOS time's, as writers do then.
pub(super) fn traditional<'a>(
    name: &'a str,
    content: &[u8],
    method: u16,
    password: &str,
    descriptor: bool,
) -> Member<'a> {
    let plain = member(name, content, method);
    let check = if descriptor {
        (DOS_TIME >> 8) as u8
    } else {
        (plain.crc >> 24) as u8
    };

    // Eleven bytes that are not a secret, then the check byte, then the data.
    let mut data: Vec<u8> = (1..12).collect();
    data.push(check);
    data.extend_from_slice(&plain.data);
    zipcrypto::Keys::new(&Password::new(password.to_owned())).encrypt(&mut data);

    Member {
        flags: 1 | if descriptor { 8 } else { 0 },
        data,
        ..plain
    }
}

/// An entry of `content` (compressed with `method`) encrypted with WinZip AES of `strength` (1 to 3), as AE-`version`.
pub(super) fn aes<'a>(
    name: &'a str,
    content: &[u8],
    method: u16,
    password: &str,
    strength: u8,
    version: u16,
) -> Member<'a> {
    let plain = member(name, content, method);
    let key = winzip_aes::key_length(strength).expect("a strength");
    let salt: Vec<u8> = (0..winzip_aes::salt_length(key) as u8)
        .map(|byte| byte * 7 + 1)
        .collect();
    let keys = Keys::derive(&Password::new(password.to_owned()), &salt, key);

    let mut sealing = Sealing::begin(Vec::new(), &salt, keys).expect("a sealing writer");
    sealing.write_all(&plain.data).expect("encrypt");
    let data = sealing.finish().expect("the tag");

    let mut info = version.to_le_bytes().to_vec();
    info.extend_from_slice(b"AE");
    info.push(strength);
    info.extend_from_slice(&method.to_le_bytes());

    Member {
        method: 99,
        flags: 1,
        crc: if version == 2 { 0 } else { plain.crc },
        extra: field(0x9901, &info),
        data,
        ..plain
    }
}

/// What reading each entry of a zip gave: its path and the content, or why it could not be read.
pub(super) type Outcome = Vec<(String, io::Result<Vec<u8>>)>;

pub(super) fn index_of(bytes: &Vec<u8>) -> io::Result<ArchiveIndex> {
    build_index(
        ArchiveFormat::Zip,
        &Input {
            file: bytes,
            path: None,
        },
        &AtomicBool::new(false),
        None,
    )
}

/// Extracts the entries `wanted` (all when empty) of the zip `bytes` through [`open_source`], with `password`.
pub(super) fn read_entries(bytes: &Vec<u8>, password: Option<&str>, wanted: &[u32]) -> io::Result<Outcome> {
    let index = index_of(bytes)?;
    let cancel = AtomicBool::new(false);
    let password = password.map(|text| Password::new(text.to_owned()));
    let all: Vec<u32> = (0..index.entries().len() as u32).collect();
    let mut got = Vec::new();

    open_source(
        &index,
        &Input {
            file: bytes,
            path: None,
        },
        &cancel,
        password.as_ref(),
    )?
    .extract(if wanted.is_empty() { &all } else { wanted }, &mut |entry, data| {
        let mut content = Vec::new();
        got.push((entry.path.clone(), data.read_to_end(&mut content).map(|_| content)));

        Ok(Flow::Continue)
    })?;

    Ok(got)
}

/// The content of the only entry of `bytes`, or why it cannot be read.
pub(super) fn read_only(bytes: &Vec<u8>, password: Option<&str>) -> io::Result<Vec<u8>> {
    let mut got = read_entries(bytes, password, &[])?;
    assert_eq!(got.len(), 1);

    got.remove(0).1
}

/// The error of `read_only`, which must fail.
pub(super) fn failure(bytes: &Vec<u8>, password: Option<&str>) -> io::Error {
    read_only(bytes, password).expect_err("the entry cannot be read")
}
