//! Bounded, deterministic background folder scanning.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_SCAN_DEPTH: usize = 16;
const MAX_ENTRIES_PER_DIRECTORY: usize = 4_096;

#[derive(Clone, Debug)]
pub struct FolderScanRequest {
    pub root: PathBuf,
    pub request_id: u64,
    pub project_epoch: u64,
    pub max_files: usize,
}

#[derive(Debug)]
pub struct FolderScanResult {
    pub root: PathBuf,
    pub request_id: u64,
    pub project_epoch: u64,
    pub paths: Result<Vec<PathBuf>, String>,
    pub truncated: bool,
}

enum FolderScanCommand {
    Scan(FolderScanRequest),
    Shutdown,
}

pub struct FolderScanner {
    commands: SyncSender<FolderScanCommand>,
    results: Receiver<FolderScanResult>,
    worker: Option<JoinHandle<()>>,
}

impl FolderScanner {
    pub fn new() -> Self {
        let (commands_tx, commands_rx) = mpsc::sync_channel(1);
        let (results_tx, results_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("oneiroi-folder-scan".to_owned())
            .spawn(move || {
                while let Ok(command) = commands_rx.recv() {
                    match command {
                        FolderScanCommand::Scan(request) => {
                            let mut truncated = false;
                            let paths =
                                scan_folder(&request.root, request.max_files, &mut truncated)
                                    .map_err(|error| error.to_string());
                            if results_tx
                                .send(FolderScanResult {
                                    root: request.root,
                                    request_id: request.request_id,
                                    project_epoch: request.project_epoch,
                                    paths,
                                    truncated,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        FolderScanCommand::Shutdown => break,
                    }
                }
            })
            .expect("spawn folder scanner");
        Self {
            commands: commands_tx,
            results: results_rx,
            worker: Some(worker),
        }
    }

    pub fn submit(&self, request: FolderScanRequest) -> Result<(), FolderScanRequest> {
        match self.commands.try_send(FolderScanCommand::Scan(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(FolderScanCommand::Scan(request)))
            | Err(TrySendError::Disconnected(FolderScanCommand::Scan(request))) => Err(request),
            Err(_) => unreachable!("only scan requests are submitted"),
        }
    }

    pub fn try_recv(&self) -> Result<FolderScanResult, TryRecvError> {
        self.results.try_recv()
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<FolderScanResult, mpsc::RecvTimeoutError> {
        self.results.recv_timeout(timeout)
    }
}

impl Default for FolderScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FolderScanner {
    fn drop(&mut self) {
        let _ = self.commands.send(FolderScanCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn scan_folder(
    root: &Path,
    max_files: usize,
    truncated: &mut bool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(max_files.min(32));
    scan_directory(root, 0, max_files, &mut paths, truncated)?;
    paths.sort();
    paths.truncate(max_files);
    Ok(paths)
}

fn scan_directory(
    directory: &Path,
    depth: usize,
    max_files: usize,
    paths: &mut Vec<PathBuf>,
    truncated: &mut bool,
) -> std::io::Result<()> {
    if paths.len() >= max_files {
        *truncated = true;
        return Ok(());
    }
    if depth > MAX_SCAN_DEPTH {
        *truncated = true;
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        if entries.len() >= MAX_ENTRIES_PER_DIRECTORY {
            *truncated = true;
            break;
        }
        entries.push(entry?);
    }
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        if paths.len() >= max_files {
            *truncated = true;
            break;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_directory(&entry.path(), depth + 1, max_files, paths, truncated)?;
        } else if file_type.is_file() && is_supported_media_path(&entry.path()) {
            paths.push(entry.path());
        }
    }
    Ok(())
}

pub fn is_supported_media_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mov" | "mp4" | "m4v" | "mkv" | "avi" | "webm" | "mxf" | "png" | "jpg" | "jpeg"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("oneiroi-folder-scan-{}-{id}", std::process::id()));
            fs::create_dir_all(path.join("nested")).unwrap();
            fs::write(path.join("z.mov"), []).unwrap();
            fs::write(path.join("a.txt"), []).unwrap();
            fs::write(path.join("nested").join("b.MP4"), []).unwrap();
            fs::write(path.join("nested").join("a.png"), []).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recursively_filters_and_sorts_supported_media() {
        let directory = TestDirectory::new();
        let mut truncated = false;
        let paths = scan_folder(&directory.0, 32, &mut truncated).unwrap();
        let relative: Vec<_> = paths
            .iter()
            .map(|path| path.strip_prefix(&directory.0).unwrap().to_path_buf())
            .collect();
        assert_eq!(
            relative,
            [
                PathBuf::from("nested/a.png"),
                PathBuf::from("nested/b.MP4"),
                PathBuf::from("z.mov")
            ]
        );
        assert!(!truncated);
    }

    #[test]
    fn file_limit_marks_a_scan_truncated() {
        let directory = TestDirectory::new();
        let mut truncated = false;
        let paths = scan_folder(&directory.0, 2, &mut truncated).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(truncated);
    }

    #[test]
    fn worker_preserves_request_and_project_epochs() {
        let directory = TestDirectory::new();
        let scanner = FolderScanner::new();
        scanner
            .submit(FolderScanRequest {
                root: directory.0.clone(),
                request_id: 41,
                project_epoch: 9,
                max_files: 1,
            })
            .unwrap();
        let result = scanner.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(result.request_id, 41);
        assert_eq!(result.project_epoch, 9);
        assert_eq!(result.paths.unwrap().len(), 1);
        assert!(result.truncated);
    }
}
