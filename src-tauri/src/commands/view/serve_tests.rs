//! Tests of the media scheme and its tokens: what a token serves, the answers to ranges, and when a token stops working.
//! They run `serve` on the test's own thread, with the time given, so nothing waits and no app is needed.

use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};

use pairpane_core::model::{IpcErrorCode, ViewKind};
use pairpane_core::view::MAX_READ;
use tauri::http::{HeaderMap, Method, StatusCode, header};

use super::media::{IDLE, MAX_TOKENS};
use super::serve::{Reply, serve};
use super::tests::{code, fill, folder_with, open};
use super::*;

const SIZE: usize = 1000;

/// A FLAC-looking file of `SIZE` bytes whose byte at `i` is `i % 251` after the 4 signature bytes.
fn flac() -> Vec<u8> {
    let mut bytes = b"fLaC".to_vec();
    bytes.extend((4..SIZE).map(|at| (at % 251) as u8));

    bytes
}

fn token_of(viewer: &Viewer, path: &str, name: &str, now: Instant) -> (u32, String) {
    let session = open(viewer, path, name).session;

    (session, viewer.mint(session, now).expect("token"))
}

/// A viewer with a session on `a.flac` and its token, minted at `now`; the folder is kept until the rig is dropped.
struct Rig {
    viewer: Viewer,
    now: Instant,
    token: String,
    _folder: tempfile::TempDir,
}

fn rig() -> Rig {
    let (folder, path) = folder_with(&[("a.flac", &flac())]);
    let viewer = Viewer::default();
    let now = Instant::now();
    let (_, token) = token_of(&viewer, &path, "a.flac", now);

    Rig {
        viewer,
        now,
        token,
        _folder: folder,
    }
}

fn get(viewer: &Viewer, now: Instant, token: &str, range: Option<&str>) -> Reply {
    serve(viewer, now, &Method::GET, &format!("/{token}"), range)
}

fn header_of(headers: &HeaderMap, name: header::HeaderName) -> &str {
    headers.get(name).expect("header").to_str().expect("text")
}

fn assert_guarded(reply: &Reply) {
    let headers = reply.headers();

    assert_eq!(header_of(headers, header::CACHE_CONTROL), "no-store");
    assert_eq!(header_of(headers, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(
        header_of(headers, header::CONTENT_SECURITY_POLICY),
        "default-src 'none'; sandbox"
    );
}

fn assert_empty(reply: &Reply, status: StatusCode) {
    assert_eq!(reply.status(), status);
    assert!(reply.body().is_empty());
    assert_eq!(header_of(reply.headers(), header::CONTENT_LENGTH), "0");
    assert_guarded(reply);
}

#[test]
fn a_token_is_32_lowercase_hex_characters_and_every_one_is_new() {
    let (_folder, path) = folder_with(&[("a.flac", &flac())]);
    let viewer = Viewer::default();
    let now = Instant::now();

    let tokens: Vec<String> = (0..4).map(|_| token_of(&viewer, &path, "a.flac", now).1).collect();

    for token in &tokens {
        assert_eq!(token.len(), 32);
        assert!(
            token.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "{token}"
        );
    }
    let mut unique = tokens.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), tokens.len());
}

#[test]
fn only_audio_and_video_get_a_token() {
    let (_folder, path) = folder_with(&[("a.txt", b"hello"), ("a.flac", &flac())]);
    let viewer = Viewer::default();
    let text = open(&viewer, &path, "a.txt");
    let audio = open(&viewer, &path, "a.flac");

    assert_eq!(
        code(viewer.mint(text.session, Instant::now())),
        IpcErrorCode::InvalidInput
    );
    assert!(matches!(audio.kind, ViewKind::Audio { .. }));
    assert!(viewer.mint(audio.session, Instant::now()).is_ok());
}

#[test]
fn a_whole_file_of_at_most_the_cap_is_200_with_every_header() {
    let Rig { viewer, now, token, .. } = rig();

    let reply = get(&viewer, now, &token, None);

    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(reply.body(), &flac());
    assert_eq!(header_of(reply.headers(), header::CONTENT_TYPE), "audio/flac");
    assert_eq!(header_of(reply.headers(), header::CONTENT_LENGTH), "1000");
    assert_eq!(header_of(reply.headers(), header::ACCEPT_RANGES), "bytes");
    assert!(reply.headers().get(header::CONTENT_RANGE).is_none());
    assert_guarded(&reply);
}

