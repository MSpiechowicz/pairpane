//! One job, shared by the queue, its worker and the callers that pause, cancel and answer it.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use crate::model::{JobId, JobPhase, JobView, OpsEvent};

use super::control::{Control, locked};
use super::{Request, Task};

/// The events of a job that threads other than its worker send, waiting for the thread that is sending them (see
/// `Inner::send`).
#[derive(Default)]
pub(super) struct Outbox {
    pub(super) events: VecDeque<OpsEvent>,
    /// A thread is emptying the outbox right now.
    pub(super) sending: bool,
    /// The job's `ended` went in: nothing else may.
    pub(super) closed: bool,
}

/// One job, shared by the queue, its worker and the callers that pause, cancel and answer it.
pub(super) struct Job {
    pub(super) id: JobId,
    pub(super) request: Request,
    pub(super) control: Control,
    view: Mutex<JobView>,
    /// Someone is telling the listener about a pause or a resume right now (see `Inner::announce_paused`).
    pub(super) announcing: Mutex<bool>,
    pub(super) outbox: Mutex<Outbox>,
}

impl Job {
    pub(super) fn new(id: JobId, request: Request) -> Self {
        let label = match request.names.as_slice() {
            [only] => only.clone(),
            names => format!("{} items", names.len()),
        };
        let view = JobView {
            id,
            kind: request.kind,
            label,
            from: request.from.to_string_lossy().into_owned(),
            to: shown_target(&request),
            items: u32::try_from(request.names.len()).unwrap_or(u32::MAX),
            verify: request.verify,
            phase: JobPhase::Queued,
            paused: false,
            done_bytes: 0,
            total_bytes: 0,
            done_files: 0,
            total_files: 0,
            current: None,
            bytes_per_sec: 0,
            eta_secs: None,
        };

        Self {
            id,
            request,
            control: Control::default(),
            view: Mutex::new(view),
            announcing: Mutex::new(false),
            outbox: Mutex::new(Outbox::default()),
        }
    }

    pub(super) fn view(&self) -> MutexGuard<'_, JobView> {
        locked(&self.view)
    }
}

/// Where the job writes, as the queue shows it: the archive itself for a pack or an add to an archive (whose request
/// names the folder that holds it), the destination folder otherwise.
fn shown_target(request: &Request) -> String {
    match &request.task {
        Task::Pack { name, .. } | Task::AddToArchive { name, .. } => {
            request.to.join(name).to_string_lossy().into_owned()
        }
        _ => request.to.to_string_lossy().into_owned(),
    }
}
