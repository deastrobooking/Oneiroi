//! Headless performance harness for the program render path.
//!
//! Answers the question the roadmap has been unable to answer: does a given
//! deck count, source format and effect load hold the target frame rate on this
//! machine? It drives the real `FourDeckCompositor` and `MasterEffectProcessor`
//! against synthetic frames, so the numbers move when the render code changes
//! and not when the media library does.
//!
//! ```sh
//! cargo run --release -p oneiroi-render --example perf -- --decks 4
//! cargo run --release -p oneiroi-render --example perf -- --decks 2 --source rgba
//! cargo run --release -p oneiroi-render --example perf -- --preset halation --master both
//! ```
//!
//! Every configuration is measured twice, because the two numbers answer
//! different questions:
//!
//! - **Sustained** submits frames with a bounded number in flight, the way the
//!   real render loop does, and waits once per batch. This is the throughput
//!   number: whether the machine holds the frame rate.
//! - **Latency** waits for the GPU after every single frame. That removes all
//!   CPU/GPU pipelining, so it reads high and is dominated by fence overhead,
//!   but its percentiles expose per-frame spikes that a mean would hide.
//!
//! Gate on sustained; read latency percentiles for jitter. Both are sensitive
//! to what else is running — record baselines on an idle show machine, not on
//! a laptop that is also compiling.

use std::fmt::Write as _;
use std::time::Instant;

use oneiroi_hap::{CompressedPlaneFormat, DecodedFrame, DecodedPlane};
use oneiroi_media::{RgbaFrame, VideoFramePayload};
use oneiroi_render::{
    DeckEffects, EffectPreset, FourDeckCompositor, MasterEffectChain, MasterEffectKind,
    MasterEffectProcessor, MasterEffectSlot, MixerBus, MixerParams, PROGRAM_FORMAT, ProgramTarget,
};

/// Distinct frames cycled per deck. More than one so the upload path is
/// exercised repeatedly rather than measuring a single warmed texture, but
/// few enough that the working set stays representative.
const FRAME_CYCLE: usize = 3;

fn main() {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let Some(gpu) = Gpu::new(args.source, args.gpu_timing) else {
        eprintln!(
            "no GPU adapter with the required features; \
             HAP source needs TEXTURE_COMPRESSION_BC"
        );
        std::process::exit(1);
    };

    // Repeat the whole measurement. A single run on a machine that is doing
    // anything else is worse than no number at all: observed spread on a busy
    // laptop reached 4x for an identical configuration.
    let reports = (0..args.runs).map(|_| run(&gpu, &args)).collect::<Vec<_>>();
    let summary = Summary::new(&reports);

    if args.json {
        println!("{}", summary.to_json(&args, &gpu));
    } else {
        println!("{}", summary.to_text(&args, &gpu));
    }

    // A non-zero exit makes this usable as a release gate in CI or a pre-show
    // check, not just something a human reads. The median run decides, so one
    // unlucky run neither passes nor fails the build on its own.
    if summary.sustained_median_ms > args.budget_ms {
        eprintln!(
            "FAIL: median sustained {:.2} ms exceeds the {:.2} ms budget",
            summary.sustained_median_ms, args.budget_ms
        );
        std::process::exit(1);
    }
}

const USAGE: &str = "\
usage: perf [options]

  --decks N          active decks, 1-4          (default 4)
  --frames N         measured frames            (default 600)
  --warmup N         discarded warmup frames    (default 60)
  --width N          composition width          (default 1920)
  --height N         composition height         (default 1080)
  --source hap|rgba  per-deck frame format      (default hap)
  --preset NAME      deck effect preset: neutral|neon|blacklight|glitch|halation
  --master MODE      none|blur|feedback|both    (default none)
  --inflight N       frames submitted before a wait (default 3)
  --runs N           repeat the measurement N times (default 3)
  --budget MS        sustained budget for exit status (default 16.67)
  --gpu-timing       bracket each frame with GPU timestamps (not on Metal)
  --json             emit machine-readable output";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Hap,
    Rgba,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Self::Hap => "HAP (BC1)",
            Self::Rgba => "RGBA8",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Master {
    None,
    Blur,
    Feedback,
    Both,
}

