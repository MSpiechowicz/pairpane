use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::super::handle::{Facts, NO_TRASH, open_folder};
use super::super::testing::{Env, is_root, me, set_mode};
use super::super::top::{Top, top_of};
use super::{Context, Kind, Reopened, choose, data_home, reopen, volume};

fn os(text: &str) -> Option<OsString> {
    Some(OsString::from(text))
}

/// A folder standing for the top of a drive: the way the Trash of a drive is found, without a real mount.
struct Drive {
    temp: TempDir,
    top: Top,
    device: u64,
}

impl Drive {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir");
        let fd = open_folder(temp.path()).expect("folder");
        let device = Facts::of_handle(&fd).expect("facts").device;

        Self {
            temp,
            top: Top { fd, steps: 0 },
            device,
        }
    }

    fn path(&self) -> PathBuf {
        fs::canonicalize(self.temp.path()).expect("canonical")
    }

    fn trash_of(&self, uid: u32, device: u64) -> io::Result<super::Chosen> {
        volume(uid, &self.path(), &self.top, device)
    }

    fn choose(&self) -> io::Result<super::Chosen> {
        self.trash_of(me(), self.device)
    }

    fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.path().join(name)
    }

    /// Makes `.Trash` (sticky, open to everyone) and returns the place of the folder of the user in it.
    fn shared_trash(&self) -> PathBuf {
        fs::create_dir(self.join(".Trash")).expect("folder");
        set_mode(&self.join(".Trash"), 0o1777);

        self.join(".Trash").join(me().to_string())
    }
}

fn is_no_trash(result: &io::Result<super::Chosen>) -> bool {
    matches!(result, Err(error) if error.kind() == io::ErrorKind::Other && error.to_string() == NO_TRASH)
}

/// The folder `work` of `env`: its resolved path, its handle and its device.
fn work_of(env: &Env) -> (PathBuf, std::os::fd::OwnedFd, u64) {
    let work = fs::canonicalize(env.work()).expect("canonical");
    let folder = open_folder(&work).expect("folder");
    let device = Facts::of_handle(&folder).expect("facts").device;

    (work, folder, device)
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("metadata").mode() & 0o7777
}

#[test]
fn the_home_trash_is_in_the_data_folder_by_the_specification() {
    assert_eq!(data_home(os("/data"), os("/home/me")), Some(PathBuf::from("/data")));
    assert_eq!(
        data_home(None, os("/home/me")),
        Some(PathBuf::from("/home/me/.local/share"))
    );
    // An empty or relative variable counts as not set.
    assert_eq!(
        data_home(os(""), os("/home/me")),
        Some(PathBuf::from("/home/me/.local/share"))
    );
    assert_eq!(
        data_home(os("data"), os("/home/me")),
        Some(PathBuf::from("/home/me/.local/share"))
    );
    assert_eq!(data_home(None, os("home")), None);
    assert_eq!(data_home(None, None), None);
    assert_eq!(data_home(os(""), os("")), None);
}

#[test]
fn a_drive_with_no_trash_gets_one_of_the_user_that_is_private() {
    let drive = Drive::new();

    let chosen = drive.choose().expect("a Trash");

    assert_eq!(chosen.kind, Kind::Own);
    assert_eq!(chosen.base, drive.path());
    let own = drive.join(format!(".Trash-{}", me()));
    assert_eq!(mode_of(&own), 0o700);
    assert_eq!(mode_of(&own.join("files")), 0o700);
    assert_eq!(mode_of(&own.join("info")), 0o700);
}

#[test]
fn a_sticky_trash_with_a_folder_of_the_user_is_the_one_used() {
    let drive = Drive::new();
    let user = drive.shared_trash();
    fs::create_dir(&user).expect("folder");
    set_mode(&user, 0o700);

    let chosen = drive.choose().expect("a Trash");

    assert_eq!(chosen.kind, Kind::Shared);
    assert!(user.join("files").is_dir());
    assert!(!drive.join(format!(".Trash-{}", me())).exists());
}

