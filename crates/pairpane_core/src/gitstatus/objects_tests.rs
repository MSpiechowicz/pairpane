use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;

use super::*;
use crate::gitstatus::testing;
#[cfg(unix)]
use crate::gitstatus::{
    Limits, fixture::Repo, helper::prepare, helper_request, locate::Found, note::Note, request::Request,
    status_in_process, tests::clean,
};

/// A look with no limit but the entries a repository may have.
pub(super) fn fresh() -> Looking<'static> {
    Looking::new(Gate::never())
}

#[cfg(unix)]
pub(super) fn prepared(root: &Path) -> Found<Request> {
    prepare(root, None, Limits::default(), &AtomicBool::new(false)).expect("prepared")
}

/// Whether the folder of `repo` is read as a repository, and with the note it was refused with, if it was.
#[cfg(unix)]
pub(super) fn read(root: &Path) -> (bool, Option<String>) {
    let answer = status_in_process(root, Limits::default(), &AtomicBool::new(false)).expect("a status");

    (answer.repo, answer.note)
}

/// An object folder of its own, as a repository that is read has: `info`, `pack` and a folder of loose objects with one object.
pub(super) fn store(folder: &Path) -> PathBuf {
    let objects = folder.join("objects");
    for name in ["info", "pack", "ab"] {
        fs::create_dir_all(objects.join(name)).expect("folders");
    }
    fs::write(objects.join("ab/cdef"), b"x").expect("an object");

    objects
}

/// The first loose object of the repository `repo`.
#[cfg(unix)]
fn a_loose_object(repo: &Repo) -> PathBuf {
    let objects = repo.git_dir().join("objects");

    for folder in fs::read_dir(&objects).expect("objects") {
        let folder = folder.expect("an entry").path();
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        if name.len() != 2 {
            continue;
        }
        if let Some(object) = fs::read_dir(&folder).expect("a folder").next() {
            return object.expect("an entry").path();
        }
    }

    panic!("a repository with a commit has a loose object");
}

#[test]
fn a_pointer_of_the_list_of_alternates_that_is_not_local_is_refused() {
    let folder = crate::gitstatus::testing::tempdir();
    let common = folder.path().join("repo.git");
    fs::create_dir_all(common.join("objects/info")).expect("folders");
    let alternates = common.join("objects/info/alternates");

    fs::write(&alternates, "# comment\n\n../../other/objects\n").expect("a list");
    assert_eq!(object_folders(&common, Gate::never()).expect("local").len(), 2);

    for line in [
        r"\\host\share\objects",
        "//host/share/objects",
        r"\\?\UNC\host\share\objects",
    ] {
        fs::write(&alternates, format!("../../fine/objects\n{line}\n")).expect("a list");

        assert_eq!(object_folders(&common, Gate::never()), Err(Refusal::Odd), "{line}");
    }
}

#[test]
fn a_chain_of_alternates_is_followed_and_a_loop_ends() {
    let folder = crate::gitstatus::testing::tempdir();
    let (one, two) = (folder.path().join("one/objects"), folder.path().join("two/objects"));
    for objects in [&one, &two] {
        fs::create_dir_all(objects.join("info")).expect("folders");
    }
    fs::write(one.join("info/alternates"), format!("{}\n", two.display())).expect("a list");
    fs::write(two.join("info/alternates"), format!("{}\n", one.display())).expect("a list");

    // Each step names the other: each store is looked at once, and the loop ends there.
    let stores = object_folders(&folder.path().join("one"), Gate::never()).expect("local");

    assert_eq!(stores.len(), 2);
    assert_eq!(stores[0], one);
    assert_eq!(stores[1], two);
}

#[cfg(unix)]
#[test]
fn a_link_among_the_loose_objects_is_no_repository_and_nothing_is_started_for_it() {
    use std::os::unix::fs::symlink;

    let repo = clean(&[("a.txt", b"a")]);
    repo.put("a.txt", b"changed");
    assert!(read(repo.root()).0, "the control: the repository is read");

    // The object that a link leads to, a copy of it elsewhere (on Windows it could be `\\host\share\x`).
    let object = a_loose_object(&repo);
    let outside = testing::tempdir();
    fs::copy(&object, outside.path().join("copy")).expect("a copy");
    fs::remove_file(&object).expect("removed");
    symlink(outside.path().join("copy"), &object).expect("a link");

    assert_eq!(prepared(repo.root()), Found::Refused(Note::Elsewhere));
    assert_eq!(helper_request(repo.root(), Limits::default()).expect("asked"), None);
    assert_eq!(read(repo.root()), (false, Some(Note::Elsewhere.text().to_owned())));
}

