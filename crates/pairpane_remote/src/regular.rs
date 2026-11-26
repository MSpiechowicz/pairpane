//! Opening a file that must be a regular one: a key file, `known_hosts`, `ssh_config` and what it includes.

use std::fs;
use std::io;
use std::path::Path;

/// Why a file was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    /// There is no such file.
    Missing,
    /// It is there and cannot be opened.
    Unreadable,
    /// Something that is not a regular file (a pipe or a device would wait for ever; a folder holds nothing).
    NotRegular,
}

impl Opening {
    fn of(error: &io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::Missing
        } else {
            Self::Unreadable
        }
    }
}

/// Opens `path` without waiting (a pipe would wait for its other end to open) and requires that what was opened is a
/// regular file: the check is on the open file, so nothing can be put in its place between looking and opening. A link
/// to a regular file is a regular file.
#[cfg(unix)]
pub(crate) fn open_regular(path: &Path) -> Result<fs::File, Opening> {
    use rustix::fs::{Mode, OFlags};

    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rustix::fs::open(path, flags, Mode::empty()).map_err(|errno| Opening::of(&errno.into()))?;
    let file = fs::File::from(fd);

    match file.metadata() {
        Ok(metadata) if metadata.is_file() => Ok(file),
        Ok(_) => Err(Opening::NotRegular),
        Err(_) => Err(Opening::Unreadable),
    }
}

/// Without `O_NONBLOCK` to open with, what is looked at before opening is a regular file.
#[cfg(not(unix))]
pub(crate) fn open_regular(path: &Path) -> Result<fs::File, Opening> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(Opening::NotRegular),
        Err(error) => return Err(Opening::of(&error)),
    }

    let file = fs::File::open(path).map_err(|error| Opening::of(&error))?;

    // What was opened is the file that was looked at.
    if file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        Ok(file)
    } else {
        Err(Opening::NotRegular)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_pipe_is_not_waited_for() {
        let folder = tempfile::tempdir().expect("temp dir");
        let pipe = folder.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("mkfifo runs");

        assert!(made.success());

        let started = Instant::now();

        assert_eq!(open_regular(&pipe).expect_err("not a file"), Opening::NotRegular);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            open_regular(&folder.path().join("none")).expect_err("missing"),
            Opening::Missing
        );
        assert_eq!(open_regular(folder.path()).expect_err("a folder"), Opening::NotRegular);
    }
}