#[test]
fn a_trash_that_is_not_sticky_or_has_no_folder_of_the_user_is_passed_over() {
    for sticky in [false, true] {
        let drive = Drive::new();
        let user = drive.shared_trash();
        set_mode(&drive.join(".Trash"), if sticky { 0o1777 } else { 0o755 });
        if !sticky {
            fs::create_dir(&user).expect("folder");
        }

        let chosen = drive.choose().expect("a Trash");

        assert_eq!(chosen.kind, Kind::Own, "sticky: {sticky}");
    }
}

#[test]
fn a_trash_that_is_a_link_or_a_file_is_not_the_shared_one() {
    let drive = Drive::new();
    fs::create_dir(drive.join("real")).expect("folder");
    set_mode(&drive.join("real"), 0o1777);
    fs::create_dir(drive.join("real").join(me().to_string())).expect("folder");
    symlink(drive.join("real"), drive.join(".Trash")).expect("link");

    let chosen = drive.choose().expect("a Trash");

    assert_eq!(chosen.kind, Kind::Own);
}

#[test]
fn a_user_folder_in_the_trash_that_cannot_be_trusted_is_passed_over_for_the_own_trash() {
    // A link, and a folder that others can write to: each planted where the folder of the user should be.
    for planted in ["link", "open"] {
        let drive = Drive::new();
        let place = drive.shared_trash();
        fs::create_dir(drive.join("elsewhere")).expect("folder");
        if planted == "link" {
            symlink(drive.join("elsewhere"), &place).expect("link");
        } else {
            fs::create_dir(&place).expect("folder");
            set_mode(&place, 0o777);
        }

        let chosen = drive.choose().expect(planted);

        assert_eq!(chosen.kind, Kind::Own, "{planted}");
        assert!(drive.join(format!(".Trash-{}/files", me())).is_dir(), "{planted}");
        assert!(
            !drive.join("elsewhere/files").exists(),
            "{planted}: nothing is made behind the link"
        );
        assert!(
            !place.join("files").exists(),
            "{planted}: nothing is made in the planted folder"
        );

        // The undo opens the Trash that was used, and not the planted folder.
        let again = reopen(me(), chosen.kind, &chosen.base, drive.device).expect("reopened");
        assert!(matches!(again, Reopened::Open(_)), "{planted}");
    }
}

#[test]
fn an_undo_of_the_shared_trash_whose_user_folder_became_unusable_is_refused_not_passed_over() {
    let drive = Drive::new();
    let place = drive.shared_trash();
    fs::create_dir(&place).expect("folder");
    set_mode(&place, 0o700);
    let chosen = drive.choose().expect("a Trash");
    assert_eq!(chosen.kind, Kind::Shared);

    set_mode(&place, 0o777);

    assert!(reopen(me(), Kind::Shared, &chosen.base, drive.device).is_err());
}

#[test]
fn a_top_folder_that_others_can_rename_in_is_refused_unless_sticky() {
    let drive = Drive::new();

    for (mode, usable) in [(0o777, false), (0o775, false), (0o755, true), (0o1777, true)] {
        set_mode(&drive.path(), mode);

        let chosen = drive.choose();

        assert_eq!(chosen.is_ok(), usable, "{mode:o}");
        if !usable {
            assert!(is_no_trash(&chosen), "{mode:o}");
            assert!(
                !drive.join(format!(".Trash-{}", me())).exists(),
                "{mode:o}: nothing is made"
            );
        }
    }
}

#[test]
fn a_top_folder_of_another_user_is_refused() {
    let drive = Drive::new();

    // The folder is the tester's, so to a user with another number it is a stranger's.
    assert!(is_no_trash(&drive.trash_of(me() + 1, drive.device)));
}

