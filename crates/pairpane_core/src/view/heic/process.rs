//! The decoding in a child process, both ends of it.
//!
//! The child is the app's own executable started with the argument `--pairpane-decode-heic`; the shell hands it to
//! [`child_main`] before anything else starts. The child reads the file from its standard input (at most
//! [`MAX_IMAGE_BYTES`]), decodes it with the system's codec and writes one status byte to its standard output and, for
//! a picture, the buffer of [`decode`](super::decode) after it. Nothing else is written there, nothing touches the file
//! system and nothing is started from it.
//!
//! | status | meaning | after the byte |
//! | ------ | ------- | -------------- |
//! | 0 | the picture | header and RGBA, exactly as long as the header says |
//! | 1 | the system has no decoder | nothing |
//! | 2 | the file cannot be decoded | nothing |
//! | 3 | the file or the picture is over a cap | nothing |
//!
//! The parent ([`run`]) never trusts the child: the answer is read as it is framed (the status, then the header, then
//! exactly the number of bytes the header allows, which are reserved before they are read and of which one more is
//! looked for), and a child that crashes, exits with an error, answers wrongly, runs on after it has answered or does
//! not answer within [`DEADLINE`] counts as a file that cannot be decoded. The parent polls the cancel flag while it
//! waits and kills the child when it is raised, and it never waits for the child's output without a bound: when the
//! child has been killed and its output still does not end (something else holds the pipe open), the reading thread is
//! left behind. The child is reaped on every path. On Linux the child limits and confines itself (`sandbox.rs`), on
//! Windows the parent puts it in a Job Object right after the start (`job.rs`).

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{HEADER, MAX_IMAGE_BYTES, cancelled, check_size, no_decoder, too_big, undecodable};

const STATUS_OK: u8 = 0;
const STATUS_NO_DECODER: u8 = 1;
const STATUS_UNDECODABLE: u8 = 2;
const STATUS_TOO_BIG: u8 = 3;

/// How often the parent looks at the cancel flag and at the child while it waits.
const POLL: Duration = Duration::from_millis(25);

/// How long the child may take, from its start to its end. The biggest picture decodes in seconds; a child that is
/// still at it after this is hung, and is killed.
pub(super) const DEADLINE: Duration = Duration::from_secs(60);

/// How long the parent waits for one of the two ends of an answer (the child ending, its output ending) once the other
/// has come, and for the reading thread to end after the child is killed.
const GRACE: Duration = Duration::from_secs(2);

/// The environment variables the helper is given; it starts with no others. The loader and libheif need these to find the
/// libraries and the plugins the app was started with (an AppImage sets `LD_LIBRARY_PATH` for the libraries it bundles; the
/// executable itself needs nothing else to start), and a Windows process needs its system directory and its temporary
/// folder (the Imaging Component reads them).
const KEPT_ENV: &[&str] = &[
    "LIBHEIF_PLUGIN_PATH",
    "LD_LIBRARY_PATH",
    #[cfg(windows)]
    "SystemRoot",
    #[cfg(windows)]
    "WINDIR",
    #[cfg(windows)]
    "TEMP",
    #[cfg(windows)]
    "TMP",
];

/// `CREATE_NO_WINDOW`: the child of a program without a console gets none either.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The entry of the child: reads the file from the standard input, decodes it and writes the answer to the standard
/// output. Returns the exit code of the process (0 when an answer was written, which a failed decoding is too).
///
/// On Linux the child first closes the descriptors it inherited, then limits itself, then loads the codecs (they open
/// their plugins at once, which needs the file system) and then gives up the file system and the network and installs
/// its seccomp filter, all before the first byte is read (`sandbox.rs`).
pub fn child_main() -> i32 {
    // Before anything else: the descriptors of the app that this process inherited are no use to it and no risk to us.
    #[cfg(target_os = "linux")]
    super::sandbox::close_inherited_fds();

    #[cfg(target_os = "linux")]
    if let Err(error) = super::sandbox::limit() {
        tracing::debug!(%error, "the heic helper cannot limit itself");

        return 1;
    }

    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        let _ = super::libheif::api();
    }

    #[cfg(target_os = "linux")]
    if let Err(error) = super::sandbox::single_threaded().and_then(|()| super::sandbox::confine()) {
        tracing::debug!(%error, "the heic helper cannot confine itself");

        return 1;
    }

    run_child(io::stdin().lock(), io::stdout().lock())
}

