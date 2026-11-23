//! The tests of what the parent and the helper say to each other (see [`super`]).

use super::*;

fn request() -> Request {
    let root = if cfg!(windows) { r"C:\work" } else { "/work" };
    let at = |name: &str| Path::new(root).join(name);

    Request {
        folder: at("repo/sub"),
        limits: Limits {
            time: Duration::from_millis(1500),
            entries: 77,
            compare_bytes: 9,
        },
        layout: Layout {
            dot_git: at("repo/.git"),
            git_dir: at("repo/.git"),
            common_dir: at("repo/.git"),
            workdir: at("repo"),
            alternates: vec![at("other/objects/info"), at("other/objects/pack")],
            info_rules: 12,
        },
        grants: vec![
            Grant {
                path: at("repo"),
                device: 64_769,
                inode: 1_234_567,
            },
            Grant {
                path: at("repo/.git"),
                device: 64_769,
                inode: 7,
            },
        ],
    }
}

fn entry(name: &str, mark: GitMark) -> GitEntry {
    GitEntry {
        name: name.to_owned(),
        mark,
    }
}

#[test]
fn a_request_is_read_back_as_it_was_written() {
    let bytes = request().encode().expect("encoded");

    assert_eq!(Request::decode(&bytes).expect("decoded"), request());
}

#[test]
fn a_request_that_is_cut_or_longer_or_not_absolute_or_too_long_in_time_is_refused() {
    let bytes = request().encode().expect("encoded");

    for cut in [0, 1, 8, bytes.len() / 2, bytes.len() - 1] {
        assert!(Request::decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Request::decode(&longer).is_err());

    let mut other = bytes.clone();
    other[0] = 2;
    assert!(Request::decode(&other).is_err());

    let mut slow = request();
    slow.limits.time = MAX_TIME + Duration::from_millis(1);
    assert!(Request::decode(&slow.encode().expect("encoded")).is_err());

    let mut relative = request();
    relative.layout.workdir = PathBuf::from("repo");
    assert!(Request::decode(&relative.encode().expect("encoded")).is_err());
}

#[test]
fn every_answer_is_read_back_and_an_error_is_only_its_kind() {
    let status = GitStatus {
        repo: true,
        marks: vec![entry("a", GitMark::Modified), entry("b", GitMark::Ignored)],
        note: None,
    };

    assert_eq!(
        decode_answer(&encode_answer(&Ok(status.clone())), 100).expect("a status"),
        status
    );
    for error in [
        GitError::TooBig,
        GitError::TimedOut,
        GitError::Cancelled,
        GitError::Repository,
        GitError::Folder(io::ErrorKind::NotFound),
        GitError::Folder(io::ErrorKind::PermissionDenied),
        GitError::Folder(io::ErrorKind::NotADirectory),
        GitError::Folder(io::ErrorKind::Other),
    ] {
        let bytes = encode_answer(&Err(match &error {
            GitError::TooBig => GitError::TooBig,
            GitError::TimedOut => GitError::TimedOut,
            GitError::Cancelled => GitError::Cancelled,
            GitError::Repository => GitError::Repository,
            GitError::Folder(kind) => GitError::Folder(*kind),
        }));

        assert_eq!(bytes.len(), 1);
        assert_eq!(
            decode_answer(&bytes, 100).expect_err("an error").to_string(),
            error.to_string()
        );
    }
}

#[test]
fn an_answer_that_is_not_exactly_an_answer_cannot_be_read() {
    let good = encode_answer(&Ok(GitStatus {
        repo: true,
        marks: vec![entry("a", GitMark::Staged)],
        note: None,
    }));
    let name_at = good.len() - 1;

    let mut longer = good.clone();
    longer.push(0);
    let mut another_mark = good.clone();
    another_mark[6] = 9;
    let mut separator = good.clone();
    separator[name_at] = b'/';
    let mut nul = good.clone();
    nul[name_at] = 0;
    let dots = encode_answer(&Ok(GitStatus {
        repo: true,
        marks: vec![entry("..", GitMark::Staged)],
        note: None,
    }));
    let twice = encode_answer(&Ok(GitStatus {
        repo: true,
        marks: vec![entry("a", GitMark::Staged), entry("a", GitMark::Modified)],
        note: None,
    }));
    let mut many = good.clone();
    many[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
    let marks_without_repo = {
        let mut bytes = good.clone();
        bytes[1] = 0;
        bytes
    };
    let error_with_more = vec![TAG_TOO_BIG, 0];

    for (what, bytes) in [
        ("empty", Vec::new()),
        ("cut", good[..good.len() - 1].to_vec()),
        ("longer", longer),
        ("another mark", another_mark),
        ("a separator in a name", separator),
        ("a NUL in a name", nul),
        ("dots", dots),
        ("the same name twice", twice),
        ("a count the bytes cannot hold", many),
        ("marks of no repository", marks_without_repo),
        ("an unknown tag", vec![99]),
        ("an error with more", error_with_more),
    ] {
        assert!(
            matches!(decode_answer(&bytes, 100), Err(GitError::Repository)),
            "{what}"
        );
    }
}

#[test]
fn an_answer_with_more_marks_than_the_status_may_walk_entries_cannot_be_read() {
    let marks: Vec<GitEntry> = (0..5)
        .map(|number| entry(&format!("n{number}"), GitMark::Modified))
        .collect();
    let bytes = encode_answer(&Ok(GitStatus {
        repo: true,
        marks,
        note: None,
    }));

    assert_eq!(decode_answer(&bytes, 5).expect("at the cap").marks.len(), 5);
    assert!(matches!(decode_answer(&bytes, 4), Err(GitError::Repository)));
    assert!(matches!(decode_answer(&bytes, 0), Err(GitError::Repository)));
}
