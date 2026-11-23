//! The path of the folder below the work tree, as the index spells it: `/` separated, and without a slash at its end.

use std::path::Path;

/// The path of `folder` below `workdir` (both real paths), `/` separated and without a slash at its end (empty for the work
/// tree's root), or `None` when `folder` is not inside it, is inside the `.git` folder, or has a name that is not valid text.
///
/// Where the system's paths are not spelled as the repository's are, they are made to match: on macOS a name can be
/// decomposed (NFD) on disk while the index and gix have it precomposed (NFC), so both are precomposed before they are
/// compared and the result is precomposed; on Windows the comparison of the work tree's path ignores case, after the
/// separators and a `\\?\` prefix are made alike.
pub(super) fn prefix_of(folder: &Path, workdir: &Path) -> Option<String> {
    let workdir = gix::path::realpath(workdir).ok()?;
    let rest = match (folder.to_str(), workdir.to_str()) {
        (Some(folder), Some(workdir)) => rest_after(folder, workdir, Spelling::of_this_system())?,
        // A name above the work tree that is not text does not matter; one below it does (it is not a name of the index).
        _ if cfg!(not(windows)) => folder
            .strip_prefix(&workdir)
            .ok()?
            .to_str()?
            .split('/')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => return None,
    };

    if rest
        .first()
        .is_some_and(|name| is_git_folder(name, cfg!(any(windows, target_os = "macos"))))
    {
        return None;
    }

    Some(rest.join("/"))
}

/// How the names of this system differ from the repository's.
#[derive(Clone, Copy)]
pub(super) struct Spelling {
    /// Names are compared without case, and `\` separates (Windows).
    windows: bool,
    /// Names are compared precomposed (macOS), and the result is precomposed.
    precompose: bool,
}

impl Spelling {
    pub(super) fn of_this_system() -> Self {
        Self {
            windows: cfg!(windows),
            precompose: cfg!(target_os = "macos"),
        }
    }
}

