//! Detail hero policy: dynamic action controls, people credits, playback facts and their exact
//! width/priority rules. Painting orchestration stays in [`super::DetailScreen`]; the decisions
//! here are explicit-value helpers so host tests need no global focus state.
//!
//! Control selection and geometry take explicit `Detail`/`HeroSet` values. The playback-facts
//! helpers also consult cached application policy and the text renderer; they are not all pure
//! host-only functions. The screen owns the metadata identity and supplies the current item.
//!
//! These helpers are orchestrated by the completed screen's direct `Focusable`/`Screen` impls;
//! they do not own an independent focus cursor or screen lifetime.
//!
//! # The `ElemKey` numbering convention this package establishes (spec §13's own instruction:
//! "reserve a range so the next package can pick a disjoint one")
//!
//! Hero controls occupy the `u32` elem range **`[0, HERO_ELEM_RANGE_END)` = `[0, 64)`**. Every
//! constant below is an IDENTITY, not a POSITION — `ELEM_MARK_WATCHED`/`ELEM_MARK_UNWATCHED` are
//! two DIFFERENT ids for the two faces one control wears, never both present at once, which is
//! what lets [`DetailScreen::reconcile`](super::DetailScreen) carry the focus across a press that
//! flips the toggle (see that impl's doc — the engine calls `reconcile` with whatever key focus
//! currently holds, every frame, so identity-as-elem plus a membership test reproduces
//! the legacy `hero_col` correction without a second cursor). Section helpers use disjoint LOCAL
//! slot ranges starting at [`HERO_ELEM_RANGE_END`]; `DetailScreen` projects repeated items to stable
//! engine identities instead of exposing those positional slots as focus keys.
use std::ffi::CStr;

use crate::metadata::{Detail, Episode, Extra};
use crate::ui::label::HAlign;
use crate::ui::machine::{GroupId, Measure};
use crate::ui::text_view::TextView;
use crate::ui::widgets::PosterMark;
use crate::ui::{theme, Rect};

/// The hero's whole `u32` elem namespace. A later Detail package's own range must start here.
pub(crate) const HERO_ELEM_RANGE_END: u32 = 64;

pub(crate) const ELEM_PLAY: u32 = 0;
pub(crate) const ELEM_RESTART: u32 = 1;
pub(crate) const ELEM_ALT: u32 = 2;
pub(crate) const ELEM_MARK_WATCHED: u32 = 3;
pub(crate) const ELEM_MARK_UNWATCHED: u32 = 4;
pub(crate) const ELEM_TRAILER: u32 = 5;
pub(crate) const ELEM_FAVORITE: u32 = 6;
/// The audio-track disc: which language the film starts in, chosen before the picture.
pub(crate) const ELEM_AUDIO: u32 = 7;
/// The subtitle disc: the track the film starts with, or Off.
pub(crate) const ELEM_SUBTITLES: u32 = 8;
/// The hero synopsis' `MORE` mark. Not a disc in the action row: it sits under the prose and its
/// own group places it there (`screens::detail::SYNOPSIS_GROUP`).
pub(crate) const ELEM_SYNOPSIS_MORE: u32 = 9;
/// **The artwork disc**: the pictures the metadata providers already hold for the item, so a
/// poster the agent got wrong can be replaced without leaving the page. It is the entry the
/// pre-playback window's three-dot menu carried ("Cambiar imagen"), and the owner asked for it to
/// survive that window's retirement.
pub(crate) const ELEM_ARTWORK: u32 = 10;

/// The hero's one focus group. Local to this screen — nothing outside `screens::detail` ever names
/// it — so, like `screens::profiles`'s `ROSTER_GROUP`/`FOOTER_GROUP`, any small integer would do;
/// `0` matches this family's own convention of seating group `GroupId(0)` as a screen's primary
/// group.
pub(crate) const HERO_GROUP: GroupId = GroupId(0);

/// The Play/Resume pill's minimum width — a pathologically short label still gets a pill.
const PW: f32 = 168.0;

/// The focused disc's verb caption fades in at the row's own pace - the profile chip's unfurl
/// stiffness, which is what this interaction is: a name arriving beside a control that does not
/// move. Not [`crate::ui::widgets`]'s, because that widget no longer has an unfurl to lend.
pub(crate) const K_DISC_VERB: f32 = 240.0;

/// The row's inter-control air and its disc diameter — the SHARED control-family numbers
/// (`ui::widgets::CTRL_GAP`/`StatusOverlay::CTRL_H`), not a second copy of them.
const CGAP: f32 = crate::ui::widgets::CTRL_GAP;
pub(crate) const CD: f32 = crate::ui::widgets::StatusOverlay::CTRL_H;
const PEOPLE_MAX_LINES: usize = 2;
const PEOPLE_CAST: usize = 3;
const PEOPLE_LABEL_INK: [f32; 4] = theme::TEXT_TERTIARY;
/// The facts row runs to the safe edge: the people band is BELOW it, not beside it, so the row no
/// longer has a column to stop short of (issue #323). The owner's 2026-11-10 pass justifies the
/// whole action row to the RIGHT safe edge, so the two tests that hold the widest control set now
/// measure against `SCR_W - MARGIN_X` directly.

/// Duplicated from `widgets::Button`'s own PRIVATE `BTN_ICON_RATIO`/`BTN_ICON_GAP` (1.15, 12.0):
/// those two constants are `widgets.rs`-internal, and this module may not reach past that file's
/// `pub`/`pub(crate)` surface (the layer rule: a screen names `ui/`'s public surface, never its
/// internals). `BTN_PILL_AIR` IS `pub(crate)` and is reused directly rather than copied.
///
/// A drift here is COSMETIC, not a hit-test miss: whatever width this formula computes is fed to
/// BOTH [`Focusable::place`](super::DetailScreen) and `draw`'s `Button::new(label, sz, rect)` (which
/// only ever fills the rect it is given — see `widgets::Button::new`'s doc), so the two paths can
/// disagree with the REAL widget's own preferred width, never with EACH OTHER.
const HERO_ICON_RATIO: f32 = 1.15;
const HERO_ICON_GAP: f32 = 12.0;

pub(crate) fn alt_label() -> &'static CStr { crate::i18n::msg::browse_detail_also_available_c() }

fn mark_watched_label() -> &'static CStr { crate::i18n::msg::browse_detail_mark_watched_c() }
fn mark_unwatched_label() -> &'static CStr { crate::i18n::msg::browse_detail_mark_unwatched_c() }
fn mark_show_watched_label() -> &'static CStr { crate::i18n::msg::browse_detail_mark_show_watched_c() }
fn mark_show_unwatched_label() -> &'static CStr { crate::i18n::msg::browse_detail_mark_show_unwatched_c() }
fn play_from_start_label() -> &'static CStr { crate::i18n::msg::browse_detail_play_start_c() }
fn trailer_label() -> &'static CStr { crate::i18n::msg::browse_detail_trailer_c() }

