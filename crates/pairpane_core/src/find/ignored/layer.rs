//! The rules that apply in one folder, and the share of the budget they and the layer itself hold.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use ignore::Match;
use ignore::gitignore::{Gitignore, Glob};

use super::budget::Charge;

/// The rules of one ignore file, with the share of the budget they hold until they are dropped.
pub(super) struct Rules {
    pub(super) gitignore: Gitignore,
    pub(super) charge: Charge,
}

/// The rules that apply in a folder: its own files and, through `parent`, those of the folders above. A folder that has no
/// rules and is not a repository has no layer: that of the nearest folder above that has one stands for it, so a chain of
/// layers is as long as the number of folders with rules or a `.git`, not as deep as the tree.
///
/// Every layer made while searching holds a share of the budget (`cost`) for itself, whatever it holds: a folder with a
/// `.git` and nothing else takes memory too. The exception is the boundary of a folder with a `.git` that there was no room
/// for ([`Layer::boundary`]).
pub(super) struct Layer {
    pub(super) parent: Option<Arc<Layer>>,
    pub(super) dir: PathBuf,
    /// How many layers are above this one.
    pub(super) depth: usize,
    pub(super) ignore: Option<Rules>,
    pub(super) git: Option<Rules>,
    pub(super) exclude: Option<Rules>,
    /// The folder holds a `.git`.
    pub(super) repository: bool,
    /// The folder is in a git repository: it holds a `.git` or a folder above does.
    pub(super) in_repository: bool,
    /// The share of the budget the layer itself holds; none for the layer that stands for the folder searched when it has
    /// no rules, which lives for the whole search, and for a boundary.
    pub(super) cost: Option<Charge>,
    /// When the folder was last asked for.
    pub(super) used: AtomicU64,
}

impl Layer {
    pub(super) fn empty(dir: &Path) -> Self {
        Self {
            parent: None,
            dir: dir.to_owned(),
            depth: 0,
            ignore: None,
            git: None,
            exclude: None,
            repository: false,
            in_repository: false,
            cost: None,
            used: AtomicU64::new(0),
        }
    }

    /// The layer of a folder with a `.git` that there is no room for: it holds no rules and costs nothing, and has no path
    /// (it is never matched against one), but ends the reach of git's files above it and keeps the folders below in a
    /// repository, as a layer of the folder would. It is as small as a layer gets, and held by the folder's place alone, so
    /// no more of them are held than there are places.
    pub(super) fn boundary(parent: Option<&Arc<Layer>>, used: u64) -> Self {
        Self {
            parent: parent.cloned(),
            dir: PathBuf::new(),
            depth: parent.map_or(0, |parent| parent.depth + 1),
            ignore: None,
            git: None,
            exclude: None,
            repository: true,
            in_repository: true,
            cost: None,
            used: AtomicU64::new(used),
        }
    }

    /// Whether the folder has ignore rules of its own.
    pub(super) fn has_rules(&self) -> bool {
        self.ignore.is_some() || self.git.is_some() || self.exclude.is_some()
    }

    /// The lines and bytes of the budget this layer holds, with its rules, not those of the folders above.
    pub(super) fn charged(&self) -> (usize, usize) {
        let own = self.cost.iter().map(|cost| (cost.lines, cost.bytes));
        let rules = [&self.ignore, &self.git, &self.exclude]
            .into_iter()
            .flatten()
            .map(|rules| (rules.charge.lines, rules.charge.bytes));

        own.chain(rules)
            .fold((0, 0), |(lines, bytes), (more_lines, more_bytes)| {
                (lines + more_lines, bytes + more_bytes)
            })
    }
}

pub(super) fn matched<'a>(rules: &'a Option<Rules>, relative: &Path, is_dir: bool) -> Match<&'a Glob> {
    match rules {
        Some(rules) => rules.gitignore.matched(relative, is_dir),
        None => Match::None,
    }
}
