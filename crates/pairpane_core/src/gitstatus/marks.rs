//! The marks of the direct children of the folder, merged from the three results of a status.

use std::collections::BTreeMap;
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

/// How strongly a mark says something, for the child that has several: `conflicted` over `modified` over `staged` over
/// `untracked` over `ignored`.
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

    /// Notes that `mark` was found for `rela_path` (repository-relative, `/` separated). `folder_like` says the entry is a
    /// folder or a submodule, so that its mark is the folder's one.
    ///
    /// The mark goes to the direct child of the folder that `rela_path` is, or is below. A child that is a folder (it has
    /// something below it, or it is a folder itself) is `modified` for any change, and `ignored` only when it is itself
    /// ignored, never because something below it is.
    pub(super) fn note(&mut self, rela_path: &BStr, mark: GitMark, folder_like: bool) {
        let Some(rest) = rela_path.strip_prefix(self.prefix.as_slice()) else {
            self.note_enclosing(rela_path, mark);

            return;
        };
        let (child, deeper) = match rest.find_byte(b'/') {
            Some(at) => (&rest[..at], true),
            None => (rest, false),
        };
        if child.is_empty() || child == b".git" {
            return;
        }

        let mark = if deeper || folder_like {
            match mark {
                GitMark::Ignored if deeper => return,
                GitMark::Ignored => GitMark::Ignored,
                _ => GitMark::Modified,
            }
        } else {
            mark
        };

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

    /// The marks, by name. When the folder as a whole is untracked or ignored, each of its children has that mark too
    /// (a folder child that is untracked is `modified`); `folder` is read for their names, and the read counts in `meter`.
    pub(super) fn finish(mut self, folder: &Path, meter: &mut Meter<'_>) -> io::Result<Vec<GitEntry>> {
        if let Some(whole) = self.whole {
            for entry in fs::read_dir(folder)? {
                if !meter.spend(1) {
                    break;
                }

                let entry = entry?;
                let name = entry.file_name();
                if name == OsStr::new(".git") {
                    continue;
                }

                let mark = match (whole, entry.file_type()?.is_dir()) {
                    (GitMark::Untracked, true) => GitMark::Modified,
                    (mark, _) => mark,
                };
                self.keep(name.to_string_lossy().into_owned(), mark);
            }
        }

        if self.by_name.keys().any(|name| !name.is_ascii()) {
            let listing = fs::read_dir(folder)?
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<io::Result<Vec<_>>>()?;
            meter.spend(listing.len());
            self.by_name = spelled_as_listed(self.by_name, &listing);
        }

        Ok(self
            .by_name
            .into_iter()
            .map(|(name, mark)| GitEntry { name, mark })
            .collect())
    }
}

/// The names of `marks` as the folder spells them. On macOS gix gives the names precomposed (NFC) while a file can be
/// named in the decomposed form (NFD) on disk, and the page matches the names it lists exactly; a name that is not listed
/// as it is takes the spelling of the listed name that is the same text once precomposed.
fn spelled_as_listed(marks: BTreeMap<String, GitMark>, listing: &[String]) -> BTreeMap<String, GitMark> {
    let precomposed = |name: &str| gix::utils::str::precompose(name.into()).into_owned();
    let listed: std::collections::HashMap<String, &String> =
        listing.iter().map(|name| (precomposed(name), name)).collect();

    marks
        .into_iter()
        .map(|(name, mark)| {
            let exact = listing.contains(&name);
            let name = match listed.get(&precomposed(&name)) {
                Some(spelled) if !exact => (*spelled).clone(),
                _ => name,
            };

            (name, mark)
        })
        .collect()
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
    fn a_change_below_a_child_marks_the_child_as_a_folder_with_modified() {
        let mut marks = Marks::new("src");
        marks.note(b"src/a.rs".as_bstr(), GitMark::Staged, false);
        marks.note(b"src/dir/deep/b.rs".as_bstr(), GitMark::Untracked, false);
        marks.note(b"src/other/c.rs".as_bstr(), GitMark::Conflicted, false);
        marks.note(b"elsewhere/d.rs".as_bstr(), GitMark::Modified, false);

        assert_eq!(
            marked(marks),
            vec![
                pair("a.rs", GitMark::Staged),
                pair("dir", GitMark::Modified),
                pair("other", GitMark::Modified),
            ]
        );
    }

    #[test]
    fn an_ignored_entry_below_a_child_does_not_mark_it_but_an_ignored_child_is_ignored() {
        let mut marks = Marks::new("");
        marks.note(b"target/debug/x".as_bstr(), GitMark::Ignored, false);
        marks.note(b"build".as_bstr(), GitMark::Ignored, true);
        marks.note(b"app.log".as_bstr(), GitMark::Ignored, false);

        assert_eq!(
            marked(marks),
            vec![pair("app.log", GitMark::Ignored), pair("build", GitMark::Ignored)]
        );
    }

    #[test]
    fn the_strongest_mark_of_a_child_wins() {
        let mut marks = Marks::new("");
        marks.note(b"a".as_bstr(), GitMark::Staged, false);
        marks.note(b"a".as_bstr(), GitMark::Modified, false);
        marks.note(b"b".as_bstr(), GitMark::Modified, false);
        marks.note(b"b".as_bstr(), GitMark::Conflicted, false);
        marks.note(b"d".as_bstr(), GitMark::Ignored, true);
        marks.note(b"d/x".as_bstr(), GitMark::Staged, false);
        marks.note(b"u".as_bstr(), GitMark::Untracked, true);

        assert_eq!(
            marked(marks),
            vec![
                pair("a", GitMark::Modified),
                pair("b", GitMark::Conflicted),
                pair("d", GitMark::Modified),
                pair("u", GitMark::Modified),
            ]
        );
    }

    #[test]
    fn the_git_folder_and_the_folder_itself_are_never_marked() {
        let mut marks = Marks::new("sub");
        marks.note(b"sub/.git".as_bstr(), GitMark::Untracked, true);
        marks.note(b"sub/.git/config".as_bstr(), GitMark::Modified, false);
        marks.note(b"sub".as_bstr(), GitMark::Modified, true);
        marks.note(b"subway/x".as_bstr(), GitMark::Modified, false);

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

        let spelled = spelled_as_listed(marks.clone(), &[decomposed.to_owned(), "plain.txt".to_owned()]);
        assert_eq!(
            spelled,
            BTreeMap::from([
                (decomposed.to_owned(), GitMark::Modified),
                ("plain.txt".to_owned(), GitMark::Staged)
            ])
        );

        // A name that is listed as it is stays, and a name that is not listed at all stays too.
        assert_eq!(spelled_as_listed(marks.clone(), &[composed.to_owned()]), marks);
        assert_eq!(spelled_as_listed(marks.clone(), &[]), marks);
    }

    #[test]
    fn a_child_of_the_work_tree_root_is_found_with_an_empty_prefix() {
        let mut marks = Marks::new("");
        marks.note(b"top.txt".as_bstr(), GitMark::Untracked, false);
        marks.note(b"dir/inner.txt".as_bstr(), GitMark::Modified, false);

        assert_eq!(
            marked(marks),
            vec![pair("dir", GitMark::Modified), pair("top.txt", GitMark::Untracked)]
        );
    }
}
