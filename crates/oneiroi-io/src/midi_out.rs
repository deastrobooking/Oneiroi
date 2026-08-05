//! Non-blocking platform MIDI output, and a dedicated beat-clock sender.
//!
//! Clock pulses are emitted from their own thread rather than the render loop.
//! At 120 BPM a pulse is due every 20.8 ms; a 60 Hz frame can only place one
//! every 16.7 ms, so frame-driven pulses would arrive in clumps and every
//! synced device downstream would hear the frame rate as swing. The thread
//! sleeps in short steps against the generator's own schedule instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use midir::{MidiOutput, MidiOutputPort};
use oneiroi_core::MidiClockGenerator;

/// Coarsest sleep the sender takes while clocking. One pulse at the fastest
/// supported tempo is 6.25 ms, so a millisecond step never misses one and
/// costs about a thousand wakeups a second.
const MAX_SLEEP: Duration = Duration::from_millis(1);

/// Sleep while idle. Nothing is due, so this only bounds command latency.
const IDLE_SLEEP: Duration = Duration::from_millis(5);

/// A pulse this far past its deadline is counted as late for diagnostics.
const LATE_THRESHOLD_MICROS: u64 = 2_000;

const CLOCK: u8 = 0xf8;
const START: u8 = 0xfa;
const CONTINUE: u8 = 0xfb;
const STOP: u8 = 0xfc;
const SONG_POSITION: u8 = 0xf2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MidiOutputDevice {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MidiOutputStats {
    /// Clock pulses handed to the platform.
    pub pulses: u64,
    /// Transport and position messages sent.
    pub transport: u64,
    pub errors: u64,
    /// Pulses emitted more than two milliseconds behind schedule.
    pub late: u64,
    /// Times the schedule was re-based after the thread lost the CPU.
    pub resyncs: u64,
    /// Worst observed lateness, in microseconds.
    pub worst_late_micros: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum MidiOutputError {
    #[error("could not initialize MIDI output: {0}")]
    Initialize(String),
    #[error("MIDI output device is no longer available: {0}")]
    DeviceMissing(String),
    #[error("could not connect MIDI output: {0}")]
    Connect(String),
}

pub fn discover_midi_outputs() -> Result<Vec<MidiOutputDevice>, MidiOutputError> {
    let output = MidiOutput::new("oneiroi-discovery")
        .map_err(|error| MidiOutputError::Initialize(error.to_string()))?;
    Ok(describe_ports(&output)?
        .into_iter()
        .map(|(device, _)| device)
        .collect())
}

/// What the sender thread is asked to do.
#[derive(Clone, Copy, Debug)]
enum ClockCommand {
    SetBpm(f64),
    Start,
    Continue,
    Stop,
    SongPosition(u16),
    Shutdown,
}

/// Owns a MIDI output port and clocks it from a dedicated thread.
pub struct MidiClockSender {
    device_id: String,
    commands: Sender<ClockCommand>,
    running: Arc<AtomicU64>,
    pulses: Arc<AtomicU64>,
    transport: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    late: Arc<AtomicU64>,
    resyncs: Arc<AtomicU64>,
    worst_late_micros: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl MidiClockSender {
    /// Open `device_id` and start the sender thread, stopped.
    ///
    /// The port is opened on the thread that will use it: the platform
    /// connection is not required to be `Send`, and moving it would tie this
    /// to one backend's guarantees.
    pub fn connect(device_id: &str, bpm: f64) -> Result<Self, MidiOutputError> {
        let (commands, command_receiver) = channel();
        let (ready, ready_receiver) = channel();
        let counters: [Arc<AtomicU64>; 7] = std::array::from_fn(|_| Arc::new(AtomicU64::new(0)));
        let [
            running,
            pulses,
            transport,
            errors,
            late,
            resyncs,
            worst_late_micros,
        ] = counters;
        let thread_counters = Counters {
            running: running.clone(),
            pulses: pulses.clone(),
            transport: transport.clone(),
            errors: errors.clone(),
            late: late.clone(),
            resyncs: resyncs.clone(),
            worst_late_micros: worst_late_micros.clone(),
        };
        let owned_id = device_id.to_owned();
        let thread = thread::Builder::new()
            .name("oneiroi-midi-clock".to_owned())
            .spawn(move || {
                let connection = match open_port(&owned_id) {
                    Ok(connection) => {
                        let _ = ready.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                run(connection, command_receiver, thread_counters, bpm);
            })
            .map_err(|error| MidiOutputError::Initialize(error.to_string()))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                device_id: device_id.to_owned(),
                commands,
                running,
                pulses,
                transport,
                errors,
                late,
                resyncs,
                worst_late_micros,
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(error),
            // The thread died before reporting; join to surface nothing worse
            // than a generic failure rather than leaking the handle.
            Err(_) => {
                let _ = thread.join();
                Err(MidiOutputError::Connect(
                    "sender thread stopped before the port opened".to_owned(),
                ))
            }
        }
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Whether clock pulses are currently being emitted.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed) != 0
    }

    pub fn set_bpm(&self, bpm: f64) {
        self.send(ClockCommand::SetBpm(bpm));
    }

    /// Send Start (rewind downstream to the top) and begin clocking.
    pub fn start(&self) {
        self.send(ClockCommand::Start);
    }

    /// Send Continue (resume in place) and begin clocking.
    pub fn continue_transport(&self) {
        self.send(ClockCommand::Continue);
    }

    pub fn stop(&self) {
        self.send(ClockCommand::Stop);
    }

    /// Address a position in sixteenth notes from the top of the song.
    pub fn send_song_position(&self, sixteenths: u16) {
        self.send(ClockCommand::SongPosition(sixteenths));
    }

    pub fn stats(&self) -> MidiOutputStats {
        MidiOutputStats {
            pulses: self.pulses.load(Ordering::Relaxed),
            transport: self.transport.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            late: self.late.load(Ordering::Relaxed),
            resyncs: self.resyncs.load(Ordering::Relaxed),
            worst_late_micros: self.worst_late_micros.load(Ordering::Relaxed),
        }
    }

    fn send(&self, command: ClockCommand) {
        // A dead thread means the port is gone; the operator sees it in the
        // stats and reconnects. Dropping the command is the right failure.
        let _ = self.commands.send(command);
    }
}

impl Drop for MidiClockSender {
    fn drop(&mut self) {
        // Leaving a downstream device clocked by a closed port would strand
        // it running forever, so the thread sends Stop on its way out.
        let _ = self.commands.send(ClockCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Counters {
    running: Arc<AtomicU64>,
    pulses: Arc<AtomicU64>,
    transport: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    late: Arc<AtomicU64>,
    resyncs: Arc<AtomicU64>,
    worst_late_micros: Arc<AtomicU64>,
}

fn open_port(device_id: &str) -> Result<midir::MidiOutputConnection, MidiOutputError> {
    let output = MidiOutput::new("oneiroi-clock")
        .map_err(|error| MidiOutputError::Initialize(error.to_string()))?;
    let Some((_, port)) = describe_ports(&output)?
        .into_iter()
        .find(|(device, _)| device.id == device_id)
    else {
        return Err(MidiOutputError::DeviceMissing(device_id.to_owned()));
    };
    output
        .connect(&port, "oneiroi-clock")
        .map_err(|error| MidiOutputError::Connect(error.to_string()))
}

fn run(
    mut connection: midir::MidiOutputConnection,
    commands: Receiver<ClockCommand>,
    counters: Counters,
    bpm: f64,
) {
    let origin = Instant::now();
    let mut generator = MidiClockGenerator::new(bpm);
    let emit = |connection: &mut midir::MidiOutputConnection, bytes: &[u8]| {
        if connection.send(bytes).is_err() {
            counters.errors.fetch_add(1, Ordering::Relaxed);
        }
    };
    loop {
        let mut shutdown = false;
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    let seconds = origin.elapsed().as_secs_f64();
                    match command {
                        ClockCommand::SetBpm(bpm) => generator.set_bpm(bpm, seconds),
                        ClockCommand::Start => {
                            emit(&mut connection, &[START]);
                            counters.transport.fetch_add(1, Ordering::Relaxed);
                            generator.start(seconds);
                            counters.running.store(1, Ordering::Relaxed);
                        }
                        ClockCommand::Continue => {
                            emit(&mut connection, &[CONTINUE]);
                            counters.transport.fetch_add(1, Ordering::Relaxed);
                            generator.start(seconds);
                            counters.running.store(1, Ordering::Relaxed);
                        }
                        ClockCommand::Stop => {
                            generator.stop();
                            counters.running.store(0, Ordering::Relaxed);
                            emit(&mut connection, &[STOP]);
                            counters.transport.fetch_add(1, Ordering::Relaxed);
                        }
                        ClockCommand::SongPosition(sixteenths) => {
                            let value = sixteenths.min(0x3fff);
                            emit(
                                &mut connection,
                                &[
                                    SONG_POSITION,
                                    (value & 0x7f) as u8,
                                    ((value >> 7) & 0x7f) as u8,
                                ],
                            );
                            counters.transport.fetch_add(1, Ordering::Relaxed);
                        }
                        ClockCommand::Shutdown => shutdown = true,
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
            }
        }
        if shutdown {
            if generator.is_running() {
                emit(&mut connection, &[STOP]);
                counters.transport.fetch_add(1, Ordering::Relaxed);
                counters.running.store(0, Ordering::Relaxed);
            }
            return;
        }

        let seconds = origin.elapsed().as_secs_f64();
        let deadline = generator.next_pulse_seconds();
        let resyncs_before = generator.resyncs();
        let due = generator.pulses_due(seconds);
        if due > 0 {
            if let Some(deadline) = deadline {
                let lateness = ((seconds - deadline) * 1_000_000.0).max(0.0) as u64;
                if lateness >= LATE_THRESHOLD_MICROS {
                    counters.late.fetch_add(1, Ordering::Relaxed);
                }
                counters
                    .worst_late_micros
                    .fetch_max(lateness, Ordering::Relaxed);
            }
            for _ in 0..due {
                emit(&mut connection, &[CLOCK]);
            }
            counters.pulses.fetch_add(u64::from(due), Ordering::Relaxed);
            counters.resyncs.fetch_add(
                generator.resyncs().saturating_sub(resyncs_before),
                Ordering::Relaxed,
            );
        }

        let sleep = match generator.next_pulse_seconds() {
            Some(next) => {
                let remaining = next - origin.elapsed().as_secs_f64();
                if remaining <= 0.0 {
                    continue;
                }
                Duration::from_secs_f64(remaining).min(MAX_SLEEP)
            }
            None => IDLE_SLEEP,
        };
        thread::sleep(sleep);
    }
}

fn describe_ports(
    output: &MidiOutput,
) -> Result<Vec<(MidiOutputDevice, MidiOutputPort)>, MidiOutputError> {
    // Two identical controllers report identical port names; the suffix keeps
    // saved projects pointing at the same one across restarts, matching how
    // input devices are identified.
    let mut labels = std::collections::HashMap::<String, usize>::new();
    output
        .ports()
        .into_iter()
        .map(|port| {
            let label = output
                .port_name(&port)
                .map_err(|error| MidiOutputError::Initialize(error.to_string()))?;
            let duplicate = labels.entry(label.clone()).or_default();
            let id = if *duplicate == 0 {
                label.clone()
            } else {
                format!("{label} #{}", *duplicate + 1)
            };
            *duplicate += 1;
            Ok((MidiOutputDevice { id, label }, port))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_position_splits_into_two_seven_bit_bytes() {
        // The encoding used by the sender thread, checked without a port.
        let value: u16 = 144;
        let bytes = [
            SONG_POSITION,
            (value & 0x7f) as u8,
            ((value >> 7) & 0x7f) as u8,
        ];
        assert_eq!(bytes, [0xf2, 0x10, 0x01]);
        assert_eq!(
            oneiroi_core::midi_clock::MidiRealtime::SongPosition(value),
            crate::parse_realtime_message(&bytes).expect("round trip")
        );
    }

    #[test]
    fn connecting_a_missing_port_fails_without_stranding_the_thread() {
        // The connect handshake blocks on the sender thread's first message.
        // A port that does not exist has to come back as an error rather than
        // parking the caller — this runs on the UI thread.
        let Err(error) = MidiClockSender::connect("oneiroi-nonexistent-port", 120.0) else {
            panic!("a port that does not exist cannot connect");
        };
        assert!(matches!(
            error,
            MidiOutputError::DeviceMissing(_) | MidiOutputError::Initialize(_)
        ));
    }

    #[test]
    fn discovery_reports_a_list_or_a_clean_error() {
        // CI machines have no MIDI hardware; either outcome is valid, a panic
        // or a hang is not.
        match discover_midi_outputs() {
            Ok(devices) => assert!(devices.iter().all(|device| !device.id.is_empty())),
            Err(error) => assert!(!error.to_string().is_empty()),
        }
    }
}
