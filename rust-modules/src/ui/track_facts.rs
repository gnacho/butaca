//! **The media facts a track sheet and the pre-playback window both state** — the content model
//! of `screens::tracks_panel`, moved here when a second screen needed the same numbers (issue
//! #367).
//!
//! It is a LIBRARY module rather than a screen's because the sibling rule forbids one screen
//! naming another, and the alternative — a second copy of `fmt_size`/`audio_choice`/`subtitle_rows`
//! — is exactly the drift this app's shared formatters exist to prevent. Every spelling a viewer
//! can compare across two surfaces lives here once: the binary gigabyte, the kbps/Mbps split, the
//! STORED frame size rather than the `"4k"` class, the container's own uppercase, a language the
//! server named, and the technical codec spelling (`HEVC`, `EAC3`) both surfaces use.
//!
//! The panel keeps the DRAWING and this module keeps the WORDS, which is also what makes the words
//! gradeable without a font: everything here is pure and takes its inputs by argument.

use crate::metadata::{Detail, Stream};

/// One `label` over its `value`, the unit a facts grid is built from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Pair {
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

/// One track line: a name on the left, its distinguishing detail right-aligned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct TrackRow {
    pub(crate) name: String,
    pub(crate) detail: String,
}

/// A byte count as the design writes it — `"32.28 GB"`. **GiB, two decimals**: 34659545780 bytes
/// reads as 32.28 GB in the mock, which is `/1024³` and not `/1000³` (that would be 34.66). The
/// clients this app replaces used the binary units here.
pub(crate) fn fmt_size(bytes: i64) -> Option<String> {
    if bytes <= 0 {
        return None;
    }
    // Below a kilobyte the Info panel spells the count in words rather than as a bare `B`.
    Some(if bytes < 1024 {
        crate::i18n::msg::widgets_tracks_bytes(bytes)
    } else {
        crate::ui::fmt::bytes(bytes, (2, 2, 2))
    })
}

/// A server bitrate (which is in **kbps**) as `"28.9 Mbps"`, or `"640 kbps"` below a megabit. The
/// split is what keeps an audio track's 448 from reading as `0.4 Mbps`.
pub(crate) fn fmt_bitrate(kbps: i64) -> Option<String> {
    match kbps {
        k if k <= 0 => None,
        k => Some(crate::ui::fmt::bitrate(k)),
    }
}

/// `"HEVC (Main 10)"` — the video codec in the spelling `ui::info_panel` already uses, with its
/// profile Title-Cased in brackets. A codec with no profile is the bare name; no codec at all is
/// no row.
pub(crate) fn fmt_codec_profile(codec: &str, profile: &str) -> Option<String> {
    let name = crate::ui::info_panel::video_codec_name(codec);
    if name.is_empty() {
        return None;
    }
    let p = title_case(profile);
    Some(if p.is_empty() {
        name
    } else {
        format!("{name} ({p})")
    })
}

