use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ffmpeg_next as ffmpeg;
use oneiroi_core::MediaTime;
use oneiroi_hap::{CompressedPlaneFormat, Decoder};
use oneiroi_media::{
    CameraConfig, CameraDevice, ClipAddress, ClipRestoreRequest, ClipRestorer, DeckDecoder, DeckId,
    DeckState, DecodePath, DecoderEvent, DecoderFailureInjection, DiscontinuityPolicy,
    FfmpegVideoDecoder, FourDeckMixer, FrameBufferPool, FrameScheduler, FrameSelection, HapDemuxer,
    MediaHealth, MediaImporter, ThumbnailRequest, ThumbnailWorker, VideoFramePayload, probe_movie,
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
    let pool = FrameBufferPool::new(2);
    let mut decoder = FfmpegVideoDecoder::open_with_pool(&fixture.path, pool.clone()).unwrap();

    let red = decoder.next_frame().unwrap().unwrap();

    assert_eq!(red.pts, MediaTime::ZERO);
    assert_eq!(red.duration, Some(MediaTime::new(1, 30).unwrap()));
    assert_eq!(red.pixels.extent, [16, 16]);
    assert_eq!(&red.pixels.data[..4], &[255, 0, 0, 255]);
    drop(red);
    let green = decoder.next_frame().unwrap().unwrap();
    assert_eq!(green.pts, MediaTime::new(1, 30).unwrap());
    assert_eq!(&green.pixels.data[..4], &[0, 255, 0, 255]);
    let stats = pool.stats();
    assert_eq!(stats.allocations, 1);
    assert_eq!(stats.reuses, 1);
    assert!(decoder.next_frame().unwrap().is_none());
    drop(green);
    drop(decoder);
    let mut reopened = FfmpegVideoDecoder::open_with_pool(&fixture.path, pool.clone()).unwrap();
    let reopened_frame = reopened.next_frame().unwrap().unwrap();
    assert_eq!(&reopened_frame.pixels.data[..4], &[255, 0, 0, 255]);
    assert_eq!(pool.stats().allocations, 1);
    assert_eq!(pool.stats().reuses, 2);
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

#[test]
fn injected_midstream_failure_is_reported_and_the_worker_recovers() {
    let fixture = Fixture::raw_rgba_mov();
    let decoder = DeckDecoder::spawn_with_failure(
        2,
        DecoderFailureInjection::after_frames(1, "injected decoder failure"),
    );
    decoder.load(fixture.path.clone(), DecodePath::FfmpegVideo, 50);

    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Loaded { generation: 50 }
    );
    assert_eq!(
        decoder
            .recv_frame_timeout(Duration::from_secs(2))
            .unwrap()
            .generation,
        50
    );
    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Error {
            generation: 50,
            message: "injected decoder failure".to_owned(),
        }
    );

    decoder.load(fixture.path.clone(), DecodePath::FfmpegVideo, 51);
    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Loaded { generation: 51 }
    );
    assert_eq!(
        decoder
            .recv_frame_timeout(Duration::from_secs(2))
            .unwrap()
            .generation,
        51
    );
}

#[test]
fn repeated_decoder_reopen_soak_keeps_rgba_allocations_bounded() {
    decoder_reopen_soak(64);
}

#[test]
#[ignore = "extended manual soak; run explicitly before a show build"]
fn extended_decoder_reopen_soak() {
    decoder_reopen_soak(10_000);
}

fn decoder_reopen_soak(reopens: u64) {
    let fixture = Fixture::raw_rgba_mov();
    let decoder = DeckDecoder::spawn(2);

    for generation in 1..=reopens {
        decoder.load(fixture.path.clone(), DecodePath::FfmpegVideo, generation);
        assert_eq!(
            decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
            DecoderEvent::Loaded { generation }
        );
        for _ in 0..2 {
            let frame = decoder.recv_frame_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(frame.generation, generation);
        }
        assert_eq!(
            decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
            DecoderEvent::Ended { generation }
        );
    }

    let stats = decoder.frame_pool_stats();
    assert!(stats.allocations <= 2, "{stats:?}");
    assert!(stats.reuses >= (reopens - 1) * 2, "{stats:?}");
    assert_eq!(stats.in_flight, 0);
}

