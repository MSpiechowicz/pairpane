//! The trust model of [`super`]: repositories whose files try to start a program while their status is read.
//!
//! Each program is a script that creates a marker file; after the status is read the marker must not be there. The scripts
//! are Unix scripts with a `#!` line that names `touch` and the marker (so that no shell is involved), made executable, so
//! they would run if anything started them: a first check starts one by hand to show that, and a repository opened with
//! full trust and no restriction does run a clean filter of the same kind (so that the tests can fail). Windows has no `#!`
//! scripts, and its batch files would need a shell, so these tests are for Unix; the code they test is the same on every
//! system (there is no driver, no hook runner and no fsmonitor code to start).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use super::fixture::Repo;
use super::testing::status;
use super::*;
use crate::model::GitMark::{Modified, Untracked};

/// A place outside the repository with scripts that create `marker`.
struct Trap {
    dir: tempfile::TempDir,
    marker: PathBuf,
    script: PathBuf,
}

impl Trap {
    /// `None` when scripts of this kind cannot run here (there is no `touch` at the usual places, or the temporary folder
    /// does not allow running files), in which case a test has nothing to show.
    fn new() -> Option<Self> {
        let touch = ["/usr/bin/touch", "/bin/touch"]
            .into_iter()
            .find(|candidate| Path::new(candidate).exists())?;
        let dir = tempfile::tempdir().expect("a folder");
        let marker = dir.path().join("marker");
        let script = Self::make(&dir.path().join("trap"), touch, &marker);

        // The control: the script does what it is for when it is started.
        let started = Command::new(&script).status().is_ok_and(|status| status.success());
        if !started || !marker.exists() {
            eprintln!("skipped: scripts cannot be run from the temporary folder here");

            return None;
        }
        fs::remove_file(&marker).expect("the marker is removed");

        Some(Self { dir, marker, script })
    }

    fn make(path: &Path, touch: &str, marker: &Path) -> PathBuf {
        fs::write(path, format!("#!{touch} {}\n", marker.display())).expect("a script");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");

        path.to_path_buf()
    }

    fn script(&self) -> String {
        self.script.display().to_string()
    }

    /// A folder with a hook of each kind git has, and the script as a file for the ones gix would look for first.
    fn hooks(&self) -> PathBuf {
        let hooks = self.dir.path().join("hooks");
        fs::create_dir_all(&hooks).expect("a folder");
        let touch = fs::read_to_string(&self.script).expect("the script");
        let (line, _) = touch.split_once('\n').expect("a line");
        for name in [
            "pre-commit",
            "post-checkout",
            "post-merge",
            "post-index-change",
            "fsmonitor-watchman",
            "reference-transaction",
            "pre-push",
            "post-rewrite",
            "post-commit",
            "pre-rebase",
            "commit-msg",
            "update",
        ] {
            let hook = hooks.join(name);
            fs::write(&hook, format!("{line}\n")).expect("a hook");
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("executable");
        }

        hooks
    }

    /// A config file outside the repository that names the script for everything that can start something.
    fn hostile_config(&self) -> PathBuf {
        let path = self.dir.path().join("outside.cfg");
        fs::write(&path, hostile_sections(&self.script())).expect("a config file");

        path
    }

    fn sprung(&self) -> bool {
        self.marker.exists()
    }
}

/// Config sections that name `program` wherever git starts one.
fn hostile_sections(program: &str) -> String {
    format!(
        "[core]\n\tfsmonitor = {program}\n\tsshCommand = {program}\n\taskPass = {program}\n\teditor = {program}\n\tpager = {program}\n\
         [filter \"x\"]\n\tclean = {program}\n\tsmudge = {program}\n\tprocess = {program}\n\trequired = true\n\
         [diff]\n\texternal = {program}\n\
         [diff \"x\"]\n\ttextconv = {program}\n\tcommand = {program}\n\
         [credential]\n\thelper = {program}\n\
         [gpg]\n\tprogram = {program}\n\
         [merge \"x\"]\n\tdriver = {program}\n\
         [uploadpack]\n\tpackObjectsHook = {program}\n"
    )
}

