//! Bounded, asynchronous camera-frame recording.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use ffmpeg_next as ffmpeg;

use crate::RgbaFrame;

const RECORD_QUEUE_CAPACITY: usize = 8;

#[derive(Debug)]
pub struct CameraRecordingResult {
    pub path: PathBuf,
    pub frames: u64,
    pub dropped_frames: u64,
    pub result: Result<(), String>,
}

/// Sends cheap, reference-counted RGBA frame handles to a dedicated muxer
/// thread. A full queue drops the incoming frame instead of stalling render.
pub struct CameraRecorder {
    sender: Option<SyncSender<RgbaFrame>>,
    completion: Receiver<CameraRecordingResult>,
    worker: Option<JoinHandle<()>>,
    dropped_frames: Arc<AtomicU64>,
}

impl CameraRecorder {
    pub fn start(path: PathBuf, fps: u32) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "recording path has no parent directory".to_owned())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create recording directory: {error}"))?;
        let (sender, frames) = mpsc::sync_channel(RECORD_QUEUE_CAPACITY);
        let (finished, completion) = mpsc::channel();
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let worker_drops = Arc::clone(&dropped_frames);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("oneiroi-camera-recorder".to_owned())
            .spawn(move || {
                let (frames_written, result) = write_recording(&worker_path, fps.max(1), frames);
                let _ = finished.send(CameraRecordingResult {
                    path: worker_path,
                    frames: frames_written,
                    dropped_frames: worker_drops.load(Ordering::Relaxed),
                    result,
                });
            })
            .map_err(|error| format!("start camera recorder: {error}"))?;
        Ok(Self {
            sender: Some(sender),
            completion,
            worker: Some(worker),
            dropped_frames,
        })
    }

    pub fn try_push(&self, frame: &RgbaFrame) {
        let Some(sender) = &self.sender else {
            return;
        };
        match sender.try_send(frame.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn stop(&mut self) {
        self.sender.take();
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    pub fn try_finish(&mut self) -> Option<CameraRecordingResult> {
        match self.completion.try_recv() {
            Ok(result) => {
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                Some(result)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for CameraRecorder {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_recording(
    path: &Path,
    fps: u32,
    frames: Receiver<RgbaFrame>,
) -> (u64, Result<(), String>) {
    let Some(first) = frames.recv().ok() else {
        return (
            0,
            Err("recording stopped before the first camera frame".to_owned()),
        );
    };
    let extent = first.extent;
    let result = (|| {
        ffmpeg::init().map_err(|error| format!("initialize FFmpeg: {error}"))?;
        let mut output = ffmpeg::format::output_as(path, "mov")
            .map_err(|error| format!("create recording: {error}"))?;
        let mut parameters = ffmpeg::codec::Parameters::new();
        // SAFETY: `parameters` uniquely owns the allocated AVCodecParameters.
        unsafe {
            let parameters = &mut *parameters.as_mut_ptr();
            parameters.codec_type = ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            parameters.codec_id = ffmpeg::ffi::AVCodecID::AV_CODEC_ID_RAWVIDEO;
            parameters.codec_tag = u32::from_le_bytes(*b"RGBA");
            parameters.format = ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i32;
            parameters.width = extent[0] as i32;
            parameters.height = extent[1] as i32;
        }
        let stream_index = {
            let mut stream = output
                .add_stream(ffmpeg::codec::Id::RAWVIDEO)
                .map_err(|error| format!("add recording stream: {error}"))?;
            stream.set_parameters(parameters);
            stream.set_time_base((1, fps as i32));
            stream.set_rate((fps as i32, 1));
            stream.set_avg_frame_rate((fps as i32, 1));
            stream.index()
        };
        output
            .write_header()
            .map_err(|error| format!("write recording header: {error}"))?;
        let mux_time_base = output
            .stream(stream_index)
            .expect("new recording stream exists")
            .time_base();
        let mut count = 0_u64;
        for frame in std::iter::once(first).chain(frames) {
            if frame.extent != extent {
                continue;
            }
            let mut packet = ffmpeg::Packet::copy(&frame.data);
            packet.set_stream(stream_index);
            packet.set_pts(Some(count as i64));
            packet.set_dts(Some(count as i64));
            packet.set_duration(1);
            packet.set_flags(ffmpeg::codec::packet::Flags::KEY);
            packet.rescale_ts((1, fps as i32), mux_time_base);
            packet
                .write_interleaved(&mut output)
                .map_err(|error| format!("write recording frame: {error}"))?;
            count += 1;
        }
        output
            .write_trailer()
            .map_err(|error| format!("finalize recording: {error}"))?;
        Ok(count)
    })();
    match result {
        Ok(count) => (count, Ok(())),
        Err(error) => (0, Err(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{FrameData, probe_movie};

    #[test]
    fn records_rgba_frames_into_a_probeable_movie() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oneiroi-camera-recording-{}-{stamp}.mov",
            std::process::id()
        ));
        let mut recorder = CameraRecorder::start(path.clone(), 30).unwrap();
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            recorder.try_push(&RgbaFrame {
                extent: [16, 16],
                data: FrameData::from(color.repeat(16 * 16)),
            });
        }
        recorder.stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = recorder.try_finish() {
                break result;
            }
            assert!(Instant::now() < deadline, "recording did not finalize");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.frames, 2);
        result.result.unwrap();
        let movie = probe_movie(&path).unwrap();
        assert_eq!(movie.visible_extent, [16, 16]);
        let _ = std::fs::remove_file(path);
    }
}
