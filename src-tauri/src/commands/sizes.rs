//! Folder sizes: `pane_folder_size` walks one subfolder of the pane's folder on a blocking thread and reports its
//! size over its own channel.
//!
//! A pane runs one size job at a time (the UI queues them), so a page cannot pile up walks. A request while the
//! pane's earlier job still has its thread is refused, not queued: a cancelled walk on a hung mount never stops, and
//! each new one would take another thread (see `limit.rs`). The job belongs to the pane's folder: it stops when the pane lists another folder
//! (see [`Sizing::move_to`]) and keeps running when the same folder is only reloaded or another folder fails to open.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pairpane_core::listing;
use pairpane_core::model::{IpcError, IpcErrorCode, PaneId, SizeEvent};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use super::limit::{Permit, ThreadCap};
use super::panes::{Panes, folder_error, resolve};
use crate::error::{CommandError, CommandResult};

/// A pane's size state: the folder whose subfolders may be measured and the job running for it.
#[derive(Default)]
pub(super) struct Sizing {
    /// The latest folder of the pane that listed successfully; `None` until one has.
    folder: Option<PathBuf>,
    /// Stops the running job (set on cancel).
    job: Option<Arc<AtomicBool>>,
    /// The thread of the latest job, which may outlive its cancel (see [`THREADS`]).
    threads: ThreadCap,
}

/// A started size job: what to measure, the flag that stops it, and the place its thread holds until it ends.
#[derive(Debug)]
pub(super) struct Job {
    pub(super) folder: PathBuf,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) permit: Permit,
}

/// Size threads a pane may have at once: one, so a walk stuck on a hung mount blocks new ones, not the thread pool.
const THREADS: usize = 1;

impl Sizing {
    /// Makes `folder` the one measured in, stopping the running job unless it already is that folder.
    pub(super) fn move_to(&mut self, folder: &Path) {
        if self.folder.as_deref() == Some(folder) {
            return;
        }

        self.cancel_job();
        self.folder = Some(folder.to_owned());
    }

    /// Starts a job for the folder `name` inside `path`, which must be the folder the pane shows. Cancels the pane's
    /// earlier job; refused (leaving it alone) while that job's thread has not ended yet.
    pub(super) fn begin(&mut self, path: String, name: &str) -> CommandResult<Job> {
        let folder = resolve(Some(path), Some(name))?;
        let current = self
            .folder
            .as_deref()
            .filter(|current| folder.parent() == Some(current));
        if current.is_none() {
            return Err(refused());
        }

        let permit = self.threads.acquire(THREADS).ok_or_else(busy)?;

        self.cancel_job();
        let stop = Arc::new(AtomicBool::new(false));
        self.job = Some(Arc::clone(&stop));

        Ok(Job { folder, stop, permit })
    }

    fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.store(true, Ordering::Relaxed);
        }
    }
}

/// Calculates the size of the folder `name` inside the folder `pane` shows (`path`): `progress` events while it
/// runs, then `done`, `error` or `cancelled` (stopped by the pane's next request, or by listing another folder), so
/// the page always learns that a job ended. The check that `path` is the pane's current folder keeps this consistent with
/// `pane_open`; it is not an authorization boundary (the page can already open any folder).
#[tauri::command]
pub async fn pane_folder_size(
    panes: State<'_, Panes>,
    pane: PaneId,
    path: String,
    name: String,
    on_event: Channel<SizeEvent>,
) -> CommandResult<()> {
    let job = panes
        .with_sizing(pane, |sizing| sizing.begin(path, &name))
        .unwrap_or_else(|| Err(refused()))?;

    async_runtime::spawn_blocking(move || {
        let progress = |bytes| {
            on_event.send(SizeEvent::Progress { bytes }).ok();
        };

        let result = listing::size::folder_size(&job.folder, &job.stop, progress);

        // The thread is done with the disk: free its place before telling the page, which may ask for the next
        // folder at once.
        drop(job.permit);

        match result {
            Ok(Some(bytes)) => {
                on_event.send(SizeEvent::Done { bytes }).ok();
            }
            Ok(None) => {
                on_event.send(SizeEvent::Cancelled).ok();
            }
            Err(error) => {
                on_event
                    .send(SizeEvent::Error {
                        error: size_error(&error),
                    })
                    .ok();
            }
        }
    });

    Ok(())
}

/// The pane has not listed a folder yet.
fn refused() -> CommandError {
    CommandError::new(
        IpcErrorCode::InvalidInput,
        "only a folder of the pane's current folder can be measured",
    )
}

