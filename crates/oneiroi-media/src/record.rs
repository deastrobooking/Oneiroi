//! Bounded, asynchronous camera-frame recording.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use ffmpeg_next as ffmpeg;
use oneiroi_core::MediaTime;

use crate::RgbaFrame;

const RECORD_QUEUE_CAPACITY: usize = 8;
// Fine enough for capture jitter without MOV's high-timescale warning.
const RECORD_TIMESCALE: i64 = 60_000;

struct RecordingFrame {
    pixels: RgbaFrame,
    pts: MediaTime,
    duration: Option<MediaTime>,
}

fn recording_ticks(time: MediaTime) -> Result<i64, String> {
    let ticks =
        i128::from(time.ticks()) * i128::from(RECORD_TIMESCALE) / i128::from(time.timescale());
    i64::try_from(ticks).map_err(|_| "recording timestamp overflow".to_owned())
}

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
    sender: Option<SyncSender<RecordingFrame>>,
    completion: Receiver<CameraRecordingResult>,
    worker: Option<JoinHandle<()>>,
    dropped_frames: Arc<AtomicU64>,
    capture_end: Arc<AtomicI64>,
    origin: Option<MediaTime>,
    fallback_duration: MediaTime,
}

impl CameraRecorder {
    pub fn start(path: PathBuf, fps: u32) -> Result<Self, String> {
        let (sender, frames) = mpsc::sync_channel(RECORD_QUEUE_CAPACITY);
        let (finished, completion) = mpsc::channel();
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let worker_drops = Arc::clone(&dropped_frames);
        let fps = fps.clamp(1, 1000);
        let capture_end = Arc::new(AtomicI64::new(0));
        let worker_end = Arc::clone(&capture_end);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("oneiroi-camera-recorder".to_owned())
            .spawn(move || {
                let (frames_written, result) =
                    write_recording(&worker_path, fps, frames, &worker_end);
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
            capture_end,
            origin: None,
            fallback_duration: MediaTime::new(1, i64::from(fps)).expect("positive recording FPS"),
        })
    }