/// The parts of `path`: for Windows, with `\` and `/` alike and no `\\?\` prefix, and no empty part.
fn parts(path: &str, windows: bool) -> Vec<&str> {
    if windows {
        let path = path
            .strip_prefix(r"\\?\")
            .or_else(|| path.strip_prefix("//?/"))
            .unwrap_or(path);

        path.split(['\\', '/']).filter(|part| !part.is_empty()).collect()
    } else {
        path.split('/').filter(|part| !part.is_empty()).collect()
    }
}

/// The text a name is compared as.
fn same_text(name: &str, spelling: Spelling) -> String {
    let name = if spelling.precompose {
        gix::utils::str::precompose(name.into()).into_owned()
    } else {
        name.to_owned()
    };

    if spelling.windows { name.to_lowercase() } else { name }
}

/// The parts of `folder` below `workdir`, or `None` when it is not inside it. A part is spelled as the index has it.
pub(super) fn rest_after(folder: &str, workdir: &str, spelling: Spelling) -> Option<Vec<String>> {
    let (folder, workdir) = (parts(folder, spelling.windows), parts(workdir, spelling.windows));
    if folder.len() < workdir.len() {
        return None;
    }

    let inside = folder
        .iter()
        .zip(&workdir)
        .all(|(name, parent)| same_text(name, spelling) == same_text(parent, spelling));
    if !inside {
        return None;
    }

    Some(
        folder[workdir.len()..]
            .iter()
            .map(|name| {
                if spelling.precompose {
                    gix::utils::str::precompose((*name).into()).into_owned()
                } else {
                    (*name).to_owned()
                }
            })
            .collect(),
    )
}

/// Whether `name` is the `.git` folder (where names are compared without case, `.GIT` too).
fn is_git_folder(name: &str, fold_case: bool) -> bool {
    if fold_case {
        name.eq_ignore_ascii_case(".git")
    } else {
        name == ".git"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Spelling = Spelling {
        windows: false,
        precompose: false,
    };
    const WINDOWS: Spelling = Spelling {
        windows: true,
        precompose: false,
    };
    const MAC: Spelling = Spelling {
        windows: false,
        precompose: true,
    };

    #[test]
    fn a_folder_below_the_work_tree_gives_the_parts_between() {
        assert_eq!(
            rest_after("/w/p/sub/deep", "/w/p", PLAIN),
            Some(vec!["sub".to_owned(), "deep".to_owned()])
        );
        assert_eq!(rest_after("/w/p", "/w/p", PLAIN), Some(vec![]));
        assert_eq!(rest_after("/w/other", "/w/p", PLAIN), None);
        assert_eq!(rest_after("/w", "/w/p", PLAIN), None);
        // A name that only starts like the work tree's is not inside it.
        assert_eq!(rest_after("/w/project/x", "/w/p", PLAIN), None);
        // Case tells names apart here.
        assert_eq!(rest_after("/W/P/sub", "/w/p", PLAIN), None);
    }

    #[test]
    fn on_windows_the_work_tree_is_found_without_case_and_with_any_separator_and_prefix() {
        let sub = Some(vec!["Sub".to_owned(), "deep".to_owned()]);

        assert_eq!(rest_after(r"C:\Work\Proj\Sub\deep", r"c:\work\proj", WINDOWS), sub);
        assert_eq!(rest_after(r"\\?\C:\Work\Proj\Sub\deep", r"C:\work\PROJ", WINDOWS), sub);
        assert_eq!(rest_after(r"C:\Work\Proj\Sub\deep", r"\\?\c:\Work\Proj", WINDOWS), sub);
        assert_eq!(rest_after("C:/Work/Proj/Sub/deep", r"C:\Work\Proj", WINDOWS), sub);
        assert_eq!(
            rest_after(r"\\?\UNC\host\share\p\Sub\deep", r"\\?\UNC\HOST\share\P", WINDOWS),
            sub
        );
        assert_eq!(rest_after(r"D:\Work\Proj\Sub", r"C:\Work\Proj", WINDOWS), None);
        assert_eq!(rest_after(r"C:\Work\Projects", r"C:\Work\Proj", WINDOWS), None);
        // The names of the folder keep the spelling it has.
        assert_eq!(
            rest_after(r"C:\WORK\PROJ\Sub", r"C:\work\proj", WINDOWS),
            Some(vec!["Sub".to_owned()])
        );
    }

    #[test]
    fn on_macos_a_decomposed_name_is_found_below_a_work_tree_that_is_precomposed_and_the_other_way() {
        let composed = "caf\u{e9}";
        let decomposed = "cafe\u{301}";

        assert_eq!(
            rest_after(
                &format!("/w/{decomposed}/sub/{decomposed}"),
                &format!("/w/{composed}"),
                MAC
            ),
            Some(vec!["sub".to_owned(), composed.to_owned()])
        );
        assert_eq!(
            rest_after(&format!("/w/{composed}/{composed}"), &format!("/w/{decomposed}"), MAC),
            Some(vec![composed.to_owned()])
        );
        // Without the precomposing the two spellings are two names.
        assert_eq!(
            rest_after(&format!("/w/{decomposed}/x"), &format!("/w/{composed}"), PLAIN),
            None
        );
    }

    #[test]
    fn the_git_folder_is_found_by_name_and_with_any_case_where_case_is_folded() {
        assert!(is_git_folder(".git", false));
        assert!(!is_git_folder(".GIT", false));
        assert!(is_git_folder(".GIT", true));
        assert!(!is_git_folder(".github", true));
    }

    #[test]
    fn the_folder_of_a_real_work_tree_is_its_path_below_it() {
        let work = crate::gitstatus::testing::tempdir();
        let root = gix::path::realpath(work.path()).expect("a real path");
        let sub = root.join("sub").join("deep");
        std::fs::create_dir_all(&sub).expect("folders");

        assert_eq!(prefix_of(&sub, &root), Some("sub/deep".to_owned()));
        assert_eq!(prefix_of(&root, &root), Some(String::new()));
        assert_eq!(prefix_of(&root.join(".git").join("refs"), &root), None);
        assert_eq!(prefix_of(&root.join(".git"), &root), None);
    }
}