#[test]
fn a_range_is_206_with_its_bytes_and_content_range() {
    let Rig { viewer, now, token, .. } = rig();

    let cases = [
        ("bytes=10-19", 10, 19),
        ("bytes=990-", 990, 999),
        ("bytes=-5", 995, 999),
        ("bytes=0-0", 0, 0),
        ("bytes=900-5000", 900, 999),
    ];
    for (header, first, last) in cases {
        let reply = get(&viewer, now, &token, Some(header));

        assert_eq!(reply.status(), StatusCode::PARTIAL_CONTENT, "{header}");
        assert_eq!(reply.body(), &flac()[first..=last], "{header}");
        assert_eq!(
            header_of(reply.headers(), header::CONTENT_RANGE),
            format!("bytes {first}-{last}/1000"),
            "{header}"
        );
        assert_eq!(
            header_of(reply.headers(), header::CONTENT_LENGTH),
            (last - first + 1).to_string()
        );
        assert_eq!(header_of(reply.headers(), header::CONTENT_TYPE), "audio/flac");
        assert_eq!(header_of(reply.headers(), header::ACCEPT_RANGES), "bytes");
        assert_guarded(&reply);
    }

    // A header that is not one range of bytes is ignored: the whole file.
    for ignored in ["bytes=0-1,5-6", "items=0-1", "bytes=x-y", "bytes=5-1"] {
        assert_eq!(
            get(&viewer, now, &token, Some(ignored)).status(),
            StatusCode::OK,
            "{ignored}"
        );
    }
}

