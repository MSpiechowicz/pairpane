//! The errors of this crate: always an `io::Error` with a fixed message. A message never holds a host, a user, a path or
//! a secret (the errors are shown to the user and written to the log), and the kind says what happened.

use std::io;

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;

/// An error of `kind` with a message that is the same every time.
pub(crate) fn fail(kind: io::ErrorKind, message: &'static str) -> io::Error {
    io::Error::new(kind, message)
}

/// A step took longer than its limit.
pub(crate) fn timed_out() -> io::Error {
    fail(io::ErrorKind::TimedOut, "the server did not answer in time")
}

/// The connection is gone: the next call may open it again.
pub(crate) fn lost() -> io::Error {
    fail(
        io::ErrorKind::ConnectionAborted,
        "the connection to the server was lost",
    )
}

/// The caller asked to stop a read that was waiting for the server.
pub(crate) fn cancelled() -> io::Error {
    fail(io::ErrorKind::Interrupted, "the read was cancelled")
}

/// The site was closed, or the service that runs it was shut down.
pub(crate) fn closed() -> io::Error {
    fail(io::ErrorKind::NotConnected, "the connection is closed")
}

/// What an SFTP request that failed says, as a plain error. `UnexpectedEof` is the end of a listing, not a failure.
pub(crate) fn from_sftp(error: SftpError) -> io::Error {
    match error {
        SftpError::Timeout => timed_out(),
        SftpError::Status(status) => match status.status_code {
            StatusCode::Eof => fail(io::ErrorKind::UnexpectedEof, "the end of the listing"),
            StatusCode::NoSuchFile => fail(io::ErrorKind::NotFound, "no such file or folder"),
            StatusCode::PermissionDenied => fail(io::ErrorKind::PermissionDenied, "permission denied"),
            StatusCode::OpUnsupported => fail(io::ErrorKind::Unsupported, "the server does not support this"),
            StatusCode::NoConnection | StatusCode::ConnectionLost => lost(),
            _ => fail(io::ErrorKind::Other, "the server could not do it"),
        },
        // The stream or the session task is gone.
        SftpError::IO(_) | SftpError::UnexpectedBehavior(_) => lost(),
        SftpError::Limited(_) => fail(io::ErrorKind::Other, "the server's limits do not allow it"),
        SftpError::UnexpectedPacket => fail(io::ErrorKind::InvalidData, "the server sent something unexpected"),
    }
}

/// Why a connection could not be made or kept, from the SSH layer.
pub(crate) fn from_ssh(error: &russh::Error) -> io::Error {
    match error {
        russh::Error::ConnectionTimeout | russh::Error::KeepaliveTimeout | russh::Error::InactivityTimeout => {
            timed_out()
        }
        russh::Error::IO(cause) if cause.kind() == io::ErrorKind::ConnectionRefused => {
            fail(io::ErrorKind::ConnectionRefused, "the server refused the connection")
        }
        russh::Error::IO(cause)
            if matches!(
                cause.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
            ) =>
        {
            lost()
        }
        russh::Error::Disconnect | russh::Error::HUP | russh::Error::SendError | russh::Error::RecvError => lost(),
        _ => fail(io::ErrorKind::Other, "the SSH connection could not be set up"),
    }
}

/// Why a TCP connection could not be made.
pub(crate) fn from_tcp(error: &io::Error) -> io::Error {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => fail(io::ErrorKind::ConnectionRefused, "the server refused the connection"),
        io::ErrorKind::TimedOut => timed_out(),
        _ => fail(io::ErrorKind::NotConnected, "the server could not be reached"),
    }
}