impl Master {
    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Blur => "blur",
            Self::Feedback => "feedback",
            Self::Both => "blur+feedback",
        }
    }

    fn chain(self) -> MasterEffectChain {
        let slot = |kind| MasterEffectSlot {
            kind,
            ..Default::default()
        };
        let slots = match self {
            Self::None => [slot(MasterEffectKind::None), slot(MasterEffectKind::None)],
            Self::Blur => [slot(MasterEffectKind::Blur), slot(MasterEffectKind::None)],
            Self::Feedback => [
                slot(MasterEffectKind::Feedback),
                slot(MasterEffectKind::None),
            ],
            Self::Both => [
                slot(MasterEffectKind::Blur),
                slot(MasterEffectKind::Feedback),
            ],
        };
        MasterEffectChain { slots }.sanitized()
    }
}

struct Args {
    decks: usize,
    frames: usize,
    warmup: usize,
    width: u32,
    height: u32,
    source: Source,
    preset: EffectPreset,
    master: Master,
    inflight: usize,
    runs: usize,
    budget_ms: f64,
    gpu_timing: bool,
    json: bool,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self {
            decks: 4,
            frames: 600,
            warmup: 60,
            width: 1920,
            height: 1080,
            source: Source::Hap,
            preset: EffectPreset::Neutral,
            master: Master::None,
            inflight: 3,
            runs: 3,
            budget_ms: 1000.0 / 60.0,
            gpu_timing: false,
            json: false,
        };
        let mut args = args.peekable();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| format!("{flag} requires a value"))
            };
            match flag.as_str() {
                "--decks" => parsed.decks = parse_field(&flag, value()?)?,
                "--frames" => parsed.frames = parse_field(&flag, value()?)?,
                "--warmup" => parsed.warmup = parse_field(&flag, value()?)?,
                "--width" => parsed.width = parse_field(&flag, value()?)?,
                "--height" => parsed.height = parse_field(&flag, value()?)?,
                "--inflight" => parsed.inflight = parse_field(&flag, value()?)?,
                "--runs" => parsed.runs = parse_field(&flag, value()?)?,
                "--budget" => parsed.budget_ms = parse_field(&flag, value()?)?,
                "--source" => {
                    parsed.source = match value()?.as_str() {
                        "hap" => Source::Hap,
                        "rgba" => Source::Rgba,
                        other => return Err(format!("unknown source `{other}`")),
                    }
                }
                "--master" => {
                    parsed.master = match value()?.as_str() {
                        "none" => Master::None,
                        "blur" => Master::Blur,
                        "feedback" => Master::Feedback,
                        "both" => Master::Both,
                        other => return Err(format!("unknown master mode `{other}`")),
                    }
                }
                "--preset" => {
                    parsed.preset = match value()?.as_str() {
                        "neutral" => EffectPreset::Neutral,
                        "neon" => EffectPreset::NeonNight,
                        "blacklight" => EffectPreset::Blacklight,
                        "glitch" => EffectPreset::Glitch,
                        "halation" => EffectPreset::Halation,
                        other => return Err(format!("unknown preset `{other}`")),
                    }
                }
                "--gpu-timing" => parsed.gpu_timing = true,
                "--json" => parsed.json = true,
                "--help" | "-h" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                other => return Err(format!("unknown option `{other}`")),
            }
        }

        if !(1..=4).contains(&parsed.decks) {
            return Err("--decks must be between 1 and 4".into());
        }
        if parsed.frames == 0 {
            return Err("--frames must be greater than zero".into());
        }
        // Unbounded in-flight frames would queue staging memory for every
        // upload without ever draining it.
        if !(1..=8).contains(&parsed.inflight) {
            return Err("--inflight must be between 1 and 8".into());
        }
        if parsed.runs == 0 {
            return Err("--runs must be greater than zero".into());
        }
        // BC1 blocks are 4x4; a non-multiple would need padding logic that the
        // real decoder handles through coded vs visible extents.
        if !parsed.width.is_multiple_of(4) || !parsed.height.is_multiple_of(4) {
            return Err("--width and --height must be multiples of 4".into());
        }
        Ok(parsed)
    }
}