#[test]
fn a_trash_of_the_user_that_is_a_link_or_open_to_others_is_refused() {
    let drive = Drive::new();
    let own = drive.join(format!(".Trash-{}", me()));
    fs::create_dir(drive.join("elsewhere")).expect("folder");
    symlink(drive.join("elsewhere"), &own).expect("link");

    assert!(is_no_trash(&drive.choose()), "a link");
    assert!(!drive.join("elsewhere/files").exists());

    fs::remove_file(&own).expect("remove");
    fs::create_dir(&own).expect("folder");
    set_mode(&own, 0o777);
    assert!(is_no_trash(&drive.choose()), "open to others");

    // The modes of a FAT drive mounted by the user are accepted: nobody else can write there.
    set_mode(&own, 0o755);
    assert!(drive.choose().is_ok(), "closed to others");
}

#[test]
fn a_drive_the_user_cannot_write_to_has_no_trash() {
    if is_root() {
        return;
    }
    let drive = Drive::new();
    set_mode(&drive.path(), 0o555);

    let chosen = drive.choose();

    set_mode(&drive.path(), 0o755);
    assert!(is_no_trash(&chosen));
}

#[test]
fn a_trash_on_another_disk_than_the_item_is_refused() {
    let drive = Drive::new();

    assert!(is_no_trash(&drive.trash_of(me(), drive.device + 1)));
}

#[test]
fn a_top_folder_that_the_path_does_not_lead_to_is_refused() {
    let drive = Drive::new();
    let elsewhere = TempDir::new().expect("temp dir");
    let top = Top {
        fd: open_folder(elsewhere.path()).expect("folder"),
        steps: 0,
    };

    let chosen = volume(me(), &drive.path(), &top, drive.device);

    assert!(is_no_trash(&chosen));
}

#[test]
fn an_item_on_the_drive_of_the_home_goes_to_the_home_trash_made_with_its_parents() {
    let env = Env::new();
    let (work, folder, device) = work_of(&env);

    let chosen = choose(&env.context, &work, &folder, device).expect("a Trash");

    assert_eq!(chosen.kind, Kind::Home);
    assert_eq!(Some(chosen.base.clone()), env.context.data_home);
    assert_eq!(mode_of(&env.temp.path().join("share")), 0o700);
    assert_eq!(mode_of(&env.trash()), 0o700);
    assert_eq!(mode_of(&env.trash().join("files")), 0o700);
}

#[test]
fn a_home_with_no_data_folder_named_has_no_trash() {
    let env = Env::new();
    let (work, folder, _) = work_of(&env);
    let context = Context {
        uid: me(),
        data_home: None,
    };

    assert!(is_no_trash(&choose(&context, &work, &folder, 0)));
}

/// A folder on a drive that is not the one of the home, on the real system: the temporary folder is often a drive of its
/// own (tmpfs), the home another.
struct Away {
    /// Kept so that the folder stays.
    _env: Env,
    home: PathBuf,
    work: PathBuf,
    folder: std::os::fd::OwnedFd,
    top_path: PathBuf,
}

impl Away {
    /// `None` where the temporary folder is on the drive of the home, or there is no home.
    fn find() -> Option<Self> {
        let home = std::env::home_dir()?;
        let env = Env::new();
        let work = fs::canonicalize(env.work()).expect("canonical");
        let folder = open_folder(&work).expect("folder");
        let folder_top = top_of(&folder).expect("top");
        let home_top = top_of(&open_folder(&home).expect("home")).expect("top");

        if Facts::of_handle(&folder_top.fd)
            .expect("facts")
            .is_same_place(&Facts::of_handle(&home_top.fd).expect("facts"))
        {
            return None;
        }
        let top_path = work.ancestors().nth(folder_top.steps).expect("top path").to_path_buf();

        Some(Self {
            _env: env,
            home,
            work,
            folder,
            top_path,
        })
    }