/// The pane's earlier size calculation is still running, though cancelled (a folder on a mount that does not respond).
fn busy() -> CommandError {
    CommandError::new(IpcErrorCode::Busy, "the previous size calculation has not stopped yet")
}

/// What the pane says when a folder's size cannot be calculated.
fn size_error(error: &std::io::Error) -> IpcError {
    folder_error(error, "read this folder", "Cannot calculate this size")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static str {
        if cfg!(windows) { "C:\\data" } else { "/data" }
    }

    fn listed(folder: &Path) -> Sizing {
        let mut sizing = Sizing::default();
        sizing.move_to(folder);

        sizing
    }

    #[test]
    fn only_a_folder_of_the_listed_folder_can_be_measured() {
        let mut sizing = listed(Path::new(root()));

        let job = sizing.begin(root().to_owned(), "photos").expect("current folder");
        assert_eq!(job.folder, Path::new(root()).join("photos"));
        drop(job);

        let other = if cfg!(windows) { "C:\\other" } else { "/other" };
        for (path, name) in [
            (other, "photos"),
            (root(), ".."),
            (root(), "a/b"),
            (root(), ""),
            ("relative", "photos"),
        ] {
            let error = sizing.begin(path.to_owned(), name).expect_err(name);
            assert_eq!(error.code(), IpcErrorCode::InvalidInput, "{path} {name}");
        }

        let nested = Path::new(root()).join("photos").to_string_lossy().into_owned();
        assert!(sizing.begin(nested, "2024").is_err());
    }

    #[test]
    fn nothing_can_be_measured_before_a_folder_has_listed() {
        let mut sizing = Sizing::default();

        assert!(sizing.begin(root().to_owned(), "photos").is_err());
    }

    #[test]
    fn a_pane_runs_one_job_the_latest() {
        let mut sizing = listed(Path::new(root()));

        let stops: Vec<_> = (0..1000)
            .map(|_| sizing.begin(root().to_owned(), "photos").expect("job").stop)
            .collect();

        let (last, earlier) = stops.split_last().expect("jobs");
        assert!(earlier.iter().all(|stop| stop.load(Ordering::Relaxed)));
        assert!(!last.load(Ordering::Relaxed));
    }

    #[test]
    fn a_refused_request_leaves_the_running_job_alone() {
        let mut sizing = listed(Path::new(root()));
        let stop = sizing.begin(root().to_owned(), "photos").expect("job").stop;

        assert!(sizing.begin(root().to_owned(), "..").is_err());

        assert!(!stop.load(Ordering::Relaxed));
    }

    #[test]
    fn the_same_folder_keeps_the_job_and_another_stops_it() {
        let mut sizing = listed(Path::new(root()));
        let stop = sizing.begin(root().to_owned(), "photos").expect("job").stop;

        sizing.move_to(Path::new(root()));
        assert!(!stop.load(Ordering::Relaxed), "the same folder listing again");

        let photos = Path::new(root()).join("photos");
        sizing.move_to(&photos);
        assert!(stop.load(Ordering::Relaxed), "another folder");

        let fresh = sizing
            .begin(photos.to_string_lossy().into_owned(), "x")
            .expect("new folder")
            .stop;
        assert!(!fresh.load(Ordering::Relaxed));
        assert!(
            sizing.begin(root().to_owned(), "photos").is_err(),
            "the old folder is out"
        );
    }

    #[test]
    fn a_request_is_refused_while_the_earlier_job_still_has_its_thread() {
        let mut sizing = listed(Path::new(root()));
        let running = sizing.begin(root().to_owned(), "photos").expect("job");

        let error = sizing.begin(root().to_owned(), "other").expect_err("busy");
        assert_eq!(error.code(), IpcErrorCode::Busy);
        assert!(
            !running.stop.load(Ordering::Relaxed),
            "the refused request leaves the job alone"
        );

        // Cancelled (the pane moved on) but stuck on a hung mount: its thread still counts.
        sizing.move_to(&Path::new(root()).join("photos"));
        assert!(running.stop.load(Ordering::Relaxed));
        let nested = Path::new(root()).join("photos").to_string_lossy().into_owned();
        assert!(sizing.begin(nested.clone(), "x").is_err(), "still busy");

        drop(running.permit);
        assert!(sizing.begin(nested, "x").is_ok(), "the thread ended");
    }
}
