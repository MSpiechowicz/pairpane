//! Links in a server's listing: what they point to, how many are looked at, and a connection lost while looking.

use pretty_assertions::assert_eq;

use super::*;

#[test]
fn a_link_shows_as_what_it_points_to() {
    let mut server = Server {
        entries: vec![
            item("to-dir", RemoteKind::Link),
            item("to-file", RemoteKind::Link),
            item("dangling", RemoteKind::Link),
        ],
        ..Server::default()
    };
    server
        .targets
        .insert("/srv/to-dir".to_owned(), meta(RemoteKind::Dir, 4096, 0o755));
    server
        .targets
        .insert("/srv/to-file".to_owned(), meta(RemoteKind::File, 99, 0o400));
    let server = Arc::new(server);
    let shown = show_in(server.clone(), "/srv", false).expect("listed");

    let seen: Vec<_> = shown
        .entries()
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind, entry.size, entry.readonly))
        .collect();
    assert_eq!(
        seen,
        [
            ("to-dir", EntryKind::Dir, 0, false),
            ("dangling", EntryKind::Other, 0, false),
            ("to-file", EntryKind::File, 99, true),
        ]
    );
    assert_eq!(server.stats.load(Ordering::Relaxed), 3);
}

#[test]
fn only_the_first_links_of_a_listing_are_looked_at() {
    let links = MAX_LINK_STATS + 5;
    let mut server = Server::default();
    for number in 0..links {
        let name = format!("link-{number:04}");
        server
            .targets
            .insert(format!("/srv/{name}"), meta(RemoteKind::Dir, 0, 0o755));
        server.entries.push(item(&name, RemoteKind::Link));
    }
    let server = Arc::new(server);
    let shown = show_in(server.clone(), "/srv", false).expect("listed");

    let dirs = shown
        .entries()
        .iter()
        .filter(|entry| entry.kind == EntryKind::Dir)
        .count();
    let others = shown
        .entries()
        .iter()
        .filter(|entry| entry.kind == EntryKind::Other)
        .count();
    assert_eq!((dirs, others), (MAX_LINK_STATS, 5));
    assert_eq!(server.stats.load(Ordering::Relaxed), MAX_LINK_STATS);
    assert_eq!(shown.start().3, u32::try_from(links).expect("small"));
}

#[test]
fn hidden_links_that_are_left_out_cost_no_request() {
    let server = Arc::new(Server {
        entries: vec![item(".hidden-link", RemoteKind::Link)],
        ..Server::default()
    });
    show_in(server.clone(), "/srv", false).expect("listed");

    assert_eq!(server.stats.load(Ordering::Relaxed), 0);
}

#[test]
fn a_link_whose_target_cannot_be_told_is_other_but_a_lost_connection_ends_the_listing() {
    let broken = |kind| Server {
        entries: vec![item("link", RemoteKind::Link)],
        stat_broken: Some(kind),
        ..Server::default()
    };

    let denied = show_in(broken(io::ErrorKind::PermissionDenied), "/srv", false).expect("listed");
    let timed_out = show_in(broken(io::ErrorKind::TimedOut), "/srv", false).expect_err("ended");
    let lost = show_in(broken(io::ErrorKind::ConnectionAborted), "/srv", false).expect_err("ended");

    assert_eq!(denied.entries()[0].kind, EntryKind::Other);
    assert_eq!(timed_out.kind(), io::ErrorKind::TimedOut);
    assert_eq!(lost.kind(), io::ErrorKind::ConnectionAborted);
}