#[test]
fn deck_worker_decodes_a_bounded_live_capture_source() {
    let config = CameraConfig {
        device: CameraDevice {
            id: "testsrc=size=16x16:rate=30".to_owned(),
            label: "Test pattern".to_owned(),
            backend: "lavfi".to_owned(),
        },
        requested_extent: None,
        requested_fps: None,
    };
    let decoder = DeckDecoder::spawn(1);
    decoder.connect_camera(config, 44);

    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Loaded { generation: 44 }
    );
    let frame = decoder.recv_frame_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(frame.generation, 44);
    assert!(frame.pts >= MediaTime::ZERO);
    assert!(matches!(
        frame.payload,
        VideoFramePayload::Rgba8(ref rgba)
            if rgba.extent == [16, 16] && rgba.data.len() == 16 * 16 * 4
    ));
}

#[test]
fn deck_worker_seek_discards_pre_target_frames_and_changes_epoch() {
    let fixture = Fixture::raw_rgba_mov();
    let movie = probe_movie(&fixture.path).unwrap();
    assert_eq!(movie.keyframes.len(), 2);
    assert!(movie.keyframes.is_complete());
    let target = MediaTime::new(1, 30).unwrap();
    let anchor = movie.keyframes.nearest_preceding(target);
    assert_eq!(anchor, Some(target));
    let decoder = DeckDecoder::spawn(2);
    decoder.load_indexed(
        fixture.path.clone(),
        DecodePath::FfmpegVideo,
        43,
        Some(target),
        anchor,
    );

    assert_eq!(
        decoder.recv_event_timeout(Duration::from_secs(2)).unwrap(),
        DecoderEvent::Loaded { generation: 43 }
    );
    let frame = decoder.recv_frame_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(frame.generation, 43);
    assert_eq!(frame.pts, target);
    assert!(matches!(
        frame.payload,
        VideoFramePayload::Rgba8(ref rgba) if rgba.data[..4] == [0, 255, 0, 255]
    ));
}

#[test]
fn project_restorer_probes_slots_independently() {
    let fixture = Fixture::raw_rgba_mov();
    let restorer = ClipRestorer::new(2);
    let address = ClipAddress {
        deck: DeckId::C,
        slot: 6,
    };
    restorer
        .submit(ClipRestoreRequest {
            address,
            path: fixture.path.clone(),
            project_epoch: 9,
        })
        .unwrap();
    let result = restorer
        .recv_timeout(Duration::from_secs(2))
        .expect("restore result");
    assert_eq!(result.address, address);
    assert_eq!(result.project_epoch, 9);
    assert_eq!(result.metadata.unwrap().visible_extent, [16, 16]);
}

#[test]
fn thumbnail_worker_decodes_and_bounds_preview() {
    let fixture = Fixture::raw_rgba_mov();
    let worker = ThumbnailWorker::new(2);
    let address = ClipAddress {
        deck: DeckId::A,
        slot: 3,
    };
    worker
        .submit(ThumbnailRequest {
            address,
            path: fixture.path.clone(),
            request_id: 77,
        })
        .unwrap();
    let result = worker
        .recv_timeout(Duration::from_secs(2))
        .expect("thumbnail result");
    let thumbnail = result.thumbnail.expect("decoded thumbnail");
    assert_eq!(result.address, address);
    assert_eq!(result.request_id, 77);
    assert_eq!(thumbnail.extent, [16, 16]);
    assert_eq!(&thumbnail.rgba[..4], &[255, 0, 0, 255]);
    assert!(thumbnail.preload.extent[0] <= 640);
    assert!(thumbnail.preload.extent[1] <= 360);
    assert_eq!(
        thumbnail.preload.data.len(),
        thumbnail.preload.extent[0] as usize * thumbnail.preload.extent[1] as usize * 4
    );
}

#[test]
fn thumbnail_worker_uses_offline_ffmpeg_path_for_hap_preview() {
    let fixture = Fixture::hap_mov();
    let worker = ThumbnailWorker::new(1);
    worker
        .submit(ThumbnailRequest {
            address: ClipAddress {
                deck: DeckId::D,
                slot: 7,
            },
            path: fixture.path.clone(),
            request_id: 78,
        })
        .unwrap();
    let result = worker
        .recv_timeout(Duration::from_secs(2))
        .expect("HAP thumbnail result");
    let thumbnail = result.thumbnail.expect("decoded HAP thumbnail");
    assert_eq!(thumbnail.extent, [16, 16]);
    assert!(thumbnail.rgba[0] > 240);
    assert!(thumbnail.rgba[1] < 10);
    assert!(thumbnail.rgba[2] < 10);
}