/// `"main 10"` → `"Main 10"`. Per WORD, and only the first character of each — upper-casing the
/// whole run would give `MAIN 10`, which reads as a second acronym beside `HEVC`.
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, w) in s.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut cs = w.chars();
        if let Some(c) = cs.next() {
            out.extend(c.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    out
}

/// `"10-bit · 4:2:0"` — bit depth and chroma as ONE fact, because neither is meaningful without
/// the other. Either half alone still draws, so a container that reports only one is not silent.
pub(crate) fn fmt_depth_chroma(bit_depth: i64, chroma: &str) -> Option<String> {
    let d = (bit_depth > 0).then(|| crate::i18n::msg::widgets_tracks_depth(bit_depth));
    let c = (!chroma.trim().is_empty()).then(|| chroma.trim().to_string());
    match (d, c) {
        (Some(d), Some(c)) => Some(format!("{d} \u{b7} {c}")),
        (Some(d), None) => Some(d),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    }
}

/// `"3840 × 1602"` — the STORED frame size, with U+00D7, never the `videoResolution` class. The
/// class for this file is the string `"4k"`, which is a badge and not a resolution: the whole
/// point of this row is that the file is 3840×1602 and not 3840×2160.
pub(crate) fn fmt_frame_size(w: i64, h: i64) -> Option<String> {
    (w > 0 && h > 0).then(|| format!("{w} \u{d7} {h}"))
}

/// `"23.976 fps"` — three decimals, trailing zeros trimmed, so 24.0 reads `"24 fps"` and 23.976
/// keeps every digit that distinguishes it from 24.
pub(crate) fn fmt_fps(fps: f64) -> Option<String> {
    if !(fps > 0.0) {
        return None;
    }
    let rate = localized_precision(fps, 3);
    Some(crate::i18n::msg::widgets_tracks_fps(&rate))
}

/// `"2.35"`, the display aspect as the server sends it, trailing zeros trimmed.
pub(crate) fn fmt_aspect(ar: f64) -> Option<String> {
    if !(ar > 0.0) {
        return None;
    }
    Some(localized_precision(ar, 2))
}

/// Keep the source's useful decimal precision, then let the selected region write separators.
fn localized_precision(value: f64, max_scale: i16) -> String {
    let mut coefficient = (value * 10_i64.pow(max_scale as u32) as f64).round() as i64;
    let mut scale = max_scale;
    while scale > 0 && coefficient % 10 == 0 {
        coefficient /= 10;
        scale -= 1;
    }
    crate::i18n::current().decimal(coefficient, scale)
}

/// A channel layout in the design's spelling: `"5.1(side)"` → `"5.1"`, `"stereo"` → `"Stereo"`.
///
/// The parenthetical is ffmpeg's channel-ORDER note, not a different layout — `5.1(side)` and
/// `5.1` are both six channels — so it is dropped rather than shown, and a word layout is
/// capitalised because it sits mid-sentence beside a codec. Falls back to the channel COUNT when
/// the server sent no layout at all, which is the only other thing that can be said.
pub(crate) fn fmt_layout(layout: &str, channels: i64) -> String {
    let base = layout.split('(').next().unwrap_or("").trim();
    if base.is_empty() {
        return match channels {
            0 => String::new(),
            1 => crate::i18n::msg::widgets_tracks_mono().to_string(),
            2 => crate::i18n::msg::widgets_tracks_stereo().to_string(),
            n => crate::i18n::msg::widgets_tracks_channels(n),
        };
    }
    // a numeric layout ("5.1", "7.1") is already how it is written; a word one is capitalised
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.to_string()
    } else {
        match base {
            "mono" => crate::i18n::msg::widgets_tracks_mono().to_string(),
            "stereo" => crate::i18n::msg::widgets_tracks_stereo().to_string(),
            _ => title_case(base),
        }
    }
}

/// Does this audio track carry Dolby Atmos? — [`Stream::has_atmos`], which is where the rule and
/// its evidence live now.
///
/// It moved to the data layer when the Load payload gained a second consumer for the same answer
/// (`contents.immersive`): a predicate that decides what we tell the television's pipeline is not
/// a property of the panel that happened to need it first. This stays as the free-function spelling
/// the draw code around it reads in.
pub(crate) fn has_atmos(s: &Stream) -> bool {
    s.has_atmos()
}

/// One audio track's right-hand detail: `"EAC3 5.1 + Atmos · 768 kbps · playing"`.
///
/// Every part of it exists to separate tracks a language cannot. `playing` is `Stream.selected` —
/// the server's current pick for this part, i.e. the track that will be used — and it closes the
/// line because it is the only clause about US rather than about the file.
pub(crate) fn audio_detail(s: &Stream) -> String {
    let mut line = audio_choice(s);
    if s.selected {
        if line.is_empty() {
            line = crate::i18n::msg::widgets_tracks_playing().to_string();
        } else {
            line.push_str(" \u{b7} ");
            line.push_str(crate::i18n::msg::widgets_tracks_playing());
        }
    }
    line
}

/// One audio track as a CHOICE: `"EAC3 5.1 + Atmos · 768 kbps"` — every clause [`audio_detail`]
/// composes except its `playing` tail, because a picker states the option and the cursor states
/// the state. `screens::preplay_panel` draws its rows from this, so the window and the inspector
/// cannot spell one track two ways.
pub(crate) fn audio_choice(s: &Stream) -> String {
    let mut head = s.codec.to_uppercase();
    let layout = fmt_layout(&s.layout, s.channels);
    if !layout.is_empty() {
        head = if head.is_empty() {
            layout
        } else {
            format!("{head} {layout}")
        };
    }
    if has_atmos(s) {
        head = if head.is_empty() {
            "Atmos".to_string()
        } else {
            format!("{head} + Atmos")
        };
    }
    let mut parts: Vec<String> = Vec::new();
    if !head.is_empty() {
        parts.push(head);
    }
    if let Some(b) = fmt_bitrate(s.bitrate) {
        parts.push(b);
    }
    parts.join(" \u{b7} ")
}

