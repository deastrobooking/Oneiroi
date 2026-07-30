//! Native audio input and bounded analysis worker.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream};
use oneiroi_core::{AUDIO_ANALYSIS_SIZE, AudioAnalysisSettings, AudioAnalyzer, AudioSnapshot};
use thiserror::Error;

const AUDIO_QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioInputDevice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioInputSnapshot {
    pub analysis: AudioSnapshot,
    pub sample_rate: u32,
    pub channels: u16,
    pub queue_overruns: u64,
    pub callback_errors: u64,
}

#[derive(Debug, Error)]
pub enum AudioInputError {
    #[error("enumerate audio input devices: {0}")]
    Enumerate(String),
    #[error("audio input device `{0}` is unavailable")]
    DeviceUnavailable(String),
    #[error("query default audio input format: {0}")]
    DefaultConfig(String),
    #[error("unsupported audio input sample format {0:?}")]
    UnsupportedFormat(SampleFormat),
    #[error("build audio input stream: {0}")]
    BuildStream(String),
    #[error("start audio input stream: {0}")]
    StartStream(String),
}

#[derive(Clone)]
struct AudioChunk {
    samples: [f32; AUDIO_ANALYSIS_SIZE],
    len: usize,
}

impl Default for AudioChunk {
    fn default() -> Self {
        Self {
            samples: [0.0; AUDIO_ANALYSIS_SIZE],
            len: 0,
        }
    }
}

pub fn discover_audio_inputs() -> Result<Vec<AudioInputDevice>, AudioInputError> {
    Ok(input_devices()?
        .into_iter()
        .map(|(descriptor, _)| descriptor)
        .collect())
}

pub struct AudioInput {
    stream: Option<Stream>,
    analysis: Arc<Mutex<AudioSnapshot>>,
    settings: Arc<Mutex<AudioAnalysisSettings>>,
    queue_overruns: Arc<AtomicU64>,
    callback_errors: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
    worker: Option<JoinHandle<()>>,
}

impl AudioInput {
    pub fn connect(
        device_id: &str,
        settings: AudioAnalysisSettings,
    ) -> Result<Self, AudioInputError> {
        let (_, device) = input_devices()?
            .into_iter()
            .find(|(descriptor, _)| descriptor.id == device_id)
            .ok_or_else(|| AudioInputError::DeviceUnavailable(device_id.to_owned()))?;
        let supported = device
            .default_input_config()
            .map_err(|error| AudioInputError::DefaultConfig(error.to_string()))?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;
        let (sender, receiver) = sync_channel(AUDIO_QUEUE_CAPACITY);
        let queue_overruns = Arc::new(AtomicU64::new(0));
        let callback_errors = Arc::new(AtomicU64::new(0));
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                &config,
                sender,
                queue_overruns.clone(),
                callback_errors.clone(),
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                &config,
                sender,
                queue_overruns.clone(),
                callback_errors.clone(),
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                &config,
                sender,
                queue_overruns.clone(),
                callback_errors.clone(),
            ),
            other => return Err(AudioInputError::UnsupportedFormat(other)),
        }?;
        let analysis = Arc::new(Mutex::new(AudioSnapshot::default()));
        let settings = Arc::new(Mutex::new(settings.sanitized()));
        let worker =
            spawn_analysis_worker(receiver, sample_rate, analysis.clone(), settings.clone());
        stream
            .play()
            .map_err(|error| AudioInputError::StartStream(error.to_string()))?;
        Ok(Self {
            stream: Some(stream),
            analysis,
            settings,
            queue_overruns,
            callback_errors,
            sample_rate,
            channels,
            worker: Some(worker),
        })
    }

    pub fn set_settings(&self, settings: AudioAnalysisSettings) {
        *self.settings.lock().expect("audio settings lock") = settings.sanitized();
    }

    pub fn snapshot(&self) -> AudioInputSnapshot {
        let callback_errors = self.callback_errors.load(Ordering::Relaxed);
        AudioInputSnapshot {
            analysis: if callback_errors == 0 {
                *self.analysis.lock().expect("audio analysis lock")
            } else {
                AudioSnapshot::default()
            },
            sample_rate: self.sample_rate,
            channels: self.channels,
            queue_overruns: self.queue_overruns.load(Ordering::Relaxed),
            callback_errors,
        }
    }
}

