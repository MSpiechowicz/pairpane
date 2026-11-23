//! Real repositories for the tests, built with gix itself in a temporary folder (the `git` program is never needed).

use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::BString;
use gix::index::entry::{Flags, Mode, Stage, Stat};
use gix::objs::tree::EntryKind;

use crate::model::{GitEntry, GitMark};

/// The commit every gitlink of the tests points to; it is in no repository.
fn gitlink_target() -> gix::ObjectId {
    gix::ObjectId::from_hex(b"0123456789012345678901234567890123456789").expect("an id")
}

/// A repository with a work tree in a temporary folder.
pub(super) struct Repo {
    /// Kept so the folder lives as long as the repository.
    pub(super) dir: tempfile::TempDir,
    repo: gix::Repository,
    /// The real path of the work tree.
    root: PathBuf,
}

/// One entry of an index to write: a path, the content of its blob, its stage (0 unless it is in conflict) and its mode.
pub(super) struct Staged<'a> {
    pub(super) path: &'a str,
    pub(super) content: &'a [u8],
    pub(super) stage: Stage,
    pub(super) mode: Mode,
    /// The id of the entry, when it is not the id of a blob of `content` (a sparse directory is a tree).
    pub(super) id: Option<gix::ObjectId>,
    /// The entry is marked skip-worktree: its file is not to be looked for in the work tree, and what reads it reads the blob.
    pub(super) skip_worktree: bool,
}

impl<'a> Staged<'a> {
    pub(super) fn file(path: &'a str, content: &'a [u8]) -> Self {
        Self {
            path,
            content,
            stage: Stage::Unconflicted,
            mode: Mode::FILE,
            id: None,
            skip_worktree: false,
        }
    }

    /// The entry marked skip-worktree.
    pub(super) fn skipped(self) -> Self {
        Self {
            skip_worktree: true,
            ..self
        }
    }

    /// A directory of a sparse index: one entry, with the path of the folder and a slash at its end, for the tree `tree`, which
    /// is not checked out.
    pub(super) fn sparse(path: &'a str, tree: gix::ObjectId) -> Self {
        Self {
            mode: Mode::DIR,
            id: Some(tree),
            ..Self::file(path, b"")
        }
    }

    pub(super) fn conflict(path: &'a str, content: &'a [u8], stage: Stage) -> Self {
        Self {
            stage,
            ..Self::file(path, content)
        }
    }

    /// A submodule: a gitlink to a commit that is not in this repository.
    pub(super) fn gitlink(path: &'a str) -> Self {
        Self {
            mode: Mode::COMMIT,
            ..Self::file(path, b"")
        }
    }

    /// A file that is executable in the index.
    pub(super) fn executable(path: &'a str, content: &'a [u8]) -> Self {
        Self {
            mode: Mode::FILE_EXECUTABLE,
            ..Self::file(path, content)
        }
    }
}

impl Repo {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().expect("a folder");
        let root = gix::path::realpath(dir.path()).expect("its real path");
        let repo = gix::init(&root).expect("a repository");

        Self { dir, repo, root }
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// A linked work tree `name` of this repository, next to it: its folder, with a `.git` file, and its own folder in the
    /// repository (`HEAD` and the index are copies of the repository's). The folders of both are returned.
    pub(super) fn linked_worktree(&self, name: &str) -> (PathBuf, PathBuf) {
        self.linked_worktree_at(self.dir.path().join(name), name)
    }

    /// Like [`Self::linked_worktree`], with the work tree at `linked` (a folder that is not made yet).
    pub(super) fn linked_worktree_at(&self, linked: PathBuf, name: &str) -> (PathBuf, PathBuf) {
        let admin = self.git_dir().join("worktrees").join(name);
        fs::create_dir_all(&linked).expect("the worktree folder");
        fs::create_dir_all(&admin).expect("the admin folder");
        fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).expect("the git file");
        fs::write(admin.join("gitdir"), format!("{}\n", linked.join(".git").display())).expect("gitdir");
        fs::write(admin.join("commondir"), "../..\n").expect("commondir");
        fs::copy(self.git_dir().join("HEAD"), admin.join("HEAD")).expect("HEAD");
        fs::copy(self.git_dir().join("index"), admin.join("index")).expect("the index");

