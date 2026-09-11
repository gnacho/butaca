//! profile — the DeviceProfile this television hands to Jellyfin's Media Decision Engine
//! (`POST /Items/{id}/PlaybackInfo`), the Jellyfin counterpart of the Plex profile
//! `transcoder.rs::profile_for` builds.
//!
//! ## One source of truth, read twice
//!
//! Every capability in the profile is read off [`crate::devcaps::caps`] — the SAME table
//! `route::video_direct_plays` gates on — because a profile that claims more than the local gate
//! is the over-claim bug class in both directions: the server would direct-offer what the
//! pipeline cannot decode, or transcode down what it can. Building the profile from the table
//! (rather than hand-keeping a second list) is what makes "the two cannot drift apart" literal
//! (`Caps::audio`'s own doc makes the same promise for the Plex profile).
//!
//! ## What the profile deliberately does not say
//!
//! - **No VP9/AV1 in DirectPlayProfiles even when the panel decodes them** — the buffer-feed
//!   pipeline can only feed H.264/HEVC (the gate's own comment), so the panel's decoder is
//!   unreachable and advertising it would be an over-claim.
//! - **No image subtitle formats** (PGS/VobSub) — the direct-play path renders text subtitles
//!   itself; an image format offered as `External` would arrive as a bitmap the renderer has no
//!   path for. Omitting them lets the server's MDE route a PGS-selected item to the transcoder,
//!   which is the honest PoC answer (on-device validation decides if burn-in is acceptable).
//! - **Resolution bounds come from `hevc_max`** — the per-axis MIN across both decoder rows,
//!   i.e. the one bound that is true for every codec at once, which is exactly what a
//!   `CodecProfile` with a `*`-style scope must express (see `Caps::hevc_max`'s doc for the
//!   SoC shape that rule exists for).

/// The DeviceProfile as a JSON body, ready to POST. `ceiling` is the user's quality rung when
/// one is set (`route::Quality::ceiling()`): it becomes `MaxStreamingBitrate`, so a picked rung
/// binds the transcode the same way `maxVideoBitrate` binds a Plex one.
pub(crate) fn device_profile(ceiling: Option<crate::plex::Ceiling>) -> serde_json::Value {
    let caps = crate::devcaps::caps();
    let max_bps = ceiling
        .map(|c| c.max_kbps.saturating_mul(1_000))
        .unwrap_or(120_000_000);
    // The video codec list the pipeline can be FED — `route::video_direct_plays`'s exact
    // vocabulary (h264 always, hevc where the SoC's table lists the decoder).
    let video_codecs = if caps.hevc { "h264,hevc" } else { "h264" };
    let (max_w, max_h) = caps.hevc_max;
    let mut codec_profiles = Vec::new();
    if max_w > 0 && max_h > 0 {
        // One resolution bound applied to BOTH codecs — the per-axis min, per hevc_max's doc.
        for codec in ["h264", "hevc"] {
            codec_profiles.push(serde_json::json!({
                "Type": "Video",
                "Codec": codec,
                "Conditions": [
                    {"Condition": "LessThanEqual", "Property": "Width", "Value": max_w.to_string(), "IsRequired": false},
                    {"Condition": "LessThanEqual", "Property": "Height", "Value": max_h.to_string(), "IsRequired": false}
                ]
            }));
        }
    }
    serde_json::json!({
        "MaxStreamingBitrate": max_bps,
        "MaxStaticBitrate": max_bps,
        // The containers the demuxer actually streams — `route::part_is_streamable`'s list,
        // spelled Jellyfin's way. Anything else (avi, mov, wtv…) must come back as a transcode,
        // where Jellyfin remuxes-or-encodes as its own transcoder decides.
        "DirectPlayProfiles": [
            {"Type": "Video", "Container": "mkv", "VideoCodec": video_codecs, "AudioCodec": caps.audio},
            {"Type": "Video", "Container": "mp4,m4v", "VideoCodec": video_codecs, "AudioCodec": caps.audio},
            {"Type": "Audio", "Container": "mp3,aac,flac,ogg", "AudioCodec": "mp3,aac,flac,vorbis"}
        ],
        "TranscodingProfiles": [
            {
                "Type": "Video",
                "Container": "ts",
                "Protocol": "hls",
                // The re-encode TARGET is h264+aac even on an HEVC panel: Jellyfin's HEVC HLS
                // (fMP4, hvc1 tagging, webOS HLS player quirks) is precisely the unvalidated
                // surface the PoC must not depend on — 1080p h264 is the floor every path
                // already trusts. The direct-play gate keeps 4K HEVC OUT of this branch.
                "VideoCodec": "h264",
                "AudioCodec": "aac",
                "Context": "Streaming",
                "BreakOnNonKeyFrames": true,
                "MinSegments": 2,
                "SegmentLength": 6
            }
        ],
        "ContainerProfiles": [],
        "CodecProfiles": codec_profiles,
        // Text subtitles render client-side (External); image formats are deliberately absent —
        // see the module doc.
        "SubtitleProfiles": [
            {"Format": "srt", "Method": "External"},
            {"Format": "subrip", "Method": "External"},
            {"Format": "ass", "Method": "External"},
            {"Format": "ssa", "Method": "External"},
            {"Format": "srt", "Method": "Embed"},
            {"Format": "ass", "Method": "Embed"},
            {"Format": "ssa", "Method": "Embed"}
        ],
        "ResponseProfiles": []
    })
}

/// The whole `PlaybackInfo` request body for one item. `UserId` is deliberately ABSENT — the
/// client fills it from its own token state at POST time, so this builder never touches
/// credential material. `start_ticks` carries the resume point when this POST decides a
/// RESUME's transcode (the HLS playlist is cut from that position server-side); a fresh start
/// passes 0 and the field simply says so.
pub(crate) fn playback_info_body(
    ceiling: Option<crate::plex::Ceiling>,
    start_ticks: i64,
) -> serde_json::Value {
    serde_json::json!({
        "DeviceProfile": device_profile(ceiling),
        "StartTimeTicks": start_ticks,
        "EnableDirectPlay": true,
        "EnableDirectStream": true,
        "EnableTranscoding": true,
        "AllowVideoStreamCopy": true,
        "AllowAudioStreamCopy": true,
        "AutoOpenLiveStream": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_claims_exactly_what_the_gate_allows() {
        let p = device_profile(None);
        let s = serde_json::to_string(&p).unwrap();
        // direct-play containers are the demuxer's own list — no avi, no mov
        assert!(s.contains("\"Container\":\"mkv\""));
        assert!(s.contains("mp4,m4v"));
        assert!(!s.contains("avi"));
        // no image subtitle formats anywhere
        assert!(!s.to_lowercase().contains("pgs"));
        assert!(!s.to_lowercase().contains("vobsub"));
        // the transcode target is the trusted floor, not the panel's best decoder
        assert!(s.contains("\"VideoCodec\":\"h264\""));
        assert!(s.contains("\"Protocol\":\"hls\""));
    }

    #[test]
    fn a_quality_rung_becomes_max_streaming_bitrate() {
        let c = crate::plex::Ceiling { max_kbps: 8_000, max_w: 1920, max_h: 1080 };
        let p = device_profile(Some(c));
        assert_eq!(p["MaxStreamingBitrate"], 8_000_000);
        let p = device_profile(None);
        assert_eq!(p["MaxStreamingBitrate"], 120_000_000);
    }
}