/// [`child_main`] over any input and output.
pub(super) fn run_child(input: impl Read, mut output: impl Write) -> i32 {
    let mut bytes = Vec::new();
    let read = input.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes);

    let result = match read {
        Err(_) => Err(undecodable()),
        Ok(_) if bytes.len() as u64 > MAX_IMAGE_BYTES => Err(too_big()),
        Ok(_) => convert(&bytes),
    };
    drop(bytes);

    match write_answer(&mut output, &result).and_then(|()| output.flush()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// The picture of the file `bytes` through this system's decoder (the work of the child).
pub(super) fn convert(bytes: &[u8]) -> io::Result<Vec<u8>> {
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        super::libheif::api().ok_or_else(no_decoder)?.decode(bytes)
    }

    #[cfg(windows)]
    {
        super::wic::decode(bytes)
    }

    #[cfg(not(any(
        windows,
        all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
    )))]
    {
        let _ = bytes;

        Err(no_decoder())
    }
}

/// The status byte for a failure: what the decoders report as kinds (`Unsupported`: no decoder, `InvalidInput`: over a
/// cap) and everything else as a file that cannot be decoded.
fn status_of(error: &io::Error) -> u8 {
    match error.kind() {
        io::ErrorKind::Unsupported => STATUS_NO_DECODER,
        io::ErrorKind::InvalidInput => STATUS_TOO_BIG,
        _ => STATUS_UNDECODABLE,
    }
}

/// Writes the answer for `result`: the status byte and, for a picture, the buffer.
pub(super) fn write_answer(output: &mut impl Write, result: &io::Result<Vec<u8>>) -> io::Result<()> {
    match result {
        Ok(picture) => {
            output.write_all(&[STATUS_OK])?;

            output.write_all(picture)
        }
        Err(error) => output.write_all(&[status_of(error)]),
    }
}

/// The result an answer read from `input` stands for. A picture is taken only when the header is a size [`check_size`]
/// allows and the answer is exactly as long as it says; a failure status must stand alone; anything else is
/// undecodable. The buffer for the picture is reserved at its exact size (and one byte more, to tell a longer answer
/// from an exact one) before the pixels are read, and what is returned is that buffer: the header and the pixels.
pub(super) fn read_answer(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut status = [0_u8; 1];
    input.read_exact(&mut status).map_err(|_| undecodable())?;

    match status[0] {
        STATUS_OK => {}
        STATUS_NO_DECODER | STATUS_UNDECODABLE | STATUS_TOO_BIG => {
            let mut more = Vec::new();
            input.take(1).read_to_end(&mut more).map_err(|_| undecodable())?;

            return Err(match (more.is_empty(), status[0]) {
                (false, _) => undecodable(),
                (true, STATUS_NO_DECODER) => no_decoder(),
                (true, STATUS_TOO_BIG) => too_big(),
                (true, _) => undecodable(),
            });
        }
        _ => return Err(undecodable()),
    }

    let mut header = [0_u8; HEADER];
    input.read_exact(&mut header).map_err(|_| undecodable())?;
    let width = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let height = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let (width, height) = check_size(i64::from(width), i64::from(height)).map_err(|_| undecodable())?;

    let pixels = u64::from(width) * u64::from(height) * 4;
    let total = usize::try_from(pixels)
        .ok()
        .and_then(|pixels| pixels.checked_add(HEADER));
    let total = total.ok_or_else(too_big)?;

    let mut picture = Vec::new();
    picture.try_reserve_exact(total + 1).map_err(|_| too_big())?;
    picture.extend_from_slice(&header);
    input
        .take(pixels + 1)
        .read_to_end(&mut picture)
        .map_err(|_| undecodable())?;

    if picture.len() != total {
        return Err(undecodable());
    }

    Ok(picture)
}