/// A control in the hero action row, named rather than numbered — ported verbatim from
/// `ui/detail.rs::HeroCtl`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HeroCtl {
    /// the Play/Resume pill — always present, always first
    Play,
    /// the ↺ disc, present only while there is a resume point for it to ignore
    Restart,
    /// the Trailer disc, present only while the loaded item has a playable extra
    Trailer,
    /// the *Also available* pill, present only while a second pinned source holds this item
    Alt,
    /// the ✓ face of the watched TOGGLE — worn while the item is not watched (part-watched
    /// included)
    MarkWatched,
    /// the − face of that same toggle — worn once the item IS watched
    MarkUnwatched,
    /// the heart: worn only on a backend that HAS favorites (the Jellyfin flavor) — filled when
    /// the item is one. One identity for both states, like the watched pair shares its slot.
    Favorite,
    /// the audio-language disc (issue #4 of the owner's pass): the track the play will start in.
    /// Shown only when the file HAS a choice to make.
    Audio,
    /// the subtitle disc: the track the play will start with, or Off.
    Subtitles,
    /// the artwork disc: the provider pictures this item may wear instead of the one it has.
    Artwork,
}

impl HeroCtl {
    /// This control's fixed `u32` identity (never its position — see the module doc).
    pub(crate) const fn elem(self) -> u32 {
        match self {
            HeroCtl::Play => ELEM_PLAY,
            HeroCtl::Restart => ELEM_RESTART,
            HeroCtl::Trailer => ELEM_TRAILER,
            HeroCtl::Alt => ELEM_ALT,
            HeroCtl::MarkWatched => ELEM_MARK_WATCHED,
            HeroCtl::MarkUnwatched => ELEM_MARK_UNWATCHED,
            HeroCtl::Favorite => ELEM_FAVORITE,
            HeroCtl::Audio => ELEM_AUDIO,
            HeroCtl::Subtitles => ELEM_SUBTITLES,
            HeroCtl::Artwork => ELEM_ARTWORK,
        }
    }
    /// The inverse of [`elem`](Self::elem) — `None` for any `u32` outside the six identities
    /// above, which is every elem a NON-hero group can mint (the completed sections use
    /// their own disjoint range, per the module doc).
    pub(crate) fn of_elem(e: u32) -> Option<Self> {
        match e {
            ELEM_PLAY => Some(HeroCtl::Play),
            ELEM_RESTART => Some(HeroCtl::Restart),
            ELEM_TRAILER => Some(HeroCtl::Trailer),
            ELEM_ALT => Some(HeroCtl::Alt),
            ELEM_MARK_WATCHED => Some(HeroCtl::MarkWatched),
            ELEM_MARK_UNWATCHED => Some(HeroCtl::MarkUnwatched),
            ELEM_FAVORITE => Some(HeroCtl::Favorite),
            ELEM_AUDIO => Some(HeroCtl::Audio),
            ELEM_SUBTITLES => Some(HeroCtl::Subtitles),
            ELEM_ARTWORK => Some(HeroCtl::Artwork),
            _ => None,
        }
    }
    /// Is `self` the watched toggle, wearing either face?
    pub(crate) fn is_watch(self) -> bool {
        matches!(self, HeroCtl::MarkWatched | HeroCtl::MarkUnwatched)
    }
}

/// Which of the conditional controls the row is showing: the three independent bits (restart,
/// trailer, Also available), plus the item's watch state, which decides which FACE the watched
/// toggle wears.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct HeroSet {
    pub(crate) restart: bool,
    pub(crate) trailer: bool,
    pub(crate) alt: bool,
    pub(crate) mark: PosterMark,
    /// The heart: `Some(favorited)` on a backend that has favorites, `None` where the control
    /// must not exist at all (a backend with no such flag).
    pub(crate) favorite: Option<bool>,
    /// The file offers more than one audio track, so there is a language to choose.
    pub(crate) audio: bool,
    /// The file carries at least one subtitle track, so Off is a choice worth offering.
    pub(crate) subs: bool,
    /// The server can offer other pictures for this item, so there is artwork to change.
    pub(crate) artwork: bool,
}

/// The row's controls, in drawn order, for a given set. A fixed 9-slot array (Play + Restart +
/// Trailer + Alt + one watch face + the heart + the two language discs + the artwork disc is the
/// widest the row ever gets) plus a live count, so no per-frame allocation.
pub(crate) fn hero_ctls(set: HeroSet) -> ([HeroCtl; 9], usize) {
    let mut v = [HeroCtl::Play; 9];
    let mut n = 1;
    if set.restart {
        v[n] = HeroCtl::Restart;
        n += 1;
    }
    if set.alt {
        v[n] = HeroCtl::Alt;
        n += 1;
    }
    v[n] = if set.mark == PosterMark::Watched {
        HeroCtl::MarkUnwatched
    } else {
        HeroCtl::MarkWatched
    };
    n += 1;
    if set.favorite.is_some() {
        v[n] = HeroCtl::Favorite;
        n += 1;
    }
    if set.audio {
        v[n] = HeroCtl::Audio;
        n += 1;
    }
    if set.subs {
        v[n] = HeroCtl::Subtitles;
        n += 1;
    }
    // The owner's 2026-11-10 pass: the trailer sits beside the language and subtitle discs, not up
    // by the primary Play/Restart pair - it is the same kind of "what this file offers" control.
    if set.trailer {
        v[n] = HeroCtl::Trailer;
        n += 1;
    }
    if set.artwork {
        v[n] = HeroCtl::Artwork;
        n += 1;
    }
    (v, n)
}

/// Whether `ctl` may hold — or be enumerated as holding — focus right now, given whether
/// full-trailer mode owns the screen. The ONE predicate [`visible_ctls`] (group extent) and
/// `DetailScreen::reconcile`/`valid` (focus legitimacy) all gate on, so the row cannot offer a
/// control the mode has taken away — the exact class of bug this predicate replaced (two
/// independent "what's visible" checks that had silently drifted apart).
///
/// **Play survives as the ANCHOR, not as a reachable control.** Full-trailer mode fades the whole
/// row out together (`DetailScreen::draw_buttons` draws every control from [`hero_ctls`]
/// regardless of this predicate — see its own doc — and lets the caller's `chrome`/`preview_chrome`
/// alpha carry the fade; `screens::detail::trailer` puts the trailer's transport up instead) — but
/// the engine still needs somewhere legitimate for the focus to stand while the mode is up, and an
/// EMPTY hero group would push it into the sections below, scrolling the page out from under a
/// trailer nobody asked to leave. Keys never reach the row anyway: the mode answers them itself,
/// before the engine.
pub(crate) fn focusable(ctl: HeroCtl, full_trailer: bool) -> bool {
    !full_trailer || ctl == HeroCtl::Play
}

/// [`hero_ctls`], filtered through [`focusable`] for full-trailer mode's FOCUS/hit-testing extent
/// only — the transient UI state where the row narrows to its Play anchor alone and everything
/// else (Restart/Trailer/Alt/the watch toggle) stops being reachable the instant `full_trailer()`
/// flips. **This is not what gets drawn.** `DetailScreen::draw_buttons` draws every control
/// [`hero_ctls`] returns, full_trailer or not, so the row fades out together with the rest of the
/// chrome instead of four pills hard-cutting a frame ahead of the one that fades (2026-09-17
/// fix — a control losing its focus must not also lose its paint on the same frame). Deliberately
/// NOT a flag on [`HeroSet`]: that struct describes what the ITEM offers, not a screen's transient
/// presentation mode. Every call site that enumerates the row for focus/hit-testing extent must go
/// through this rather than `hero_ctls` directly, or the two can disagree about which controls
/// exist right now.
pub(crate) fn visible_ctls(set: HeroSet, full_trailer: bool) -> ([HeroCtl; 9], usize) {
    let (all, n) = hero_ctls(set);
    if !full_trailer {
        return (all, n);
    }
    let mut v = [HeroCtl::Play; 9];
    let mut count = 0;
    for &c in &all[..n] {
        if focusable(c, full_trailer) {
            v[count] = c;
            count += 1;
        }
    }
    (v, count)
}

