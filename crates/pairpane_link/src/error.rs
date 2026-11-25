//! The one error type of the link crate. Every text is fixed, lowercase English and holds no key, code, path or
//! address, so a caller can show it as it is; the system's own message of a failed call is kept as the `source`.

use std::error::Error as StdError;
use std::fmt;
use std::io;

/// Why a link operation failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file of the link (the key file or the device list) could not be read or written.
    Io(io::Error),
    /// The OS keychain refused or failed; the text is the keychain's own message.
    Keychain(String),
    /// The key file can be read by other users (its mode has group or other bits), so it is not trusted.
    InsecureKeyFile,
    /// The stored device key is not 32 bytes long.
    CorruptKey,
    /// There is no keychain at all on this system (no Secret Service is running); a file may be used instead.
    KeychainUnavailable(String),
    /// The device was set up with a key store that cannot be used now (the keychain is locked or refused, or the key is
    /// gone). No new key is made, because that would give the device a new identity and unpair it from every other.
    KeyUnavailable(String),
    /// The stored device key is not the one this device was set up with.
    KeyMismatch,
    /// The identity marker file (`link-identity.json`, which names the store of the device key) is damaged or of a
    /// version this build does not know. Nothing secret is in it; the key itself is not affected.
    CorruptIdentityMarker,
    /// Another link service already runs on the same data folder.
    AlreadyRunning,
    /// The device list is too large, is not valid JSON, has a field it should not have or holds a bad record.
    CorruptDevices,
    /// A device name is empty, longer than 64 characters or has a control character.
    InvalidName,
    /// The text is not a pairing code.
    InvalidCode,
    /// The text is not a device to connect to: a node id, optionally followed by `@` and socket addresses.
    InvalidTarget,
    /// The device list holds as many devices as it may.
    TooManyDevices,
    /// There is no paired device with that id.
    UnknownDevice,
    /// The network endpoint could not be started; the text is the reason.
    Bind(String),
    /// The other device could not be reached (no route, no answer in time, or it speaks another protocol).
    Unreachable(String),
    /// The other device turned the pairing down: the code is wrong, used, expired or cancelled.
    PairingRejected,
    /// The other device did not answer a pairing step in time.
    PairingTimedOut,
    /// The other device sent bytes that are not part of the protocol.
    Protocol(&'static str),
    /// The system's random generator failed.
    Random,
    /// The link service has been shut down.
    Closed,
    /// The call needs a running async runtime on the calling thread (the pairing timer is a task of it) and there is none.
    NoRuntime,
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => out.write_str("a file of the link could not be read or written"),
            Self::Keychain(reason) => write!(out, "the keychain failed: {reason}"),
            Self::KeychainUnavailable(reason) => write!(out, "there is no keychain: {reason}"),
            Self::KeyUnavailable(reason) => {
                write!(
                    out,
                    "the device key cannot be read (unlock your keyring and try again): {reason}"
                )
            }
            Self::KeyMismatch => out.write_str("the stored device key is not the key of this device"),
            Self::CorruptIdentityMarker => out.write_str("the file that names the device key's store is damaged"),
            Self::AlreadyRunning => out.write_str("the link already runs on this folder"),
            Self::InsecureKeyFile => out.write_str("the key file can be read by other users"),
            Self::CorruptKey => out.write_str("the stored device key is damaged"),
            Self::CorruptDevices => out.write_str("the list of paired devices is damaged"),
            Self::InvalidName => out.write_str("a device name must be 1 to 64 characters without control characters"),
            Self::InvalidCode => out.write_str("that is not a pairing code"),
            Self::InvalidTarget => out.write_str("that is not a device id with addresses"),
            Self::TooManyDevices => out.write_str("too many paired devices"),
            Self::UnknownDevice => out.write_str("there is no such paired device"),
            Self::Bind(reason) => write!(out, "the link could not start: {reason}"),
            Self::Unreachable(reason) => write!(out, "the device could not be reached: {reason}"),
            Self::PairingRejected => out.write_str("the code is wrong, used, expired or cancelled"),
            Self::PairingTimedOut => out.write_str("the other device did not answer in time"),
            Self::Protocol(what) => write!(out, "the other device broke the protocol: {what}"),
            Self::Random => out.write_str("the system's random generator failed"),
            Self::Closed => out.write_str("the link is shut down"),
            Self::NoRuntime => out.write_str("no async runtime is running on this thread"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

/// The result of a link operation.
pub type Result<T> = std::result::Result<T, Error>;
