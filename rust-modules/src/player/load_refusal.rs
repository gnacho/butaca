//! **What a Load refusal NAMES** — the prerequisite the corrective dual-axis reload of upstream
//! plx-native `62354d903` needs, and nothing more than that.
//!
//! Today a refusal is one fact: the pipeline said no before any picture, `load_failed` latches, and
//! the pump turns it into `Error` with `FailureKind::TvPipeline`. What it never said is WHICH axis
//! it refused. Without that, a corrective reload would have to guess: re-declare both codecs and
//! hope, which is a second play whose failure tells you no more than the first.
//!
//! The lab report this module is written from (`docs/webos10-lab-report.md` §3.5) records one
//! refusal payload — `type=18 num=601 "Resource Allocation Error"` — which names no codec at all,
//! and the event arm's own comment says the lab could not establish that 18 is the only refusal
//! type. So the honest classification is not "codec vs resource": it is **"the pipeline named an
//! axis" versus "it did not"**, and only the first licenses anything.
//!
//! [`classify`] answers from the refusal's own text and from the vocabulary this app declares
//! ([`crate::player::engine::audio_payload_codec`]'s names and the video codecs the Load payload
//! can carry), so a named axis is evidence rather than a guess. Anything else is [`Refusal::Unnamed`]
//! and the caller keeps behaving exactly as it does today: log the number, fail the source.

/// Which half of the declaration a refusal points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis {
    Video,
    Audio,
}

/// What a refusal payload told us.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    /// The payload names a codec this app declares, so the axis is evidence.
    Named(Axis),
    /// The payload names no codec we recognize — the lab's own case, and the ordinary case for a
    /// resource failure. No claim is made, and no corrective reload is licensed.
    Unnamed,
}

/// The video codecs the Load payload can name (`build_av_payload`'s `video` field).
const VIDEO: [&str; 2] = ["h264", "hevc"];
/// The audio codecs the payload table accepts (`audio_payload_codec`), family names included: a
/// pipeline that names `dca` is naming the DTS family.
const AUDIO: [&str; 7] = ["aac", "eac3", "ec-3", "ac3", "dts", "dca", "truehd"];

/// **Classify a refusal from its own payload.** A hit on either vocabulary names that axis; a
/// payload with none of them — the lab's `Resource Allocation Error` included — is
/// [`Refusal::Unnamed`], which is the answer that keeps today's behaviour.
///
/// Video is checked first because it is the larger failure: a picture the set cannot decode costs
/// the whole play, where an audio refusal is the half a corrective reload can repair by itself.
pub(crate) fn classify(payload: &str) -> Refusal {
    // **`H.264` is how the pipeline itself spells it** (the sink echo in the lab report is
    // `0 video/x-h264 …`, but a refusal names the decoder the way people do), so a dot is dropped
    // before matching. Only a dot: `ec-3` is a codec name with a hyphen in it, and the vocabulary
    // below is written the way those arrive.
    let lowered = payload.to_ascii_lowercase().replace('.', "");
    if VIDEO.iter().any(|codec| mentions(&lowered, codec)) {
        return Refusal::Named(Axis::Video);
    }
    if AUDIO.iter().any(|codec| mentions(&lowered, codec)) {
        return Refusal::Named(Axis::Audio);
    }
    Refusal::Unnamed
}

/// Does the payload mention this codec as a TOKEN? Plain `contains` is wrong in both directions: it
/// would find `h264` inside a longer hex run, and it would match `ac3` inside `eac3` — so a refusal
/// naming `eac3` would be read as AC-3, the one confusion that has a device history here (declaring
/// AC-3 over E-AC-3 bytes).
fn mentions(payload: &str, codec: &str) -> bool {
    let bytes = payload.as_bytes();
    let needle = codec.as_bytes();
    let mut from = 0;
    while let Some(at) = payload[from..].find(codec) {
        let start = from + at;
        let end = start + needle.len();
        // `ac3` inside `eac3` has a codec character before it (`e`), so this one test excludes it;
        // there is deliberately no special case for the prefix, because that would be a second
        // rule for the same confusion.
        let before_ok = start == 0 || !is_codec_char(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_codec_char(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Characters that make a codec name part of a longer word: a hex digest (`h264` inside `0a264c`)
/// must not count, and `x264` must not read as `h264`.
fn is_codec_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The lab's own refusal names no axis**, and that is the case that must keep today's
    /// behaviour: fail the source, make no claim about which half was refused.
    #[test]
    fn the_labs_resource_refusal_names_no_axis() {
        assert_eq!(classify("Resource Allocation Error"), Refusal::Unnamed);
        assert_eq!(classify(""), Refusal::Unnamed);
        assert_eq!(classify("load completed but no sink"), Refusal::Unnamed);
    }

    /// A refusal that DOES name a codec names the axis — the only shape that licenses a corrective
    /// reload, and the reason this module exists.
    #[test]
    fn a_refusal_that_names_a_codec_names_its_axis() {
        assert_eq!(classify("unsupported video codec vp9"), Refusal::Unnamed, "not a codec we declare");
        assert_eq!(classify("H.264 decoder refused"), Refusal::Named(Axis::Video));
        assert_eq!(classify("hevc: unsupported level"), Refusal::Named(Axis::Video));
        assert_eq!(classify("audio/x-dts not supported"), Refusal::Named(Axis::Audio));
        assert_eq!(classify("AAC decoder allocation failed"), Refusal::Named(Axis::Audio));
    }

    /// **`eac3` is not `ac3`.** Declaring AC-3 over E-AC-3 bytes is the confusion with a device
    /// history here, so a refusal naming E-AC-3 must read as E-AC-3 — and a hex run that happens to
    /// contain `h264` must not read as a codec at all.
    #[test]
    fn a_codec_name_has_to_be_a_token() {
        assert_eq!(classify("eac3 decoder refused"), Refusal::Named(Axis::Audio));
        assert_eq!(classify("0a264c1b failure"), Refusal::Unnamed, "a hex digest is not a codec");
        assert_eq!(classify("x264 refused"), Refusal::Unnamed, "x264 is not the h264 we declare");
    }

    /// Video outranks audio when a payload names both: a picture the set cannot decode costs the
    /// whole play, so the axis a corrective reload would attack first is the video one.
    #[test]
    fn a_payload_naming_both_axes_is_a_video_refusal() {
        assert_eq!(classify("h264 + aac unsupported"), Refusal::Named(Axis::Video));
    }
}