fn parse_field<T: std::str::FromStr>(flag: &str, raw: String) -> Result<T, String> {
    raw.parse()
        .map_err(|_| format!("{flag} got an invalid value `{raw}`"))
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    backend: String,
    timestamps: Option<Timestamps>,
}

/// GPU-side frame timing, when the adapter can write timestamps outside a pass.
struct Timestamps {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    period_ns: f32,
}

impl Gpu {
    fn new(source: Source, gpu_timing: bool) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;

        let available = adapter.features();
        let mut required = wgpu::Features::empty();
        if source == Source::Hap {
            if !available.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
                return None;
            }
            required |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        let info = adapter.get_info();

        // Encoder-level timestamps deadlock on Metal: Apple GPUs sample at
        // stage boundaries, not the blit boundary `write_timestamp` uses, and
        // the command buffer never completes. Per-pass `timestamp_writes` is
        // the supported route there, but that has to be plumbed through the
        // compositor and master passes rather than bolted on from outside.
        let timing_features =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        let metal = info.backend == wgpu::Backend::Metal;
        let timing_supported = gpu_timing && available.contains(timing_features) && !metal;
        if gpu_timing && !timing_supported {
            eprintln!(
                "note: GPU timing unavailable on this adapter{}; reporting CPU frame time only",
                if metal {
                    " (encoder timestamps are unsupported on Metal)"
                } else {
                    ""
                }
            );
        }
        if timing_supported {
            required |= timing_features;
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("oneiroi-perf"),
            required_features: required,
            ..Default::default()
        }))
        .ok()?;

        let timestamps = timing_supported.then(|| Timestamps::new(&device, &queue));

        Some(Self {
            device,
            queue,
            adapter_name: info.name,
            backend: format!("{:?}", info.backend),
            timestamps,
        })
    }
}

impl Timestamps {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("perf-timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("perf-timestamp-resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("perf-timestamp-readback"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            period_ns: queue.get_timestamp_period(),
        }
    }

    /// Read the bracketed GPU duration for the frame just completed.
    fn read_ms(&self, device: &wgpu::Device) -> Option<f64> {
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .ok()?;
        let ms = {
            let view = self.readback.slice(..).get_mapped_range();
            let raw: &[u64] = bytemuck::cast_slice(&view);
            // Timestamps can go backwards across a queue reset; treat that as
            // a missing sample rather than a negative duration.
            let delta = raw[1].checked_sub(raw[0])?;
            f64::from(self.period_ns) * delta as f64 / 1_000_000.0
        };
        self.readback.unmap();
        Some(ms)
    }
}

/// One deck's worth of pre-generated frames, cycled during the run.
struct DeckFrames {
    payloads: Vec<VideoFramePayload>,
    bytes_per_frame: usize,
}

fn generate_frames(source: Source, width: u32, height: u32, deck: usize) -> DeckFrames {
    let payloads = (0..FRAME_CYCLE)
        .map(|index| match source {
            Source::Rgba => rgba_frame(width, height, deck, index),
            Source::Hap => hap_frame(width, height, deck, index),
        })
        .collect::<Vec<_>>();
    let bytes_per_frame = match &payloads[0] {
        VideoFramePayload::Rgba8(frame) => frame.data.len(),
        VideoFramePayload::BlockCompressed(frame) => {
            frame.planes.iter().map(|plane| plane.data.len()).sum()
        }
    };
    DeckFrames {
        payloads,
        bytes_per_frame,
    }
}

fn rgba_frame(width: u32, height: u32, deck: usize, index: usize) -> VideoFramePayload {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&[
                (x.wrapping_add(index as u32 * 17)) as u8,
                (y.wrapping_add(deck as u32 * 53)) as u8,
                (x ^ y) as u8,
                255,
            ]);
        }
    }
    VideoFramePayload::Rgba8(RgbaFrame {
        extent: [width, height],
        data: data.into(),
    })
}