/// The control at index `i` in `set`, or `None` for an index the set does not have.
pub(crate) fn ctl_at(set: HeroSet, i: usize) -> Option<HeroCtl> {
    let (v, n) = hero_ctls(set);
    (i < n).then(|| v[i])
}

/// Where `ctl` sits in `set`, or `None` when the set does not include it.
pub(crate) fn index_of(set: HeroSet, ctl: HeroCtl) -> Option<usize> {
    let (v, n) = hero_ctls(set);
    v[..n].iter().position(|&c| c == ctl)
}

/// Where the watched toggle sits in `set` — always present, so `Some` for every set the row can
/// be in. Keyed on the control's JOB rather than either of its two faces, which is what lets
/// [`DetailScreen::reconcile`](super::DetailScreen) carry the focus across a press that flips it.
pub(crate) fn watch_index(set: HeroSet) -> Option<usize> {
    let (v, n) = hero_ctls(set);
    v[..n].iter().position(|&c| c.is_watch())
}

/// Has this show been started AT ALL? The server puts `S1E1 off=0` on deck for a show nobody
/// touched, so "there is something on deck" is not the same question as "this show is underway" —
/// ported verbatim from `ui/detail.rs::show_started`.
pub(crate) fn show_started(d: &Detail) -> bool {
    d.on_deck.as_ref().map(|e| e.resume_ms > 0).unwrap_or(false)
        || d.seasons.iter().any(|s| s.viewed_leaf_count > 0)
}

/// The episode a show's hero is ABOUT — the server's on-deck episode, once the show has actually
/// been started. `None` means the hero presents the SERIES instead (finished, or never opened).
/// Ported verbatim from `ui/detail.rs::hero_episode`.
pub(crate) fn hero_episode(d: &Detail) -> Option<&Episode> {
    let ep = d.on_deck.as_ref()?;
    show_started(d).then_some(ep)
}

/// The resume position (ns) the hero's Play control would ACTUALLY apply. Ported verbatim from
/// `ui/detail.rs::hero_resume_ns`.
pub(crate) fn hero_resume_ns(d: &Detail) -> i64 {
    if d.is_show {
        hero_episode(d)
            .map(|e| crate::metadata::resume_ns(e.resume_ms, e.dur_ms))
            .unwrap_or(0)
    } else {
        crate::metadata::resume_ns(d.resume_ms, d.dur_ms)
    }
}

/// Is there a resume point for the restart disc to ignore?
pub(crate) fn has_restart(resume_ns: i64) -> bool {
    resume_ns > 0
}

/// The loaded item's watch state, in the app's one watch-state vocabulary. Ported verbatim from
/// `ui/detail.rs::hero_watch_state`.
pub(crate) fn hero_watch_state(
    is_show: bool,
    show_started: bool,
    watched: bool,
    resume_ns: i64,
) -> PosterMark {
    let mid_run = is_show && show_started && !watched;
    if resume_ns > 0 || mid_run {
        PosterMark::InProgress
    } else if watched {
        PosterMark::Watched
    } else {
        PosterMark::None
    }
}

/// [`hero_watch_state`] for `d`. Ported verbatim from `ui/detail.rs::hero_mark`.
pub(crate) fn hero_mark(d: &Detail) -> PosterMark {
    hero_watch_state(d.is_show, show_started(d), d.watched, hero_resume_ns(d))
}

/// Does the toggle's subject differ from the one the rest of the hero is about — true exactly
/// while the hero is showing an on-deck EPISODE. Ported verbatim from
/// `ui/detail.rs::watch_names_show`.
pub(crate) fn watch_names_show(d: &Detail) -> bool {
    hero_episode(d).is_some()
}

/// A disc's slot (`[restart, trailer, watch, heart, audio, subtitles]`) and the verb it unfurls
/// to — `None` for the two PILLS.
pub(crate) fn disc_verb(ctl: HeroCtl, name_show: bool, favorited: bool) -> Option<(usize, &'static CStr)> {
    match (ctl, name_show) {
        (HeroCtl::Restart, _) => Some((0, play_from_start_label())),
        (HeroCtl::Trailer, _) => Some((1, trailer_label())),
        (HeroCtl::MarkWatched, false) => Some((2, mark_watched_label())),
        (HeroCtl::MarkWatched, true) => Some((2, mark_show_watched_label())),
        (HeroCtl::MarkUnwatched, false) => Some((2, mark_unwatched_label())),
        (HeroCtl::MarkUnwatched, true) => Some((2, mark_show_unwatched_label())),
        (HeroCtl::Favorite, _) => Some((3, if favorited { unfavorite_label() } else { favorite_label() })),
        (HeroCtl::Audio, _) => Some((4, audio_label())),
        (HeroCtl::Subtitles, _) => Some((5, subtitles_label())),
        (HeroCtl::Artwork, _) => Some((6, artwork_label())),
        _ => None,
    }
}

/// The heart's two verbs, in the Jellyfin flavor's own labels.
fn favorite_label() -> &'static CStr { crate::i18n::msg::browse_detail_favorite_c() }
fn unfavorite_label() -> &'static CStr { crate::i18n::msg::browse_detail_unfavorite_c() }
fn audio_label() -> &'static CStr { crate::i18n::msg::browse_detail_audio_c() }
fn subtitles_label() -> &'static CStr { crate::i18n::msg::settings_audio_subtitles_c() }
fn artwork_label() -> &'static CStr { crate::i18n::msg::widgets_preplay_menu_edit_images_c() }

/// The Trailer disc's play fields — extra identity and HUD title, always from start.
/// `None` when the loaded item has no playable trailer (no button, no-op activate).
pub(crate) fn trailer_play(d: &Detail) -> Option<(&Extra, &str)> {
    let extra = d.trailer().filter(|e| e.playable())?;
    Some((extra, extra.hud_title(d.title.as_str())))
}

/// The Play pill's label — the word the press will actually perform.
///
/// The DETAIL hero no longer draws it (the owner's 2026-11-10 pass: the primary control keeps only
/// its icon). It survives for the row's measurement, so the pill's width stays a stable function of
/// the verb and the row's right-justified start cannot jump between Resume and Play.
pub(crate) fn hero_pill_label(has_restart: bool) -> &'static CStr {
    if has_restart {
        crate::i18n::msg::browse_detail_resume_c()
    } else {
        crate::i18n::msg::browse_detail_play_c()
    }
}

