//! The contract between a pane's listing and a server it browses: [`RemotePath`], what a server says about a path
//! ([`RemoteMeta`], [`RemoteEntry`]) and the [`RemoteFs`] that asks it.
//!
//! The core knows no protocol: the crate that speaks to a server implements [`RemoteFs`], and the folder that lists
//! one (`RemoteFolder`) works through the trait alone. Paths are the server's, in POSIX form, and never
//! touch the local file system.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::model::SiteId;

/// The longest path accepted, in bytes (what Linux allows).
pub const MAX_REMOTE_PATH: usize = 4096;

/// An absolute path on a server: `/` or `/a/b`. Components are plain names: never empty, `.`, `..`, or holding a NUL;
/// there is no trailing slash. A path is checked when made, so every `RemotePath` can be sent to a server as it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemotePath(String);

impl RemotePath {
    /// The root, `/`.
    pub fn root() -> Self {
        Self("/".to_owned())
    }

    /// `text` as a path. `InvalidInput` (the error holds no path) when it is not absolute, has an empty, `.` or `..`
    /// component or a NUL, ends in `/` (other than the root itself) or is longer than [`MAX_REMOTE_PATH`].
    pub fn new(text: &str) -> io::Result<Self> {
        if text.len() > MAX_REMOTE_PATH {
            return Err(invalid("the path is too long"));
        }

        let Some(below_root) = text.strip_prefix('/') else {
            return Err(invalid("the path is not absolute"));
        };

        if !below_root.is_empty() {
            for name in below_root.split('/') {
                check_name(name)?;
            }
        }

        Ok(Self(text.to_owned()))
    }

    /// The path as text, for the wire and the screen.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// This is the root.
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    /// The path of `name` inside this folder. `name` must be one plain name: not empty, `.` or `..`, no `/`, no NUL;
    /// the result must stay within [`MAX_REMOTE_PATH`]. `InvalidInput` otherwise.
    pub fn join(&self, name: &str) -> io::Result<Self> {
        check_name(name)?;

        let mut joined = String::with_capacity(self.0.len() + 1 + name.len());
        joined.push_str(&self.0);

        if !self.is_root() {
            joined.push('/');
        }

        joined.push_str(name);

        if joined.len() > MAX_REMOTE_PATH {
            return Err(invalid("the path is too long"));
        }

        Ok(Self(joined))
    }

    /// The folder above; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }

        let cut = self.0.rfind('/').unwrap_or(0);

        Some(Self(if cut == 0 {
            "/".to_owned()
        } else {
            self.0[..cut].to_owned()
        }))
    }

    /// The names from the root down; none for the root.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|name| !name.is_empty())
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

/// Whether `name` is one plain name a server may hold: not empty, `.` or `..`, and holding neither `/` nor NUL. A server
/// that is faulty or hostile can send anything else; a caller that shows or uses a name from one checks it with this.
pub fn is_remote_name(name: &str) -> bool {
    check_name(name).is_ok()
}

/// A fixed message without the path that failed; the error is shown to the user and logged.
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// One component: not empty, `.` or `..`, and holding neither `/` nor NUL.
fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(invalid("the path has an empty, '.' or '..' name"));
    }

    if name.contains(['/', '\0']) {
        return Err(invalid("the path has a name with '/' or NUL"));
    }

    Ok(())
}

/// What an item on a server is. A symbolic link is its own kind until its target is looked at (see [`RemoteFs::stat`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    Dir,
    File,
    /// A symbolic link.
    Link,
    /// A device, socket, pipe, or anything the server does not say.
    Other,
}

/// What a server reports about one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteMeta {
    pub kind: RemoteKind,
    /// Size in bytes; 0 when the server does not say, and for folders.
    pub size: u64,
    /// Last modification, in milliseconds since the Unix epoch; `None` when the server does not say.
    pub modified_ms: Option<i64>,
    /// Unix permission bits (`0o644`, without the file type); `None` when the server does not say.
    pub mode: Option<u32>,
}

/// One item of a remote folder, with the attributes the listing returned for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// The name inside the folder. It comes from the server: a caller must not trust it to be a plain name.
    pub name: String,
    pub meta: RemoteMeta,
}

