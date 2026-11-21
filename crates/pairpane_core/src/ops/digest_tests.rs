//! The hashes and the checksum-file format, without any file.

use pretty_assertions::assert_eq;

use crate::model::HashAlgorithm;

use super::digest::{
    Digester, Listed, algorithm_of_length, algorithm_of_name, extension, format_line, parse_line, stays_inside,
};

fn hash(algorithm: HashAlgorithm, parts: &[&[u8]]) -> String {
    let mut digester = Digester::new(algorithm);
    for part in parts {
        digester.update(part);
    }

    digester.finish()
}

#[test]
fn the_three_hashes_match_their_known_values() {
    let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let md5 = "900150983cd24fb0d6963f7d28e17f72";
    let blake = blake3::hash(b"abc").to_hex().to_string();

    assert_eq!(hash(HashAlgorithm::Sha256, &[b"abc"]), sha);
    assert_eq!(hash(HashAlgorithm::Md5, &[b"abc"]), md5);
    assert_eq!(hash(HashAlgorithm::Blake3, &[b"abc"]), blake);
    assert_eq!(
        blake,
        "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    );
}

#[test]
fn the_hash_of_nothing_and_of_pieces_is_what_it_is() {
    assert_eq!(
        hash(HashAlgorithm::Sha256, &[]),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(hash(HashAlgorithm::Md5, &[]), "d41d8cd98f00b204e9800998ecf8427e");

    for algorithm in [HashAlgorithm::Sha256, HashAlgorithm::Blake3, HashAlgorithm::Md5] {
        assert_eq!(
            hash(algorithm, &[b"a", b"", b"bc"]),
            hash(algorithm, &[b"abc"]),
            "{algorithm:?}"
        );
    }
}

#[test]
fn a_plain_path_is_a_line_of_sha256sum() {
    let line = format_line("ab12", "dir/a.txt");

    assert_eq!(line, "ab12  dir/a.txt\n");
    assert_eq!(
        parse_line("ab12  dir/a.txt"),
        Some(Listed {
            hex: "ab12".to_owned(),
            path: "dir/a.txt".to_owned()
        })
    );
}

#[test]
fn a_path_with_a_backslash_or_a_line_break_is_written_escaped_and_read_back() {
    for path in ["a\\b", "two\nlines", "cr\rhere", "\\leading", "end\\", "a\\nb"] {
        let line = format_line("00ff", path);
        let text = line.strip_suffix('\n').expect("a line feed");

        assert!(text.starts_with('\\'), "{text:?} is marked");
        assert!(!text.contains(['\n', '\r']), "{text:?} is one line");
        assert_eq!(parse_line(text).expect("a line").path, path, "{path:?}");
    }

    assert_eq!(format_line("00ff", "a\\b"), "\\00ff  a\\\\b\n");
    assert_eq!(format_line("00ff", "x\ny"), "\\00ff  x\\ny\n");
}

#[test]
fn lines_of_other_tools_are_read_too() {
    // The binary mark, and upper case hex.
    let binary = parse_line("BA78 *file.bin").expect("a line");
    assert_eq!((binary.hex.as_str(), binary.path.as_str()), ("ba78", "file.bin"));

    // A path that starts with a space keeps it (the mark is the first character after the two spaces).
    assert_eq!(parse_line("ab   spaced").expect("a line").path, " spaced");
}

#[test]
fn what_is_not_a_line_of_a_checksum_file_is_refused() {
    for line in [
        "",
        "ab12",
        "ab12 ",
        "ab12  ",
        "ab12 file",
        "zz12  file",
        "  file",
        "\\ab12  bad\\escape",
        "\\ab12  trailing\\",
    ] {
        assert_eq!(parse_line(line), None, "{line:?}");
    }
}

#[test]
fn the_algorithm_comes_from_the_name_or_the_length() {
    for (name, algorithm) in [
        ("a.sha256", HashAlgorithm::Sha256),
        ("a.SHA256SUM", HashAlgorithm::Sha256),
        ("a.blake3", HashAlgorithm::Blake3),
        ("a.b3", HashAlgorithm::Blake3),
        ("a.md5", HashAlgorithm::Md5),
        ("a.md5sum", HashAlgorithm::Md5),
    ] {
        assert_eq!(algorithm_of_name(name), Some(algorithm), "{name}");
    }
    assert_eq!(algorithm_of_name("SUMS"), None);
    assert_eq!(algorithm_of_name("a.txt"), None);

    assert_eq!(algorithm_of_length(32), Some(HashAlgorithm::Md5));
    assert_eq!(algorithm_of_length(64), Some(HashAlgorithm::Sha256));
    assert_eq!(algorithm_of_length(40), None);

    assert_eq!(extension(HashAlgorithm::Blake3), "blake3");
}

#[test]
fn only_paths_of_plain_names_stay_inside_the_folder() {
    for path in ["a", "a/b", "a b/c.txt"] {
        assert!(stays_inside(path), "{path}");
    }
    for path in ["", "/etc/passwd", "../x", "a/../b", "a//b", "./a", "a/", "a\0b"] {
        assert!(!stays_inside(path), "{path:?}");
    }
}