/// `widgets::Button::pill_w_full`, reproduced over [`Measure`] rather than `crate::text` directly
/// — see [`HERO_ICON_RATIO`]'s doc for why the icon-box constants are duplicated, and
/// `screens::login::status_action_rect`'s own doc for the general pattern (host-test-safe geometry
/// shared by `Focusable::place` and `draw`).
fn pill_w(measure: &dyn Measure, label: &CStr, sz: i32, icon: bool, trailing: bool) -> f32 {
    let (isz, gap) = if icon {
        (sz as f32 * HERO_ICON_RATIO, HERO_ICON_GAP)
    } else {
        (0.0, 0.0)
    };
    let (tsz, tgap) = if trailing {
        (sz as f32 * HERO_ICON_RATIO, HERO_ICON_GAP)
    } else {
        (0.0, 0.0)
    };
    isz + gap + measure.width(label, sz, true) + tgap + tsz + crate::ui::widgets::BTN_PILL_AIR
}

pub(crate) fn hero_pill_w(measure: &dyn Measure, has_restart: bool) -> f32 {
    // The PLAY control is a DISC now (the owner's 2026-11-10 pass: "el botón de play al mismo
    // tamaño que el resto"): its width is its own diameter, not a pill's measured text. `has_restart`
    // is kept in the signature so every caller stays one shape, and so the future return of a
    // labelled pill does not have to touch them.
    let _ = (measure, has_restart);
    CD
}

pub(crate) fn alt_pill_w(measure: &dyn Measure) -> f32 {
    pill_w(measure, alt_label(), theme::size::BODY, false, true)
}

/// Every measured width the row's accumulation needs, as one value — ported verbatim from
/// `ui/detail.rs::HeroWidths`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeroWidths {
    pub(crate) pill: f32,
    pub(crate) alt: f32,
    /// the discs, in [`disc_verb`]'s slot order, each already unfurled
    pub(crate) disc: [f32; 7],
}

/// The accumulation itself, PURE: the drawn frame of control `i` in `set`, given both pills'
/// measured widths. Ported verbatim from `ui/detail.rs::hero_btn_rect_at`, but STARTING at
/// [`crate::ui::detail_layout::HERO_TEXT_X`] rather than the left margin: the owner's 2026-11-10
/// pass makes the poster the dominant left column, so the controls live in the copy column beside
/// it and no longer sit under the taller poster.
pub(crate) fn hero_btn_rect_at(set: HeroSet, i: usize, y: f32, cw: HeroWidths) -> Rect {
    let (v, n) = hero_ctls(set);
    let width_of = |c: HeroCtl| match c {
        HeroCtl::Play => cw.pill,
        HeroCtl::Alt => cw.alt,
        HeroCtl::Restart => cw.disc[0],
        HeroCtl::Trailer => cw.disc[1],
        HeroCtl::MarkWatched | HeroCtl::MarkUnwatched => cw.disc[2],
        HeroCtl::Favorite => cw.disc[3],
        HeroCtl::Audio => cw.disc[4],
        HeroCtl::Subtitles => cw.disc[5],
        HeroCtl::Artwork => cw.disc[6],
    };
    // Justified to the RIGHT margin (the owner's 2026-11-10 pass: "justificados a derecha"): the
    // row's own total width decides its left edge, so the last control ends on the safe edge.
    let total: f32 = v[..n].iter().map(|&c| width_of(c)).sum::<f32>()
        + CGAP * (n.saturating_sub(1)) as f32;
    let mut x = crate::ui::consts::SCR_W - crate::ui::consts::MARGIN_X - total;
    let mut w = cw.pill;
    for (k, &c) in v[..n].iter().enumerate() {
        w = width_of(c);
        if k >= i {
            break;
        }
        x += w + CGAP;
    }
    Rect::new(x, y, w, CD)
}

/// **Every disc's width - always its own diameter.**
///
/// This used to be a budget dance: a focused disc grew into a capsule carrying its verb, so the row
/// had to prove the widest verb still fitted before the People column, and the discs' widths were a
/// function of which one was lit. The owner's 2026-11-10 pass ended the unfurl ("los iconos de
/// reproducir, audio, etc. que no se expandan con texto"), so the row is now a fixed sum of
/// [`CD`]-wide discs and the answer cannot depend on focus at all. The verb is still NAMED - the
/// row's right end carries the focused disc's own word (see [`disc_verb`]) - it just is not part of
/// the control.
pub(crate) fn disc_caps() -> [f32; 7] {
    [CD; 7]
}

/// The live widths, combining [`hero_pill_w`]/[`alt_pill_w`]/[`disc_caps`] — the one function
/// `DetailScreen`'s `Focusable` queries and `draw` both call, so they can never drift (mirrors
/// `ui/detail.rs::hero_widths`, over `Measure` instead of `crate::text` directly).
pub(crate) fn hero_widths(measure: &dyn Measure, has_restart: bool) -> HeroWidths {
    HeroWidths {
        pill: hero_pill_w(measure, has_restart),
        alt: alt_pill_w(measure),
        disc: disc_caps(),
    }
}

pub(crate) fn hero_credit(d: &Detail) -> Option<(&'static str, Vec<&str>)> {
    if d.is_show {
        let names: Vec<&str> = d
            .crew
            .iter()
            .filter(|credit| credit.role.contains("Writer"))
            .map(|credit| credit.tag.as_str())
            .collect();
        return (!names.is_empty()).then_some((crate::i18n::msg::browse_detail_created_by(), names));
    }
    let names: Vec<&str> = d.directors.iter().map(String::as_str).collect();
    (!names.is_empty()).then_some((crate::i18n::msg::browse_detail_directed_by(), names))
}

pub(crate) fn has_people(d: &Detail) -> bool {
    !d.cast.is_empty() || hero_credit(d).is_some()
}

/// The height the cast and crew band will take, without painting it: the hero chain has to know it
/// before the action row can be placed (issue #315/#323).
pub(crate) fn people_h(d: &Detail, measure: &dyn Measure) -> f32 {
    let mut h = 0.0;
    if !d.cast.is_empty() {
        let names: Vec<&str> = d
            .cast
            .iter()
            .take(PEOPLE_CAST)
            .map(|credit| credit.tag.as_str())
            .collect();
        h += people_line_h("", &names, measure);
    }
    if let Some((label, names)) = hero_credit(d) {
        h += people_line_h(label, &names, measure);
    }
    h
}