#[test]
fn a_range_past_the_end_is_416_with_the_size() {
    let Rig { viewer, now, token, .. } = rig();

    for header in ["bytes=1000-", "bytes=5000-6000"] {
        let reply = get(&viewer, now, &token, Some(header));

        assert_empty(&reply, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(header_of(reply.headers(), header::CONTENT_RANGE), "bytes */1000");
    }
}

#[test]
fn a_file_over_the_cap_is_answered_in_parts_of_at_most_the_cap() {
    let total = MAX_READ * 2 + 10;
    let folder = tempfile::tempdir().expect("folder");
    let mut file = File::create(folder.path().join("big.ogg")).expect("file");
    // Sparse zeros after the signature that makes it Ogg.
    file.write_all(b"OggS").expect("signature");
    file.set_len(total as u64).expect("sparse");
    drop(file);
    let path = folder.path().to_string_lossy().into_owned();
    let viewer = Viewer::default();
    let now = Instant::now();
    let (_, token) = token_of(&viewer, &path, "big.ogg", now);
    let last = MAX_READ - 1;

    // No range: the first part, though the page asked for no part.
    let first = get(&viewer, now, &token, None);
    assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(first.body().len(), MAX_READ);
    assert_eq!(
        header_of(first.headers(), header::CONTENT_RANGE),
        format!("bytes 0-{last}/{total}")
    );
    assert_eq!(header_of(first.headers(), header::CONTENT_TYPE), "audio/ogg");

    // A range for more than the cap is cut to it.
    let cut = get(&viewer, now, &token, Some("bytes=10-"));
    assert_eq!(cut.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(cut.body().len(), MAX_READ);
    assert_eq!(
        header_of(cut.headers(), header::CONTENT_RANGE),
        format!("bytes 10-{}/{total}", 10 + last)
    );

    // The tail is shorter.
    let tail = get(&viewer, now, &token, Some(&format!("bytes={}-", MAX_READ * 2)));
    assert_eq!(tail.body().len(), 10);
    assert_eq!(
        header_of(tail.headers(), header::CONTENT_RANGE),
        format!("bytes {}-{}/{total}", MAX_READ * 2, total - 1)
    );
}

#[test]
fn a_file_that_became_empty_is_200_without_a_body_and_any_range_of_it_is_416() {
    let Rig {
        viewer,
        now,
        token,
        _folder: folder,
    } = rig();
    // The session holds the file open, so truncating it is seen by the token.
    File::create(folder.path().join("a.flac")).expect("truncated");

    let whole = get(&viewer, now, &token, None);
    assert_eq!(whole.status(), StatusCode::OK);
    assert!(whole.body().is_empty());
    assert_eq!(header_of(whole.headers(), header::CONTENT_LENGTH), "0");

    assert_empty(
        &get(&viewer, now, &token, Some("bytes=0-")),
        StatusCode::RANGE_NOT_SATISFIABLE,
    );
}

#[test]
fn head_has_the_headers_of_get_and_no_body() {
    let Rig { viewer, now, token, .. } = rig();
    let uri = format!("/{token}");

    let reply = serve(&viewer, now, &Method::HEAD, &uri, None);
    assert_eq!(reply.status(), StatusCode::OK);
    assert!(reply.body().is_empty());
    assert_eq!(header_of(reply.headers(), header::CONTENT_LENGTH), "1000");
    assert_eq!(header_of(reply.headers(), header::CONTENT_TYPE), "audio/flac");
    assert_eq!(header_of(reply.headers(), header::ACCEPT_RANGES), "bytes");
    assert_guarded(&reply);

    let part = serve(&viewer, now, &Method::HEAD, &uri, Some("bytes=10-19"));
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert!(part.body().is_empty());
    assert_eq!(header_of(part.headers(), header::CONTENT_LENGTH), "10");
    assert_eq!(header_of(part.headers(), header::CONTENT_RANGE), "bytes 10-19/1000");
}

#[test]
fn other_methods_are_405() {
    let Rig { viewer, now, token, .. } = rig();

    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::OPTIONS,
        Method::PATCH,
    ] {
        let reply = serve(&viewer, now, &method, &format!("/{token}"), None);

        assert_empty(&reply, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header_of(reply.headers(), header::ALLOW), "GET, HEAD");
    }
}

#[test]
fn a_path_that_is_not_exactly_a_token_is_404() {
    let Rig { viewer, now, token, .. } = rig();
    let upper = token.to_uppercase();

    let paths = [
        "/".to_owned(),
        String::new(),
        token.clone(),
        format!("/{token}/"),
        format!("/{token}/extra"),
        format!("//{token}"),
        format!("/x/{token}"),
        format!("/{}", &token[1..]),
        format!("/{token}0"),
        format!("/{upper}"),
        "/a.flac".to_owned(),
        format!("/{token}%2f"),
        format!("/../{token}"),
    ];
    for path in paths {
        let reply = serve(&viewer, now, &Method::GET, &path, None);

        assert_empty(&reply, StatusCode::NOT_FOUND);
    }
    // The one exact path still works.
    assert_eq!(get(&viewer, now, &token, None).status(), StatusCode::OK);
}

#[test]
fn an_unknown_token_is_404() {
    let viewer = Viewer::default();

    assert_empty(
        &get(&viewer, Instant::now(), &"0".repeat(32), None),
        StatusCode::NOT_FOUND,
    );
}

#[test]
fn a_token_stops_working_when_its_session_is_closed_or_evicted_or_replaced() {
    let (_folder, path) = folder_with(&[("a.flac", &flac()), ("a.txt", b"hello")]);
    let viewer = Viewer::default();
    let now = Instant::now();
    let status = |token: &str| get(&viewer, now, token, None).status();

    let (closed, closed_token) = token_of(&viewer, &path, "a.flac", now);
    assert_eq!(status(&closed_token), StatusCode::OK);
    viewer.close(closed);
    assert_eq!(status(&closed_token), StatusCode::NOT_FOUND);

    let (replaced, old) = token_of(&viewer, &path, "a.flac", now);
    let new = viewer.mint(replaced, now).expect("token");
    assert_eq!((status(&old), status(&new)), (StatusCode::NOT_FOUND, StatusCode::OK));

    let (_, evicted) = token_of(&viewer, &path, "a.flac", now);
    for _ in 0..MAX_SESSIONS {
        open(&viewer, &path, "a.txt");
    }
    assert_eq!(status(&evicted), StatusCode::NOT_FOUND);
}

#[test]
fn a_token_idle_for_thirty_minutes_expires_and_every_use_renews_it() {
    let (_folder, path) = folder_with(&[("a.flac", &flac())]);
    let viewer = Viewer::default();
    let start = Instant::now();
    let (_, token) = token_of(&viewer, &path, "a.flac", start);

    let almost = IDLE - Duration::from_secs(1);
    assert_eq!(get(&viewer, start + almost, &token, None).status(), StatusCode::OK);
    // Used at `almost`, so it lasts until `almost + IDLE`.
    assert_eq!(get(&viewer, start + almost * 2, &token, None).status(), StatusCode::OK);

    let later = start + almost * 2 + IDLE;
    assert_empty(&get(&viewer, later, &token, None), StatusCode::NOT_FOUND);
    // An expired token does not come back.
    assert_eq!(get(&viewer, start, &token, None).status(), StatusCode::NOT_FOUND);
}

#[test]
fn at_most_four_tokens_live_and_the_least_recently_used_goes_first() {
    let (_folder, path) = folder_with(&[("a.flac", &flac())]);
    let viewer = Viewer::default();
    let start = Instant::now();

    let tokens: Vec<String> = (0..=MAX_TOKENS as u64)
        .map(|second| token_of(&viewer, &path, "a.flac", start + Duration::from_secs(second)).1)
        .collect();

    let later = start + Duration::from_secs(10);
    assert_eq!(get(&viewer, later, &tokens[0], None).status(), StatusCode::NOT_FOUND);
    for token in &tokens[1..] {
        assert_eq!(get(&viewer, later, token, None).status(), StatusCode::OK);
    }
}

#[test]
fn a_read_is_refused_as_503_when_the_media_cap_is_full_but_nothing_else_is() {
    let Rig { viewer, now, token, .. } = rig();
    let held = fill(&viewer.media, MEDIA_THREADS);

    let busy = get(&viewer, now, &token, None);
    assert_empty(&busy, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header_of(busy.headers(), header::RETRY_AFTER), "1");
    assert_empty(&get(&viewer, now, &"1".repeat(32), None), StatusCode::NOT_FOUND);

    drop(held);
    assert_eq!(get(&viewer, now, &token, None).status(), StatusCode::OK);
}