/// A connected server, as far as browsing and downloading go. The implementation is blocking and may be called from
/// several threads at once; every call ends (with `TimedOut` at the latest) even when the server stops answering.
///
/// Errors are `io::Error`s with a fixed message and no path, host, user or secret in it, whatever the cause: the
/// caller shows them as they are, and they are logged. The kind says what happened (`NotFound`, `PermissionDenied`,
/// `TimedOut`, `ConnectionAborted` for a lost connection, `Unsupported`, `Other`).
///
/// A call that reads (`read_dir`, `stat`, `lstat`, `read_link`, `open_read`) may be repeated on a new connection when
/// the old one was lost. A call that changes the server (`remove_file`, `remove_dir`) never is: after a lost
/// connection the caller cannot tell whether it took effect, and says so. Only `read_dir` and [`RemoteReader::read`]
/// react to a cancel; a remove runs to its own limit.
///
/// A download that moves a file tells a change of it by its size and its time (a whole second on SFTP): a file that is
/// rewritten to the same size within the same second goes unseen, and the move then removes a newer original. SFTP has no
/// call that closes this; a caller that cannot accept it copies instead of moving.
///
/// A server cannot be made to remove an item only if it is still where it was looked at: the calls that change it name
/// a path, and a folder on that path may be swapped for a link between a check and the call. [`RemoteFs::realpath`]
/// narrows that window (a caller checks the folder right before it removes), it does not close it.
///
/// `lstat`, `realpath`, `open_read`, `remove_file` and `remove_dir` have a default that fails with `Unsupported`, so a
/// server that only browses implements the first five methods. A caller that removes things also asks `read_link` of each
/// link on the way to its folder (a server that does not resolve links in `realpath` shows a link repointed only by
/// where it leads), and refuses to move through a link where the server says `Unsupported`. A server whose `realpath`
/// does not resolve links at all is refused a move through a link: the folders its target leads through cannot be told.
pub trait RemoteFs: Send + Sync {
    /// The connection this answers for.
    fn site(&self) -> SiteId;

    /// What the path bar calls the server.
    fn label(&self) -> &str;

    /// The entries of `dir`, in no particular order and without `.` and `..` when the server leaves them out (a caller
    /// drops them anyway). The attributes are those of the entries themselves (`lstat`): a symbolic link is
    /// [`RemoteKind::Link`], not what it points to. Stops early with an `Interrupted` error once `cancel` is set,
    /// between the pages the server sends.
    fn read_dir(&self, dir: &RemotePath, cancel: &AtomicBool) -> io::Result<Vec<RemoteEntry>>;

    /// The attributes of `path`, following symbolic links (`stat`); `None` when it does not exist, which for a link
    /// is a link that points nowhere.
    fn stat(&self, path: &RemotePath) -> io::Result<Option<RemoteMeta>>;

    /// Where the symbolic link `path` points, as written in the link.
    fn read_link(&self, path: &RemotePath) -> io::Result<String>;

    /// The attributes of `path` itself (`lstat`): a symbolic link is [`RemoteKind::Link`], not what it points to.
    /// `None` when it does not exist.
    fn lstat(&self, _path: &RemotePath) -> io::Result<Option<RemoteMeta>> {
        Err(unsupported())
    }

    /// [`RemoteFs::lstat`] of each of `paths`, one answer for each and in the same order. A server that can have several
    /// requests in flight answers them in about the time of one (a caller checks a long way of folders with it); the
    /// default asks one after the other. Once `cancel` is set, the answers not yet had are `Interrupted` errors. There is
    /// one answer for each path: a caller treats any other count as a server that cannot be trusted. A long way costs
    /// round trips in proportion to its length (SFTP keeps 64 requests in flight, so about `paths / 64`); a move of a tree
    /// `n` folders deep asks about `n * n / 64` looks in all, a bound that is accepted.
    fn lstat_many(&self, paths: &[RemotePath], cancel: &AtomicBool) -> Vec<io::Result<Option<RemoteMeta>>> {
        paths
            .iter()
            .map(|path| {
                if cancel.load(Ordering::Relaxed) {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    self.lstat(path)
                }
            })
            .collect()
    }

    /// `path` with every symbolic link in it resolved, as the server spells it: `NotFound` when it does not exist. A
    /// caller that removes things on a server compares this with where it expects the folder to be (the path of the
    /// folder it started in, resolved once, and the plain names below it), to see a folder swapped for a link.
    fn realpath(&self, _path: &RemotePath) -> io::Result<RemotePath> {
        Err(unsupported())
    }

    /// Opens the file `path` for reading. `NotFound` when it does not exist. The reader keeps the server's
    /// resources until it is dropped.
    fn open_read(&self, _path: &RemotePath) -> io::Result<Box<dyn RemoteReader>> {
        Err(unsupported())
    }

    /// Removes the file `path` (never a folder). Not repeated on a new connection (see above).
    fn remove_file(&self, _path: &RemotePath) -> io::Result<()> {
        Err(unsupported())
    }

    /// Removes the folder `path` when it is empty. Not repeated on a new connection (see above).
    fn remove_dir(&self, _path: &RemotePath) -> io::Result<()> {
        Err(unsupported())
    }
}

/// A file of a server opened for reading (see [`RemoteFs::open_read`]). Dropping it releases the server's handle.
pub trait RemoteReader: Send {
    /// Reads at most `buf.len()` bytes, in order, from where the last read stopped. Returns `Ok(0)` only at the end of
    /// the file (the caller's `buf` is not empty), and fails with
    /// `Interrupted` once `cancel` is set (the reader stays usable). Fixed messages, as for [`RemoteFs`].
    fn read(&mut self, buf: &mut [u8], cancel: &AtomicBool) -> io::Result<usize>;

    /// What the server says of the open file now (`fstat`): the size and the times it has at this moment.
    fn meta(&mut self) -> io::Result<RemoteMeta>;
}

