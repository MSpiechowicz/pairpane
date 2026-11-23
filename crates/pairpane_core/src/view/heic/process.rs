//! The decoding in a child process, both ends of it. The thumbnail helper (`view::thumb`) shares the framing and the
//! parent's end ([`run_within`], [`read_picture`], [`write_answer`]); it is another child argument with another child.
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
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::process::{self, GRACE, Started, Wait};

use super::{
    HEADER, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, cancelled, check_size, no_decoder, passing_undecodable, too_big,
    undecodable,
};

const STATUS_OK: u8 = 0;
const STATUS_NO_DECODER: u8 = 1;
const STATUS_UNDECODABLE: u8 = 2;
const STATUS_TOO_BIG: u8 = 3;

/// How long the child may take, from its start to its end. The biggest picture decodes in seconds; a child that is
/// still at it after this is hung, and is killed.
pub(in crate::view) const DEADLINE: Duration = Duration::from_secs(60);

/// The committed memory a HEIC helper may have on Windows (the Job Object's limit; Linux limits the address space in
/// `sandbox.rs`), in bytes: what a picture of the pixel cap needs (the input, the decoder's planes, the converted pixels
/// and the answer, together well under 2 GiB) and room to spare.
#[cfg(windows)]
const MEMORY: usize = 3 * 1024 * 1024 * 1024;

/// The environment variables the helper is given on top of the ones every child gets ([`process::command`]: a Windows
/// process needs its system directory and its temporary folder, the Imaging Component reads them); it starts with no
/// others. The loader and libheif need these to find the libraries and the plugins the app was started with (an AppImage
/// sets `LD_LIBRARY_PATH` for the libraries it bundles; the executable itself needs nothing else to start).
const KEPT_ENV: &[&str] = &["LIBHEIF_PLUGIN_PATH", "LD_LIBRARY_PATH"];

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
        tracing::debug!(%error, "the helper cannot limit itself");

        return 1;
    }

    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        let _ = super::libheif::api();
    }

    #[cfg(target_os = "linux")]
    if let Err(error) = super::sandbox::single_threaded()
        .and_then(|()| super::sandbox::confine(super::sandbox::ADDRESS_SPACE, super::sandbox::Filter::DenyList))
    {
        tracing::debug!(%error, "the helper cannot confine itself");

        return 1;
    }

    run_child(io::stdin().lock(), io::stdout().lock())
}

/// [`child_main`] over any input and output.
pub(super) fn run_child(input: impl Read, output: impl Write) -> i32 {
    serve(input, output, convert)
}

/// The work of every helper: reads the file from `input` (at most [`MAX_IMAGE_BYTES`]), turns it into an answer with
/// `convert` and writes the answer to `output` (see the module). Returns the exit code of the process, 0 when an answer
/// was written, which a failed conversion is too.
pub(in crate::view) fn serve(
    input: impl Read,
    mut output: impl Write,
    convert: impl FnOnce(&[u8]) -> io::Result<Vec<u8>>,
) -> i32 {
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
pub(in crate::view) fn write_answer(output: &mut impl Write, result: &io::Result<Vec<u8>>) -> io::Result<()> {
    match result {
        Ok(picture) => {
            output.write_all(&[STATUS_OK])?;

            output.write_all(picture)
        }
        Err(error) => output.write_all(&[status_of(error)]),
    }
}

/// The result an answer read from `input` stands for, for a picture of any size [`check_size`] allows: [`read_picture`]
/// with the side cap of the viewer (the tests read the answers of the HEIC helper with it).
#[cfg(test)]
pub(super) fn read_answer(input: impl Read) -> io::Result<Vec<u8>> {
    read_picture(input, MAX_IMAGE_SIDE)
}

/// The result an answer read from `input` stands for. A picture is taken only when the header is a size [`check_size`]
/// allows, no side of it is over `max_side` (a thumbnail is small, and nothing is reserved for a bigger one) and the
/// answer is exactly as long as it says; a failure status must stand alone; anything else is
/// undecodable. The buffer for the picture is reserved at its exact size (and one byte more, to tell a longer answer
/// from an exact one) before the pixels are read, and what is returned is that buffer: the header and the pixels.
pub(in crate::view) fn read_picture(mut input: impl Read, max_side: u32) -> io::Result<Vec<u8>> {
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
    if width > max_side || height > max_side {
        return Err(undecodable());
    }

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
    let mut command = process::command(program, args, KEPT_ENV);
    command.stdin(Stdio::piped());

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
    run_within(program, args, bytes, cancel, deadline, MAX_IMAGE_SIDE)
}

/// [`run`] for a helper whose picture has no side over `max_side` (see [`read_picture`]).
pub(in crate::view) fn run_within(
    program: &Path,
    args: &[&str],
    bytes: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
    max_side: u32,
) -> io::Result<Vec<u8>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }

    #[cfg(target_os = "linux")]
    super::sandbox::note_kernel();

    let mut child = command(program, args).spawn().map_err(|error| {
        tracing::debug!(%error, "the helper did not start");

        // Not our text and not a kind the page maps to a message (a missing helper is not a missing file).
        io::Error::other("the helper did not start")
    })?;

    // The job is made before a byte is sent, so the helper has nothing to run that is not in it. It is kept until the
    // child is reaped below; closing it ends whatever is still in it.
    #[cfg(windows)]
    let _job = match super::job::contain(&child, MEMORY) {
        Ok(job) => job,
        Err(error) => {
            tracing::debug!(%error, "the helper cannot be contained");
            process::stop(&mut child);

            return Err(io::Error::other("the helper cannot be contained"));
        }
    };

    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        process::stop(&mut child);

        return Err(io::Error::other("the helper has no pipes"));
    };

    run_started(child, stdin, stdout, bytes, cancel, deadline, max_side)
}