/// A BC1 plane of arbitrary-but-valid blocks. Contents are irrelevant to
/// timing; size and format are what the upload path reacts to.
fn hap_frame(width: u32, height: u32, deck: usize, index: usize) -> VideoFramePayload {
    let blocks = ((width / 4) * (height / 4)) as usize;
    let mut data = Vec::with_capacity(blocks * 8);
    for block in 0..blocks {
        let seed = block.wrapping_mul(31).wrapping_add(deck * 7 + index * 13) as u16;
        data.extend_from_slice(&seed.to_le_bytes());
        data.extend_from_slice(&seed.rotate_left(5).to_le_bytes());
        data.extend_from_slice(&(seed as u32).to_le_bytes());
    }
    VideoFramePayload::BlockCompressed(DecodedFrame {
        planes: std::iter::once(DecodedPlane {
            format: CompressedPlaneFormat::Bc1Rgb,
            coded_extent: [width, height],
            visible_extent: [width, height],
            data,
        })
        .collect(),
    })
}

struct Report {
    frame_ms: Vec<f64>,
    upload_ms: Vec<f64>,
    gpu_ms: Vec<f64>,
    /// Wall-clock milliseconds per frame with rendering pipelined.
    sustained_ms: f64,
    upload_bytes_per_frame: usize,
}

