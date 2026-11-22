//! Tests of `archive_open`: the listing of a tar and a zip with its crumbs, hidden and read-only entries, what rejects
//! the command (a folder that is not there, a file that is not an archive, a password the archive needs), and a later
//! open of the pane superseding the earlier one.

use pairpane_core::archive::Password;
use pairpane_core::model::ListEvent;
use tauri::async_runtime::block_on;

use super::super::lock;
use super::*;
use crate::commands::archives::fixtures::{
    SEVENZ_PASSWORD, archive_in_folder, collecting, item, path_text, sevenz_with_encrypted_headers, tar_of, zip_of,
};
use crate::error::CommandError;

/// Opens `dir` of `archive` in the left pane. The command answers once the listing is sent, so the events are all there.
fn opened(
    panes: &Panes,
    archives: &Archives,
    archive: &str,
    dir: &str,
    show_hidden: bool,
) -> (CommandResult<()>, Vec<ListEvent>) {
    let (channel, events) = collecting();
    let result = block_on(start(
        panes,
        archives,
        PaneId::Left,
        archive.to_owned(),
        dir.to_owned(),
        show_hidden,
        channel,
    ));
    let seen = events.lock().expect("not poisoned").clone();

    (result, seen)
}

/// The events of a listing that succeeded.
fn listed(panes: &Panes, archives: &Archives, archive: &str, dir: &str, show_hidden: bool) -> Vec<ListEvent> {
    let (result, events) = opened(panes, archives, archive, dir, show_hidden);
    result.expect("listed");
    assert!(matches!(events.last(), Some(ListEvent::Done { .. })), "{events:?}");

    events
}

/// The error an open is rejected with, which no event may repeat.
fn rejected(archives: &Archives, archive: &str, dir: &str) -> CommandError {
    let (result, events) = opened(&Panes::default(), archives, archive, dir, false);

    assert!(events.is_empty(), "a rejected open sends no event: {events:?}");
    result.expect_err("rejected")
}

fn message_of(error: &CommandError) -> String {
    let value = serde_json::to_value(error).expect("serializable");

    value["message"].as_str().expect("a message").to_owned()
}

fn names_of(events: &[ListEvent]) -> Vec<(String, EntryKind)> {
    events
        .iter()
        .flat_map(|event| match event {
            ListEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .map(|entry| (entry.name, entry.kind))
        .collect()
}

fn sample_tar() -> Vec<u8> {
    tar_of(&[
        item("docs/", b""),
        item("docs/guide.txt", b"guide"),
        item("docs/deep/more.txt", b"more"),
        item("readme.txt", b"hello"),
        item(".secret", b"s"),
    ])
}

#[test]
fn the_root_of_a_tar_is_listed_folders_first_with_the_crumbs_of_its_place() {
    let (folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let events = listed(&Panes::default(), &Archives::default(), &archive, "", false);

    let ListEvent::Start {
        path,
        parent,
        crumbs,
        total,
        archive: spot,
        ..
    } = &events[0]
    else {
        panic!("start comes first: {events:?}");
    };
    assert_eq!(*path, path_text(folder.path()));
    assert_eq!(parent.as_deref(), folder.path().parent().map(path_text).as_deref());
    assert_eq!(*total, 2);
    assert_eq!(
        spot,
        &Some(ArchiveSpot {
            archive: archive.clone(),
            dir: String::new()
        })
    );

    let last = crumbs.last().expect("crumbs");
    assert_eq!((last.name.as_str(), last.path.as_str()), ("pack.tar", archive.as_str()));
    assert_eq!(last.inside.as_deref(), Some(""));
    let real = &crumbs[crumbs.len() - 2];
    assert_eq!(real.path, path_text(folder.path()));
    assert_eq!(real.inside, None);

    assert_eq!(
        names_of(&events),
        vec![
            ("docs".to_owned(), EntryKind::Dir),
            ("readme.txt".to_owned(), EntryKind::File)
        ]
    );
    assert!(matches!(
        events.last(),
        Some(ListEvent::Done {
            dirs: 1,
            files: 1,
            bytes: 5,
            ..
        })
    ));
}

#[test]
fn an_inner_folder_is_listed_with_one_crumb_per_folder_and_every_entry_is_read_only() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let events = listed(&Panes::default(), &Archives::default(), &archive, "docs/deep", false);

    let ListEvent::Start {
        crumbs, archive: spot, ..
    } = &events[0]
    else {
        panic!("start comes first");
    };
    let inside: Vec<_> = crumbs
        .iter()
        .filter_map(|crumb| crumb.inside.as_deref().map(|inside| (crumb.name.as_str(), inside)))
        .collect();
    assert_eq!(inside, [("pack.tar", ""), ("docs", "docs"), ("deep", "docs/deep")]);
    assert_eq!(spot.as_ref().map(|spot| spot.dir.as_str()), Some("docs/deep"));

    let entries: Vec<Entry> = events
        .iter()
        .flat_map(|event| match event {
            ListEvent::Batch { entries, .. } => entries.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "more.txt");
    assert_eq!(entries[0].size, 4);
    assert!(entries[0].readonly);
    assert_eq!(entries[0].mode, Some(0o644));
    assert!(entries[0].modified.is_some());
}

#[test]
fn hidden_names_show_only_when_asked_for() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let (panes, archives) = (Panes::default(), Archives::default());

    let without = names_of(&listed(&panes, &archives, &archive, "", false));
    let with = names_of(&listed(&panes, &archives, &archive, "", true));

    assert!(without.iter().all(|(name, _)| name != ".secret"));
    assert!(with.iter().any(|(name, _)| name == ".secret"));
}

#[test]
fn a_stored_zip_lists_its_folders_including_those_only_its_paths_imply() {
    let zip = zip_of(&[item("a/b/c.txt", b"data"), item("top.txt", b"t")]);
    let (_folder, archive) = archive_in_folder("pack.zip", &zip);

    let events = listed(&Panes::default(), &Archives::default(), &archive, "a", false);

    assert_eq!(names_of(&events), vec![("b".to_owned(), EntryKind::Dir)]);
}

#[test]
fn a_folder_that_is_not_in_the_archive_rejects_the_open_as_not_found() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());

    let error = rejected(&Archives::default(), &archive, "nope");

    assert_eq!(error.code(), IpcErrorCode::NotFound);
}