/// The answer of a server that does not do what was asked; a fixed message without a path.
fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "this server does not support this")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn path(text: &str) -> RemotePath {
        RemotePath::new(text).expect("valid path")
    }

    #[test]
    fn valid_paths_are_kept_as_written() {
        for text in ["/", "/home", "/home/me/.config", "/a b/ü/-x"] {
            assert_eq!(path(text).as_str(), text);
        }
    }

    #[test]
    fn invalid_paths_are_refused_without_naming_them() {
        let long = format!("/{}", "a".repeat(MAX_REMOTE_PATH));
        let refused = [
            "", "home", "./a", "/a/", "//", "/a//b", "/a/./b", "/a/..", "/..", "/a\0b", &long,
        ];

        for text in refused {
            let error = RemotePath::new(text).expect_err("refused");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{text:?}");
            assert!(!error.to_string().contains("home"), "the message names no path");
        }
    }

    #[test]
    fn the_longest_path_is_accepted() {
        let longest = format!("/{}", "a".repeat(MAX_REMOTE_PATH - 1));

        assert_eq!(path(&longest).as_str().len(), MAX_REMOTE_PATH);
    }

    #[test]
    fn join_adds_one_plain_name() {
        assert_eq!(path("/").join("etc").expect("joined").as_str(), "/etc");
        assert_eq!(path("/etc").join("hosts").expect("joined").as_str(), "/etc/hosts");
        assert_eq!(path("/a").join("b c").expect("joined").as_str(), "/a/b c");
    }

    #[test]
    fn join_refuses_what_is_not_a_plain_name() {
        for name in ["", ".", "..", "a/b", "/a", "a\0"] {
            let error = path("/etc").join(name).expect_err("refused");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
    }

    #[test]
    fn join_refuses_to_pass_the_length_cap() {
        let full = path(&format!("/{}", "a".repeat(MAX_REMOTE_PATH - 3)));
        let one_short = path(&format!("/{}", "a".repeat(MAX_REMOTE_PATH - 4)));

        assert_eq!(full.join("b").expect("at the cap").as_str().len(), MAX_REMOTE_PATH);
        assert!(
            path(&format!("/{}", "a".repeat(MAX_REMOTE_PATH - 2)))
                .join("b")
                .is_err()
        );
        assert!(one_short.join("b").is_ok());
    }

    #[test]
    fn parent_goes_up_to_the_root_and_stops() {
        assert_eq!(path("/a/b").parent(), Some(path("/a")));
        assert_eq!(path("/a").parent(), Some(RemotePath::root()));
        assert_eq!(RemotePath::root().parent(), None);
    }

    #[test]
    fn components_run_from_the_root_down() {
        assert_eq!(path("/a/b c/d").components().collect::<Vec<_>>(), ["a", "b c", "d"]);
        assert_eq!(RemotePath::root().components().count(), 0);
    }

    #[test]
    fn remote_names_are_plain_names() {
        for name in ["a", "a b", ".hidden", "...", "ü"] {
            assert!(is_remote_name(name), "{name:?}");
        }

        for name in ["", ".", "..", "a/b", "/", "a\0"] {
            assert!(!is_remote_name(name), "{name:?}");
        }
    }

    /// A server that only browses.
    struct Browsing;

    impl RemoteFs for Browsing {
        fn site(&self) -> SiteId {
            SiteId(1)
        }

        fn label(&self) -> &str {
            "me@host"
        }

        fn read_dir(&self, _: &RemotePath, _: &AtomicBool) -> io::Result<Vec<RemoteEntry>> {
            Ok(Vec::new())
        }

        fn stat(&self, _: &RemotePath) -> io::Result<Option<RemoteMeta>> {
            Ok(None)
        }

        fn read_link(&self, _: &RemotePath) -> io::Result<String> {
            Err(io::ErrorKind::NotFound.into())
        }
    }

    #[test]
    fn a_server_that_only_browses_refuses_the_rest_without_naming_a_path() {
        let server = Browsing;
        let secret = path("/secret/name");
        let refused = [
            server.lstat(&secret).map(|_| ()),
            server
                .lstat_many(std::slice::from_ref(&secret), &AtomicBool::new(false))
                .remove(0)
                .map(|_| ()),
            server.realpath(&secret).map(|_| ()),
            server.open_read(&secret).map(|_| ()),
            server.remove_file(&secret),
            server.remove_dir(&secret),
        ];

        for result in refused {
            let error = result.expect_err("refused");
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(!error.to_string().contains("secret"), "the message names no path");
        }
    }

    #[test]
    fn many_looks_default_to_one_answer_for_each_path_and_stop_with_an_interrupt_once_cancelled() {
        let paths = [path("/a"), path("/a/b"), path("/a/b/c")];

        for (cancelled, kind) in [(false, io::ErrorKind::Unsupported), (true, io::ErrorKind::Interrupted)] {
            let answers = Browsing.lstat_many(&paths, &AtomicBool::new(cancelled));

            assert_eq!(answers.len(), 3, "one answer for each path");
            for answer in answers {
                assert_eq!(answer.expect_err("refused").kind(), kind, "cancelled: {cancelled}");
            }
        }
    }

    #[test]
    fn display_shows_the_path() {
        assert_eq!(path("/a/b").to_string(), "/a/b");
    }
}