/// The command that starts the helper `program` with `args`: the standard input and output are pipes (the only way in and
/// out), the standard error goes nowhere, and the environment is cleared but for [`KEPT_ENV`], so nothing of the app's
/// own (tokens, proxy settings, a session) reaches the codecs.
pub(super) fn command(program: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for name in KEPT_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // A relative or empty entry of a kept search path must not resolve against wherever the app happens to be.
    #[cfg(unix)]
    command.current_dir("/");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

/// Starts `program` with `args`, gives it `bytes` on its standard input and returns the result of its answer (see the
/// module). Stops the child when `cancel` is raised, which is `Interrupted`, and when it has run for `deadline`.
pub(super) fn run(
    program: &Path,
    args: &[&str],
    bytes: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
) -> io::Result<Vec<u8>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    #[cfg(target_os = "linux")]
    super::sandbox::note_kernel();

    let mut child = command(program, args).spawn().map_err(|error| {
        tracing::debug!(%error, "the heic helper did not start");

        // Not our text and not a kind the page maps to a message (a missing helper is not a missing file).
        io::Error::other("the heic helper did not start")
    })?;

    // The job is made before a byte is sent, so the helper has nothing to run that is not in it. It is kept until the
    // child is reaped below; closing it ends whatever is still in it.
    #[cfg(windows)]
    let _job = match super::job::contain(&child) {
        Ok(job) => job,
        Err(error) => {
            tracing::debug!(%error, "the heic helper cannot be contained");
            let _ = child.kill();
            let _ = child.wait();

            return Err(io::Error::other("the heic helper cannot be contained"));
        }
    };

    let answer = supervise(&mut child, bytes, cancel, deadline);

    // Whatever happened above, the child is stopped and reaped here; both calls are harmless for one that has ended.
    let _ = child.kill();
    let _ = child.wait();

    answer
}

/// Feeds the child, collects its answer and stops the child (it is reaped by the caller). The answer is the result of
/// the reading thread, taken only when the child has ended well too.
fn supervise(child: &mut Child, bytes: Vec<u8>, cancel: &AtomicBool, deadline: Duration) -> io::Result<Vec<u8>> {
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(undecodable());
    };

    // The writer ends when everything is written (the pipe is closed with it) or when the child is gone. It is never
    // waited for: a child that does not read is killed, which ends the writer too.
    thread::Builder::new()
        .name("heic-stdin".to_owned())
        .spawn(move || {
            let _ = stdin.write_all(&bytes);
        })
        .map_err(|_| undecodable())?;

    let reader = thread::Builder::new()
        .name("heic-stdout".to_owned())
        .spawn(move || read_answer(stdout))
        .map_err(|_| undecodable())?;

    let waited = wait_for_answer(child, &reader, cancel, deadline);

    // The child is stopped in every case, and its output closes with it, which ends the reader. If it does not end (the
    // pipe is held open by something else) it is left behind.
    let _ = child.kill();
    let _ = child.wait();
    let answer = join_within(reader, GRACE);

    waited?;

    answer.unwrap_or_else(|| Err(undecodable()))
}

/// Waits until the reader has the whole answer and the child has ended with success. `Err` for a raised `cancel`, a child
/// that ends with an error, one that runs `deadline` and one that does not end (or whose output does not) within
/// [`GRACE`] of the other.
fn wait_for_answer(
    child: &mut Child,
    reader: &JoinHandle<io::Result<Vec<u8>>>,
    cancel: &AtomicBool,
    deadline: Duration,
) -> io::Result<()> {
    let started = Instant::now();
    let mut exited = None;
    let mut read = None;

    loop {
        let now = Instant::now();
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        if now.duration_since(started) >= deadline {
            tracing::debug!("the heic helper took too long");

            return Err(undecodable());
        }

        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => exited = Some(now),
                Ok(Some(_)) | Err(_) => return Err(undecodable()),
                Ok(None) => {}
            }
        }
        if read.is_none() && reader.is_finished() {
            read = Some(now);
        }

        match (exited, read) {
            (Some(_), Some(_)) => return Ok(()),
            (Some(first), None) | (None, Some(first)) if now.duration_since(first) >= GRACE => {
                return Err(undecodable());
            }
            _ => {}
        }

        thread::sleep(POLL);
    }
}

/// The result of `thread` when it ends within `bound`; `None` when it does not (it is then left to run alone) or when
/// it panicked.
pub(super) fn join_within<T>(thread: JoinHandle<T>, bound: Duration) -> Option<T> {
    let until = Instant::now() + bound;

    while !thread.is_finished() {
        if Instant::now() >= until {
            return None;
        }

        thread::sleep(Duration::from_millis(5));
    }

    thread.join().ok()
}
