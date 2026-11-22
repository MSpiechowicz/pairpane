//! Zips written by a tool that is not ours (libarchive's `bsdtar`, 3.8, `--passphrase secret --options
//! zip:encryption=<scheme>,zip:compression=deflate`), each holding `a.txt` with [`CONTENT`], so the readers are checked
//! against another writer and not only against the encryptors of the tests. All three have the sizes and CRC after the
//! data (flag bit 3). Regenerate with:
//!
//! ```text
//! printf 'hello hello hello pairpane\n' > a.txt
//! bsdtar --passphrase secret --format zip --options zip:encryption=aes256,zip:compression=deflate -cf out.zip a.txt
//! ```

/// The password of the vectors.
pub(super) const PASSWORD: &str = "secret";

/// The content of `a.txt` in each of them.
pub(super) const CONTENT: &[u8] = b"hello hello hello pairpane\n";

/// Traditional ZipCrypto (`zip:encryption=zipcrypt`).
pub(super) fn zipcrypto() -> Vec<u8> {
    let hex = concat!(
        "504b0304140009000800a150765d00000000000000000000000005002000612e74787475780b000104e803000004e803",
        "000055540d00073fb0026b3fb0026b3fb0026b9dae3cd273317fe8955f99ed2af565e5a5a7c97ceffbdf9a3c3fab01ab",
        "74f8504b070852609b521f0000001b000000504b01021403140009000800a150765d52609b521f0000001b0000000500",
        "18000000000000000000a48100000000612e74787475780b000104e803000004e803000055540500013fb0026b504b05",
        "0600000000010001004b000000720000000000",
    );

    decode(hex)
}

/// WinZip AES-128, AE-1 (`zip:encryption=aes128`).
pub(super) fn aes128() -> Vec<u8> {
    let hex = concat!(
        "504b0304140009006300a150765d00000000000000000000000005002b00612e74787475780b000104e803000004e803",
        "0000019907000100414501080055540d00073fb0026b3fb0026b3fb0026b3a8b8e040803e04c02740229e28edfe4b3f8",
        "3798dda2ea1e2e83f35439104e4af48994f3a21115504b070852609b52270000001b000000504b010214031400090063",
        "00a150765d52609b52270000001b000000050023000000000000000000a48100000000612e74787475780b000104e803",
        "000004e8030000019907000100414501080055540500013fb0026b504b05060000000001000100560000008500000000",
        "00",
    );

    decode(hex)
}

/// WinZip AES-256, AE-1 (`zip:encryption=aes256`).
pub(super) fn aes256() -> Vec<u8> {
    let hex = concat!(
        "504b0304140009006300a150765d00000000000000000000000005002b00612e74787475780b000104e803000004e803",
        "0000019907000100414503080055540d00073fb0026b3fb0026b3fb0026bc205424969b8e58d5ba06230b42506790367",
        "de1ae7e65eed19a7545eac8eb6f37be6ecaf81e0a8a40fb580f37217c0504b070852609b522f0000001b000000504b01",
        "021403140009006300a150765d52609b522f0000001b000000050023000000000000000000a48100000000612e747874",
        "75780b000104e803000004e8030000019907000100414503080055540500013fb0026b504b0506000000000100010056",
        "0000008d0000000000",
    );

    decode(hex)
}

fn decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
        .collect()
}
