//! The marks of the direct children of the folder, merged from the three results of a status.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

use gix::bstr::{BStr, ByteSlice};

use super::watch::Meter;
use crate::model::{GitEntry, GitMark};

/// The marks found so far, by the name of the child of the folder they are for.
pub(super) struct Marks {
    /// The repository-relative path of the folder with a slash at its end, or empty for the root of the work tree.
    prefix: Vec<u8>,
    by_name: BTreeMap<String, GitMark>,
    /// The folder itself is untracked or ignored as a whole (or lies inside such a folder): the directory walk says so
    /// with one entry for the folder or one of its parents, and not with one for each child.
    whole: Option<GitMark>,
}

/// How severe a mark is, for the child that has several (and for a folder, which takes the most severe mark below it):
/// `conflicted` (U) over `modified` (M) over `staged` (S) over `untracked` (?) over `ignored`.
fn rank(mark: GitMark) -> u8 {
    match mark {
        GitMark::Conflicted => 5,
        GitMark::Modified => 4,
        GitMark::Staged => 3,
        GitMark::Untracked => 2,
        GitMark::Ignored => 1,
    }
}

impl Marks {
    /// `prefix` is the repository-relative path of the folder, `/` separated and without a slash at its end.
    pub(super) fn new(prefix: &str) -> Self {
        let mut prefix = prefix.as_bytes().to_vec();
        if !prefix.is_empty() {
            prefix.push(b'/');
        }

        Self {
            prefix,
            by_name: BTreeMap::new(),
            whole: None,
        }
    }

    /// The repository-relative path of the folder with a slash at its end, as the index spells it.
    pub(super) fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// Notes that `mark` was found for `rela_path` (repository-relative, `/` separated).
    ///
    /// The mark goes to the direct child of the folder that `rela_path` is, or is below. A child with something below it
    /// takes the most severe mark found below it (see [`rank`]), except that an ignored entry below it never marks it: a
    /// child is ignored only when it is itself ignored. So a folder with only staged files is `staged`, one that is
    /// untracked as a whole, or a nested repository, is `untracked`, and a staged submodule (a gitlink) is `staged`.
    pub(super) fn note(&mut self, rela_path: &BStr, mark: GitMark) {
        let Some(rest) = rela_path.strip_prefix(self.prefix.as_slice()) else {
            self.note_enclosing(rela_path, mark);

            return;
        };
        let (child, deeper) = match rest.find_byte(b'/') {
            Some(at) => (&rest[..at], true),
            None => (rest, false),
        };
        if child.is_empty() || child == b".git" || (deeper && mark == GitMark::Ignored) {
            return;
        }

        self.keep(String::from_utf8_lossy(child).into_owned(), mark);
    }

    /// `rela_path` is not inside the folder. It still matters when it is the folder or one of its parents, which the
    /// directory walk reports as a whole when everything in it is untracked or ignored.
    fn note_enclosing(&mut self, rela_path: &BStr, mark: GitMark) {
        let mut enclosing = rela_path.to_vec();
        if !enclosing.is_empty() {
            enclosing.push(b'/');
        }
        if !self.prefix.starts_with(&enclosing) {
            return;
        }

        if matches!(mark, GitMark::Ignored | GitMark::Untracked) {
            self.whole = Some(match self.whole {
                Some(GitMark::Ignored) => GitMark::Ignored,
                _ => mark,
            });
        }
    }

    fn keep(&mut self, name: String, mark: GitMark) {
        let slot = self.by_name.entry(name).or_insert(mark);
        if rank(mark) > rank(*slot) {
            *slot = mark;
        }
    }

    /// The marks, by name. When the folder as a whole is untracked or ignored, each of its children has that mark too;
    /// `folder` is read for their names, and the read counts in `meter`.
    pub(super) fn finish(mut self, folder: &Path, meter: &mut Meter<'_>) -> io::Result<Vec<GitEntry>> {
        if let Some(whole) = self.whole {
            for entry in fs::read_dir(folder)? {
                if !meter.spend(1) {
                    break;
                }

                let name = entry?.file_name();
                if name == OsStr::new(".git") {
                    continue;
                }

                self.keep(name.to_string_lossy().into_owned(), whole);
            }
        }

        // The page matches the names it lists exactly, and gix spells a name as the index does: precomposed, and in the case
        // of the commit that added it, where a file system (macOS, Windows) can list it in another way.
        let respell = cfg!(windows) || self.by_name.keys().any(|name| !name.is_ascii());
        if respell && !self.by_name.is_empty() {
            let listing = fs::read_dir(folder)?
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<io::Result<Vec<_>>>()?;
            meter.spend(listing.len());
            self.by_name = spelled_as_listed(self.by_name, &listing, cfg!(windows));
        }

        Ok(self
            .by_name
            .into_iter()
            .map(|(name, mark)| GitEntry { name, mark })
            .collect())
    }
}