fn read_ok(folder: &Path) -> GitStatus {
    status(folder, Limits::default(), &AtomicBool::new(false)).expect("a status")
}

/// A repository whose `.gitattributes` send every file through the filter and diff driver `x`.
fn attributed() -> Repo {
    let repo = Repo::new();
    repo.commit(&[
        (".gitattributes", b"* filter=x diff=x\n"),
        ("a.txt", b"a"),
        ("b.txt", b"b"),
    ]);
    repo.put("b.txt", b"b, changed");
    repo.put("c.txt", b"c");

    repo
}

/// What `attributed()` shows.
fn assert_status_is_right(repo: &Repo) {
    let answer = read_ok(repo.root());

    assert!(answer.repo);
    assert_eq!(
        super::fixture::pairs(&answer.marks),
        vec![("b.txt", Modified), ("c.txt", Untracked)]
    );
}

/// With every restriction off and full trust, a repository like this one does start its clean filter on a status: so the
/// scripts and the config of these tests are what they should be, and a marker that stays away is the restriction's doing.
fn assert_full_trust_would_run_it(repo: &Repo, trap: &Trap) {
    assert!(!trap.sprung(), "nothing ran before the control");

    let full = gix::open::Options::default_for_level(gix::sec::Trust::Full).with_enforced_trust(gix::sec::Trust::Full);
    let open = full.open(repo.root()).expect("opened with full trust");
    // The script is no filter that speaks the protocol, so the status itself fails; what counts is that it was started.
    let _ = open
        .status(gix::progress::Discard)
        .expect("a status")
        .index_worktree_submodules(None)
        .into_index_worktree_iter(Vec::new());

    assert!(
        trap.sprung(),
        "the control did not start the clean filter, so these tests would show nothing"
    );
    fs::remove_file(&trap.marker).expect("the marker is removed");
}

#[test]
fn a_clean_filter_and_a_process_filter_of_the_repositorys_config_are_never_started() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    repo.config_append(&hostile_sections(&trap.script()));

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "a filter driver was started");

    assert_full_trust_would_run_it(&repo, &trap);
}

#[test]
fn a_filter_named_in_an_included_config_is_not_started_and_the_include_is_not_followed() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    let outside = trap.hostile_config();
    repo.config_append(&format!(
        "[include]\n\tpath = {path}\n[includeIf \"gitdir:**\"]\n\tpath = {path}\n[includeIf \"onbranch:**\"]\n\tpath = {path}\n",
        path = outside.display()
    ));

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "a program of an included config was started");

    assert_full_trust_would_run_it(&repo, &trap);
}

#[test]
fn an_fsmonitor_program_is_never_started() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    repo.config_append(&format!(
        "[core]\n\tfsmonitor = {script}\n\tuntrackedCache = true\n[index]\n\tthreads = 2\n",
        script = trap.script()
    ));
    // An fsmonitor hook, where git looks for one.
    repo.config_append(&format!("[core]\n\thooksPath = {}\n", trap.hooks().display()));

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "the fsmonitor program was started");
}

#[test]
fn hooks_are_never_started() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    repo.config_append(&format!("[core]\n\thooksPath = {}\n", trap.hooks().display()));
    // And the hooks of the repository's own folder.
    for name in ["pre-commit", "post-checkout", "fsmonitor-watchman", "post-index-change"] {
        let hook = repo.git_dir().join("hooks").join(name);
        fs::create_dir_all(hook.parent().expect("a folder")).expect("a folder");
        fs::copy(&trap.script, &hook).expect("a hook");
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("executable");
    }

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "a hook was started");
}

#[test]
fn text_conversion_and_an_external_diff_are_never_started() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    repo.config_append(&format!(
        "[diff]\n\texternal = {script}\n[diff \"x\"]\n\ttextconv = {script}\n\tcommand = {script}\n\tcachetextconv = false\n",
        script = trap.script()
    ));

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "a diff program was started");
}

