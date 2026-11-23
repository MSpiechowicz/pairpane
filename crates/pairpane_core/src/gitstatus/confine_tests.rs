//! The helper's confinement on Linux (see `child`), tried for real: the test starts the test program itself, which prepares a
//! repository, confines itself as the helper does and reports what it can still do. (Confining the process of the test would
//! confine the other tests with it.) The helper's whole life, as a process of the app's executable, is tried in
//! `src-tauri/tests/git_helper.rs`.

use std::fs::{self, File, OpenOptions};
use std::net::UdpSocket;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use super::fixture::pairs;
use super::tests::clean;
use super::{Limits, child, helper, scan};
use crate::view::heic::kernel_probes::read_only_errnos;
use crate::view::testing::run_probe_with;

/// Set, to the folder of the repository, the secret file it must not read and the secret file in the folder of the alternate
/// object store (`|` between), in the test program that plays the helper.
const PROBE: &str = "PAIRPANE_GIT_SANDBOX_PROBE";

/// The name of the test that is the probe when [`PROBE`] is set.
const TEST: &str = "gitstatus::confine_tests::the_helper_reads_its_repository_and_nothing_else";

/// The errno of the failed call, or 0 when it worked.
fn errno<T>(result: std::io::Result<T>) -> i32 {
    result.err().map_or(0, |error| error.raw_os_error().unwrap_or(-1))
}

/// What the confined process reports, one line.
fn probe(root: &Path, secret: &Path, store_secret: &Path) -> String {
    let cancel = AtomicBool::new(false);
    let request = helper::prepare(root, None, Limits::default(), &cancel)
        .expect("prepared")
        .repository()
        .expect("a repository");
    child::apply(&request).expect("the helper confines itself");

    let status = scan::read(&request, &cancel).map(|status| {
        let marks = pairs(&status.marks);

        format!("{}:{marks:?}", status.repo)
    });
    let inside = fs::read(root.join("a.txt")).is_ok_and(|bytes| bytes == b"a");
    let listed = fs::read_dir(root).is_ok_and(|listing| listing.count() > 1);
    let git_dir = fs::read(root.join(".git/HEAD")).is_ok();
    let outside = errno(fs::read(secret));
    // The alternate store's own folder is not the helper's, only its `info`, `pack` and loose folders are.
    let store = store_secret.parent().expect("a folder");
    let store_secret = errno(fs::read(store_secret));
    let store_listing = errno(fs::read_dir(store));
    let store_info = errno(fs::read_dir(store.join("info")));
    let store_pack = errno(fs::read_dir(store.join("pack")));
    let outside_listing = errno(fs::read_dir(secret.parent().expect("a folder")));
    let procfs = errno(File::open("/proc/self/status"));
    let parent = errno(fs::read_dir(root.parent().expect("a parent")));

    let own = File::open(root.join("a.txt")).expect("a file of the repository");
    let [tiocsti, flock, xattr, unknown, fionread] = read_only_errnos(own.as_raw_fd());

    let overwrite = errno(OpenOptions::new().write(true).open(root.join("a.txt")));
    let create = errno(File::create(root.join("created.txt")));
    let truncate = errno(OpenOptions::new().write(true).truncate(true).open(root.join("a.txt")));
    let remove = errno(fs::remove_file(root.join("a.txt")));
    let rename = errno(fs::rename(root.join("a.txt"), root.join("b.txt")));
    let folder = errno(fs::create_dir(root.join("made")));
    let link = errno(std::os::unix::fs::symlink("a.txt", root.join("linked")));
    let socket = errno(UdpSocket::bind("127.0.0.1:0"));
    let exec = errno(Command::new("/bin/true").status());
    let hog = Vec::<u8>::new().try_reserve_exact(3 << 30).is_ok();
    let small = vec![1_u8; 64 << 20].len() == 64 << 20;
    let threads = std::thread::spawn(|| 7).join().is_ok_and(|value| value == 7);
    let unchanged = fs::read(root.join("a.txt")).is_ok_and(|bytes| bytes == b"a") && !root.join("created.txt").exists();

    format!(
        "PROBE status={status:?} inside={inside} listed={listed} git_dir={git_dir} outside={outside} \
         outside_listing={outside_listing} procfs={procfs} parent={parent} store_secret={store_secret} \
         store_listing={store_listing} store_info={store_info} store_pack={store_pack} tiocsti={tiocsti} flock={flock} \
         xattr={xattr} unknown={unknown} fionread={fionread} overwrite={overwrite} create={create} \
         truncate={truncate} remove={remove} rename={rename} folder={folder} link={link} socket={socket} exec={exec} \
         hog={hog} small={small} threads={threads} unchanged={unchanged}"
    )
}

