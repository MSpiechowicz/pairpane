//! Comparing the names of folders as the file system does. A Linux file system is case-sensitive, and a name is compared as it
//! is written, but the default file system of macOS (APFS, and HFS+ before it) is case-insensitive and does not tell two
//! spellings of the same text apart: `/NET`, `/Net`, `/Networ\u{212A}` (the Kelvin sign, a different character from `K`, which
//! is its capital to the file system) and `/Network` are all one folder there. So a check that names a folder (an automounted
//! one, the user's home) must fold what it compares on macOS, or it is a check of how a path was written and not of the folder
//! it leads to.

use std::path::{Component, Path};

use unicode_normalization::UnicodeNormalization;

/// The name `name` as a case-insensitive, normalisation-insensitive file system reads it: precomposed (NFC), each character in
/// upper and then in lower case (so that the Kelvin sign and `K` meet at `k`, and the long s and `s` at `s`), precomposed again
/// (a case change can leave a name that is not), and all of that again until nothing changes (one round is not always enough:
/// the capital sharp s `ẞ` becomes `ß` in the first and `ss` in the second, which is where `ss` lands at once).
pub(super) fn fold(name: &str) -> String {
    let mut folded = fold_once(name);

    // Two rounds reach a fixed point for every character Unicode has today; the bound only guards against one that would not.
    for _ in 0..4 {
        let again = fold_once(&folded);
        if again == folded {
            break;
        }
        folded = again;
    }

    folded
}

fn fold_once(name: &str) -> String {
    name.nfc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .nfc()
        .collect()
}

/// Whether the names `a` and `b` are the same folder's name: exactly, and on macOS (`macos`) after folding as well.
pub(super) fn same_name(a: &str, b: &str, macos: bool) -> bool {
    a == b || (macos && fold(a) == fold(b))
}

/// Whether `path` is `base` or below it, component by component: exactly, and on macOS (`macos`) with the names of the
/// components folded.
pub(super) fn starts_with(path: &Path, base: &Path, macos: bool) -> bool {
    if !macos {
        return path.starts_with(base);
    }

    let mut path = path.components();

    base.components()
        .all(|wanted| path.next().is_some_and(|found| same_component(found, wanted)))
}

/// Whether the two paths are one place by their names (see [`starts_with`]).
pub(super) fn equals(a: &Path, b: &Path, macos: bool) -> bool {
    starts_with(a, b, macos) && a.components().count() == b.components().count()
}

fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    match (a, b) {
        (Component::Normal(a), Component::Normal(b)) => {
            a == b || same_name(&a.to_string_lossy(), &b.to_string_lossy(), true)
        }
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_folded_by_case_and_by_the_way_unicode_spells_it() {
        for (written, folded) in [
            ("Network", "network"),
            ("NET", "net"),
            // The Kelvin sign is its own character, and the capital of nothing in ASCII; it is a `K` to the file system.
            ("Networ\u{212A}", "network"),
            ("Networ\u{4B}", "network"),
            // The long s folds to `s`.
            ("\u{17F}ystem", "system"),
            ("SYSTEM", "system"),
            // Decomposed and precomposed spellings of one name meet.
            ("e\u{301}", "\u{e9}"),
            ("\u{c9}", "\u{e9}"),
            ("A\u{30A}", "\u{e5}"),
        ] {
            assert_eq!(fold(written), folded, "{written:?}");
        }
        assert_ne!(fold("net2"), "net");
        assert_ne!(fold("networ\u{3BA}"), "network", "a Greek kappa is no k");
    }

    #[test]
    fn names_are_the_same_exactly_and_on_macos_after_folding() {
        assert!(same_name("Net", "Net", false));
        assert!(!same_name("Net", "net", false));
        assert!(same_name("Net", "net", true));
        assert!(same_name("Networ\u{212A}", "Network", true));
        assert!(!same_name("Networ\u{212A}", "Network", false));
        assert!(!same_name("home", "homes", true));
    }

    #[test]
    fn a_path_is_below_another_by_whole_names_and_on_macos_by_folded_ones() {
        let home = Path::new("/Users/Ann");

        for below in ["/Users/Ann", "/Users/Ann/work", "/Users/Ann/work/repo"] {
            assert!(starts_with(Path::new(below), home, false), "{below}");
            assert!(starts_with(Path::new(below), home, true), "{below}");
        }
        for below in ["/users/ann", "/USERS/ANN/work"] {
            assert!(starts_with(Path::new(below), home, true), "{below}");
            assert!(!starts_with(Path::new(below), home, false), "{below}");
        }
        // Whole names only: `Ann2` is not below `Ann`, on any system.
        assert!(!starts_with(Path::new("/Users/Ann2"), home, true));
        assert!(!starts_with(Path::new("/Users"), home, true));
        assert!(equals(Path::new("/USERS/ann"), home, true));
        assert!(!equals(Path::new("/USERS/ann/x"), home, true));
        assert!(!equals(Path::new("/USERS/ann"), home, false));
    }

    #[test]
    fn the_capital_sharp_s_folds_where_ss_does() {
        assert_eq!(fold("STRAU\u{1E9E}"), fold("strauss"));
        assert_eq!(fold("Stra\u{DF}e"), fold("STRASSE"));
        assert_eq!(fold(&fold("\u{1E9E}")), fold("\u{1E9E}"));
    }
}
