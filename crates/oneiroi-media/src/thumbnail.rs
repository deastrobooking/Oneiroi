//! Bounded, playback-independent thumbnail generation.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{ClipAddress, FfmpegVideoDecoder, RgbaFrame};

pub const THUMBNAIL_MAX_EXTENT: [u32; 2] = [160, 90];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumbnail {
    pub extent: [u32; 2],
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub struct ThumbnailRequest {
    pub address: ClipAddress,
    pub path: PathBuf,
    pub request_id: u64,
}

#[derive(Debug)]
pub struct ThumbnailResult {
    pub address: ClipAddress,
    pub path: PathBuf,
    pub request_id: u64,
    pub thumbnail: Result<Thumbnail, String>,
}

enum ThumbnailCommand {
    Generate(ThumbnailRequest),
    Shutdown,
}

pub struct ThumbnailWorker {
    commands: SyncSender<ThumbnailCommand>,
    results: Receiver<ThumbnailResult>,
    worker: Option<JoinHandle<()>>,
}

impl ThumbnailWorker {
    pub fn new(queue_capacity: usize) -> Self {
        let (commands_tx, commands_rx) = mpsc::sync_channel(queue_capacity.max(1));
        let (results_tx, results_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("oneiroi-thumbnail".to_owned())
            .spawn(move || {
                while let Ok(command) = commands_rx.recv() {
                    match command {
                        ThumbnailCommand::Generate(request) => {
                            let thumbnail =
                                generate(&request.path).map_err(|error| error.to_string());
                            if results_tx
                                .send(ThumbnailResult {
                                    address: request.address,
                                    path: request.path,
                                    request_id: request.request_id,
                                    thumbnail,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        ThumbnailCommand::Shutdown => break,
                    }
                }
            })
            .expect("spawn thumbnail worker");
        Self {
            commands: commands_tx,
            results: results_rx,
            worker: Some(worker),
        }
    }

    pub fn submit(&self, request: ThumbnailRequest) -> Result<(), ThumbnailRequest> {
        match self.commands.try_send(ThumbnailCommand::Generate(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(ThumbnailCommand::Generate(request)))
            | Err(TrySendError::Disconnected(ThumbnailCommand::Generate(request))) => Err(request),
            Err(_) => unreachable!("only thumbnail requests are submitted"),
        }
    }

    pub fn try_recv(&self) -> Result<ThumbnailResult, TryRecvError> {
        self.results.try_recv()
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ThumbnailResult, mpsc::RecvTimeoutError> {
        self.results.recv_timeout(timeout)
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(ThumbnailCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn generate(path: &std::path::Path) -> Result<Thumbnail, crate::FfmpegDecodeError> {
    let mut decoder = FfmpegVideoDecoder::open_for_thumbnail(path)?;
    let frame = decoder
        .next_frame()?
        .ok_or(crate::FfmpegDecodeError::MissingThumbnailFrame)?;
    Ok(scale_to_fit(&frame.pixels, THUMBNAIL_MAX_EXTENT))
}

fn scale_to_fit(source: &RgbaFrame, maximum: [u32; 2]) -> Thumbnail {
    let [source_width, source_height] = source.extent;
    let scale = (maximum[0] as f64 / source_width as f64)
        .min(maximum[1] as f64 / source_height as f64)
        .min(1.0);
    let width = (source_width as f64 * scale).round().max(1.0) as u32;
    let height = (source_height as f64 * scale).round().max(1.0) as u32;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        let source_y = (u64::from(y) * u64::from(source_height) / u64::from(height)) as usize;
        for x in 0..width {
            let source_x = (u64::from(x) * u64::from(source_width) / u64::from(width)) as usize;
            let source_offset = (source_y * source_width as usize + source_x) * 4;
            let target_offset = (y as usize * width as usize + x as usize) * 4;
            rgba[target_offset..target_offset + 4]
                .copy_from_slice(&source.data[source_offset..source_offset + 4]);
        }
    }
    Thumbnail {
        extent: [width, height],
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_wide_frame_to_bounded_aspect_ratio() {
        let source = RgbaFrame {
            extent: [320, 180],
            data: [10, 20, 30, 255].repeat(320 * 180),
        };
        let thumbnail = scale_to_fit(&source, [160, 90]);
        assert_eq!(thumbnail.extent, [160, 90]);
        assert_eq!(thumbnail.rgba.len(), 160 * 90 * 4);
        assert_eq!(&thumbnail.rgba[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn does_not_upscale_small_images() {
        let source = RgbaFrame {
            extent: [2, 1],
            data: vec![255; 8],
        };
        assert_eq!(scale_to_fit(&source, [160, 90]).extent, [2, 1]);
    }
}