#[test]
fn the_helper_reads_its_repository_and_nothing_else() {
    if let Some(paths) = std::env::var_os(PROBE).filter(|value| value != "1") {
        let paths = paths.to_string_lossy().into_owned();
        let mut paths = paths.split('|');
        let (root, secret, store_secret) = (paths.next(), paths.next(), paths.next());
        println!(
            "{}",
            probe(
                Path::new(root.expect("a repository")),
                Path::new(secret.expect("a secret")),
                Path::new(store_secret.expect("a secret of the store")),
            )
        );

        // The test harness would make calls the filter refuses on its way out.
        std::process::exit(0);
    }

    let repo = clean(&[("a.txt", b"a"), ("sub/b.txt", b"b")]);
    repo.put("new.txt", b"n");
    let secret_folder = crate::gitstatus::testing::tempdir();
    let secret: PathBuf = secret_folder.path().join("secret.txt");
    fs::write(&secret, b"the user's secret").expect("a file");

    // The objects are in an alternate store, which has a file of its own beside what the helper is to read.
    let store = repo.move_objects_to_an_alternate(secret_folder.path(), |store| store.display().to_string());
    let store_secret = store.join("secret.txt");
    fs::write(&store_secret, b"the store's secret").expect("a file");

    let paths = format!(
        "{}|{}|{}",
        repo.root().display(),
        secret.display(),
        store_secret.display()
    );
    let probe = run_probe_with(PROBE, &paths, TEST);
    let report = probe.report.as_str();
    let eperm = libc::EPERM;

    assert!(
        report.contains(" unchanged=true"),
        "the probe ran to its end: {report} {}",
        probe.output
    );

    // What the helper is for: the status of the repository, read in the confinement, with the marks it has outside it.
    assert!(
        report.contains(r#"status=Ok("true:[(\"new.txt\", Untracked)]")"#),
        "the status is read: {report}"
    );
    assert!(report.contains(" inside=true listed=true git_dir=true "), "{report}");
    assert!(report.contains(" small=true"), "memory it needs is there: {report}");
    assert!(report.contains(" threads=true"), "threads can be started: {report}");

    // The filter needs no Landlock: nothing is written, created, removed or renamed, there is no socket and no program, and
    // more memory than the limit is not given.
    for what in [
        "overwrite",
        "create",
        "truncate",
        "remove",
        "rename",
        "folder",
        "link",
        "socket",
        "exec",
    ] {
        assert!(
            report.contains(&format!(" {what}={eperm}")),
            "{what} is refused by the filter: {report}"
        );
    }
    assert!(report.contains(" hog=false"), "the memory is limited: {report}");

    // What a filter that writes nothing refuses beyond the file calls: a terminal's input, a lock, an attribute by a call that
    // is new, a call no list knows. A question about the descriptor is let through.
    for what in ["tiocsti", "flock", "xattr", "unknown"] {
        assert!(
            report.contains(&format!(" {what}={eperm}")),
            "{what} is refused by the filter: {report}"
        );
    }
    assert!(
        !report.contains(&format!(" fionread={eperm}")),
        "FIONREAD is let through: {report}"
    );

    // The alternate store that the status was read from is read through its `info`, `pack` and loose folders, and no other.
    assert!(report.contains(" store_info=0 store_pack=0"), "{report}");

    // Landlock (Linux 5.13 and a kernel that has it switched on): what is outside the folders of the repository cannot be read.
    if report.contains(" outside=0 ") {
        eprintln!("no Landlock on this kernel, the file system was not closed: {report}");

        return;
    }
    for what in [
        "outside",
        "outside_listing",
        "procfs",
        "parent",
        "store_secret",
        "store_listing",
    ] {
        assert!(
            report.contains(&format!(" {what}={}", libc::EACCES)),
            "{what} is out of reach: {report}"
        );
    }
}
