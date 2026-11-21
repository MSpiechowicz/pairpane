//! The `pairpane-media` scheme: audio and video for the viewer's `<audio>` and `<video>`, with byte ranges.
//!
//! Only `GET` and `HEAD` of `/<token>` are answered (a token from `view_media`); the token stands for an open file the
//! viewer holds, which is read at offsets, so nothing is ever opened by a name the page sent. Anything else is `405`
//! (another method) or an empty `404` (another path shape, an unknown, revoked or idle token). No answer holds a path.
//!
//! A `Range` gets `206` with at most 4 MiB (`416` when it starts past the end). Without one the whole file is `200`
//! when it is at most 4 MiB, else the first 4 MiB as `206`, which a player continues by asking for ranges. Everything is
//! `no-store`, `nosniff`, and sandboxed by a `Content-Security-Policy` in case a browser ever renders it.
//!
//! The lookup of the token is in memory and runs on the caller's thread; the size and the read touch the disk and run on
//! a blocking thread of the media cap (`503` with `Retry-After` when it is full).

use std::sync::Arc;
use std::time::Instant;

use pairpane_core::view::range::{self, Requested};
use pairpane_core::view::{MAX_READ, ViewFile};
use tauri::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, Runtime, UriSchemeResponder, async_runtime};

use super::{MEDIA_THREADS, Viewer};

/// The scheme's name: `pairpane-media://localhost/<token>`, on Windows `http://pairpane-media.localhost/<token>`.
pub const SCHEME: &str = "pairpane-media";

/// The characters of a token.
const TOKEN_LENGTH: usize = 32;

pub(super) type Reply = Response<Vec<u8>>;

/// What a request comes to: an answer already, or a read of a file that a blocking thread has to do.
enum Step {
    Done(Reply),
    Read(Read),
}

/// A read of the file behind a token, to answer on a blocking thread.
struct Read {
    file: Arc<ViewFile>,
    mime: String,
    range: Option<String>,
    head: bool,
}

/// Answers one request from the in-memory state, or says what to read.
fn step(viewer: &Viewer, now: Instant, method: &Method, path: &str, range: Option<&str>) -> Step {
    if method != Method::GET && method != Method::HEAD {
        let mut reply = reply(StatusCode::METHOD_NOT_ALLOWED, 0, Vec::new());
        reply
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));

        return Step::Done(reply);
    }

    let Some((file, mime)) = token_of(path).and_then(|text| viewer.token(text, now)) else {
        return Step::Done(reply(StatusCode::NOT_FOUND, 0, Vec::new()));
    };

    Step::Read(Read {
        file,
        mime,
        range: range.map(str::to_owned),
        head: method == Method::HEAD,
    })
}

/// The token of a path that is exactly `/` and a token (32 lowercase hex characters), else `None`.
fn token_of(path: &str) -> Option<&str> {
    let text = path.strip_prefix('/')?;
    let valid = text.len() == TOKEN_LENGTH && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));

    valid.then_some(text)
}

/// The answer when the media cap is full: try again in a second.
fn busy() -> Reply {
    let mut busy = reply(StatusCode::SERVICE_UNAVAILABLE, 0, Vec::new());
    busy.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));

    busy
}

impl Read {
    /// Reads what the request asks for and answers; blocks, so only on a blocking thread.
    fn run(self) -> Reply {
        let cap = MAX_READ as u64;
        let Ok(size) = self.file.size() else {
            return reply(StatusCode::INTERNAL_SERVER_ERROR, 0, Vec::new());
        };

        let (status, start, length) = match range::parse(self.range.as_deref(), size, cap) {
            Requested::Unsatisfiable => return unsatisfiable(size),
            Requested::Bytes { start, end } => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
            Requested::Whole if size <= cap => (StatusCode::OK, 0, size),
            Requested::Whole => (StatusCode::PARTIAL_CONTENT, 0, cap),
        };

        let body = if self.head {
            Vec::new()
        } else {
            // `length` is at most the cap, so it fits.
            match self.file.read_at(start, length as usize) {
                Ok(bytes) if bytes.len() as u64 == length => bytes,
                // The file changed or cannot be read: a body of the wrong length would break the player.
                _ => return reply(StatusCode::INTERNAL_SERVER_ERROR, 0, Vec::new()),
            }
        };

        let mut reply = reply(status, length, body);
        let headers = reply.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&self.mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
        );
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        if status == StatusCode::PARTIAL_CONTENT {
            let content_range = format!("bytes {start}-{}/{size}", start + length - 1);
            if let Ok(value) = HeaderValue::from_str(&content_range) {
                headers.insert(header::CONTENT_RANGE, value);
            }
        }

        reply
    }
}

fn unsatisfiable(size: u64) -> Reply {
    let mut reply = reply(StatusCode::RANGE_NOT_SATISFIABLE, 0, Vec::new());
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
        reply.headers_mut().insert(header::CONTENT_RANGE, value);
    }

    reply
}

/// An answer with the headers every answer has; `length` is the Content-Length (a `HEAD` has none of the body).
fn reply(status: StatusCode, length: u64, body: Vec<u8>) -> Reply {
    let mut reply = Response::new(body);
    *reply.status_mut() = status;

    let headers = reply.headers_mut();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );

    reply
}

/// The whole answer to one request on the calling thread, which blocks for the read: what the tests run (the scheme
/// handler does the same in steps, so the read happens on a blocking thread and never on the async runtime).
#[cfg(test)]
pub(super) fn serve(viewer: &Viewer, now: Instant, method: &Method, path: &str, range: Option<&str>) -> Reply {
    match step(viewer, now, method, path, range) {
        Step::Done(reply) => reply,
        Step::Read(read) => match viewer.media.acquire(MEDIA_THREADS) {
            Some(_permit) => read.run(),
            None => busy(),
        },
    }
}

/// The handler of the scheme. Nothing here waits on the disk: the read goes to a blocking thread.
pub fn handle<R: Runtime>(app: &AppHandle<R>, request: &Request<Vec<u8>>, responder: UriSchemeResponder) {
    let Some(viewer) = app.try_state::<Viewer>().map(|viewer| Viewer::clone(&viewer)) else {
        responder.respond(reply(StatusCode::NOT_FOUND, 0, Vec::new()));

        return;
    };

    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());

    match step(&viewer, Instant::now(), request.method(), request.uri().path(), range) {
        Step::Done(done) => responder.respond(done),
        Step::Read(read) => match viewer.media.acquire(MEDIA_THREADS) {
            Some(permit) => {
                async_runtime::spawn_blocking(move || {
                    let _permit = permit;

                    responder.respond(read.run());
                });
            }
            None => responder.respond(busy()),
        },
    }
}