#[test]
fn the_programs_of_the_network_and_of_credentials_are_never_started() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    repo.config_append(&format!(
        "[core]\n\tsshCommand = {script}\n\taskPass = {script}\n[credential]\n\thelper = {script}\n\
         [remote \"origin\"]\n\turl = ssh://example.invalid/repo.git\n\tuploadpack = {script}\n\
         [url \"ssh://example.invalid/\"]\n\tinsteadOf = https://example.org/\n\
         [gpg]\n\tprogram = {script}\n\tsshDefaultKeyCommand = {script}\n",
        script = trap.script()
    ));

    assert_status_is_right(&repo);
    assert!(!trap.sprung(), "a program of the network or of credentials was started");
}

#[test]
fn a_worktree_redirect_in_the_config_does_not_move_the_status() {
    let Some(trap) = Trap::new() else { return };
    let repo = attributed();
    let elsewhere = trap.dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("a folder");
    fs::write(elsewhere.join("decoy.txt"), b"d").expect("a file");
    repo.config_append(&format!("[core]\n\tworktree = {}\n", elsewhere.display()));

    assert_status_is_right(&repo);
}

#[test]
fn a_submodule_with_hostile_config_and_a_hostile_gitmodules_is_never_opened() {
    let Some(trap) = Trap::new() else { return };
    let repo = Repo::new();
    repo.commit_with_submodules(&[(".gitattributes", b"* filter=x\n"), ("a.txt", b"a")], &["vendor/lib"]);
    repo.put(
        ".gitmodules",
        format!(
            "[submodule \"vendor/lib\"]\n\tpath = vendor/lib\n\turl = ssh://example.invalid/lib.git\n\tupdate = !{}\n",
            trap.script()
        )
        .as_bytes(),
    );
    repo.config_append(&hostile_sections(&trap.script()));
    let inner = repo.path("vendor/lib");
    fs::create_dir_all(&inner).expect("the submodule folder");
    let sub = gix::init(&inner).expect("the submodule's repository");
    fs::write(
        sub.git_dir().join("config"),
        format!("[core]\n\tbare = false\n{}", hostile_sections(&trap.script())),
    )
    .expect("config");
    fs::write(inner.join(".gitattributes"), b"* filter=x\n").expect("attributes");
    fs::write(inner.join("loose.txt"), b"loose").expect("a file");

    let answer = read_ok(repo.root());
    assert!(answer.repo);
    assert!(
        !trap.sprung(),
        "a program of the superproject or of the submodule was started"
    );
    assert!(answer.marks.iter().all(|entry| entry.name != "vendor"), "{answer:?}");

    // Browsing into the submodule reads its repository, under the same restrictions.
    let inside = read_ok(&inner);
    assert!(inside.repo);
    assert!(
        !trap.sprung(),
        "a program of the submodule was started when it was browsed"
    );
}

#[test]
fn a_git_file_that_points_to_a_hostile_git_folder_starts_nothing() {
    let Some(trap) = Trap::new() else { return };
    // The real repository is somewhere else; the folder has only a `.git` file that points to it.
    let real = attributed();
    real.config_append(&hostile_sections(&trap.script()));
    let folder = trap.dir.path().join("checkout");
    fs::create_dir_all(&folder).expect("a folder");
    fs::write(folder.join(".git"), format!("gitdir: {}\n", real.git_dir().display())).expect("the git file");
    fs::write(folder.join("a.txt"), b"a").expect("a file");
    fs::write(folder.join(".gitattributes"), b"* filter=x diff=x\n").expect("attributes");

    // Whatever it shows (the work tree of a plain `gitdir:` pointer is the folder with the file), nothing is started.
    let _ = status(&folder, Limits::default(), &AtomicBool::new(false));

    assert!(!trap.sprung(), "a program was started through a git file");
}