/// Gives a helper that has been started (by [`run_within`], or by `view::thumb::windows` in its container) the `bytes` on
/// its standard input `stdin`, reads its answer from `stdout` and stops and reaps it on every path.
pub(in crate::view) fn run_started<C: Started>(
    mut child: C,
    stdin: impl Write + Send + 'static,
    stdout: impl Read + Send + 'static,
    bytes: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
    max_side: u32,
) -> io::Result<Vec<u8>> {
    let answer = supervise(&mut child, stdin, stdout, bytes, cancel, deadline, max_side);

    // Whatever happened above, the child is stopped and reaped here; harmless for one that has ended.
    process::stop(&mut child);

    answer
}

/// Feeds the child, collects its answer and stops the child (it is reaped by the caller). The answer is the result of
/// the reading thread, taken only when the child has ended well too.
///
/// What is the state of the moment and no verdict on the file (the threads that feed and read the child could not be
/// started, the child ran past its deadline, one end of the answer did not come after the other) is a transient failure
/// ([`passing_undecodable`]), which a cache must not keep; a child that crashes or answers wrongly is the file's.
fn supervise<C: Started>(
    child: &mut C,
    mut stdin: impl Write + Send + 'static,
    stdout: impl Read + Send + 'static,
    bytes: Vec<u8>,
    cancel: &AtomicBool,
    deadline: Duration,
    max_side: u32,
) -> io::Result<Vec<u8>> {
    // The writer ends when everything is written (the pipe is closed with it) or when the child is gone. It is never
    // waited for: a child that does not read is killed, which ends the writer too.
    thread::Builder::new()
        .name("helper-stdin".to_owned())
        .spawn(move || {
            let _ = stdin.write_all(&bytes);
        })
        .map_err(|_| io::Error::other("the helper's threads did not start"))?;

    let reader = thread::Builder::new()
        .name("helper-stdout".to_owned())
        .spawn(move || read_picture(stdout, max_side))
        .map_err(|_| io::Error::other("the helper's threads did not start"))?;

    let waited = wait_for_answer(child, &reader, cancel, deadline);

    // The child is stopped in every case, and its output closes with it, which ends the reader. If it does not end (the
    // pipe is held open by something else) it is left behind.
    process::stop(child);
    let answer = process::join_within(reader, GRACE);

    waited?;

    answer.unwrap_or_else(|| Err(passing_undecodable()))
}

/// Waits until the reader has the whole answer and the child has ended with success. `Err` for a raised `cancel`, a child
/// that ends with an error, one that runs `deadline` and one that does not end (or whose output does not) within
/// [`GRACE`] of the other.
fn wait_for_answer(
    child: &mut impl Started,
    reader: &JoinHandle<io::Result<Vec<u8>>>,
    cancel: &AtomicBool,
    deadline: Duration,
) -> io::Result<()> {
    match process::wait_for(child, cancel, deadline, || reader.is_finished()) {
        Ok(status) if status.success() => Ok(()),
        Err(Wait::Cancelled) => Err(cancelled()),
        Err(Wait::Deadline) => {
            tracing::debug!("the helper took too long");

            Err(passing_undecodable())
        }
        Err(Wait::Stalled | Wait::Lost) => Err(passing_undecodable()),
        // It ended with an error: it crashed on the file.
        Ok(_) => Err(undecodable()),
    }
}
