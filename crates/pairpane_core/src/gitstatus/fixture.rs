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
}

impl<'a> Staged<'a> {
    pub(super) fn file(path: &'a str, content: &'a [u8]) -> Self {
        Self {
            path,
            content,
            stage: Stage::Unconflicted,
            mode: Mode::FILE,
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

        let signature = gix::actor::SignatureRef {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: "1700000000 +0000",
        };
        self.repo
            .commit_as(signature, signature, "HEAD", "a commit", tree, None::<gix::ObjectId>)
            .expect("a commit");

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

    /// Replaces the index with `entries`, which must be in the order of their paths. The stat of each entry is empty, as
    /// if the index had been made from a tree, so each file is compared by its content.
    pub(super) fn write_index(&self, entries: &[Staged<'_>]) {
        let mut state = gix::index::State::new(self.repo.object_hash());
        for entry in entries {
            let id = if entry.mode == Mode::COMMIT {
                gitlink_target()
            } else {
                self.repo.write_blob(entry.content).expect("a blob").detach()
            };
            state.dangerously_push_entry(
                Stat::default(),
                id,
                Flags::from_stage(entry.stage),
                entry.mode,
                BString::from(entry.path).as_ref(),
            );
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

/// The marks of a status as `(name, mark)` pairs, for short comparisons.
pub(super) fn pairs(entries: &[GitEntry]) -> Vec<(&str, GitMark)> {
    entries.iter().map(|entry| (entry.name.as_str(), entry.mark)).collect()
}
