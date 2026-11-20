//! The drive list: `volumes_list` sends the places the drive menu offers, then their free space as it is measured.
//!
//! No path comes from the webview. Free space is the one thing that can block (a volume that does not answer holds
//! its thread for good, and cancelling cannot stop it), so it is measured on a blocking thread, one volume at a time
//! in list order, and only while the app has fewer than [`THREADS`] such threads outstanding; a volume list that
//! reaches the OS on every call (Windows) is built on that thread as well. Network volumes are never measured (see
//! `pairpane_core::volumes`). When all the places are taken the list is still sent without free space (where
//! listing cannot block) or the call is refused as busy and the page keeps its last list.

use pairpane_core::model::{IpcErrorCode, VolumeEvent};
use pairpane_core::volumes::{self, Found};
use tauri::State;
use tauri::async_runtime;
use tauri::ipc::Channel;

use super::limit::{Permit, ThreadCap};
use crate::error::{CommandError, CommandResult};

/// The app's volume threads.
#[derive(Default)]
pub struct Volumes {
    threads: ThreadCap,
}

/// Volume threads the app may have at once: a stuck one is stepped around by the other, and two stuck ones stop the
/// measuring (the lists still come) without ever taking more of the thread pool.
const THREADS: usize = 2;

/// How one `volumes_list` runs.
#[derive(Debug)]
enum Run {
    /// On a blocking thread (list and free space), holding its place until it ends.
    Thread(Permit),
    /// The list only, on the command's own thread, with no free space.
    ListOnly,
}

impl Volumes {
    /// Decides how a request runs. `listing_may_block`: building the list can wait on the OS, so it needs a thread.
    fn begin(&self, listing_may_block: bool) -> CommandResult<Run> {
        match self.threads.acquire(THREADS) {
            Some(permit) => Ok(Run::Thread(permit)),
            None if listing_may_block => Err(CommandError::new(
                IpcErrorCode::Busy,
                "the drives are not responding; try again shortly",
            )),
            None => Ok(Run::ListOnly),
        }
    }
}

/// Sends the places to offer on `on_event` as one `list`, then a `free` for each volume whose free space is
/// measured.
#[tauri::command]
pub async fn volumes_list(volumes: State<'_, Volumes>, on_event: Channel<VolumeEvent>) -> CommandResult<()> {
    let home = std::env::home_dir();

    match volumes.begin(volumes::LISTING_MAY_BLOCK)? {
        Run::Thread(permit) => {
            async_runtime::spawn_blocking(move || {
                let found = volumes::list(home.as_deref());
                if send_list(&found, &on_event) {
                    volumes::measure(&found, |path, bytes| send_free(&on_event, path, bytes));
                }

                // Free before the thread ends, so a request made on seeing the last event is not refused.
                drop(permit);
            });
        }
        Run::ListOnly => {
            send_list(&volumes::list(home.as_deref()), &on_event);
        }
    }

    Ok(())
}

/// Sends the list; false when the page is gone.
fn send_list(found: &[Found], on_event: &Channel<VolumeEvent>) -> bool {
    let volumes = found.iter().map(|found| found.volume.clone()).collect();

    on_event.send(VolumeEvent::List { volumes }).is_ok()
}

/// Sends one volume's free space; false when the page is gone.
fn send_free(on_event: &Channel<VolumeEvent>, path: &str, bytes: u64) -> bool {
    let event = VolumeEvent::Free {
        path: path.to_owned(),
        bytes,
    };

    on_event.send(event).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Takes every place the app has.
    fn take_all(volumes: &Volumes) -> Vec<Permit> {
        (0..THREADS)
            .map(|_| match volumes.begin(true).expect("a free place") {
                Run::Thread(permit) => permit,
                Run::ListOnly => panic!("a place was free"),
            })
            .collect()
    }

    #[test]
    fn a_request_gets_a_thread_while_a_place_is_free() {
        let volumes = Volumes::default();

        assert!(matches!(volumes.begin(true), Ok(Run::Thread(_))));
        assert!(matches!(volumes.begin(false), Ok(Run::Thread(_))));
    }

    #[test]
    fn with_every_place_taken_a_list_that_cannot_block_is_still_sent_without_free_space() {
        let volumes = Volumes::default();
        let _taken = take_all(&volumes);

        assert!(matches!(volumes.begin(false), Ok(Run::ListOnly)));
    }

    #[test]
    fn with_every_place_taken_a_list_that_can_block_is_refused_as_busy() {
        let volumes = Volumes::default();
        let _taken = take_all(&volumes);

        let error = volumes.begin(true).expect_err("busy");

        assert_eq!(error.code(), IpcErrorCode::Busy);
    }

    #[test]
    fn a_place_is_free_again_when_its_thread_ends() {
        let volumes = Volumes::default();
        let mut taken = take_all(&volumes);

        drop(taken.pop());

        assert!(matches!(volumes.begin(true), Ok(Run::Thread(_))));
    }
}
