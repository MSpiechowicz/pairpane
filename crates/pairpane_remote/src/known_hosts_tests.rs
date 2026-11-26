use std::fs;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use pretty_assertions::assert_eq;
use sha1::Sha1;

use crate::known_hosts::{Files, KnownHosts, Name, Verdict, append, valid_host};

const ED: &str = "ssh-ed25519";
const KEY: &[u8] = b"the key of the server";
const OTHER: &[u8] = b"some other key";

fn line(hosts: &str, algorithm: &str, key: &[u8]) -> String {
    format!("{hosts} {algorithm} {}", STANDARD.encode(key))
}

fn hashed(name: &str) -> String {
    let salt = b"twenty bytes of salt";
    let mut mac = Hmac::<Sha1>::new_from_slice(salt).expect("any key length");

    mac.update(name.as_bytes());

    format!(
        "|1|{}|{}",
        STANDARD.encode(salt),
        STANDARD.encode(mac.finalize().into_bytes())
    )
}

fn name(host: &str, port: u16) -> Name {
    Name::of(host, port).expect("a valid host")
}

fn check(file: &str, host: &str, port: u16, key: &[u8]) -> Verdict {
    KnownHosts::parse(file).check(&name(host, port), ED, key)
}

#[test]
fn a_plain_line_trusts_its_key() {
    let file = line("files.example", ED, KEY);

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "FILES.example", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "other.example", 22, KEY), Verdict::Unknown);
}

#[test]
fn a_port_is_a_part_of_the_name() {
    let file = format!(
        "{}\n{}",
        line("[files.example]:2222", ED, KEY),
        line("files.example", ED, OTHER)
    );

    assert_eq!(check(&file, "files.example", 2222, KEY), Verdict::Known);
    assert_eq!(check(&file, "files.example", 22, OTHER), Verdict::Known);
    assert_eq!(check(&file, "files.example", 2200, KEY), Verdict::Unknown);
}

#[test]
fn lists_wildcards_and_exclusions_select_hosts() {
    let file = line("*.example.com,!secret.example.com,10.0.0.?", ED, KEY);

    assert_eq!(check(&file, "files.example.com", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "10.0.0.7", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "secret.example.com", 22, KEY), Verdict::Unknown);
    assert_eq!(check(&file, "example.org", 22, KEY), Verdict::Unknown);
}

#[test]
fn a_hashed_name_is_found_by_its_hmac() {
    let file = format!(
        "{}\n{}",
        line(&hashed("files.example"), ED, KEY),
        line(&hashed("[files.example]:2222"), ED, OTHER)
    );

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "files.example", 2222, OTHER), Verdict::Known);
    assert_eq!(check(&file, "other.example", 22, KEY), Verdict::Unknown);
}

#[test]
fn another_key_of_the_same_kind_is_a_change_with_its_line_number() {
    let file = format!("# comment\n\n{}", line("files.example", ED, KEY));

    assert_eq!(check(&file, "files.example", 22, OTHER), Verdict::Changed { line: 3 });
}

#[test]
fn a_key_of_another_kind_is_unknown_not_changed() {
    let file = line("files.example", "ecdsa-sha2-nistp256", OTHER);

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Unknown);
}

#[test]
fn a_matching_line_wins_over_a_stale_one() {
    let file = format!(
        "{}\n{}",
        line("files.example", ED, OTHER),
        line("files.example", ED, KEY)
    );

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Known);
}

#[test]
fn a_revoked_key_is_refused_even_where_it_is_listed_as_trusted() {
    let file = format!("{}\n@revoked {}", line("files.example", ED, KEY), line("*", ED, KEY));

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Changed { line: 2 });
    assert_eq!(check(&file, "files.example", 22, OTHER), Verdict::Changed { line: 1 });
}

#[test]
fn broken_lines_trust_nothing() {
    let file = "files.example ssh-ed25519 not-base64!!\nfiles.example\n|1|bad|bad ssh-ed25519 AAAA";

    assert_eq!(check(file, "files.example", 22, KEY), Verdict::Unknown);
}