/// The audio section's rows — the server's own `Stream[]` order, one row per track, NOT grouped.
///
/// Deliberately unlike [`subtitle_rows`], and the design says why in its own comment: *"Language
/// and codec alone do not tell seven Russian tracks apart, so the row carries the channel layout
/// and bitrate that do."* Grouping them would collapse eight distinguishable tracks into two
/// counts and destroy exactly the information the panel exists to show.
pub(crate) fn audio_rows(audio: &[Stream]) -> Vec<TrackRow> {
    audio
        .iter()
        .map(|s| TrackRow {
            name: if s.lang.trim().is_empty() {
                crate::i18n::msg::widgets_tracks_unknown().to_string()
            } else {
                s.lang.clone()
            },
            detail: audio_detail(s),
        })
        .collect()
}

/// The subtitle section's rows — identical tracks GROUPED and counted:
/// `Russian · MOV_TEXT · 6 tracks`.
///
/// The design's reasoning, verbatim: *"These ARE identical to one another — same codec, no title,
/// no forced or SDH flag — so they are counted rather than listed nine times."* Nine rows of the
/// same two words is a list you cannot read; three rows with a count is the same information.
///
/// So the grouping KEY is everything that could make two tracks different — language, codec, and
/// the three flags a viewer would pick between (`forced`, `sdh`, `external`) — plus the track's
/// own `title` where it has one. Two tracks collapse only when there is genuinely nothing left to
/// tell them apart; a forced track never merges into a full one. First-seen order is preserved,
/// which is the server's order, so the list still reads in container sequence.
pub(crate) fn subtitle_rows(subs: &[Stream]) -> Vec<TrackRow> {
    // (key, name, detail-head, count) — a Vec rather than a map so first-seen order survives; a
    // subtitle list is a handful of entries, so the linear scan is not worth a hash.
    let mut out: Vec<(String, String, String, usize)> = Vec::new();
    for s in subs {
        let name = subtitle_name(s);
        let head = subtitle_head(s, &name);
        let key = format!("{}\u{0}{}", name, head);
        match out.iter_mut().find(|e| e.0 == key) {
            Some(e) => e.3 += 1,
            None => out.push((key, name, head, 1)),
        }
    }
    out.into_iter()
        .map(|(_, name, head, n)| {
            let detail = match (head.is_empty(), n) {
                (true, 1) => String::new(),
                (true, n) => crate::i18n::msg::widgets_tracks_count(n as i64),
                (false, 1) => head,
                (false, n) => crate::i18n::msg::widgets_tracks_count_detail(n as i64, &head),
            };
            TrackRow { name, detail }
        })
        .collect()
}

/// One subtitle track as a CHOICE line: `"English · SUBRIP · Forced"`. The same name and head the
/// grouped list composes, for ONE track and with no count — a picker lists the tracks a viewer may
/// choose between, it does not summarise them. `screens::preplay_panel` draws its rows from this.
pub(crate) fn subtitle_choice(s: &Stream) -> String {
    let name = subtitle_name(s);
    let head = subtitle_head(s, &name);
    if head.is_empty() {
        name
    } else {
        format!("{name} \u{b7} {head}")
    }
}

/// A track's display name: the server's own language string, or the app's word for "it did not
/// say".
fn subtitle_name(s: &Stream) -> String {
    if s.lang.trim().is_empty() {
        crate::i18n::msg::widgets_tracks_unknown().to_string()
    } else {
        s.lang.clone()
    }
}

/// The distinguishing half of a subtitle line: codec, then the three flags a viewer picks between,
/// then the title's own source. ONE function for the grouped list and the single-track choice, so
/// a forced track cannot read "Forced" in the inspector and nothing in the picker.
fn subtitle_head(s: &Stream, name: &str) -> String {
    use crate::metadata::track_label;
    let label = track_label::parse(&s.title, name, s.forced, s.sdh);
    let mut head = s.codec.to_uppercase();
    // Both flags can be set on one track (a forced SDH file): show both, as this panel always
    // has; the parsed kind only ADDS a flag the title spelled out.
    let (forced, sdh) = track_label::flags(&label, s.forced, s.sdh);
    for (on, tag) in [
        (forced, crate::i18n::msg::widgets_tracks_forced()),
        (sdh, crate::i18n::msg::widgets_badge_sdh()),
        (s.external, crate::i18n::msg::widgets_tracks_external()),
    ] {
        if on {
            head = if head.is_empty() {
                tag.to_string()
            } else {
                format!("{head} \u{b7} {tag}")
            };
        }
    }
    if !label.source.trim().is_empty() && !label.source.eq_ignore_ascii_case(name) {
        head = if head.is_empty() {
            label.source.clone()
        } else {
            format!("{head} \u{b7} {}", label.source.trim())
        };
    }
    head
}