#[test]
fn a_file_that_is_not_an_archive_rejects_the_open_without_its_path() {
    let (folder, archive) = archive_in_folder("notes.tar", b"this is not a tar file at all");

    let error = rejected(&Archives::default(), &archive, "");

    assert_ne!(error.code(), IpcErrorCode::PasswordRequired);
    let message = message_of(&error);
    assert!(!message.contains(&path_text(folder.path())), "{message}");
}

#[test]
fn an_open_that_was_superseded_while_it_failed_answers_ok_so_the_page_shows_no_error() {
    let (_folder, archive) = archive_in_folder("notes.tar", b"this is not a tar file at all");
    let panes = Panes::default();
    let (channel, _) = collecting();
    let open = Open {
        slots: Arc::clone(&panes.slots),
        archives: Archives::default(),
        pane: PaneId::Left,
        generation: 1,
        target: Target::parse(archive).expect("valid"),
        dir: String::new(),
        show_hidden: false,
        cancel: Arc::new(AtomicBool::new(true)),
        on_event: channel,
    };

    assert!(open.run().is_ok());
}

#[test]
fn an_archive_with_encrypted_headers_rejects_the_open_until_the_password_is_given() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let (panes, archives) = (Panes::default(), Archives::default());

    let error = rejected(&archives, &archive, "");
    assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    assert!(!message_of(&error).contains("wrong"));

    let taken = block_on(archives.remember_if_right(archive.clone(), SEVENZ_PASSWORD.to_owned())).expect("checked");
    assert!(taken);

    let events = listed(&panes, &archives, &archive, "", false);
    assert_eq!(names_of(&events), vec![("a.txt".to_owned(), EntryKind::File)]);
}

#[test]
fn a_remembered_password_that_does_not_open_the_archive_is_forgotten_so_the_page_asks_again() {
    let (_folder, archive) = archive_in_folder("locked.7z", &sevenz_with_encrypted_headers());
    let archives = Archives::default();
    archives.remember(PathBuf::from(&archive), Password::new("not-the-password".to_owned()));

    let error = rejected(&archives, &archive, "");

    assert_eq!(error.code(), IpcErrorCode::PasswordRequired);
    assert!(message_of(&error).contains("wrong"), "{}", message_of(&error));
    assert!(
        archives.password(Path::new(&archive)).is_none(),
        "the wrong password is dropped"
    );

    let again = rejected(&archives, &archive, "");
    assert!(!message_of(&again).contains("wrong"), "{}", message_of(&again));
}

#[test]
fn a_later_open_of_the_pane_supersedes_the_earlier_one_and_no_folder_is_watched() {
    let (_folder, archive) = archive_in_folder("pack.tar", &sample_tar());
    let (panes, archives) = (Panes::default(), Archives::default());

    let (first, _) = collecting();
    block_on(start(
        &panes,
        &archives,
        PaneId::Left,
        archive.clone(),
        String::new(),
        false,
        first,
    ))
    .expect("accepted");
    let earlier = lock(&panes.slots)
        .get(&PaneId::Left)
        .map(|slot| Arc::clone(&slot.cancel))
        .expect("slot");

    let events = listed(&panes, &archives, &archive, "docs", false);

    assert!(earlier.load(Ordering::Relaxed), "the earlier open was cancelled");
    assert!(matches!(&events[0], ListEvent::Start { archive: Some(spot), .. } if spot.dir == "docs"));
    let slots = lock(&panes.slots);
    let slot = slots.get(&PaneId::Left).expect("slot");
    assert_eq!(slot.generation, 2);
    assert!(slot.watch.is_none());
}

#[test]
fn a_bad_archive_path_is_refused_before_the_pane_changes() {
    let panes = Panes::default();
    let (channel, _) = collecting();

    let error = block_on(start(
        &panes,
        &Archives::default(),
        PaneId::Left,
        "relative.zip".to_owned(),
        String::new(),
        false,
        channel,
    ))
    .expect_err("refused");

    assert_eq!(error.code(), IpcErrorCode::InvalidInput);
    assert!(lock(&panes.slots).is_empty());
}
