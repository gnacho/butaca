//! The `avc3` H.264 sample entry (ISO 14496-15): SPS/PPS live in the samples, not in an `avcC`
//! box, so the demuxer must harvest them in-band and prepend them at keyframes instead of handing
//! the decoder an empty parameter-set blob (`param_sets=0` → `non-existing PPS 0 referenced`).

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

/// Synthetic in-band NALs for an avc3 fixture: SPS (type 7), PPS (type 8), an IDR slice (type 5)
/// and an ordinary non-IDR slice (type 1). The header bytes encode their nal_unit_type in the low
/// five bits, exactly as a real length-prefixed MP4 sample carries them.
fn sps() -> [u8; 7] {
    [0x67, 0x42, 0x00, 0x1e, 0xab, 0xcd, 0xef]
}

fn pps() -> [u8; 4] {
    [0x68, 0xce, 0x3c, 0x80]
}

/// The `avc3` sample entry (no avcC box) leaves `codecpar.extradata` empty, so `parse_extradata`
/// yields zero parameter sets — the broken state that made the decoder fail with
/// `non-existing PPS 0 referenced`.
#[test]
fn an_avc3_sample_entry_has_no_extradata_and_zero_parameter_sets() {
    assert_eq!(TAG_AVC3, u32::from_be_bytes(*b"avc3"));
    let (blob, nls) = unsafe { parse_extradata(std::ptr::null(), 0, false) };
    assert_eq!(blob.len(), 0, "no avcC: the parameter-set blob is empty");
    assert_eq!(nls, 4, "the default NAL length size is four bytes");
}

/// The fix: an avc3 stream carries its SPS/PPS in-band (length-prefixed per ISO 14496-15), so
/// harvest them from the samples and prepend them at keyframes — `param_sets > 0` and the decoder
/// receives the parameter sets before the IDR instead of a bare slice.
#[test]
fn an_avc3_sample_harvests_parameter_sets_from_its_samples() {
    let sample = avcc(&[&sps(), &pps(), &[0x65, 0x88, 0x84], &[0x41, 0x9a, 0x22]]);

    // The demux starts with the empty avcC-less blob, then harvests from the first sample.
    let mut blob = Vec::new();
    assert!(harvest_inband_params(&sample, 4, &mut blob), "SPS/PPS were found in-band");
    assert!(!blob.is_empty(), "param_sets > 0 after harvesting");
    assert_eq!(
        blob,
        [vec![0, 0, 0, 1], sps().to_vec(), vec![0, 0, 0, 1], pps().to_vec()].concat(),
        "SPS then PPS, each behind a 4-byte start code"
    );

    // The keyframe AU prepends those parameter sets, so the decoder gets SPS/PPS before the IDR.
    let (key, out) = to_annexb(&sample, false, &blob);
    assert!(key, "the sample carries an IDR");
    assert!(out.starts_with(&blob), "the parameter sets lead the keyframe");
}

/// A stream that repeats its parameter sets at every keyframe must not grow the prepend blob
/// without bound: the same SPS/PPS payload is appended once, not once per keyframe.
#[test]
fn a_repeated_parameter_set_is_not_appended_twice() {
    let sample = avcc(&[&sps(), &pps()]);
    let mut blob = Vec::new();
    assert!(harvest_inband_params(&sample, 4, &mut blob));
    let once = blob.clone();
    assert!(!harvest_inband_params(&sample, 4, &mut blob), "nothing new to add");
    assert_eq!(blob, once, "the same SPS/PPS is not appended twice");
}

/// A length field that claims more bytes than the sample holds stops the walk instead of indexing
/// past the slice, mirroring `packet_to_annexb`'s bounds guard — corrupt or truncated server data
/// must never panic the demux thread.
#[test]
fn a_truncated_inband_length_stops_the_harvest_without_panicking() {
    let mut sample = avcc(&[&sps()]);
    sample.extend_from_slice(&0xFFFF_FF00u32.to_be_bytes()); // absurd length, no payload
    let mut blob = Vec::new();
    harvest_inband_params(&sample, 4, &mut blob);
    assert_eq!(
        blob,
        [vec![0, 0, 0, 1], sps().to_vec()].concat(),
        "the good SPS survives, the bad length stops the walk"
    );
}