        (linked, admin)
    }

    pub(super) fn index_path(&self) -> PathBuf {
        self.repo.index_path()
    }

    /// Adds `text` (config lines) to the repository's own `config` file.
    #[cfg(unix)]
    pub(super) fn config_append(&self, text: &str) {
        use std::io::Write;

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(self.git_dir().join("config"))
            .expect("the config file");
        file.write_all(text.as_bytes()).expect("written");
    }

    /// The path of `rela` inside the work tree.
    pub(super) fn path(&self, rela: &str) -> PathBuf {
        self.root.join(rela)
    }

    /// Writes (or overwrites) a file of the work tree, with its folders.
    pub(super) fn put(&self, rela: &str, content: &[u8]) {
        let path = self.path(rela);
        fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        fs::write(path, content).expect("a file");
    }

    pub(super) fn remove(&self, rela: &str) {
        fs::remove_file(self.path(rela)).expect("removed");
    }

    /// Makes `files` the content of `HEAD` (one commit with no parent), of the index and of the work tree.
    pub(super) fn commit(&self, files: &[(&str, &[u8])]) {
        self.commit_with_submodules(files, &[]);
    }

    /// Like [`Self::commit`], with a submodule (a gitlink to a commit that is not here) at each of `submodules` too.
    pub(super) fn commit_with_submodules(&self, files: &[(&str, &[u8])], submodules: &[&str]) {
        let blobs: Vec<_> = files
            .iter()
            .map(|(path, content)| (*path, self.repo.write_blob(content).expect("a blob").detach()))
            .collect();
        let mut editor = self.repo.edit_tree(self.repo.empty_tree().id).expect("an editor");
        for (path, id) in &blobs {
            editor.upsert(*path, EntryKind::Blob, *id).expect("an entry");
        }
        for path in submodules {
            editor
                .upsert(*path, EntryKind::Commit, gitlink_target())
                .expect("a gitlink");
        }
        let tree = editor.write().expect("a tree").detach();
        self.commit_tree(tree);

        for (path, content) in files {
            self.put(path, content);
        }
        let mut entries: Vec<_> = files
            .iter()
            .map(|(path, content)| Staged::file(path, content))
            .collect();
        entries.extend(submodules.iter().map(|path| Staged::gitlink(path)));
        self.write_index(&entries);
    }

    /// The id of the tree of the folder `path` in the tree of `HEAD`.
    pub(super) fn tree_of(&self, path: &str) -> gix::ObjectId {
        let head = self.repo.head_commit().expect("a commit");
        let tree = head.tree().expect("its tree");
        let entry = tree
            .lookup_entry_by_path(path)
            .expect("a lookup")
            .expect("a folder of HEAD");

        entry.object_id()
    }

    /// Makes the tree `tree` the content of `HEAD` (one commit with no parent), leaving the index and the work tree alone.
    pub(super) fn commit_tree(&self, tree: gix::ObjectId) {
        let signature = gix::actor::SignatureRef {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: "1700000000 +0000",
        };
        self.repo
            .commit_as(signature, signature, "HEAD", "a commit", tree, None::<gix::ObjectId>)
            .expect("a commit");
    }

    /// A tree with `width` entries, all of them the folder `child`.
    pub(super) fn repeating(&self, child: gix::ObjectId, width: usize) -> gix::ObjectId {
        let entries = (0..width)
            .map(|index| gix::objs::tree::Entry {
                mode: EntryKind::Tree.into(),
                filename: format!("e{index:03}").into(),
                oid: child,
            })
            .collect();

        self.repo
            .write_object(gix::objs::Tree { entries })
            .expect("a tree")
            .detach()
    }

    /// A tree that holds one file, `name`.
    pub(super) fn leaf(&self, name: &str) -> gix::ObjectId {
        let blob = self.repo.write_blob(b"x").expect("a blob").detach();
        let entries = vec![gix::objs::tree::Entry {
            mode: EntryKind::Blob.into(),
            filename: name.into(),
            oid: blob,
        }];

        self.repo
            .write_object(gix::objs::Tree { entries })
            .expect("a tree")
            .detach()
    }

    /// Replaces the index with `entries`, which must be in the order of their paths. The stat of each entry is empty, as
    /// if the index had been made from a tree, so each file is compared by its content.
    pub(super) fn write_index(&self, entries: &[Staged<'_>]) {
        self.write_index_with(entries, false);
    }

    /// Like [`Self::write_index`], with the stat of each entry read from the file in the work tree, as `git add` does: a
    /// file that was not changed since is clean without being read.
    pub(super) fn write_index_with_disk_stat(&self, entries: &[Staged<'_>]) {
        self.write_index_with(entries, true);
    }

    fn write_index_with(&self, entries: &[Staged<'_>], disk_stat: bool) {
        let mut state = gix::index::State::new(self.repo.object_hash());
        for entry in entries {
            let id = match entry.id {
                Some(id) => id,
                None if entry.mode == Mode::COMMIT => gitlink_target(),
                None => self.repo.write_blob(entry.content).expect("a blob").detach(),
            };
            // A sparse directory is not in the work tree, and git marks it so.
            let flags = if entry.mode == Mode::DIR || entry.skip_worktree {
                Flags::from_stage(entry.stage) | Flags::SKIP_WORKTREE | Flags::EXTENDED
            } else {
                Flags::from_stage(entry.stage)
            };
            let stat = if disk_stat {
                let metadata = gix::index::fs::Metadata::from_path_no_follow(&self.path(entry.path)).expect("metadata");

                Stat::from_fs(&metadata).expect("a stat")
            } else {
                Stat::default()
            };
            state.dangerously_push_entry(stat, id, flags, entry.mode, BString::from(entry.path).as_ref());
        }
        state.sort_entries();

        self.save(state);
    }

    fn save(&self, state: gix::index::State) {
        let file = gix::index::File::from_state(state, self.repo.index_path());
        let mut out = fs::File::create(self.repo.index_path()).expect("the index file");
        file.write_to(&mut out, gix::index::write::Options::default())
            .expect("written");
    }
}

/// Runs `work` on a thread of its own and gives what it returns, or fails the test when it has not returned after
/// `seconds`: a status that waits on a named pipe would otherwise hold the test run for ever.
pub(super) fn within<T: Send + 'static>(seconds: u64, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });

    receiver
        .recv_timeout(std::time::Duration::from_secs(seconds))
        .expect("the status did not answer in time")
}

/// The marks of a status as `(name, mark)` pairs, for short comparisons.
pub(super) fn pairs(entries: &[GitEntry]) -> Vec<(&str, GitMark)> {
    entries.iter().map(|entry| (entry.name.as_str(), entry.mark)).collect()
}