#[test]
fn a_host_vouched_for_in_a_form_that_cannot_be_checked_is_unusable() {
    let ca = format!("@cert-authority {}", line("*.example", ED, KEY));
    let dss = line("files.example", "ssh-dss", KEY);
    let security_key = line("files.example", "sk-ssh-ed25519@openssh.com", KEY);
    let hashed_ca = format!("@cert-authority {}", line(&hashed("files.example"), ED, KEY));

    for file in [ca, dss, security_key, hashed_ca] {
        assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Unusable, "{file}");
        assert_eq!(check(&file, "other.test", 22, KEY), Verdict::Unknown, "{file}");
    }
}

#[test]
fn a_usable_line_still_settles_the_key_next_to_one_that_cannot_be_used() {
    let unusable = line("files.example", "ssh-dss", KEY);
    let file = format!("{unusable}\n{}", line("files.example", ED, KEY));

    assert_eq!(check(&file, "files.example", 22, KEY), Verdict::Known);
    assert_eq!(check(&file, "files.example", 22, OTHER), Verdict::Changed { line: 2 });
}

#[test]
fn algorithms_lists_the_kinds_held_for_a_host_in_order() {
    let file = [
        line("files.example", "ssh-ed25519", KEY),
        line("other.example", "ssh-rsa", KEY),
        line("files.example,x.example", "ecdsa-sha2-nistp256", KEY),
        line("files.example", "ssh-ed25519", OTHER),
    ]
    .join("\n");

    assert_eq!(
        KnownHosts::parse(&file).algorithms(&name("files.example", 22)),
        ["ssh-ed25519", "ecdsa-sha2-nistp256"]
    );
    assert!(
        KnownHosts::parse(&file)
            .algorithms(&name("none.example", 22))
            .is_empty()
    );
}

