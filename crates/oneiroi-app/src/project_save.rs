//! Serialized project writes. Submission and completion polling never wait on disk.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use oneiroi_io::{ProjectFile, save_project_atomic};

const SAVE_QUEUE_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SaveKind {
    Project,
    Recovery,
}

pub(crate) struct SaveRequest {
    pub epoch: u64,
    pub path: PathBuf,
    pub snapshot: ProjectFile,
    pub kind: SaveKind,
}

pub(crate) struct SaveCompletion {
    pub request: SaveRequest,
    pub result: Result<(), String>,
}

impl SaveCompletion {
    pub fn belongs_to(&self, epoch: u64) -> bool {
        self.request.epoch == epoch
    }
}

pub(crate) struct ProjectSaver {
    requests: Option<SyncSender<SaveRequest>>,
    completions: Option<Receiver<SaveCompletion>>,
    worker: Option<JoinHandle<()>>,
}

impl ProjectSaver {
    pub fn new() -> std::io::Result<Self> {
        Self::spawn(|request| {
            save_project_atomic(&request.path, &request.snapshot).map_err(|error| error.to_string())
        })
    }

    fn spawn(
        mut write: impl FnMut(&SaveRequest) -> Result<(), String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<SaveRequest>(SAVE_QUEUE_CAPACITY);
        let (finished, completions) = mpsc::sync_channel(SAVE_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name("oneiroi-project-save".to_owned())
            .spawn(move || {
                for request in incoming {
                    let result = write(&request);
                    if let Err(error) = &result {
                        log::error!("save {}: {error}", request.path.display());
                    }
                    // Completion backpressure is bounded and stays on this worker.
                    // Shutdown drains completions before joining.
                    let _ = finished.send(SaveCompletion { request, result });
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            completions: Some(completions),
            worker: Some(worker),
        })
    }

    pub fn submit(&self, request: SaveRequest) -> Result<(), &'static str> {
        let Some(requests) = &self.requests else {
            return Err("Project save worker has stopped.");
        };
        match requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("Save queue is busy; try again shortly."),
            Err(TrySendError::Disconnected(_)) => Err("Project save worker is unavailable."),
        }
    }

    pub fn try_recv(&self) -> Option<SaveCompletion> {
        self.completions.as_ref()?.try_recv().ok()
    }

    /// Only used once presentation has stopped. Drain accepted saves before
    /// returning so close-time recovery cannot be lost to a full queue.
    pub fn finish(&mut self) -> Vec<SaveCompletion> {
        self.requests.take();
        let completed = self
            .completions
            .take()
            .map(|receiver| receiver.into_iter().collect())
            .unwrap_or_default();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        completed
    }
}

impl Drop for ProjectSaver {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    fn request(epoch: u64) -> SaveRequest {
        SaveRequest {
            epoch,
            path: "unused.oneiroi".into(),
            snapshot: ProjectFile::default(),
            kind: SaveKind::Recovery,
        }
    }

    #[test]
    fn stalled_disk_does_not_block_submission_and_queue_is_bounded() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let (written, writes) = mpsc::channel();
        let mut first = true;
        let mut saver = ProjectSaver::spawn(move |request| {
            if first {
                first = false;
                entered.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            written.send(request.epoch).unwrap();
            Ok(())
        })
        .unwrap();
        saver.submit(request(0)).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        for epoch in 1..=SAVE_QUEUE_CAPACITY as u64 {
            saver.submit(request(epoch)).unwrap();
        }
        assert!(saver.submit(request(99)).is_err());
        assert!(saver.try_recv().is_none());
        release.send(()).unwrap();
        saver.finish();
        assert_eq!(writes.try_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn completions_report_failures_and_identify_stale_projects() {
        let saver = ProjectSaver::spawn(|_| Err("disk unavailable".to_owned())).unwrap();
        saver.submit(request(7)).unwrap();
        let completion = saver
            .completions
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(completion.belongs_to(7));
        assert!(!completion.belongs_to(8));
        assert_eq!(completion.result, Err("disk unavailable".to_owned()));
    }

    #[test]
    fn shutdown_flushes_accepted_snapshots_in_order() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("oneiroi-save-{stamp}"));
        let path = directory.join("show.oneiroi");
        let saver = ProjectSaver::new().unwrap();
        let mut last = ProjectFile::default();
        for bpm in [110.0, 125.0, 140.0] {
            last.settings.bpm = bpm;
            saver
                .submit(SaveRequest {
                    epoch: 1,
                    path: path.clone(),
                    snapshot: last.clone(),
                    kind: SaveKind::Project,
                })
                .unwrap();
        }
        drop(saver);
        assert_eq!(oneiroi_io::load_project(&path).unwrap(), last);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
