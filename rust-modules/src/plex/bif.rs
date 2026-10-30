//! The Plex BIF index parser (video preview thumbnails, issue #65's data layer).
//!
//! A BIF file (`GET /library/parts/{partId}/indexes/sd`) is a header, an index table, then the
//! concatenated JPEG frames. The index maps a timestamp to the byte offset of its frame; the
//! seek bar uses it to fetch exactly one frame (`/indexes/sd/{offsetMs}`) under the cursor.
//!
//! **The binary layout below is reverse-engineered, not from Plex documentation**: Plex publishes
//! no spec for the format, and this repo's `docs/pms-api.md` / `docs/plex-openapi.json` document
//! the ENDPOINT, not the bytes. The magic `\x89BIF\r\n\x1a\n` and the 8-byte `(u32 BE timestamp,
//! u32 BE offset)` entry shape are the widely-shared reverse-engineered reading; the entry COUNT
//! is not carried in the header, so [`parse_bif_index`] derives it from the fact that frame
//! offsets are monotonically increasing. That derivation has not been verified against a real BIF
//! file (none is available in this environment): a frame's bytes could in principle stop the scan
//! early, so the consumer must treat a short index as "no thumbnail at this position", never as a
//! hard error.

/// One index entry: the frame's timestamp in milliseconds and the byte offset of its JPEG within
/// the frame-data section (which follows the index table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BifEntry {
    pub(crate) timestamp_ms: u32,
    pub(crate) offset: u32,
}

/// `\x89BIF\r\n\x1a\n` — the PNG-style magic with "BIF" in place of "PNG".
const BIF_MAGIC: [u8; 8] = [0x89, 0x42, 0x49, 0x46, 0x0D, 0x0A, 0x1A, 0x0A];

/// Parse the index table from a BIF file's bytes. `None` on a wrong magic or an empty index.
///
/// The header is a fixed 12 bytes (magic + a reserved big-endian version word). The index follows
/// as 8-byte entries. Because the header carries no entry count and the frame data sits
/// immediately after the index, the scan stops at the first entry whose offset is NOT monotonic
/// (frame offsets are strictly increasing) OR whose frame would reach past the end of the file —
/// the two signals that the scan has walked off the index into the frame bytes. This is
/// best-effort: the frame data is arbitrary JPEG bytes, so a short index is possible; the
/// consumer must treat a missing entry as "no thumbnail at this position", never as a hard error.
pub(crate) fn parse_bif_index(bytes: &[u8]) -> Option<Vec<BifEntry>> {
    if bytes.len() < 12 || bytes[..8] != BIF_MAGIC {
        return None;
    }
    let total = bytes.len();
    let mut entries = Vec::new();
    let mut prev_offset: i64 = -1;
    let mut pos = 12;
    while pos + 8 <= total {
        let timestamp_ms = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        let offset = u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]);
        // The frame data begins right after the index, so an entry whose offset would land past
        // the end of the file is not an index entry.
        if (offset as i64) < prev_offset || offset as usize > total - (pos + 8) {
            break; // reached the frame data
        }
        entries.push(BifEntry { timestamp_ms, offset });
        prev_offset = offset as i64;
        pos += 8;
    }
    if entries.is_empty() { None } else { Some(entries) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bif(entries: &[(u32, u32)], frames: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&BIF_MAGIC);
        out.extend_from_slice(&0u32.to_be_bytes()); // version
        for (ts, off) in entries {
            out.extend_from_slice(&ts.to_be_bytes());
            out.extend_from_slice(&off.to_be_bytes());
        }
        out.extend_from_slice(frames);
        out
    }

    #[test]
    fn parses_the_index_table_and_stops_at_the_frame_data() {
        // Frame data: 256 bytes, JPEG SOI first then zeros. The zeros make the byte right after
        // the index read as a non-monotonic offset, so the scan stops after the three real
        // entries rather than walking into the frames.
        let mut frames = vec![0u8; 256];
        frames[0] = 0xFF;
        frames[1] = 0xD8; // JPEG SOI
        let bytes = bif(&[(0, 0), (10_000, 64), (20_000, 128)], &frames);
        let idx = parse_bif_index(&bytes).expect("parses");
        assert_eq!(
            idx,
            vec![
                BifEntry { timestamp_ms: 0, offset: 0 },
                BifEntry { timestamp_ms: 10_000, offset: 64 },
                BifEntry { timestamp_ms: 20_000, offset: 128 },
            ]
        );
    }

    #[test]
    fn an_out_of_bounds_offset_ends_the_scan() {
        // The first "frame" is only 8 bytes, so the second entry's offset (1024) would reach past
        // the end of the file: the parser must stop after the first entry.
        let bytes = bif(&[(0, 0), (10_000, 1_024)], &[0xFF, 0xD8, 0, 0, 0, 0, 0, 0]);
        let idx = parse_bif_index(&bytes).expect("parses");
        assert_eq!(idx, vec![BifEntry { timestamp_ms: 0, offset: 0 }]);
    }

    #[test]
    fn a_wrong_magic_or_an_empty_index_is_none() {
        assert!(parse_bif_index(&[0u8; 32]).is_none(), "wrong magic");
        assert!(parse_bif_index(&bif(&[], &[0xFF, 0xD8])).is_none(), "no entries");
        assert!(parse_bif_index(&BIF_MAGIC).is_none(), "header only");
    }
}