    /// Capture PTS travels with the pixels, including gaps caused by dropped
    /// frames. The requested FPS is only a fallback for the final duration.
    pub fn try_push(&mut self, frame: &RgbaFrame, pts: MediaTime, duration: Option<MediaTime>) {
        let Some(sender) = &self.sender else {
            return;
        };
        let origin = *self.origin.get_or_insert(pts);
        let duration = duration
            .filter(|duration| *duration > MediaTime::ZERO)
            .unwrap_or(self.fallback_duration);
        if let Ok(end) = pts
            .checked_sub(origin)
            .and_then(|relative| relative.checked_add(duration))
            && let Ok(ticks) = recording_ticks(end)
        {
            // Keep the capture endpoint even if this frame cannot enter the
            // queue, so a burst of drops just before Stop does not shorten it.
            self.capture_end.fetch_max(ticks, Ordering::Release);
        }
        match sender.try_send(RecordingFrame {
            pixels: frame.clone(),
            pts,
            duration: Some(duration),
        }) {
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
    frames: Receiver<RecordingFrame>,
    capture_end: &AtomicI64,
) -> (u64, Result<(), String>) {
    let Some(first) = frames.recv().ok() else {
        return (
            0,
            Err("recording stopped before the first camera frame".to_owned()),
        );
    };
    let extent = first.pixels.extent;
    let origin = first.pts;
    let result = (|| {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create recording directory: {error}"))?;
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
            stream.set_time_base((1, RECORD_TIMESCALE as i32));
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
        let mut pending: Option<(ffmpeg::Packet, i64, i64)> = None;
        for frame in std::iter::once(first).chain(frames) {
            if frame.pixels.extent != extent {
                return Err("camera resolution changed during recording".to_owned());
            }
            let relative = frame
                .pts
                .checked_sub(origin)
                .map_err(|error| error.to_string())?;
            let pts = recording_ticks(relative)?;
            if pending
                .as_ref()
                .is_some_and(|(_, previous, _)| pts <= *previous)
            {
                return Err("camera timestamps did not advance during recording".to_owned());
            }
            // Hold the previous image across dropped-frame gaps. Keep one
            // packet of lookahead so the container duration matches capture.
            if let Some((mut packet, previous, _)) = pending.take() {
                packet.set_duration(
                    pts.checked_sub(previous)
                        .ok_or_else(|| "recording duration overflow".to_owned())?,
                );
                packet.rescale_ts((1, RECORD_TIMESCALE as i32), mux_time_base);
                packet
                    .write_interleaved(&mut output)
                    .map_err(|error| format!("write recording frame: {error}"))?;
                count += 1;
            }
            let duration = frame
                .duration
                .filter(|duration| *duration > MediaTime::ZERO)
                .map(recording_ticks)
                .transpose()?
                .unwrap_or(RECORD_TIMESCALE / i64::from(fps))
                .max(1);
            let mut packet = ffmpeg::Packet::copy(&frame.pixels.data);
            packet.set_stream(stream_index);
            packet.set_pts(Some(pts));
            packet.set_dts(Some(pts));
            packet.set_flags(ffmpeg::codec::packet::Flags::KEY);
            pending = Some((packet, pts, duration));
        }
        if let Some((mut packet, pts, duration)) = pending {
            let tail = capture_end.load(Ordering::Acquire).saturating_sub(pts);
            packet.set_duration(duration.max(tail));
            packet.rescale_ts((1, RECORD_TIMESCALE as i32), mux_time_base);
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
        for (index, color) in [[255, 0, 0, 255], [0, 255, 0, 255]].into_iter().enumerate() {
            recorder.try_push(
                &RgbaFrame {
                    extent: [16, 16],
                    data: FrameData::from(color.repeat(16 * 16)),
                },
                MediaTime::new(index as i64, 30).unwrap(),
                Some(MediaTime::new(1, 30).unwrap()),
            );
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

    #[test]
    fn capture_timestamps_preserve_dropped_frame_gaps_and_actual_duration() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("oneiroi-record-timing-{stamp}.mov"));
        // Camera actually delivers 25 fps; the requested fallback rate differs.
        // Frames 2, 3 and 4 were dropped upstream before reaching the recorder.
        let mut recorder = CameraRecorder::start(path.clone(), 60).unwrap();
        for (pts, color) in [(250, 10), (251, 20), (255, 30)] {
            recorder.try_push(
                &RgbaFrame {
                    extent: [16, 16],
                    data: FrameData::from(vec![color; 16 * 16 * 4]),
                },
                MediaTime::new(pts, 25).unwrap(),
                Some(MediaTime::new(1, 25).unwrap()),
            );
        }
        recorder.stop();
        let result = recorder
            .completion
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        result.result.unwrap();
        assert_eq!(result.frames, 3);
        let movie = probe_movie(&path).unwrap();
        assert!((movie.duration.unwrap().as_seconds() - 0.24).abs() < 0.001);
        let mut decoder = crate::FfmpegVideoDecoder::open(&path).unwrap();
        for (pts, color) in [(0, 10), (1, 20), (5, 30)] {
            let frame = decoder.next_frame().unwrap().unwrap();
            assert_eq!(frame.pts, MediaTime::new(pts, 25).unwrap());
            assert_eq!(frame.pixels.data[0], color);
        }
        assert!(decoder.next_frame().unwrap().is_none());
        drop(decoder);
        drop(recorder);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn recording_directory_failure_is_reported_by_the_worker() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!("oneiroi-record-blocked-{stamp}"));
        std::fs::write(&parent, b"not a directory").unwrap();
        let mut recorder = CameraRecorder::start(parent.join("capture.mov"), 30).unwrap();
        recorder.try_push(
            &RgbaFrame {
                extent: [16, 16],
                data: FrameData::from(vec![0; 16 * 16 * 4]),
            },
            MediaTime::ZERO,
            None,
        );
        recorder.stop();
        let result = recorder
            .completion
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            result
                .result
                .unwrap_err()
                .contains("create recording directory")
        );
        drop(recorder);
        std::fs::remove_file(parent).unwrap();
    }

    #[test]
    fn dropped_tail_frames_extend_the_last_image_to_the_capture_endpoint() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("oneiroi-record-tail-{stamp}.mov"));
        // Deliberately don't start the consumer until Stop: capacity one makes
        // every frame after the first deterministically hit backpressure.
        let (sender, frames) = mpsc::sync_channel(1);
        let (_, completion) = mpsc::channel();
        let capture_end = Arc::new(AtomicI64::new(0));
        let mut recorder = CameraRecorder {
            sender: Some(sender),
            completion,
            worker: None,
            dropped_frames: Arc::new(AtomicU64::new(0)),
            capture_end: Arc::clone(&capture_end),
            origin: None,
            fallback_duration: MediaTime::new(1, 25).unwrap(),
        };
        for pts in 250..256 {
            recorder.try_push(
                &RgbaFrame {
                    extent: [16, 16],
                    data: FrameData::from(vec![0; 16 * 16 * 4]),
                },
                MediaTime::new(pts, 25).unwrap(),
                None,
            );
        }
        assert_eq!(recorder.dropped_frames(), 5);
        recorder.stop();
        let (count, result) = write_recording(&path, 25, frames, &capture_end);
        result.unwrap();
        assert_eq!(count, 1);
        let movie = probe_movie(&path).unwrap();
        assert!((movie.duration.unwrap().as_seconds() - 0.24).abs() < 0.001);
        std::fs::remove_file(path).unwrap();
    }
}