/// The FILE column.
pub(crate) fn file_rows(d: &Detail) -> Vec<Pair> {
    let mut v = Vec::new();
    let mut push = |label, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            v.push(Pair { label, value });
        }
    };
    push(
        crate::i18n::msg::widgets_tracks_container(),
        (!d.container.is_empty()).then(|| d.container.to_uppercase()),
    );
    push(crate::i18n::msg::widgets_tracks_size(), fmt_size(d.size));
    push(crate::i18n::msg::widgets_tracks_total_bitrate(), fmt_bitrate(d.bitrate));
    push(
        crate::i18n::msg::widgets_tracks_duration(),
        (d.dur_ms > 0).then(|| crate::ui::fmt::clock(d.dur_ms)),
    );
    push(crate::i18n::msg::widgets_tracks_aspect(), fmt_aspect(d.aspect_ratio));
    v
}

/// The VIDEO column. Reads the video STREAM where the item has one and falls back to the
/// Media-level summary for the two facts that live on both — a show whose technicals were
/// backfilled from episode 1 has the summary but no stream.
pub(crate) fn video_rows(d: &Detail) -> Vec<Pair> {
    let vs = d.video.as_ref();
    let mut v = Vec::new();
    let mut push = |label, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            v.push(Pair { label, value });
        }
    };
    let codec = vs
        .map(|s| s.codec.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(d.vcodec.as_str());
    let profile = vs.map(|s| s.profile.as_str()).unwrap_or("");
    push(crate::i18n::msg::widgets_tracks_codec(), fmt_codec_profile(codec, profile));
    push(crate::i18n::msg::widgets_tracks_resolution(), fmt_frame_size(d.width, d.height));
    push(crate::i18n::msg::widgets_tracks_frame_rate(), fmt_fps(d.video_fps));
    // the STREAM's bitrate, not the file's — `d.bitrate` is already the FILE column's own row, and
    // repeating it here would state the same number twice under two different labels
    push(crate::i18n::msg::widgets_tracks_bitrate(), vs.and_then(|s| fmt_bitrate(s.bitrate)));
    push(
        crate::i18n::msg::widgets_tracks_bit_depth(),
        vs.and_then(|s| fmt_depth_chroma(s.bit_depth, &s.chroma)),
    );
    v
}

/// The DOLBY VISION column — **empty unless the file actually carries DV**, which is what makes
/// the third column appear on a DV item and be absent everywhere else. Every individual row is
/// still optional inside that, because "present" and "the server told us the level" are different
/// facts.
pub(crate) fn dovi_rows(d: &Detail) -> Vec<Pair> {
    if !d.dovi.present {
        return Vec::new();
    }
    let dv = &d.dovi;
    let mut v = Vec::new();
    let mut push = |label, value: Option<String>| {
        if let Some(value) = value.filter(|s: &String| !s.is_empty()) {
            v.push(Pair { label, value });
        }
    };
    push(crate::i18n::msg::widgets_tracks_profile(), (dv.profile > 0).then(|| dv.profile.to_string()));
    push(crate::i18n::msg::widgets_tracks_level(), (dv.level > 0).then(|| dv.level.to_string()));
    push(crate::i18n::msg::widgets_tracks_version(), dv.version_str());
    push(crate::i18n::msg::widgets_tracks_base_layer(), dv.bl_present.then(|| crate::i18n::msg::widgets_tracks_present().to_string()));
    push("RPU", dv.rpu_present.then(|| crate::i18n::msg::widgets_tracks_present().to_string()));
    v
}

/// A section head as it is drawn: `AUDIO · 8 TRACKS`. Caps, because the head names the group.
pub(crate) fn track_head(word: &str, n: usize) -> String {
    crate::i18n::msg::widgets_tracks_count_heading(n as i64, word)
}