/// The names of `marks` as the folder spells them. On macOS gix gives the names precomposed (NFC) while a file can be
/// named in the decomposed form (NFD) on disk, and on Windows the index can spell a name in another case than the folder
/// does (`fold_case`); the page matches the names it lists exactly. A name that is not listed as it is takes the spelling
/// of the listed name that is the same text once precomposed (and, with `fold_case`, lowercased). When two listed names
/// are the same text so, the first one listed is taken, which can only be the case on a file system that tells them apart.
fn spelled_as_listed(
    marks: BTreeMap<String, GitMark>,
    listing: &[String],
    fold_case: bool,
) -> BTreeMap<String, GitMark> {
    let same_text = |name: &str| {
        let precomposed = gix::utils::str::precompose(name.into()).into_owned();

        if fold_case {
            precomposed.to_lowercase()
        } else {
            precomposed
        }
    };
    let exact: HashSet<&str> = listing.iter().map(String::as_str).collect();
    let mut listed: HashMap<String, &String> = HashMap::new();
    for name in listing {
        listed.entry(same_text(name)).or_insert(name);
    }

    let mut respelled = BTreeMap::new();
    for (name, mark) in marks {
        let name = match listed.get(&same_text(&name)) {
            Some(spelled) if !exact.contains(name.as_str()) => (*spelled).clone(),
            _ => name,
        };
        let slot: &mut GitMark = respelled.entry(name).or_insert(mark);
        if rank(mark) > rank(*slot) {
            *slot = mark;
        }
    }

    respelled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(marks: Marks) -> Vec<(String, GitMark)> {
        let stop = super::super::watch::Stop::default();
        let mut meter = Meter::new(&stop, 100);

        marks
            .finish(Path::new("."), &mut meter)
            .expect("marks")
            .into_iter()
            .map(|entry| (entry.name, entry.mark))
            .collect()
    }

    fn pair(name: &str, mark: GitMark) -> (String, GitMark) {
        (name.to_owned(), mark)
    }

    #[test]
    fn a_folder_takes_the_most_severe_mark_below_it() {
        let mut marks = Marks::new("src");
        marks.note(b"src/a.rs".as_bstr(), GitMark::Staged);
        marks.note(b"src/staged/b.rs".as_bstr(), GitMark::Staged);
        marks.note(b"src/fresh/deep/c.rs".as_bstr(), GitMark::Untracked);
        marks.note(b"src/mixed/d.rs".as_bstr(), GitMark::Untracked);
        marks.note(b"src/mixed/e.rs".as_bstr(), GitMark::Staged);
        marks.note(b"src/edited/f.rs".as_bstr(), GitMark::Staged);
        marks.note(b"src/edited/g.rs".as_bstr(), GitMark::Modified);
        marks.note(b"src/edited/h.rs".as_bstr(), GitMark::Untracked);
        marks.note(b"src/clash/i.rs".as_bstr(), GitMark::Modified);
        marks.note(b"src/clash/j.rs".as_bstr(), GitMark::Conflicted);
        marks.note(b"elsewhere/k.rs".as_bstr(), GitMark::Modified);

        assert_eq!(
            marked(marks),
            vec![
                pair("a.rs", GitMark::Staged),
                pair("clash", GitMark::Conflicted),
                pair("edited", GitMark::Modified),
                pair("fresh", GitMark::Untracked),
                pair("mixed", GitMark::Staged),
                pair("staged", GitMark::Staged),
            ]
        );
    }

    #[test]
    fn an_ignored_entry_below_a_child_does_not_mark_it_but_an_ignored_child_is_ignored() {
        let mut marks = Marks::new("");
        marks.note(b"target/debug/x".as_bstr(), GitMark::Ignored);
        marks.note(b"build".as_bstr(), GitMark::Ignored);
        marks.note(b"app.log".as_bstr(), GitMark::Ignored);
        marks.note(b"mixed/skip.tmp".as_bstr(), GitMark::Ignored);
        marks.note(b"mixed/new.txt".as_bstr(), GitMark::Untracked);

        assert_eq!(
            marked(marks),
            vec![
                pair("app.log", GitMark::Ignored),
                pair("build", GitMark::Ignored),
                pair("mixed", GitMark::Untracked),
            ]
        );
    }

    #[test]
    fn a_nested_repository_or_a_staged_submodule_is_marked_as_one_child() {
        let mut marks = Marks::new("");
        marks.note(b"nested".as_bstr(), GitMark::Untracked);
        marks.note(b"vendor/lib".as_bstr(), GitMark::Staged);
        marks.note(b"lib".as_bstr(), GitMark::Staged);

        assert_eq!(
            marked(marks),
            vec![
                pair("lib", GitMark::Staged),
                pair("nested", GitMark::Untracked),
                pair("vendor", GitMark::Staged),
            ]
        );
    }

    #[test]
    fn the_strongest_mark_of_a_child_wins() {
        let mut marks = Marks::new("");
        marks.note(b"a".as_bstr(), GitMark::Staged);
        marks.note(b"a".as_bstr(), GitMark::Modified);
        marks.note(b"b".as_bstr(), GitMark::Modified);
        marks.note(b"b".as_bstr(), GitMark::Conflicted);
        marks.note(b"d".as_bstr(), GitMark::Ignored);
        marks.note(b"d/x".as_bstr(), GitMark::Staged);
        marks.note(b"u".as_bstr(), GitMark::Untracked);
        marks.note(b"u".as_bstr(), GitMark::Ignored);

        assert_eq!(
            marked(marks),
            vec![
                pair("a", GitMark::Modified),
                pair("b", GitMark::Conflicted),
                pair("d", GitMark::Staged),
                pair("u", GitMark::Untracked),
            ]
        );
    }

    #[test]
    fn the_git_folder_and_the_folder_itself_are_never_marked() {
        let mut marks = Marks::new("sub");
        marks.note(b"sub/.git".as_bstr(), GitMark::Untracked);
        marks.note(b"sub/.git/config".as_bstr(), GitMark::Modified);
        marks.note(b"sub".as_bstr(), GitMark::Modified);
        marks.note(b"subway/x".as_bstr(), GitMark::Modified);

        assert_eq!(marked(marks), vec![]);
    }

    #[test]
    fn a_name_takes_the_spelling_the_folder_lists_it_with() {
        let composed = "caf\u{e9}.txt";
        let decomposed = "cafe\u{301}.txt";
        let marks = BTreeMap::from([
            (composed.to_owned(), GitMark::Modified),
            ("plain.txt".to_owned(), GitMark::Staged),
        ]);

        let spelled = spelled_as_listed(marks.clone(), &[decomposed.to_owned(), "plain.txt".to_owned()], false);
        assert_eq!(
            spelled,
            BTreeMap::from([
                (decomposed.to_owned(), GitMark::Modified),
                ("plain.txt".to_owned(), GitMark::Staged)
            ])
        );

        // A name that is listed as it is stays, and a name that is not listed at all stays too.
        assert_eq!(spelled_as_listed(marks.clone(), &[composed.to_owned()], false), marks);
        assert_eq!(spelled_as_listed(marks.clone(), &[], false), marks);
    }

    #[test]
    fn a_name_that_differs_only_in_case_takes_the_listed_spelling_where_case_is_folded() {
        let marks = BTreeMap::from([
            ("README.md".to_owned(), GitMark::Modified),
            ("Src".to_owned(), GitMark::Staged),
            ("exact.txt".to_owned(), GitMark::Untracked),
        ]);
        let listing = [
            "readme.md".to_owned(),
            "src".to_owned(),
            "exact.txt".to_owned(),
            "EXACT.TXT".to_owned(),
        ];

        assert_eq!(
            spelled_as_listed(marks.clone(), &listing, true),
            BTreeMap::from([
                ("exact.txt".to_owned(), GitMark::Untracked),
                ("readme.md".to_owned(), GitMark::Modified),
                ("src".to_owned(), GitMark::Staged),
            ])
        );
        // Where case is told apart (Linux) they are other names.
        assert_eq!(spelled_as_listed(marks.clone(), &listing, false), marks);
    }

    #[test]
    fn a_child_of_the_work_tree_root_is_found_with_an_empty_prefix() {
        let mut marks = Marks::new("");
        marks.note(b"top.txt".as_bstr(), GitMark::Untracked);
        marks.note(b"dir/inner.txt".as_bstr(), GitMark::Modified);

        assert_eq!(
            marked(marks),
            vec![pair("dir", GitMark::Modified), pair("top.txt", GitMark::Untracked)]
        );
    }
}