#[cfg(unix)]
#[test]
fn a_pipe_or_a_folder_among_the_loose_objects_is_no_repository() {
    let repo = clean(&[("a.txt", b"a")]);
    let folder = a_loose_object(&repo).parent().expect("a folder").to_path_buf();

    fs::create_dir(folder.join("subfolder")).expect("a folder");
    assert_eq!(
        read(repo.root()),
        (false, Some(Note::Elsewhere.text().to_owned())),
        "a folder"
    );
    fs::remove_dir(folder.join("subfolder")).expect("removed");

    crate::view::testing::fifo(&folder, "pipe");
    assert_eq!(
        read(repo.root()),
        (false, Some(Note::Elsewhere.text().to_owned())),
        "a pipe"
    );
}

#[test]
fn the_folders_of_an_object_store_that_may_be_read_are_the_ones_that_are_there() {
    let folder = testing::tempdir();
    let objects = store(folder.path());

    let mut granted = object_folder(&objects, &mut fresh()).expect("a store");
    granted.sort();

    assert_eq!(
        granted,
        [objects.join("ab"), objects.join("info"), objects.join("pack")]
    );
}

#[test]
fn an_incremental_multi_pack_index_and_a_chain_of_commit_graphs_are_plain_files_in_real_folders() {
    let folder = testing::tempdir();
    let objects = store(folder.path());
    let midx = objects.join("pack/multi-pack-index.d");
    let graphs = objects.join("info/commit-graphs");
    for folder in [&midx, &graphs] {
        fs::create_dir(folder).expect("a folder");
    }
    fs::write(midx.join("multi-pack-index-1.midx"), b"m").expect("a file");
    fs::write(midx.join("multi-pack-index.d-chain"), b"c").expect("a file");
    fs::write(graphs.join("commit-graph-chain"), b"c").expect("a file");
    fs::write(graphs.join("graph-1.graph"), b"g").expect("a file");
    fs::write(objects.join("info/commit-graph"), b"g").expect("a file");

    assert!(object_folder(&objects, &mut fresh()).is_ok());

    // Nothing else is a folder of `pack`, and nothing in them is more than a file.
    fs::create_dir(midx.join("deeper")).expect("a folder");
    assert_eq!(object_folder(&objects, &mut fresh()), Err(Refusal::Odd));
    fs::remove_dir_all(&midx).expect("removed");
    fs::create_dir(objects.join("pack/other")).expect("a folder");
    assert_eq!(object_folder(&objects, &mut fresh()), Err(Refusal::Odd));
}

#[cfg(unix)]
#[test]
fn a_link_in_the_folders_of_a_multi_pack_index_or_a_commit_graph_is_refused() {
    use std::os::unix::fs::symlink;

    let folder = testing::tempdir();
    let objects = store(folder.path());
    let outside = folder.path().join("outside");
    fs::create_dir(&outside).expect("a folder");
    fs::write(outside.join("file"), b"x").expect("a file");

    for (place, name) in [
        ("pack/multi-pack-index.d", "multi-pack-index.d"),
        ("info/commit-graphs", "commit-graphs"),
    ] {
        // The folder itself is a link.
        let folder_here = objects.join(place);
        symlink(&outside, &folder_here).expect("a link");
        assert_eq!(
            object_folder(&objects, &mut fresh()),
            Err(Refusal::Odd),
            "{name} as a link"
        );
        fs::remove_file(&folder_here).expect("removed");

        // A link inside it.
        fs::create_dir(&folder_here).expect("a folder");
        symlink(outside.join("file"), folder_here.join("entry")).expect("a link");
        assert_eq!(
            object_folder(&objects, &mut fresh()),
            Err(Refusal::Odd),
            "a link in {name}"
        );
        fs::remove_dir_all(&folder_here).expect("removed");
    }
    assert!(object_folder(&objects, &mut fresh()).is_ok());
}

