use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ffmpeg_next as ffmpeg;
use oneiroi_core::MediaTime;
use oneiroi_hap::{CompressedPlaneFormat, Decoder};
use oneiroi_media::{DiscontinuityPolicy, FrameScheduler, FrameSelection, HapDemuxer};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn hap_mov() -> Self {
        ffmpeg::init().unwrap();
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("oneiroi-hap-demux-{}-{id}.mov", std::process::id()));
        write_hap_mov(&path);
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_hap_mov(path: &Path) {
    let mut output = ffmpeg::format::output_as(path, "mov").unwrap();
    let mut parameters = ffmpeg::codec::Parameters::new();
    // SAFETY: parameters uniquely owns this allocated AVCodecParameters.
    unsafe {
        let parameters = &mut *parameters.as_mut_ptr();
        parameters.codec_type = ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        parameters.codec_id = ffmpeg::ffi::AVCodecID::AV_CODEC_ID_HAP;
        parameters.codec_tag = u32::from_le_bytes(*b"Hap1");
        parameters.width = 16;
        parameters.height = 16;
    }

    let stream_index = {
        // The Homebrew FFmpeg build intentionally has no HAP encoder. Passing
        // the ID still creates a stream with no encoder context.
        let mut stream = output.add_stream(ffmpeg::codec::Id::HAP).unwrap();
        stream.set_parameters(parameters);
        stream.set_time_base((1, 30));
        stream.set_rate((30, 1));
        stream.set_avg_frame_rate((30, 1));
        stream.index()
    };
    output.write_header().unwrap();

    // A simple HAP section: 128-byte raw BC1 payload and section type 0xAB
    // (no second-stage compression + RGB DXT1).
    let red_bc1_block = [0x00, 0xf8, 0x00, 0x00, 0, 0, 0, 0];
    let texture = red_bc1_block.repeat(16);
    let mut hap_frame = vec![texture.len() as u8, 0, 0, 0xab];
    hap_frame.extend_from_slice(&texture);

    let mut packet = ffmpeg::Packet::copy(&hap_frame);
    packet.set_stream(stream_index);
    packet.set_pts(Some(0));
    packet.set_dts(Some(0));
    packet.set_duration(1);
    packet.set_flags(ffmpeg::codec::packet::Flags::KEY);
    let mux_time_base = output.stream(stream_index).unwrap().time_base();
    packet.rescale_ts((1, 30), mux_time_base);
    packet.write_interleaved(&mut output).unwrap();
    output.write_trailer().unwrap();
}

#[test]
fn demuxes_raw_hap_packet_and_preserves_timestamps() {
    let fixture = Fixture::hap_mov();
    let mut demuxer = HapDemuxer::open(&fixture.path).unwrap();

    let metadata = demuxer.metadata();
    assert_eq!(metadata.visible_extent, [16, 16]);
    assert_eq!(metadata.codec_tag, *b"Hap1");
    assert_eq!(metadata.average_frame_rate.unwrap().numerator, 30);
    assert_eq!(metadata.average_frame_rate.unwrap().denominator, 1);

    let packet = demuxer.next_packet().unwrap().unwrap();
    assert_eq!(packet.sequence, 0);
    assert_eq!(packet.pts, Some(MediaTime::ZERO));
    assert_eq!(packet.dts, Some(MediaTime::ZERO));
    assert_eq!(packet.duration, Some(MediaTime::new(1, 30).unwrap()));
    assert!(packet.keyframe);
    assert_eq!(&packet.data[..4], &[128, 0, 0, 0xab]);
    assert!(demuxer.next_packet().unwrap().is_none());
}

#[test]
fn decodes_demuxed_packet_without_ffmpeg_pixel_decode() {
    let fixture = Fixture::hap_mov();
    let mut demuxer = HapDemuxer::open(&fixture.path).unwrap();

    let frame = demuxer.next_decoded(&Decoder::default()).unwrap().unwrap();

    assert_eq!(frame.sequence, 0);
    assert_eq!(frame.frame.planes.len(), 1);
    assert_eq!(frame.frame.planes[0].format, CompressedPlaneFormat::Bc1Rgb);
    assert_eq!(frame.frame.planes[0].data.len(), 128);
}

#[test]
fn feeds_generation_safe_timestamp_scheduler() {
    let fixture = Fixture::hap_mov();
    let mut demuxer = HapDemuxer::open(&fixture.path).unwrap();
    let mut scheduler = FrameScheduler::new(4, 7, DiscontinuityPolicy::HoldLastFrame).unwrap();
    scheduler
        .enqueue(
            demuxer
                .next_scheduled(&Decoder::default(), 7)
                .unwrap()
                .unwrap(),
        )
        .unwrap();

    let selected = scheduler.select(MediaTime::ZERO);

    assert!(matches!(
        selected,
        FrameSelection::Advanced(frame)
            if frame.generation == 7
                && frame.payload.planes[0].format == CompressedPlaneFormat::Bc1Rgb
    ));
}
