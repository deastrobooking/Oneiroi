use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ffmpeg_next as ffmpeg;
use oneiroi_core::MediaTime;
use oneiroi_hap::{CompressedPlaneFormat, Decoder};
use oneiroi_media::{
    DeckDecoder, DeckId, DeckState, DecodePath, DecoderEvent, DiscontinuityPolicy,
    FfmpegVideoDecoder, FourDeckMixer, FrameScheduler, FrameSelection, HapDemuxer, MediaHealth,
    MediaImporter, VideoFramePayload, probe_movie,
};

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

    fn raw_rgba_mov() -> Self {
        ffmpeg::init().unwrap();
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oneiroi-rgba-decode-{}-{id}.mov",
            std::process::id()
        ));
        write_raw_rgba_mov(&path);
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

fn write_raw_rgba_mov(path: &Path) {
    let mut output = ffmpeg::format::output_as(path, "mov").unwrap();
    let mut parameters = ffmpeg::codec::Parameters::new();
    // SAFETY: parameters uniquely owns this allocated AVCodecParameters.
    unsafe {
        let parameters = &mut *parameters.as_mut_ptr();
        parameters.codec_type = ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        parameters.codec_id = ffmpeg::ffi::AVCodecID::AV_CODEC_ID_RAWVIDEO;
        parameters.codec_tag = u32::from_le_bytes(*b"RGBA");
        parameters.format = ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i32;
        parameters.width = 16;
        parameters.height = 16;
    }
    let stream_index = {
        let mut stream = output.add_stream(ffmpeg::codec::Id::RAWVIDEO).unwrap();
        stream.set_parameters(parameters);
        stream.set_time_base((1, 30));
        stream.set_rate((30, 1));
        stream.set_avg_frame_rate((30, 1));
        stream.index()
    };
    output.write_header().unwrap();
    let mux_time_base = output.stream(stream_index).unwrap().time_base();

    for (pts, pixel) in [(0, [255, 0, 0, 255]), (1, [0, 255, 0, 255])] {
        let data = pixel.repeat(16 * 16);
        let mut packet = ffmpeg::Packet::copy(&data);
        packet.set_stream(stream_index);
        packet.set_pts(Some(pts));
        packet.set_dts(Some(pts));
        packet.set_duration(1);
        packet.set_flags(ffmpeg::codec::packet::Flags::KEY);
        packet.rescale_ts((1, 30), mux_time_base);
        packet.write_interleaved(&mut output).unwrap();
    }
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

#[test]
fn general_import_probe_routes_hap_to_optimized_path() {
    let fixture = Fixture::hap_mov();

    let movie = probe_movie(&fixture.path).unwrap();

    assert_eq!(movie.visible_extent, [16, 16]);
    assert_eq!(movie.codec, "hap");
    assert_eq!(movie.decode_path, DecodePath::DirectHap);
    assert_eq!(movie.health, MediaHealth::StageReady);
}

#[test]
fn background_import_populates_one_of_four_decks() {
    let fixture = Fixture::hap_mov();
    let importer = MediaImporter::new(4);
    let mut mixer = FourDeckMixer::default();
    let request = mixer.begin_import(DeckId::D, fixture.path.clone());
    importer.submit(request).unwrap();

    let result = importer.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(mixer.complete_import(result));

    assert!(matches!(
        &mixer.deck(DeckId::D).state,
        DeckState::Ready(movie)
            if movie.decode_path == DecodePath::DirectHap
                && movie.visible_extent == [16, 16]
    ));
}

#[test]
fn ffmpeg_fallback_decodes_tightly_packed_rgba_frames() {
    let fixture = Fixture::raw_rgba_mov();
    let mut decoder = FfmpegVideoDecoder::open(&fixture.path).unwrap();

    let red = decoder.next_frame().unwrap().unwrap();
    let green = decoder.next_frame().unwrap().unwrap();

    assert_eq!(red.pts, MediaTime::ZERO);
    assert_eq!(red.duration, Some(MediaTime::new(1, 30).unwrap()));
    assert_eq!(red.pixels.extent, [16, 16]);
    assert_eq!(&red.pixels.data[..4], &[255, 0, 0, 255]);
    assert_eq!(green.pts, MediaTime::new(1, 30).unwrap());
    assert_eq!(&green.pixels.data[..4], &[0, 255, 0, 255]);
    assert!(decoder.next_frame().unwrap().is_none());
}

#[test]
fn bounded_deck_worker_decodes_off_the_calling_thread() {
    let fixture = Fixture::raw_rgba_mov();
    let decoder = DeckDecoder::spawn(2);
    decoder.load(fixture.path.clone(), DecodePath::FfmpegVideo, 42);

    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Loaded { generation: 42 }
    );
    let frame = decoder.recv_frame_timeout(Duration::from_secs(2)).unwrap();

    assert_eq!(frame.generation, 42);
    assert_eq!(frame.pts, MediaTime::ZERO);
    assert!(matches!(
        frame.payload,
        VideoFramePayload::Rgba8(ref rgba) if rgba.data[..4] == [255, 0, 0, 255]
    ));
}