#[cfg(unix)]
#[test]
fn an_alternate_that_is_the_root_or_a_file_system_of_the_kernel_is_refused() {
    let repo = clean(&[("a.txt", b"a")]);
    let list = repo.git_dir().join("objects/info");
    fs::create_dir_all(&list).expect("a folder");

    let mut lines = vec!["/"];
    if cfg!(target_os = "linux") {
        lines.push("/proc");
        lines.push("/sys");
    }
    for line in lines {
        fs::write(list.join("alternates"), format!("{line}\n")).expect("a list");

        assert_eq!(prepared(repo.root()), Found::Refused(Note::Broad), "{line}");
        assert_eq!(
            read(repo.root()),
            (false, Some(Note::Broad.text().to_owned())),
            "{line}"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_alternate_that_is_a_broad_folder_of_the_disk_is_never_granted_as_a_whole() {
    let repo = clean(&[("a.txt", b"a")]);
    let list = repo.git_dir().join("objects/info");
    fs::create_dir_all(&list).expect("a folder");
    // The home folder and the folder above it, which hold everything of the user's.
    let mut lines = vec!["/tmp".to_owned(), "/usr".to_owned()];
    if let Some(home) = crate::gitstatus::layout::home() {
        lines.push(home.display().to_string());
        lines.extend(home.parent().map(|parent| parent.display().to_string()));
    }

    for line in lines {
        fs::write(list.join("alternates"), format!("{line}\n")).expect("a list");

        // Either the repository is refused, or the helper is given the folders of the store and not the store itself.
        match prepared(repo.root()) {
            Found::Refused(_) => {}
            Found::Repository(request) => {
                let whole = Path::new(&line);
                assert!(
                    !request.grants.iter().any(|grant| grant.path == whole),
                    "{line}: {:?}",
                    request.grants
                );
                assert!(request.layout.alternates.iter().all(|folder| folder != whole), "{line}");
            }
            Found::Nowhere => panic!("{line}: a repository"),
        }
    }
}

#[cfg(unix)]
#[test]
fn an_alternate_gives_the_helper_its_info_pack_and_loose_folders_and_not_its_own_folder() {
    let repo = clean(&[("a.txt", b"a")]);
    repo.put("a.txt", b"changed");
    let folder = testing::tempdir();
    let store = repo.move_objects_to_an_alternate(folder.path(), |store| store.display().to_string());
    fs::write(store.join("secret.txt"), b"not for the helper").expect("a file");

    let Found::Repository(request) = prepared(repo.root()) else {
        panic!("a repository");
    };

    assert!(request.layout.alternates.contains(&store.join("pack")));
    assert!(request.layout.alternates.contains(&store.join("info")));
    assert!(
        request
            .layout
            .alternates
            .iter()
            .all(|folder| folder.starts_with(&store) && folder != &store)
    );
    assert!(
        request
            .layout
            .folders()
            .iter()
            .all(|folder| folder != &store && folder != store.parent().expect("a parent"))
    );
    assert!(request.grants.iter().all(|grant| grant.path != store));
    // What the alternate holds is what the status reads: the objects are there and only there.
    assert_eq!(read(repo.root()), (true, None));
}

#[cfg(unix)]
#[test]
fn an_alternate_named_through_a_link_is_the_folder_the_link_leads_to() {
    use std::os::unix::fs::symlink;

    let repo = clean(&[("a.txt", b"a")]);
    repo.put("a.txt", b"changed");
    let folder = testing::tempdir();
    // What `git clone --shared` writes when the source was named through a linked folder.
    symlink(folder.path().join("store"), folder.path().join("via")).expect("a link");
    let store = repo.move_objects_to_an_alternate(folder.path(), |store| {
        let name = store
            .strip_prefix(folder.path().join("store"))
            .expect("below the store");

        folder.path().join("via").join(name).display().to_string()
    });

    let Found::Repository(request) = prepared(repo.root()) else {
        panic!("a repository");
    };

    assert!(
        request.layout.alternates.contains(&store.join("pack")),
        "{:?}",
        request.layout.alternates
    );
    assert_eq!(read(repo.root()), (true, None));

    // A `..` straight after a link is what gix reads one way and the system the other.
    fs::write(
        repo.git_dir().join("objects/info/alternates"),
        format!("{}\n", folder.path().join("via/../store/objects").display()),
    )
    .expect("a list");
    assert_eq!(prepared(repo.root()), Found::Refused(Note::Elsewhere));
    // A `..` after a real folder of the link's target is fine.
    fs::create_dir(folder.path().join("store/real")).expect("a folder");
    fs::write(
        repo.git_dir().join("objects/info/alternates"),
        format!("{}\n", folder.path().join("via/real/../objects").display()),
    )
    .expect("a list");
    assert!(matches!(prepared(repo.root()), Found::Repository(_)));
}

#[cfg(unix)]
#[test]
fn an_alternate_that_leads_through_a_link_to_a_broad_folder_is_refused() {
    use std::os::unix::fs::symlink;

    let repo = clean(&[("a.txt", b"a")]);
    let folder = testing::tempdir();
    symlink("/", folder.path().join("root")).expect("a link");
    fs::create_dir_all(repo.git_dir().join("objects/info")).expect("a folder");
    fs::write(
        repo.git_dir().join("objects/info/alternates"),
        format!("{}\n", folder.path().join("root").display()),
    )
    .expect("a list");

    assert_eq!(prepared(repo.root()), Found::Refused(Note::Broad));
}