fn people_join(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| name.replace(' ', "\u{a0}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn people_view<'a>(joined: &'a str, label: &'a str, measure: &'a dyn Measure) -> TextView<'a> {
    use crate::ui::detail_layout::{PEOPLE_INK, PEOPLE_LEAD};
    let view = TextView::new(joined, theme::size::CAPTION, PEOPLE_INK)
        .with_measure(measure)
        .leading(PEOPLE_LEAD)
        .max_lines(PEOPLE_MAX_LINES)
        .h(HAlign::Left);
    if label.is_empty() {
        view
    } else {
        view.lead_quiet(label, PEOPLE_LABEL_INK)
    }
}

fn people_line_h(label: &str, names: &[&str], measure: &dyn Measure) -> f32 {
    let joined = people_join(names);
    people_view(&joined, label, measure).measure_h(crate::ui::detail_layout::HERO_TEXT_W)
}

/// **The item's own date, as the identity line says it**: the year alone for a film or a show, and
/// the aired date for an episode (where the day is the fact a viewer plans around). The owner's
/// pass removed the month and the day from a film's line: `26/9/2012` beside `1 h 52 min` read as
/// a record, and the only part that answers "is this the one I mean" is the year.
pub(crate) fn hero_year(d: &Detail) -> String {
    if let Some(ep) = hero_episode(d) {
        return crate::ui::fmt::pretty_date(&ep.aired, 0);
    }
    if d.year > 0 {
        return d.year.to_string();
    }
    String::new()
}

/// **The item's extent**, in the identity line beside the year: the runtime of a film or an
/// episode, and the season/episode count of a show. `None` when the server stated nothing, which
/// draws no run at all rather than a zero.
pub(crate) fn hero_extent(d: &Detail) -> Option<String> {
    if let Some(ep) = hero_episode(d) {
        return (ep.dur_ms >= 60_000).then(|| crate::ui::fmt::dur_long(ep.dur_ms));
    }
    if d.is_show {
        let seasons = d.seasons.len();
        if seasons == 0 {
            return None;
        }
        let episodes: i64 = d.seasons.iter().map(|season| season.leaf_count).sum();
        let seasons = crate::i18n::msg::browse_detail_seasons(seasons as i64);
        return Some(if episodes > 0 {
            crate::i18n::msg::browse_detail_extent(&crate::i18n::msg::browse_detail_episodes(episodes), &seasons)
        } else {
            seasons
        });
    }
    (d.dur_ms >= 60_000).then(|| crate::ui::fmt::dur_long(d.dur_ms))
}

/// **The audience score, 0-10**, when the item carries one — the `Community` entry of
/// [`Detail::ratings`]. It is the number the hero draws beside a star next to the age rating
/// (the owner's pass): the provider caption `SCORE` is gone, and the mark is the universal one.
pub(crate) fn community_score(d: &Detail) -> Option<f64> {
    d.ratings
        .iter()
        .find(|r| r.art == crate::metadata::RatingArt::Community)
        .map(|r| r.value)
        .filter(|v| v.is_finite() && *v > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // TEST ONLY: this module's few call sites still read/write metadata as free functions, a
    // shape written for the old crate-global statics. A thread-confined store (same pattern as
    // `screens::detail::tests`'s `TEST_METADATA`) gives them a real, per-owner `MetadataStore`
    // without threading one through every helper here.
    thread_local! {
        static TEST_METADATA: std::cell::UnsafeCell<crate::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(crate::stores::metadata::MetadataStore::default());
    }

    fn test_store() -> &'static mut crate::stores::metadata::MetadataStore {
        TEST_METADATA.with(|cell| unsafe { &mut *cell.get() })
    }

    fn set(restart: bool, alt: bool, mark: PosterMark) -> HeroSet {
        set_full(restart, alt, mark, false)
    }

    fn set_full(restart: bool, alt: bool, mark: PosterMark, trailer: bool) -> HeroSet {
        HeroSet {
            restart,
            trailer,
            alt,
            mark,
            favorite: None,
            audio: false,
            subs: false,
            artwork: false,
        }
    }

    /// **The row offers exactly ONE watched toggle, never both faces at once** — the property
    /// `ui/detail.rs`'s own module doc calls out by name ("this row briefly drew the two faces
    /// side by side for a part-watched item, and that was wrong").
    #[test]
    fn the_row_offers_exactly_one_watched_toggle() {
        for mark in [
            PosterMark::None,
            PosterMark::InProgress,
            PosterMark::Watched,
        ] {
            for restart in [false, true] {
                for alt in [false, true] {
                    for trailer in [false, true] {
                        let s = set_full(restart, alt, mark, trailer);
                        let (v, n) = hero_ctls(s);
                        let watch_ctls: Vec<_> = v[..n].iter().filter(|c| c.is_watch()).collect();
                        assert_eq!(watch_ctls.len(), 1, "set={s:?}");
                    }
                }
            }
        }
    }

    /// **Full-trailer mode always narrows the row's FOCUS/pointer extent to exactly `[Play]`,
    /// whatever the item's own facts offer** — swept over every `HeroSet` this row can take. The
    /// property that keeps the mode's focus anchor single and legitimate. Drawing is the OPPOSITE:
    /// `DetailScreen::draw_buttons` always paints the full `hero_ctls` row, full-trailer or not
    /// (2026-09-17 fix — the row fades out together with the rest of the chrome instead of four
    /// pills hard-cutting a frame ahead of Play), so this also pins that the two predicates
    /// genuinely diverge whenever the set has more than Play alone.
    #[test]
    fn full_trailer_mode_collapses_focus_to_play_only_but_leaves_drawing_alone() {
        for mark in [
            PosterMark::None,
            PosterMark::InProgress,
            PosterMark::Watched,
        ] {
            for restart in [false, true] {
                for alt in [false, true] {
                    for trailer in [false, true] {
                        let s = set_full(restart, alt, mark, trailer);
                        let (v, n) = visible_ctls(s, true);
                        assert_eq!(n, 1, "set={s:?}");
                        assert_eq!(v[0], HeroCtl::Play, "set={s:?}");
                        // And the non-full-trailer path must be byte-identical to `hero_ctls` —
                        // `visible_ctls` is a strict narrowing, never a second row model.
                        assert_eq!(visible_ctls(s, false), hero_ctls(s), "set={s:?}");
                        // Drawing does not narrow with `visible_ctls` — `draw_buttons` iterates
                        // `hero_ctls` directly — so drawing always sees strictly more than
                        // full-trailer mode makes focusable/pointer-reachable: the watch toggle
                        // alone (unconditional in `hero_ctls`) already outnumbers the `[Play]`
                        // focus set, before `restart`/`alt`/`trailer` add anything further.
                        let (_, drawn) = hero_ctls(s);
                        assert!(
                            drawn > n,
                            "set={s:?}: drawing must stay decoupled from the focus narrowing"
                        );
                    }
                }
            }
        }
    }

    /// Each of the row's discs writes its OWN verb: the restart disc always unfurls "Play from
    /// Start"; the watch toggle's verb depends on which face it wears AND on whether the hero is
    /// naming an episode's SHOW rather than the episode itself.
    #[test]
    fn each_watch_disc_writes_its_own_verb() {
        assert_eq!(
            disc_verb(HeroCtl::Restart, false, false),
            Some((0, play_from_start_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::Restart, true, false),
            Some((0, play_from_start_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::MarkWatched, false, false),
            Some((2, mark_watched_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::MarkWatched, true, false),
            Some((2, mark_show_watched_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::MarkUnwatched, false, false),
            Some((2, mark_unwatched_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::MarkUnwatched, true, false),
            Some((2, mark_show_unwatched_label()))
        );
        assert_eq!(
            disc_verb(HeroCtl::Trailer, false, false),
            Some((1, trailer_label()))
        );
        assert_eq!(disc_verb(HeroCtl::Trailer, true, false), Some((1, trailer_label())));
    }

    /// The heart's verb follows the FAVORITE flag, never `name_show` (the wiring bug of the
    /// first cut: the label answered "Marcar como favorito" on an item the server already held
    /// favorited, because the flag it read was the show-naming one).
    #[test]
    fn the_hearts_verb_follows_the_favorite_flag() {
        assert_eq!(disc_verb(HeroCtl::Favorite, false, false), Some((3, favorite_label())));
        assert_eq!(disc_verb(HeroCtl::Favorite, false, true), Some((3, unfavorite_label())));
        assert_eq!(disc_verb(HeroCtl::Favorite, true, false), Some((3, favorite_label())));
        assert_eq!(disc_verb(HeroCtl::Favorite, true, true), Some((3, unfavorite_label())));
        assert_eq!(
            disc_verb(HeroCtl::Play, false, false),
            None,
            "a pill has nothing to unfurl"
        );
        assert_eq!(disc_verb(HeroCtl::Alt, false, false), None);
    }

    /// The watched toggle is carried by its JOB, not by either face — `watch_index` must answer
    /// `Some` for a set wearing EITHER face, at the SAME index.
    #[test]
    fn the_watch_toggle_is_found_by_its_job_whichever_face_it_wears() {
        let unwatched = set(false, false, PosterMark::None);
        let watched = set(false, false, PosterMark::Watched);
        let i_unwatched = watch_index(unwatched).expect("always present");
        let i_watched = watch_index(watched).expect("always present");
        assert_eq!(
            i_unwatched, i_watched,
            "the toggle sits at the row's tail in both sets"
        );
        assert_eq!(ctl_at(unwatched, i_unwatched), Some(HeroCtl::MarkWatched));
        assert_eq!(ctl_at(watched, i_watched), Some(HeroCtl::MarkUnwatched));
    }

    /// A finished-then-restarted item reads `InProgress`, never `Watched` — the viewer is
    /// part-way through a re-watch, and "not watched" is the honest state for the toggle to read.
    #[test]
    fn a_restarted_finished_item_reads_in_progress_not_watched() {
        // `watched=true` but a live resume point beats it.
        assert_eq!(
            hero_watch_state(false, false, true, 4_000_000_000),
            PosterMark::InProgress
        );
    }

    /// A show mid-run (started, not finished) reads `InProgress` even with no resume point of its
    /// own — `mid_run` is `is_show && show_started && !watched`.
    #[test]
    fn a_show_mid_run_reads_in_progress_with_no_resume_point_of_its_own() {
        assert_eq!(
            hero_watch_state(true, true, false, 0),
            PosterMark::InProgress
        );
        assert_eq!(
            hero_watch_state(true, true, true, 0),
            PosterMark::Watched,
            "…but a FINISHED show is simply watched"
        );
        assert_eq!(
            hero_watch_state(true, false, false, 0),
            PosterMark::None,
            "an unstarted show is simply unstarted"
        );
    }

    /// **`hero_col`'s reconciliation, restated as identity-membership**: a restart control that
    /// vanishes from the set (a resume point was just consumed) is no longer found by
    /// `index_of`, which is exactly the signal `DetailScreen::reconcile` uses to know the focus
    /// must move off it.
    #[test]
    fn restart_disc_tracks_the_resume_the_play_would_actually_apply() {
        assert!(!has_restart(0), "no resume point, no restart disc");
        assert!(has_restart(1), "any positive resume position offers one");
        let with = set(true, false, PosterMark::None);
        let without = set(false, false, PosterMark::None);
        assert_eq!(index_of(with, HeroCtl::Restart), Some(1));
        assert_eq!(
            index_of(without, HeroCtl::Restart),
            None,
            "the disc that a completed restart just consumed is gone from the new set"
        );
    }

    /// The row's ACCUMULATED x positions never overlap and advance strictly left to right, at
    /// every set size the row can take (the property the pointer hit-test and the draw both rely
    /// on agreeing about).
    #[test]
    fn the_row_s_controls_never_overlap_at_any_set_size() {
        let cw = HeroWidths {
            pill: 200.0,
            alt: 260.0,
            disc: [CD; 7],
        };
        for restart in [false, true] {
            for alt in [false, true] {
                for trailer in [false, true] {
                    for mark in [PosterMark::None, PosterMark::Watched] {
                        let s = set_full(restart, alt, mark, trailer);
                        let (_, n) = hero_ctls(s);
                        let mut prev_right = f32::MIN;
                        for i in 0..n {
                            let r = hero_btn_rect_at(s, i, 0.0, cw);
                            assert!(r.x >= prev_right, "set={s:?} i={i} rect={r:?}");
                            prev_right = r.x + r.w;
                        }
                    }
                }
            }
        }
    }

    /// `ctl_at`/`index_of` are exact inverses over every index the set actually has.
    #[test]
    fn ctl_at_and_index_of_round_trip() {
        for restart in [false, true] {
            for alt in [false, true] {
                for trailer in [false, true] {
                    for mark in [
                        PosterMark::None,
                        PosterMark::InProgress,
                        PosterMark::Watched,
                    ] {
                        let s = set_full(restart, alt, mark, trailer);
                        let (_, n) = hero_ctls(s);
                        for i in 0..n {
                            let ctl = ctl_at(s, i).unwrap();
                            assert_eq!(index_of(s, ctl), Some(i));
                        }
                        assert_eq!(
                            ctl_at(s, n),
                            None,
                            "one past the end is None, never a panic"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_crew_credit_names_the_right_job_for_the_kind_of_item() {
        fn credit(name: &str, role: &str) -> crate::metadata::Cast {
            crate::metadata::Cast {
                tag: name.into(),
                role: role.into(),
                thumb: String::new(),
                id: 0,
                tag_key: String::new(),
            }
        }
        let movie = Detail {
            directors: vec!["Jane Doe".into()],
            ..Default::default()
        };
        assert_eq!(hero_credit(&movie), Some(("Directed by", vec!["Jane Doe"])));
        let show = Detail {
            is_show: true,
            crew: vec![
                credit("Pilot Director", "Director"),
                credit("Kerry Ehrin", "Writer"),
            ],
            ..Default::default()
        };
        assert_eq!(
            hero_credit(&show),
            Some(("Created by", vec!["Kerry Ehrin"]))
        );
    }

    #[test]
    fn restart_and_the_watch_tail_are_independent() {
        let set = HeroSet {
            restart: true,
            trailer: false,
            alt: false,
            mark: PosterMark::Watched,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        let (controls, n) = hero_ctls(set);
        assert_eq!(
            &controls[..n],
            &[HeroCtl::Play, HeroCtl::Restart, HeroCtl::MarkUnwatched]
        );
    }

    #[test]
    fn the_actions_row_grows_an_also_available_control_only_for_a_second_source() {
        let without = HeroSet {
            restart: false,
            trailer: false,
            alt: false,
            mark: PosterMark::None,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        let with = HeroSet {
            alt: true,
            ..without
        };
        assert!(index_of(without, HeroCtl::Alt).is_none());
        assert!(index_of(with, HeroCtl::Alt).is_some());
    }

    #[test]
    fn the_hero_row_carries_no_track_information_disc() {
        for alt in [false, true] {
            for trailer in [false, true] {
                let set = HeroSet {
                    restart: true,
                    trailer,
                    alt,
                    mark: PosterMark::InProgress,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
                let (controls, n) = hero_ctls(set);
                assert!(controls[..n].iter().all(|ctl| matches!(
                    ctl,
                    HeroCtl::Play
                        | HeroCtl::Restart
                        | HeroCtl::Trailer
                        | HeroCtl::Alt
                        | HeroCtl::MarkWatched
                        | HeroCtl::MarkUnwatched
                )));
            }
        }
    }

    #[test]
    fn a_shows_hero_is_about_the_servers_on_deck_episode_or_the_series() {
        let mut d = Detail {
            is_show: true,
            seasons: vec![crate::metadata::Season {
                rk: "s1".into(),
                index: 1,
                title: "Season 1".into(),
                leaf_count: 3,
                viewed_leaf_count: 0,
            }],
            on_deck: Some(Episode {
                rk: "e1".into(),
                resume_ms: 0,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(
            hero_episode(&d).is_none(),
            "an untouched show is about the series"
        );
        d.seasons[0].viewed_leaf_count = 1;
        assert_eq!(hero_episode(&d).map(|ep| ep.rk.as_str()), Some("e1"));
    }

    #[test]
    fn the_watch_state_resolver_answers_leaf_and_container_by_their_own_rules() {
        assert_eq!(hero_watch_state(false, false, false, 0), PosterMark::None);
        assert_eq!(hero_watch_state(false, false, true, 0), PosterMark::Watched);
        assert_eq!(
            hero_watch_state(true, true, false, 0),
            PosterMark::InProgress
        );
        assert_eq!(hero_watch_state(true, true, true, 0), PosterMark::Watched);
    }

    #[test]
    fn the_optimistic_flip_settles_a_leaf_at_once_and_a_container_a_round_trip_late() {
        let _guard = crate::testlock::serial();
        let sid = crate::registry::ServerId::UNSET;
        crate::metadata::set_current_for_test(test_store().state_mut(), Some(Detail {
            sid,
            rk: "movie".into(),
            resume_ms: 1_800_000,
            dur_ms: 7_200_000,
            ..Default::default()
        }));
        assert!(test_store().run(
            crate::stores::metadata::MetadataCmd::SetWatchedLocal { sid, rk: "movie".into(), on: true }
        ));
        let movie = test_store().view().current().unwrap();
        assert_eq!(hero_mark(movie), PosterMark::Watched);
        assert_eq!(
            movie.resume_ms, 0,
            "a leaf's restart disc disappears immediately"
        );

        crate::metadata::set_current_for_test(test_store().state_mut(), Some(Detail {
            sid,
            rk: "show".into(),
            is_show: true,
            seasons: vec![crate::metadata::Season {
                rk: "s1".into(),
                index: 1,
                title: "Season 1".into(),
                leaf_count: 2,
                viewed_leaf_count: 1,
            }],
            on_deck: Some(Episode {
                resume_ms: 600_000,
                dur_ms: 2_700_000,
                ..Default::default()
            }),
            ..Default::default()
        }));
        assert!(test_store().run(
            crate::stores::metadata::MetadataCmd::SetWatchedLocal { sid, rk: "show".into(), on: true }
        ));
        assert_eq!(
            hero_mark(test_store().view().current().unwrap()),
            PosterMark::InProgress,
            "container progress remains server evidence until the re-read lands"
        );
        crate::metadata::set_current_for_test(test_store().state_mut(), Some(Detail {
            sid,
            rk: "show".into(),
            is_show: true,
            watched: true,
            ..Default::default()
        }));
        assert_eq!(
            hero_mark(test_store().view().current().unwrap()),
            PosterMark::Watched
        );
        crate::metadata::set_current_for_test(test_store().state_mut(), None);
    }

    #[test]
    fn hero_indices_mean_different_actions_in_the_two_control_sets() {
        let compact = HeroSet {
            restart: false,
            trailer: false,
            alt: false,
            mark: PosterMark::None,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        let wide = HeroSet {
            restart: true,
            trailer: false,
            alt: true,
            mark: PosterMark::None,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        assert_eq!(ctl_at(compact, 1), Some(HeroCtl::MarkWatched));
        assert_eq!(ctl_at(wide, 1), Some(HeroCtl::Restart));
        assert_eq!(ctl_at(wide, 2), Some(HeroCtl::Alt));
    }

    #[test]
    fn the_actions_row_accumulates_around_a_variable_width_control() {
        let set = HeroSet {
            restart: true,
            trailer: false,
            alt: true,
            mark: PosterMark::None,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        let widths = HeroWidths {
            pill: 210.0,
            alt: 300.0,
            disc: [90.0, 120.0, 120.0, 120.0, CD, CD, CD],
        };
        let (controls, n) = hero_ctls(set);
        for i in 1..n {
            let previous = hero_btn_rect_at(set, i - 1, 0.0, widths);
            let current = hero_btn_rect_at(set, i, 0.0, widths);
            assert_eq!(
                current.x,
                previous.x + previous.w + CGAP,
                "{:?}",
                controls[i]
            );
        }
    }

    /// **The widest row the app can build still ends before the People column, at every focus
    /// state** - because the discs no longer change width at all. The old version swept unfurl
    /// phases; the contract it protected (nothing crosses the column) is now a property of the
    /// fixed sum, and this is what holds it.
    #[test]
    fn the_widest_control_row_never_crosses_the_people_column() {
        let measure = crate::ui::fixture::FixtureMeasure;
        for restart in [false, true] {
            for alt in [false, true] {
                for trailer in [false, true] {
                    for mark in [PosterMark::None, PosterMark::InProgress, PosterMark::Watched] {
                        for audio in [false, true] {
                            for subs in [false, true] {
                                for artwork in [false, true] {
                                    let set = HeroSet { restart, trailer, alt, mark, favorite: Some(false), audio, subs, artwork };
                                    let (_, n) = hero_ctls(set);
                                    let widths = hero_widths(&measure, restart);
                                    let last = hero_btn_rect_at(set, n - 1, 0.0, widths);
                                    assert!(
                                        last.x + last.w <= crate::ui::consts::SCR_W - crate::ui::consts::MARGIN_X + 0.01,
                                        "{set:?} ends at {}",
                                        last.x + last.w
                                    );
                                    assert_eq!(widths.disc, [CD; 7], "a disc is its own diameter");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The verbs are still NAMED - the row's right-end caption is the thing that names them - and
    /// they are the same words the card menu uses, so the two surfaces cannot describe one write
    /// two ways.
    #[test]
    fn every_disc_has_a_verb_and_it_is_the_menus_own() {
        for (ctl, named_show, favorited, want) in [
            (HeroCtl::Restart, false, false, "Play from Start"),
            (HeroCtl::MarkWatched, false, false, "Mark as Watched"),
            (HeroCtl::MarkUnwatched, true, false, "Mark Show as Unwatched"),
        ] {
            assert_eq!(disc_verb(ctl, named_show, favorited).unwrap().1.to_str().unwrap(), want);
        }
        // the discs that name a MODE rather than a write keep their own words
        assert!(disc_verb(HeroCtl::Audio, false, false).is_some());
        assert!(disc_verb(HeroCtl::Subtitles, false, false).is_some());
        assert!(disc_verb(HeroCtl::Artwork, false, false).is_some());
        // …and the two pills are words already, so they have no caption to add
        assert_eq!(disc_verb(HeroCtl::Play, false, false), None);
        assert_eq!(disc_verb(HeroCtl::Alt, false, false), None);
    }


    #[test]
    fn the_widest_action_row_clears_the_people_column() {
        let set = HeroSet {
            restart: true,
            trailer: true,
            alt: true,
            mark: PosterMark::InProgress,
        favorite: None,
        audio: false,
        subs: false,
        artwork: false,
    };
        let (_, n) = hero_ctls(set);
        let last = hero_btn_rect_at(
            set,
            n - 1,
            0.0,
            HeroWidths {
                pill: PW + 62.0,
                alt: 340.0,
                disc: [CD; 7],
            },
        );
        assert!(last.x + last.w <= crate::ui::consts::SCR_W - crate::ui::consts::MARGIN_X + 0.01);
    }

    #[test]
    fn trailer_false_matches_today_s_four_control_sets() {
        let set = set(true, true, PosterMark::None);
        let (controls, n) = hero_ctls(set);
        assert_eq!(
            &controls[..n],
            &[
                HeroCtl::Play,
                HeroCtl::Restart,
                HeroCtl::Alt,
                HeroCtl::MarkWatched
            ]
        );
    }

    #[test]
    fn trailer_true_sits_beside_the_language_discs() {
        let set = set_full(true, true, PosterMark::None, true);
        let (controls, n) = hero_ctls(set);
        assert_eq!(
            &controls[..n],
            &[
                HeroCtl::Play,
                HeroCtl::Restart,
                HeroCtl::Alt,
                HeroCtl::MarkWatched,
                HeroCtl::Trailer
            ]
        );
    }

    #[test]
    fn growing_a_trailer_disc_keeps_watch_identity() {
        let before = set(false, false, PosterMark::None);
        let after = set_full(false, false, PosterMark::None, true);
        assert_eq!(index_of(before, HeroCtl::Play), Some(0));
        assert_eq!(index_of(after, HeroCtl::Play), Some(0));
        assert_eq!(index_of(before, HeroCtl::Trailer), None);
        // The trailer now sits AFTER the watch toggle, beside the language discs, so it does not
        // move the watch identity at all.
        assert_eq!(index_of(after, HeroCtl::Trailer), Some(2));
        let watch = index_of(after, HeroCtl::MarkWatched).unwrap();
        assert_eq!(watch, index_of(before, HeroCtl::MarkWatched).unwrap());
        assert_eq!(ctl_at(after, watch).unwrap().elem(), ELEM_MARK_WATCHED);
    }

    #[test]
    fn losing_the_trailer_disc_drops_its_identity() {
        let with = set_full(true, false, PosterMark::None, true);
        let without = set(true, false, PosterMark::None);
        assert_eq!(index_of(with, HeroCtl::Trailer), Some(3));
        assert_eq!(index_of(without, HeroCtl::Trailer), None);
    }

    fn trailer_extra(rk: &str, part: &str, title: &str) -> Extra {
        Extra {
            rk: rk.into(),
            part: part.into(),
            title: title.into(),
            vcodec: "h264".into(),
            acodec: "aac".into(),
            subtype: "trailer".into(),
            extra_type: 1,
            dur_ms: 120_000,
            bitrate: 2_500,
            thumb: String::new(),
        }
    }

    #[test]
    fn trailer_play_uses_the_extra_not_the_parent_or_on_deck() {
        let extra = trailer_extra("9", "/library/parts/trailer", "Official Trailer");
        let movie = Detail {
            rk: "m".into(),
            title: "Movie".into(),
            part: "/library/parts/movie".into(),
            extras: vec![extra.clone()],
            ..Default::default()
        };
        let (got, title) = trailer_play(&movie).unwrap();
        assert_eq!(got.rk, "9");
        assert_eq!(got.part, "/library/parts/trailer");
        assert_eq!(title, "Official Trailer");
        assert_ne!(got.part, movie.part);

        let show = Detail {
            is_show: true,
            title: "Show".into(),
            on_deck: Some(Episode {
                rk: "ep".into(),
                part: "/library/parts/ep".into(),
                ..Default::default()
            }),
            extras: vec![extra],
            ..Default::default()
        };
        let (got, title) = trailer_play(&show).unwrap();
        assert_eq!(got.rk, "9");
        assert_eq!(got.part, "/library/parts/trailer");
        assert_eq!(title, "Official Trailer");
        assert_ne!(got.rk, show.on_deck.as_ref().unwrap().rk);
        assert!(trailer_play(&Detail::default()).is_none());
        assert!(
            trailer_play(&Detail {
                extras: vec![trailer_extra("", "/p", "T")],
                ..Default::default()
            })
            .is_none(),
            "a part without rk is not playable"
        );
        assert!(
            trailer_play(&Detail {
                extras: vec![trailer_extra("9", "", "T")],
                ..Default::default()
            })
            .is_none(),
            "an rk without part is not playable"
        );
        let untitled = Detail {
            title: "Parent".into(),
            extras: vec![trailer_extra("9", "/p", "")],
            ..Default::default()
        };
        assert_eq!(trailer_play(&untitled).unwrap().1, "Parent");
    }

    #[test]
    fn the_verb_caption_spring_reports_while_opening_and_is_quiet_at_rest() {
        let _g = crate::testlock::serial();
        crate::ui::idle::reset_for_test();
        // The row's verb caption is the ONLY spring the discs still carry: the controls themselves
        // are fixed (`disc_caps`). It has to report while it opens, and settle to silence, or the
        // present gate spins a still page at 60 fps.
        let mut springs = [crate::ui::Spring::at(0.0); 3];
        for spring in springs.iter_mut() {
            spring.step(0.0, K_DISC_VERB, 1.0 / 60.0);
            assert!(
                spring.pos.abs() < 0.01 && spring.vel.abs() < 0.01,
                "a disc with no verb must not keep springs moving"
            );
        }

        crate::ui::idle::note_present(10_000);
        crate::ui::idle::frame_begin(1.0 / 60.0);
        crate::ui::idle::note_spring(0.0, 0.0, 0.0);
        assert!(
            !crate::ui::idle::should_present(10_016),
            "a row with no verb must not keep the present gate awake"
        );

        crate::ui::idle::frame_begin(1.0 / 60.0);
        springs[1].step(1.0, K_DISC_VERB, 1.0 / 60.0);
        assert!(
            crate::ui::idle::should_present(10_032),
            "opening the caption reports motion"
        );
        assert!(springs[1].pos > 0.01 || springs[1].vel.abs() > 0.01);

        for _ in 0..240 {
            crate::ui::idle::frame_begin(1.0 / 60.0);
            springs[1].step(1.0, K_DISC_VERB, 1.0 / 60.0);
        }
        assert!(
            (springs[1].pos - 1.0).abs() < 0.01 && springs[1].vel.abs() < 0.01,
            "the caption's spring settles"
        );
        crate::ui::idle::frame_begin(1.0 / 60.0);
        crate::ui::idle::note_spring(1.0, 1.0, 0.0);
        let _ = crate::ui::idle::should_present(19_000);
        crate::ui::idle::note_present(20_000);
        crate::ui::idle::frame_begin(1.0 / 60.0);
        crate::ui::idle::note_spring(1.0, 1.0, 0.0);
        assert!(
            !crate::ui::idle::should_present(20_016),
            "the caption's spring is quiet at rest"
        );
    }
}
