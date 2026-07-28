use std::ffi::{c_uint, c_ulong, c_void};

use oneiroi_hap::{CompressedPlaneFormat, Decoder};
use oneiroi_hap_sys as sys;

fn encode_plane(data: &[u8], format: c_uint, compressor: c_uint, chunks: c_uint) -> Vec<u8> {
    let mut length = c_ulong::try_from(data.len()).unwrap();
    let mut format = format;
    let mut chunks = chunks;
    // SAFETY: each pointer addresses one initialized array element.
    let capacity = unsafe { sys::HapMaxEncodedLength(1, &mut length, &mut format, &mut chunks) };
    assert!(capacity > 0);

    let mut packet = vec![0_u8; usize::try_from(capacity).unwrap()];
    let mut input = data.as_ptr().cast::<c_void>();
    let mut compressor = compressor;
    let mut used = 0;
    // SAFETY: input and output buffers are valid for their supplied lengths.
    let result = unsafe {
        sys::HapEncode(
            1,
            &mut input,
            &mut length,
            &mut format,
            &mut compressor,
            &mut chunks,
            packet.as_mut_ptr().cast(),
            capacity,
            &mut used,
        )
    };
    assert_eq!(result, sys::HAP_RESULT_NO_ERROR);
    packet.truncate(usize::try_from(used).unwrap());
    packet
}

fn encode_two_planes(first: (&[u8], c_uint), second: (&[u8], c_uint)) -> Vec<u8> {
    let mut lengths = [
        c_ulong::try_from(first.0.len()).unwrap(),
        c_ulong::try_from(second.0.len()).unwrap(),
    ];
    let mut formats = [first.1, second.1];
    let mut chunks = [1, 1];
    // SAFETY: the arrays contain two initialized elements.
    let capacity = unsafe {
        sys::HapMaxEncodedLength(
            2,
            lengths.as_mut_ptr(),
            formats.as_mut_ptr(),
            chunks.as_mut_ptr(),
        )
    };
    let mut packet = vec![0_u8; usize::try_from(capacity).unwrap()];
    let mut inputs = [
        first.0.as_ptr().cast::<c_void>(),
        second.0.as_ptr().cast::<c_void>(),
    ];
    let mut compressors = [sys::HAP_COMPRESSOR_SNAPPY; 2];
    let mut used = 0;
    // SAFETY: all arrays contain the two elements declared to the C API.
    let result = unsafe {
        sys::HapEncode(
            2,
            inputs.as_mut_ptr(),
            lengths.as_mut_ptr(),
            formats.as_mut_ptr(),
            compressors.as_mut_ptr(),
            chunks.as_mut_ptr(),
            packet.as_mut_ptr().cast(),
            capacity,
            &mut used,
        )
    };
    assert_eq!(result, sys::HAP_RESULT_NO_ERROR);
    packet.truncate(usize::try_from(used).unwrap());
    packet
}

#[test]
fn reference_encoder_and_safe_decoder_round_trip_snappy_bc1() {
    // 16x16 BC1 is 16 blocks. Repeated blocks ensure the reference encoder
    // retains its Snappy path rather than falling back to raw storage.
    let red_block = [0x00, 0xf8, 0x00, 0x00, 0, 0, 0, 0];
    let texture = red_block.repeat(16);
    let packet = encode_plane(
        &texture,
        sys::HAP_TEXTURE_FORMAT_RGB_DXT1,
        sys::HAP_COMPRESSOR_SNAPPY,
        4,
    );

    let decoded = Decoder::default().decode(&packet, 16, 16).unwrap();

    assert_eq!(decoded.planes.len(), 1);
    assert_eq!(decoded.planes[0].format, CompressedPlaneFormat::Bc1Rgb);
    assert_eq!(decoded.planes[0].visible_extent, [16, 16]);
    assert_eq!(decoded.planes[0].coded_extent, [16, 16]);
    assert_eq!(decoded.planes[0].data, texture);
}

#[test]
fn decodes_hap_q_alpha_as_two_distinct_planes() {
    let ycocg = vec![0x55; 16 * 16];
    let alpha = vec![0xaa; 8 * 16];
    let packet = encode_two_planes(
        (&ycocg, sys::HAP_TEXTURE_FORMAT_YCOCG_DXT5),
        (&alpha, sys::HAP_TEXTURE_FORMAT_A_RGTC1),
    );

    let decoded = Decoder::default().decode(&packet, 16, 16).unwrap();

    assert_eq!(decoded.planes.len(), 2);
    assert_eq!(
        decoded.planes[0].format,
        CompressedPlaneFormat::Bc3ScaledYCoCg
    );
    assert_eq!(decoded.planes[1].format, CompressedPlaneFormat::Bc4Alpha);
    assert_eq!(decoded.planes[0].data, ycocg);
    assert_eq!(decoded.planes[1].data, alpha);
}

#[test]
fn malformed_packets_are_rejected() {
    let error = Decoder::default().decode(&[1, 2, 3], 16, 16).unwrap_err();
    assert!(error.to_string().contains("rejected"));
}