impl Drop for AudioInput {
    fn drop(&mut self) {
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn input_devices() -> Result<Vec<(AudioInputDevice, cpal::Device)>, AudioInputError> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let devices = host
        .input_devices()
        .map_err(|error| AudioInputError::Enumerate(error.to_string()))?;
    let mut named_counts = std::collections::HashMap::<String, usize>::new();
    let mut result = Vec::new();
    for device in devices {
        let label = device
            .name()
            .unwrap_or_else(|_| "Unnamed audio input".to_owned());
        let occurrence = named_counts.entry(label.clone()).or_default();
        let id = format!("{label}#{occurrence}");
        *occurrence += 1;
        result.push((
            AudioInputDevice {
                id,
                is_default: default_name.as_deref() == Some(label.as_str()),
                label,
            },
            device,
        ));
    }
    Ok(result)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: SyncSender<AudioChunk>,
    queue_overruns: Arc<AtomicU64>,
    callback_errors: Arc<AtomicU64>,
) -> Result<Stream, AudioInputError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut chunk = AudioChunk::default();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                for frame in data.chunks(channels) {
                    let mono = frame
                        .iter()
                        .map(|sample| sample.to_sample::<f32>())
                        .sum::<f32>()
                        / frame.len().max(1) as f32;
                    chunk.samples[chunk.len] = mono;
                    chunk.len += 1;
                    if chunk.len == AUDIO_ANALYSIS_SIZE {
                        let full = std::mem::take(&mut chunk);
                        if sender.try_send(full).is_err() {
                            queue_overruns.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            },
            move |error| {
                callback_errors.fetch_add(1, Ordering::Relaxed);
                log::error!("audio input callback: {error}");
            },
            None,
        )
        .map_err(|error| AudioInputError::BuildStream(error.to_string()))
}

fn spawn_analysis_worker(
    receiver: Receiver<AudioChunk>,
    sample_rate: u32,
    analysis: Arc<Mutex<AudioSnapshot>>,
    settings: Arc<Mutex<AudioAnalysisSettings>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("oneiroi-audio-analysis".to_owned())
        .spawn(move || {
            let mut analyzer = AudioAnalyzer::new(sample_rate);
            while let Ok(chunk) = receiver.recv() {
                let settings = *settings.lock().expect("audio settings lock");
                let snapshot = analyzer.analyze(&chunk.samples[..chunk.len], settings);
                *analysis.lock().expect("audio analysis lock") = snapshot;
            }
        })
        .expect("spawn audio analysis worker")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_worker_publishes_latest_bounded_chunk() {
        let (sender, receiver) = sync_channel(1);
        let analysis = Arc::new(Mutex::new(AudioSnapshot::default()));
        let settings = Arc::new(Mutex::new(AudioAnalysisSettings {
            noise_floor: 0.0,
            attack_ms: 1.0,
            release_ms: 1.0,
            ..Default::default()
        }));
        let worker = spawn_analysis_worker(receiver, 48_000, analysis.clone(), settings);
        let mut chunk = AudioChunk {
            len: AUDIO_ANALYSIS_SIZE,
            ..Default::default()
        };
        for (index, sample) in chunk.samples.iter_mut().enumerate() {
            *sample = (std::f32::consts::TAU * 1_000.0 * index as f32 / 48_000.0).sin() * 0.8;
        }
        sender.send(chunk).unwrap();
        drop(sender);
        worker.join().unwrap();
        let snapshot = *analysis.lock().unwrap();
        assert!(snapshot.mid > 0.4);
        assert!(snapshot.mid > snapshot.bass * 3.0);
    }
}