#[test]
fn a_missing_file_holds_nothing_and_an_unreadable_one_is_an_error() {
    let dir = tempfile::tempdir().expect("temp dir");

    let missing = KnownHosts::load(&dir.path().join("known_hosts")).expect("a missing file holds nothing");

    assert_eq!(missing.check(&name("files.example", 22), ED, KEY), Verdict::Unknown);
    // A folder is where the file should be: reading it fails, and that must not read as "nothing is known".
    assert!(KnownHosts::load(dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn a_pipe_where_the_file_should_be_is_an_error_and_is_not_waited_for() {
    let dir = tempfile::tempdir().expect("temp dir");
    let pipe = dir.path().join("known_hosts");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .expect("mkfifo runs");

    assert!(made.success());

    let started = std::time::Instant::now();

    assert!(KnownHosts::load(&pipe).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn append_adds_a_plain_line_and_keeps_what_was_there() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(".ssh").join("known_hosts");

    append(&path, &name("Files.Example", 22), ED, KEY, false).expect("appended");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        format!("{}\n", line("files.example", ED, KEY))
    );

    // A file without a final newline gets one before the new line; the old text stays as it was.
    fs::write(&path, "# mine\nold.example ssh-rsa AAAA").expect("written");
    append(&path, &name("files.example", 2222), ED, OTHER, false).expect("appended");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        format!(
            "# mine\nold.example ssh-rsa AAAA\n{}\n",
            line("[files.example]:2222", ED, OTHER)
        )
    );
    assert_eq!(
        check(&fs::read_to_string(&path).expect("read"), "files.example", 2222, OTHER),
        Verdict::Known
    );
}

#[test]
fn append_can_write_a_hashed_name_with_a_random_salt_that_is_found_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("known_hosts");

    append(&path, &name("files.example", 2222), ED, KEY, true).expect("appended");
    append(&path, &name("files.example", 2222), ED, KEY, true).expect("appended");

    let text = fs::read_to_string(&path).expect("read");
    let salts: Vec<&str> = text
        .lines()
        .map(|line| line.split('|').nth(2).expect("a salt"))
        .collect();

    assert_eq!(text.lines().count(), 2);
    assert!(text.lines().all(|line| line.starts_with("|1|")), "{text}");
    assert!(!text.contains("files.example"), "{text}");
    assert_ne!(salts[0], salts[1], "the salt is new for every line");
    assert_eq!(STANDARD.decode(salts[0]).expect("base64").len(), 20);

    // The name that was hashed is `[host]:port`, as it is for a plain line; any other name is not it.
    assert_eq!(check(&text, "files.example", 2222, KEY), Verdict::Known);
    assert_eq!(check(&text, "files.example", 22, KEY), Verdict::Unknown);
    assert_eq!(check(&text, "files.example", 2222, OTHER), Verdict::Changed { line: 1 });
}

#[test]
fn a_name_is_made_only_of_what_cannot_corrupt_the_file() {
    for host in ["", "a b", "a\nb ssh-rsa AAAA", "*.example", "a,b", "|1|x", "[a]"] {
        assert_eq!(Name::of(host, 22), None, "{host:?}");
        assert_eq!(Name::alias(host), None, "{host:?}");
    }

    assert!(valid_host("files.example") && valid_host("::1") && valid_host("fe80::1%eth0"));
    assert_eq!(name("Files.Example", 22).as_str(), "files.example");
    assert_eq!(name("files.example", 2200).as_str(), "[files.example]:2200");
    // An alias stands for the server as it is, whatever the port.
    assert_eq!(Name::alias("Vault.Example").expect("valid").as_str(), "vault.example");
}

#[test]
fn the_first_user_file_has_line_numbers_and_the_others_are_another_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (first, second, global) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("g"));
    let (mine, theirs) = (line("files.example", ED, OTHER), line("files.example", ED, KEY));

    fs::write(&first, format!("# one\n{mine}\n")).expect("written");
    fs::write(&second, format!("{mine}\n")).expect("written");
    fs::write(&global, format!("{theirs}\n")).expect("written");

    let files = |user: &[&std::path::Path], global: &[&std::path::Path]| Files {
        user: user.iter().map(|path| path.to_path_buf()).collect(),
        global: global.iter().map(|path| path.to_path_buf()).collect(),
        ..Files::default()
    };
    let verdict = |files: &Files| {
        KnownHosts::load_all(files)
            .expect("loaded")
            .check(&name("files.example", 22), ED, KEY)
    };

    // The global file knows the key; the user's first file holds another one for the host, which counts first.
    assert_eq!(verdict(&files(&[&first], &[&global])), Verdict::Known);
    assert_eq!(verdict(&files(&[&first], &[])), Verdict::Changed { line: 2 });
    assert_eq!(verdict(&files(&[&second, &first], &[])), Verdict::Changed { line: 1 });
    assert_eq!(verdict(&files(&[&global, &second], &[])), Verdict::Known);
    assert_eq!(
        verdict(&files(&[&dir.path().join("none")], &[&second])),
        Verdict::ChangedSystem
    );
    assert_eq!(files(&[&first, &second], &[]).writable(), Some(first.as_path()));
    assert_eq!(files(&[], &[&global]).writable(), None);
}

#[cfg(unix)]
#[test]
fn append_creates_a_private_folder_and_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let folder = dir.path().join(".ssh");
    let path = folder.join("known_hosts");

    append(&path, &name("files.example", 22), ED, KEY, false).expect("appended");

    let mode = |path: &std::path::Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;

    assert_eq!(mode(&folder), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn a_joined_path_is_written_only_where_its_folder_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let folder = dir.path().join("my home");
    let path = folder.join("kh");
    let files = |joined: bool| Files {
        user: vec![path.clone()],
        joined: if joined { vec![path.clone()] } else { Vec::new() },
        global: Vec::new(),
    };

    // No folder: a guess is not written, a path as it is gets its folder made.
    assert_eq!(files(true).writable(), None);
    assert_eq!(files(false).writable(), Some(path.as_path()));

    fs::create_dir(&folder).expect("a folder");
    assert_eq!(files(true).writable(), Some(path.as_path()));
}