    /// Chooses the Trash of the folder with `context`: the Trash of its own drive, or the refusal of a drive the user
    /// cannot write to; never the home Trash. What the choice made is taken away again.
    fn check(&self, context: &Context) {
        let device = Facts::of_handle(&self.folder).expect("facts").device;
        let own = self.top_path.join(format!(".Trash-{}", me()));
        let existed = own.exists();

        let chosen = choose(context, &self.work, &self.folder, device);

        match chosen {
            Ok(chosen) => {
                assert!(matches!(chosen.kind, Kind::Own | Kind::Shared));
                assert_eq!(chosen.base, self.top_path);
                if !existed {
                    let _ = fs::remove_dir_all(own);
                }
            }
            // A drive the user cannot write to has no Trash, and the item is not sent to the home Trash instead.
            Err(error) => assert_eq!(error.to_string(), NO_TRASH),
        }
    }
}

#[test]
fn an_item_on_another_drive_than_the_home_goes_to_the_trash_of_its_drive() {
    let Some(away) = Away::find() else { return };
    // The data folder of the home is in the home, so the item is not on its drive.
    let context = Context {
        uid: me(),
        data_home: Some(away.home.join(".local/share")),
    };

    away.check(&context);
}

#[test]
fn the_trash_is_opened_again_from_what_was_recorded_and_nothing_is_made() {
    let drive = Drive::new();
    let chosen = drive.choose().expect("a Trash");

    let again = reopen(me(), chosen.kind, &chosen.base, drive.device).expect("reopened");
    assert!(matches!(again, Reopened::Open(_)));

    fs::remove_dir_all(drive.join(format!(".Trash-{}", me()))).expect("emptied");
    let gone = reopen(me(), Kind::Own, &chosen.base, drive.device).expect("reopened");

    assert!(matches!(gone, Reopened::Gone));
    assert!(!drive.join(format!(".Trash-{}", me())).exists(), "nothing is made");
}

#[test]
fn a_trash_that_is_open_to_others_or_on_another_disk_is_not_opened_again() {
    let drive = Drive::new();
    let chosen = drive.choose().expect("a Trash");

    assert!(reopen(me(), chosen.kind, &chosen.base, drive.device + 1).is_err());

    set_mode(&drive.join(format!(".Trash-{}/info", me())), 0o777);
    assert!(reopen(me(), chosen.kind, &chosen.base, drive.device).is_err());
    set_mode(&drive.join(format!(".Trash-{}/info", me())), 0o700);

    set_mode(&drive.path(), 0o777);
    assert!(reopen(me(), chosen.kind, &chosen.base, drive.device).is_err());
}

/// A folder of the user's, with the data folder of a home below a folder that cannot be searched.
fn unreachable_home(env: &Env) -> Context {
    let locked = env.temp.path().join("locked");
    fs::create_dir_all(locked.join("share")).expect("folders");
    set_mode(&locked, 0o000);

    Context {
        uid: me(),
        data_home: Some(locked.join("share")),
    }
}

#[test]
fn a_home_that_cannot_be_reached_refuses_the_items_of_its_drive_and_makes_nothing() {
    if is_root() {
        return;
    }
    let env = Env::new();
    let context = unreachable_home(&env);
    let (work, folder, device) = work_of(&env);

    let chosen = choose(&context, &work, &folder, device);

    set_mode(&env.temp.path().join("locked"), 0o755);
    assert!(is_no_trash(&chosen));
    assert!(!env.temp.path().join("locked/share/Trash").exists());
}

#[test]
fn a_home_that_cannot_be_reached_costs_only_the_items_of_its_own_drive() {
    if is_root() {
        return;
    }
    let Some(away) = Away::find() else { return };
    // The data folder is in the home, below a folder that cannot be searched.
    let locked = tempfile::Builder::new()
        .prefix("pairpane-locked-home-")
        .tempdir_in(&away.home)
        .expect("temp dir");
    fs::create_dir(locked.path().join("share")).expect("folder");
    set_mode(locked.path(), 0o000);
    let context = Context {
        uid: me(),
        data_home: Some(locked.path().join("share")),
    };

    away.check(&context);

    set_mode(locked.path(), 0o755);
}