fn run(gpu: &Gpu, args: &Args) -> Report {
    let extent = [args.width, args.height];
    let mut compositor = FourDeckCompositor::new(&gpu.device, &gpu.queue, PROGRAM_FORMAT);
    let program = ProgramTarget::new(&gpu.device, extent);
    let mut master = MasterEffectProcessor::new(&gpu.device, &program);
    let chain = args.master.chain();

    let decks = (0..args.decks)
        .map(|deck| generate_frames(args.source, args.width, args.height, deck))
        .collect::<Vec<_>>();
    let upload_bytes_per_frame = decks.iter().map(|deck| deck.bytes_per_frame).sum();

    let mut params = MixerParams {
        output_aspect: args.width as f32 / args.height as f32,
        ..Default::default()
    };
    // Split the decks across both buses so the crossfade path is exercised
    // rather than short-circuited, matching how a set is actually mixed.
    for deck in 0..4 {
        params.buses[deck] = if deck % 2 == 0 {
            MixerBus::A
        } else {
            MixerBus::B
        };
        params.levels[deck] = if deck < args.decks { 1.0 } else { 0.0 };
        params.effects[deck] = DeckEffects::preset(args.preset);
    }
    params.crossfade_gains = [0.5, 0.5];

    // One closure for both passes so the two numbers can never drift apart by
    // measuring subtly different work.
    let render_frame = |compositor: &mut FourDeckCompositor,
                        master: &mut MasterEffectProcessor,
                        params: &mut MixerParams,
                        frame: usize|
     -> f64 {
        let started = Instant::now();
        for (deck, frames) in decks.iter().enumerate() {
            let payload = &frames.payloads[frame % FRAME_CYCLE];
            compositor
                .upload(&gpu.device, &gpu.queue, deck, payload)
                .expect("upload deck frame");
        }
        let upload_ms = started.elapsed().as_secs_f64() * 1000.0;

        params.time_seconds = frame as f32 / 60.0;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("perf-frame"),
            });
        if let Some(timestamps) = &gpu.timestamps {
            encoder.write_timestamp(&timestamps.query_set, 0);
        }
        compositor.draw(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            program.composition_view(),
            *params,
        );
        master.draw_at(
            &gpu.queue,
            &mut encoder,
            &program,
            &chain,
            params.time_seconds,
        );
        if let Some(timestamps) = &gpu.timestamps {
            encoder.write_timestamp(&timestamps.query_set, 1);
            encoder.resolve_query_set(&timestamps.query_set, 0..2, &timestamps.resolve, 0);
            encoder.copy_buffer_to_buffer(&timestamps.resolve, 0, &timestamps.readback, 0, 16);
        }
        gpu.queue.submit([encoder.finish()]);
        upload_ms
    };

    let wait = || {
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll frames to completion");
    };

    for frame in 0..args.warmup {
        render_frame(&mut compositor, &mut master, &mut params, frame);
        wait();
    }

    // Pass one: sustained throughput with a bounded number of frames in
    // flight, then a single wall-clock division. Nothing is timed per frame,
    // so scheduler noise averages out instead of landing in a percentile.
    let sustained_started = Instant::now();
    for frame in 0..args.frames {
        render_frame(&mut compositor, &mut master, &mut params, frame);
        if frame % args.inflight == args.inflight - 1 {
            wait();
        }
    }
    wait();
    let sustained_ms = sustained_started.elapsed().as_secs_f64() * 1000.0 / args.frames as f64;

    // Pass two: per-frame latency. Deliberately unpipelined.
    let mut frame_ms = Vec::with_capacity(args.frames);
    let mut upload_ms = Vec::with_capacity(args.frames);
    let mut gpu_ms = Vec::with_capacity(args.frames);
    for frame in 0..args.frames {
        let started = Instant::now();
        let upload = render_frame(&mut compositor, &mut master, &mut params, frame);
        wait();
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        upload_ms.push(upload);

        if let Some(timestamps) = &gpu.timestamps
            && let Some(ms) = timestamps.read_ms(&gpu.device)
        {
            gpu_ms.push(ms);
        }
    }

    Report {
        sustained_ms,
        frame_ms,
        upload_ms,
        gpu_ms,
        upload_bytes_per_frame,
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Nearest-rank percentile over an ascending slice.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// Aggregate of every run of one configuration.
struct Summary {
    sustained: Vec<f64>,
    sustained_best_ms: f64,
    sustained_median_ms: f64,
    sustained_worst_ms: f64,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    upload_mean_ms: f64,
    gpu_mean_ms: Option<f64>,
    upload_bytes_per_frame: usize,
    frames: usize,
}

impl Summary {
    fn new(reports: &[Report]) -> Self {
        let mut sustained = reports
            .iter()
            .map(|report| report.sustained_ms)
            .collect::<Vec<_>>();
        sustained.sort_by(f64::total_cmp);

        // Latency percentiles pool every run's frames: the whole point of the
        // latency pass is catching rare spikes, and rare spikes are exactly
        // what a per-run average throws away.
        let pooled = reports
            .iter()
            .flat_map(|report| report.frame_ms.iter().copied())
            .collect::<Vec<_>>();
        let mut sorted = pooled.clone();
        sorted.sort_by(f64::total_cmp);

        let gpu_samples = reports
            .iter()
            .flat_map(|report| report.gpu_ms.iter().copied())
            .collect::<Vec<_>>();

        Self {
            sustained_best_ms: sustained.first().copied().unwrap_or_default(),
            sustained_median_ms: percentile(&sustained, 0.50),
            sustained_worst_ms: sustained.last().copied().unwrap_or_default(),
            sustained,
            mean_ms: mean(&pooled),
            p50_ms: percentile(&sorted, 0.50),
            p95_ms: percentile(&sorted, 0.95),
            p99_ms: percentile(&sorted, 0.99),
            max_ms: sorted.last().copied().unwrap_or_default(),
            upload_mean_ms: mean(
                &reports
                    .iter()
                    .flat_map(|report| report.upload_ms.iter().copied())
                    .collect::<Vec<_>>(),
            ),
            gpu_mean_ms: (!gpu_samples.is_empty()).then(|| mean(&gpu_samples)),
            upload_bytes_per_frame: reports
                .first()
                .map(|report| report.upload_bytes_per_frame)
                .unwrap_or_default(),
            frames: pooled.len(),
        }
    }

    fn upload_gb_per_second(&self, frame_ms: f64) -> f64 {
        if frame_ms <= 0.0 {
            return 0.0;
        }
        self.upload_bytes_per_frame as f64 * (1000.0 / frame_ms) / 1_000_000_000.0
    }

    /// Ratio of worst to best sustained run. Anything much above 1 means the
    /// machine was busy and the absolute numbers should not be quoted.
    fn spread(&self) -> f64 {
        if self.sustained_best_ms <= 0.0 {
            return 1.0;
        }
        self.sustained_worst_ms / self.sustained_best_ms
    }

    fn to_text(&self, args: &Args, gpu: &Gpu) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} ({})\n{}x{} · {} decks · {} · deck fx {} · master {}\n{} runs x {} frames, {} discarded as warmup",
            gpu.adapter_name,
            gpu.backend,
            args.width,
            args.height,
            args.decks,
            args.source.label(),
            args.preset.label(),
            args.master.label(),
            args.runs,
            args.frames,
            args.warmup,
        );
        let _ = writeln!(
            out,
            "\n  sustained  best {:6.2}  median {:6.2}  worst {:6.2}  ms/frame  ({:.1} fps median, {} in flight)",
            self.sustained_best_ms,
            self.sustained_median_ms,
            self.sustained_worst_ms,
            1000.0 / self.sustained_median_ms,
            args.inflight,
        );
        let _ = writeln!(
            out,
            "  latency    mean {:6.2}  p50 {:6.2}  p95 {:6.2}  p99 {:6.2}  max {:6.2}  ms  ({} frames)",
            self.mean_ms, self.p50_ms, self.p95_ms, self.p99_ms, self.max_ms, self.frames,
        );
        let _ = writeln!(out, "  upload     mean {:6.2} ms", self.upload_mean_ms);
        if let Some(gpu_ms) = self.gpu_mean_ms {
            let _ = writeln!(out, "  gpu        mean {:6.2} ms", gpu_ms);
        }
        let _ = writeln!(
            out,
            "\n  {:.2} MB uploaded per frame · {:.2} GB/s at the median rate",
            self.upload_bytes_per_frame as f64 / 1_000_000.0,
            self.upload_gb_per_second(self.sustained_median_ms),
        );

        let spread = self.spread();
        if spread > 1.25 {
            let _ = writeln!(
                out,
                "\n  UNRELIABLE: slowest run was {spread:.2}x the fastest. Something else is \
                 using this\n  machine; record baselines on an idle system before quoting these \
                 numbers."
            );
        }
        let _ = write!(
            out,
            "\n  {} median sustained {:.2} ms against a {:.2} ms budget ({:.2}x headroom)",
            if self.sustained_median_ms <= args.budget_ms {
                "PASS"
            } else {
                "FAIL"
            },
            self.sustained_median_ms,
            args.budget_ms,
            args.budget_ms / self.sustained_median_ms,
        );
        out
    }

    fn to_json(&self, args: &Args, gpu: &Gpu) -> String {
        let runs = self
            .sustained
            .iter()
            .map(|ms| format!("{ms:.4}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                r#"{{"adapter":"{}","backend":"{}","width":{},"height":{},"decks":{},"#,
                r#""source":"{}","preset":"{}","master":"{}","runs":{},"frames":{},"warmup":{},"#,
                r#""inflight":{},"sustained_runs_ms":[{}],"sustained_best_ms":{:.4},"#,
                r#""sustained_median_ms":{:.4},"sustained_worst_ms":{:.4},"sustained_median_fps":{:.2},"#,
                r#""spread":{:.4},"mean_ms":{:.4},"p50_ms":{:.4},"p95_ms":{:.4},"p99_ms":{:.4},"#,
                r#""max_ms":{:.4},"upload_mean_ms":{:.4},"gpu_mean_ms":{},"#,
                r#""upload_bytes_per_frame":{},"upload_gb_per_second":{:.4},"#,
                r#""budget_ms":{:.4},"pass":{}}}"#
            ),
            gpu.adapter_name,
            gpu.backend,
            args.width,
            args.height,
            args.decks,
            args.source.label(),
            args.preset.label(),
            args.master.label(),
            args.runs,
            args.frames,
            args.warmup,
            args.inflight,
            runs,
            self.sustained_best_ms,
            self.sustained_median_ms,
            self.sustained_worst_ms,
            1000.0 / self.sustained_median_ms,
            self.spread(),
            self.mean_ms,
            self.p50_ms,
            self.p95_ms,
            self.p99_ms,
            self.max_ms,
            self.upload_mean_ms,
            match self.gpu_mean_ms {
                Some(ms) => format!("{ms:.4}"),
                None => "null".to_string(),
            },
            self.upload_bytes_per_frame,
            self.upload_gb_per_second(self.sustained_median_ms),
            args.budget_ms,
            self.sustained_median_ms <= args.budget_ms,
        )
    }
}
