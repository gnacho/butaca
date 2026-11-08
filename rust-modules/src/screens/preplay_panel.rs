//! **The pre-playback window** (issue #367, under #355): the surface a press on Play opens
//! INSTEAD of the film.
//!
//! It is the one panel in the family that is not read-only. The left column is the item's own
//! record — backdrop, title, the facts a person plans an evening with (resolution, codecs,
//! container, size, the wall-clock time it ENDS, genres), the synopsis, and the cast and crew. The
//! right column is the decision: which audio track, which subtitles, and the primary action.
//!
//! **The choice is the point.** #314 put the track menu on top of a film that had already started,
//! pausing it; #355 supersedes that, and this window is where the choice belongs. OK here starts
//! the playback with exactly the tracks drawn, and BACK returns to the page with nothing started.
//!
//! ## Why it reads `metadata::current()`
//!
//! The window is presented over the page whose Play opened it, and that page is the item's own
//! page: `DetailScreen::play_hero` is its only opener, and it only opens for a leaf the page has
//! loaded in full (a movie, or an episode's own page). So the loaded detail IS the item being
//! played — the same fact `screens::tracks_panel` and `screens::about_panel` rely on — and the
//! surface needs nothing on its argument but the one bit the item cannot answer: whether the press
//! was *Play from Start*.
//!
//! A show's page does not open this window: the episode its Play would start is not the item the
//! page holds, and a window offering the SHOW's backfilled track list for an episode would be
//! offering another file's tracks. That press keeps playing directly, which is honest rather than
//! wrong.
//!
//! ## The end time
//!
//! `dur_ms` plus the clock at the moment the window is drawn, recomputed every frame — a stale
//! "ends 23:40" is worse than none (#355's own note). The run is SIZED from a fixed sample rather
//! than from the string: a recording's measurement keys are the text itself, so a live clock in a
//! key would miss on every replay of the same recording.
//!
//! ## The pointer
//!
//! The window answers the Magic Remote's pointer the way every other screen in the app does
//! (`ui/route_screen`'s rule 11, restated in `ui/table_screen`): **hover PARKS focus and nothing
//! else.** The four rows are the focus model's four elements, so a pointer move over one is an
//! ordinary engine move (`Hover::Focus` on the stop), the row lights in the very face the arrow
//! keys give it, and a click runs the row it parked. There is no separate hover treatment to keep
//! in step, and no second ladder for the pointer to disagree with. A pointer that leaves the window
//! leaves the row it last hovered focused, which is what the table rows do: the arrow ladder
//! continues from there rather than being stranded, and a D-pad press re-arms
//! `ui::hit`'s 120 px hover gate so the two cannot fight (#359).

use crate::metadata::Detail;
use crate::ui::consts::{SCR_H, SCR_W, SDLK_LEFT, SDLK_RIGHT};
use crate::ui::icons::Icon;
use crate::ui::label::{Label, VAlign};
use crate::ui::text_view::TextView;
use crate::ui::{theme, widgets};
use crate::ui::{Painter, Rect};
use std::ffi::CString;


// ---- geometry ----------------------------------------------------------------------------------

/// The sheet's frame. Wider than the read-only alerts on purpose: this one holds two columns, and
/// the left one carries a backdrop and a synopsis.
const PANEL_W: f32 = 1560.0;
/// The family's own ceiling: the screen less the glass edge-clear on both sides
/// (`theme::ALERT_PANEL_RAD`'s argument). The left column spends it on a backdrop beside the
/// title and the facts, then the prose under them; [`main_synopsis_lines`] is what keeps the
/// prose inside it rather than a hand-counted line budget.
const PANEL_H: f32 = SCR_H - 2.0 * EDGE_CLEAR;
const PAD: f32 = theme::alert::PAD;
/// The gap between the two columns.
const COL_GAP: f32 = theme::space::LG;
/// The options column: a third of the sheet, which is what makes the facts column the MAIN area.
const OPTIONS_W: f32 = 380.0;
/// The left column's width, derived so the two columns and their gap exactly fill the padding box.
const MAIN_W: f32 = PANEL_W - 2.0 * PAD - COL_GAP - OPTIONS_W;

/// The design system's `--glass-edge-clear`, as `screens::about_panel` states it: the sheet never
/// comes closer than this to any screen edge.
const EDGE_CLEAR: f32 = 68.0;

/// The backdrop, at the head of the main column and to the LEFT of the title: a 16:9 still at the
/// size the server scales once for a hero.
const ART_W: f32 = 420.0;
const ART_H: f32 = 236.0;
/// The gap between the backdrop and the title/facts stack beside it.
const GAP_ART_TEXT: f32 = theme::space::MD;
/// The title's lead, from the family ladder.
const TITLE_H: f32 = theme::alert::TITLE_LEAD;
const GAP_TITLE_FACTS: f32 = theme::space::MD;
/// One label-over-value pair, from `screens::tracks_panel`'s own rhythm.
const LABEL_H: f32 = theme::size::CAPTION as f32 * 1.25;
const VALUE_H: f32 = theme::size::LABEL as f32 * 1.25;
const PAIR_GAP: f32 = 6.0;
const ROW_GAP: f32 = theme::space::MD;
const SECTION_GAP: f32 = theme::space::MD;
const HEAD_H: f32 = theme::size::CAPTION as f32;
const GENRE_LEAD: f32 = theme::size::CAPTION as f32 * 1.3;
const SYN_LEAD: f32 = theme::size::LABEL as f32 * 1.35;
const CREDIT_LEAD: f32 = theme::size::CAPTION as f32 * 1.35;
/// The scores line under the title: one band tall enough for the LABEL number the percentage is
/// set in, which is also the band the stars are centred in.
const RATING_ROW_H: f32 = theme::size::LABEL as f32 * 1.25;
/// Scores → the facts under them. A tight gap: the two are one statement about the item.
const GAP_RATINGS_FACTS: f32 = theme::space::SM;

/// One option row's height — a capsule tall enough for a value and its chevrons.
const OPT_H: f32 = 76.0;
const OPT_GAP: f32 = theme::space::SM;
/// The primary action's capsule.
const ACT_H: f32 = 84.0;

/// How far the sheet rises as it appears — `Popover::RISE`, the one number the whole panel family
/// shares.
const RISE: f32 = crate::ui::popover::Popover::RISE;

/// The most synopsis lines the window ever draws: the left column has the credits under it, and a
/// synopsis nobody can read in one glance is what the About sheet is for.
const SYN_LINES_MAX: usize = 4;
/// The most cast names drawn, and the most crew names after them. A film's cast list runs to
/// hundreds; the window states who is in it, not the whole call sheet.
const CAST_MAX: usize = 6;
const CREW_MAX: usize = 4;

fn panel_rect() -> Rect {
    Rect::new(
        (SCR_W - PANEL_W) * 0.5,
        (SCR_H - PANEL_H) * 0.5,
        PANEL_W,
        PANEL_H,
    )
}

/// The main (left) column's box, below nothing in particular: the whole column.
fn main_rect() -> Rect {
    let r = panel_rect();
    Rect::new(r.x + PAD, r.y + PAD, MAIN_W, PANEL_H - 2.0 * PAD)
}

/// The options (right) column's box.
fn options_rect() -> Rect {
    let r = panel_rect();
    Rect::new(
        r.x + PAD + MAIN_W + COL_GAP,
        r.y + PAD,
        OPTIONS_W,
        PANEL_H - 2.0 * PAD,
    )
}

/// The three-dot control's capsule at the head of the options column: right-aligned, because the
/// eye already reads this column's right edge (the chevrons, the action capsule) and a control in
/// the middle of a header reads as a title.
const MORE_W: f32 = 96.0;
const MORE_H: f32 = 56.0;

/// Where the three-dot control sits, derived from the column it heads. ONE function, so the rect
/// it is DRAWN in and the rect the menu is anchored to cannot drift.
fn more_rect(col: Rect) -> Rect {
    Rect::new(col.x + col.w - MORE_W, col.y, MORE_W, MORE_H)
}

/// Where each row of the options column is drawn, in ladder order. **ONE function for the painter
/// and the focus model**: the capsule a person aims at, the one the pointer's stop covers and the
/// one `place` answers are then the same rectangle, and the stops can never drift off the pills.
fn option_rects() -> [(OptionRow, Rect); 4] {
    let col = options_rect();
    let top = col.y + MORE_H + SECTION_GAP + HEAD_H + theme::space::XS;
    let audio = Rect::new(col.x, top, col.w, OPT_H);
    let subs = Rect::new(col.x, audio.y + OPT_H + OPT_GAP + HEAD_H + theme::space::XS, col.w, OPT_H);
    let action = Rect::new(col.x, subs.y + OPT_H + OPT_GAP + SECTION_GAP, col.w, ACT_H);
    [
        (OptionRow::More, more_rect(col)),
        (OptionRow::Audio, audio),
        (OptionRow::Subtitles, subs),
        (OptionRow::Action, action),
    ]
}

/// Where one row is drawn: the entry of [`option_rects`] for it (the array is in `index()` order),
/// so a `place` and a pill cannot disagree.
fn option_rect(row: OptionRow) -> Rect {
    option_rects()[row.index()].1
}

// ---- the choice model (PURE — this half is what the host suite grades) -------------------------

/// Which row of the options column holds the cursor.
///
/// **`More` is the first row, not a chrome glyph**: the three-dot control at the head of the column
/// is where the remote reaches the item's own actions (#369), and putting it in the ladder means
/// UP from the audio row lands on it rather than it being a control with no way to reach it. OK on
/// it opens the menu; OK on the other three starts the film.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OptionRow {
    More,
    Audio,
    Subtitles,
    Action,
}

impl OptionRow {
    pub(crate) const ALL: [Self; 4] = [Self::More, Self::Audio, Self::Subtitles, Self::Action];

    pub(crate) fn index(self) -> usize {
        match self {
            Self::More => 0,
            Self::Audio => 1,
            Self::Subtitles => 2,
            Self::Action => 3,
        }
    }

    pub(crate) fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    /// One rung DOWN the ladder (or UP), clamped at both ends. **The ends do not wrap**: one press
    /// from the action would otherwise skip the whole column and land on the menu control. One
    /// function for the keyboard's ladder and the engine's, so the two cannot disagree about where
    /// the ends are.
    pub(crate) fn stepped(self, down: bool) -> Self {
        let i = self.index();
        Self::from_index(if down {
            (i + 1).min(Self::ALL.len() - 1)
        } else {
            i.saturating_sub(1)
        })
    }
}

/// The audio tracks a viewer may pick from: every one the file carries, in server order. Unlike
/// the subtitle list these are NOT grouped — language and codec alone do not tell seven Russian AC3
/// tracks apart, and the bitrate that does is on the row
/// (`screens::tracks_panel::audio_choice`).
pub(crate) fn audio_choices(d: &Detail) -> Vec<(i64, String)> {
    d.audio
        .iter()
        .map(|s| (s.id, crate::ui::track_facts::audio_choice(s)))
        .collect()
}

/// The subtitle tracks a viewer may pick from: the EMBEDDED ones, because a sidecar has no
/// container ordinal and can only be shown by burning it into a transcode. Offering it here would
/// be offering a choice this window cannot carry out.
pub(crate) fn subtitle_choices(d: &Detail) -> Vec<(i64, String)> {
    d.subs
        .iter()
        .filter(|s| !s.external && s.id > 0)
        .map(|s| (s.id, crate::ui::track_facts::subtitle_choice(s)))
        .collect()
}

/// Which audio track the window opens on: the one the server marks selected, else the file's
/// flagged default, else the first — the same ladder `ui::track_menu::sync_item` reads, so the
/// window and the player's own menu cannot disagree about what "current" means.
pub(crate) fn initial_audio(d: &Detail) -> usize {
    let chosen = d.audio.iter().position(|s| s.selected)
        .or_else(|| d.audio.iter().position(|s| s.default));
    match chosen {
        Some(i) => i,
        None => 0,
    }
}

/// Which subtitle the window opens on: the server's selection when this client can render it,
/// else Off. `None` is Off.
pub(crate) fn initial_subtitle(d: &Detail) -> Option<usize> {
    let choices = subtitle_choices(d);
    choices
        .iter()
        .position(|(sid, _)| d.subs.iter().any(|s| s.id == *sid && s.selected))
}

/// The stream id of the chosen audio track, `0` when the item carries none.
pub(crate) fn audio_sid_at(d: &Detail, audio: usize) -> i64 {
    d.audio.get(audio).map(|s| s.id).unwrap_or(0)
}

/// The stream id of the chosen subtitle, `-1` for Off.
pub(crate) fn subtitle_sid_at(d: &Detail, sub: Option<usize>) -> i64 {
    let Some(i) = sub else { return 0 };
    subtitle_choices(d).get(i).map(|(sid, _)| *sid).unwrap_or(0)
}

/// Cycle `i` through `len` slots, wrapping — the LEFT/RIGHT gesture on a language row.
pub(crate) fn cycle(i: usize, len: usize, back: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if back {
        (i + len - 1) % len
    } else {
        (i + 1) % len
    }
}

/// Cycle the subtitle answer through `Off` and the list, wrapping. Off is slot 0, so the list is
/// one longer than the choices and an item with no subtitle at all still has the Off answer.
pub(crate) fn cycle_subtitle(current: Option<usize>, len: usize, back: bool) -> Option<usize> {
    let slots = len + 1;
    let at = current.map_or(0, |i| i + 1);
    let next = cycle(at, slots, back);
    (next > 0).then(|| next - 1)
}

/// The one-line genre row: the item's genres joined the way the page joins them.
pub(crate) fn genre_line(d: &Detail) -> String {
    d.genres.iter().filter(|g| !g.trim().is_empty()).cloned().collect::<Vec<_>>().join(" \u{b7} ")
}

/// **The credits row** — the cast first, then the crew, each `Name (Role)`, up to the two caps.
/// The server's own order is kept: it is the billing order for the cast and the job order for the
/// crew, and re-sorting either would be inventing a ranking.
pub(crate) fn credits_line(d: &Detail) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in d.cast.iter().take(CAST_MAX) {
        if c.tag.trim().is_empty() {
            continue;
        }
        parts.push(credit_text(&c.tag, &c.role));
    }
    for (i, c) in d.crew.iter().take(CREW_MAX).enumerate() {
        if c.tag.trim().is_empty() {
            continue;
        }
        // The crew's `role` is the JOB, and `Detail::credit_role` is the one accessor that
        // translates the two app-owned keys; `credit` addresses cast then crew in one index space.
        let role = d.credit_role(d.cast.len() + i).unwrap_or(&c.role).to_string();
        parts.push(credit_text(&c.tag, &role));
    }
    parts.join(" \u{b7} ")
}

fn credit_text(name: &str, role: &str) -> String {
    if role.trim().is_empty() {
        name.trim().to_string()
    } else {
        format!("{} ({})", name.trim(), role.trim())
    }
}

/// **The audience score**, 0-10, when the item carries one, the `Community` entry of
/// `Detail::ratings`. `None` is "the server stated no score", which draws no stars rather than
/// five empty ones: an empty row of stars would be asserting a zero the server never sent.
pub(crate) fn audience_score(d: &Detail) -> Option<f64> {
    d.ratings
        .iter()
        .find(|r| r.art == crate::metadata::RatingArt::Community)
        .map(|r| r.value)
}

/// **The facts**, in the order the window draws them: resolution, codecs, container, size, the
/// wall-clock end time and the length. Each is the item's own record through the formatters
/// `screens::tracks_panel` already owns, so the window and the Track-information sheet cannot spell
/// one number two ways.
pub(crate) fn fact_rows(d: &Detail, audio: usize, now_min: i32) -> Vec<crate::ui::track_facts::Pair> {
    use crate::ui::track_facts::{fmt_bitrate, fmt_codec_profile, fmt_frame_size, fmt_size};
    use crate::ui::track_facts::Pair;
    let mut v: Vec<Pair> = Vec::new();
    let mut push = |label: &'static str, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            v.push(Pair { label, value });
        }
    };
    let vs = d.video.as_ref();
    let vcodec = vs
        .map(|s| s.codec.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(d.vcodec.as_str());
    let profile = vs.map(|s| s.profile.as_str()).unwrap_or("");
    // The codec pair is the VIDEO one with the audio track the viewer has chosen beside it, so the
    // row answers "what am I about to hear" as well as "what am I about to see".
    let audio_codec = d.audio.get(audio).map(|s| s.codec.to_uppercase()).unwrap_or_default();
    let codec = match (
        fmt_codec_profile(vcodec, profile),
        audio_codec.is_empty(),
    ) {
        (Some(video), false) => Some(format!("{video} \u{b7} {audio_codec}")),
        (Some(video), true) => Some(video),
        (None, false) => Some(audio_codec),
        (None, true) => None,
    };
    push(crate::i18n::msg::widgets_tracks_codec(), codec);
    push(
        crate::i18n::msg::widgets_tracks_resolution(),
        fmt_frame_size(d.width, d.height),
    );
    push(
        crate::i18n::msg::widgets_tracks_container(),
        (!d.container.is_empty()).then(|| d.container.to_uppercase()),
    );
    push(crate::i18n::msg::widgets_tracks_size(), fmt_size(d.size));
    if d.dur_ms > 0 {
        push(
            crate::i18n::msg::widgets_tracks_duration(),
            Some(crate::ui::fmt::clock(d.dur_ms)),
        );
        // The one fact a person plans around. Its run is drawn into a fixed-width cell rather
        // than measured from the string (see the module doc): the clock moves, and a live clock
        // in a measurement key would miss on every replay of the same recording.
        push(
            crate::i18n::msg::widgets_tracks_ends(),
            Some(crate::ui::fmt::ends_at(now_min, d.dur_ms)),
        );
    }
    push(
        crate::i18n::msg::widgets_tracks_total_bitrate(),
        fmt_bitrate(d.bitrate),
    );
    v
}

/// How many synopsis lines fit under `room` px, with `credits_h` reserved below it.
///
/// **The panel grows to the family's ceiling and this is what keeps the prose inside it.** Every
/// other block is a constant or a cap, so the budget is arithmetic; it never returns 0, because a
/// window whose whole left column ends in a heading with nothing under it is worse than one
/// clipped line. `credits_h` is 0 for an item with no credits, and its gap goes with it.
pub(crate) fn main_synopsis_lines(room: f32, credits_h: f32, has_credits: bool) -> usize {
    let reserved = HEAD_H + theme::space::XS + credits_h + if has_credits { SECTION_GAP } else { 0.0 };
    let body = room - reserved;
    let lines = (body / SYN_LEAD).floor();
    if lines >= 1.0 {
        (lines as usize).min(SYN_LINES_MAX)
    } else {
        1
    }
}

// ---- the surface -------------------------------------------------------------------------------

/// The fields [`PrePlaybackScreen`] canonicalises, for the recorder's shape pin (§5.4). The cursor
/// and both choices are in it deliberately: nothing else in the app moves when they do, so without
/// them a replay would grade the window opening and closing with a hole between.
pub(crate) const SHAPE: &str = "PrePlaybackScreen{row:u32,audio:u32,sub:i32}";

/// What the container is asked to present.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PrePlaybackArg {
    /// The press was *Play from Start*: the playback drops the resume point.
    pub(crate) restart: bool,
}

impl crate::ui::machine::LogicalState for PrePlaybackArg {
    fn write(&self, c: &mut crate::ui::machine::Canon) {
        c.u32(u32::from(self.restart));
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_arg");
    }
}

pub(crate) struct PrePlaybackScreen {
    entry: crate::ui::machine::EntryId,
    arg: PrePlaybackArg,
    row: OptionRow,
    /// Index into `Detail::audio`.
    audio: usize,
    /// Index into [`subtitle_choices`]; `None` is Off.
    sub: Option<usize>,
    /// Has the initial choice been taken from the item? It is taken on the first draw, which is
    /// where the item is first known, and never again — a refresh landing under an open window
    /// must not silently move the viewer's cursor back to the server's default.
    seeded: bool,
}

impl PrePlaybackScreen {
    pub(crate) fn new(entry: crate::ui::machine::EntryId, arg: PrePlaybackArg) -> Self {
        Self {
            entry,
            arg,
            row: OptionRow::Action,
            audio: 0,
            sub: None,
            seeded: false,
        }
    }

    /// Take the item's current tracks as the starting choice, once.
    fn seed(&mut self, d: &Detail) {
        if self.seeded {
            return;
        }
        self.seeded = true;
        self.audio = initial_audio(d);
        self.sub = initial_subtitle(d);
    }

    /// The playback this window would start: the page's own intent, built from the item it holds,
    /// with the resume policy the press named.
    fn play_intent(&self, d: &Detail) -> crate::screens::registry::PlayIntent {
        crate::screens::registry::PlayIntent::Item {
            sid: crate::route::item_sid(d.sid),
            rk: d.rk.clone(),
            part: d.part.clone(),
            vcodec: d.vcodec.clone(),
            acodec: d.acodec.clone(),
            title: d.title.clone(),
            context: String::new(),
        }
    }

    /// The tracks the playback starts with — the whole reason the window exists.
    fn tracks(&self, d: &Detail) -> crate::route::StartTracks {
        crate::route::StartTracks {
            audio_sid: audio_sid_at(d, self.audio),
            sub_sid: subtitle_sid_at(d, self.sub),
        }
    }

    /// OK: start the film with the tracks on screen, and leave.
    fn commit<H: crate::screens::registry::AppLike>(
        &self,
        d: &Detail,
        fx: &mut crate::ui::machine::Effects<'_, H>,
    ) {
        use crate::screens::registry::{AppFx, ContentReq};
        use crate::ui::machine::Fx;
        let resume_ns = if self.arg.restart {
            0
        } else {
            crate::metadata::resume_ns(d.resume_ms, d.dur_ms)
        };
        fx.push(Fx::App(AppFx::Content(ContentReq::Play {
            play: self.play_intent(d),
            resume_ns,
            tracks: self.tracks(d),
        })));
    }

    /// OK on the three-dot control: present the item's own menu over this window. The control's
    /// drawn rect rides on the request, so the menu hangs off it exactly where it was pressed , 
    /// and the loop presents it on this window's own stack, leaving the window up behind it.
    fn open_menu<H: crate::screens::registry::AppLike>(
        &self,
        fx: &mut crate::ui::machine::Effects<'_, H>,
    ) {
        use crate::screens::registry::{AppFx, ContentPanel, ContentReq};
        use crate::ui::machine::Fx;
        let anchor = more_rect(options_rect());
        fx.push(Fx::App(AppFx::Content(ContentReq::Panel(
            ContentPanel::PrePlaybackMenu {
                anchor: [anchor.x, anchor.y, anchor.w, anchor.h].map(f32::to_bits),
            },
        ))));
    }

    /// LEFT/RIGHT on a language row. Answers whether anything moved.
    fn cycle_value(&mut self, d: &Detail, back: bool) -> bool {
        match self.row {
            OptionRow::Audio => {
                let len = d.audio.len();
                if len == 0 {
                    return false;
                }
                let next = cycle(self.audio, len, back);
                let moved = next != self.audio;
                self.audio = next;
                moved
            }
            OptionRow::Subtitles => {
                let len = subtitle_choices(d).len();
                let next = cycle_subtitle(self.sub, len, back);
                let moved = next != self.sub;
                self.sub = next;
                moved
            }
            OptionRow::Action => false,
            // The menu control holds no value to cycle: LEFT/RIGHT there move nothing.
            OptionRow::More => false,
        }
    }

    /// **The four rows as pointer stops** (rule 11: hover parks focus, a click activates). Split out
    /// of `draw` for `PlayerScreen::record_stops`'s reason: a host unit build links no GL and no
    /// SDL2_ttf, so the suite can grade the stops without the paint pass that would issue both.
    /// They ride the SETTLED geometry, as every surface's stops do: at rest (`appear == 1`) the
    /// drawn and settled rects are the same, and `place` answers these same four rectangles.
    fn record_stops<H: crate::screens::registry::AppLike>(
        &self,
        f: &mut crate::ui::screen::DrawFrame<'_, '_, H>,
    ) {
        let p = f.painter;
        for (row, rect) in option_rects() {
            f.stop(
                p,
                crate::ui::screen::Stop {
                    key: crate::ui::machine::FocusKey {
                        entry: self.entry,
                        elem: row.index() as u32,
                    },
                    rect,
                    rest_rect: rect,
                    clip: panel_rect(),
                    // The SAME focus face the arrows give it: the pointer parks focus through the
                    // engine, so there is no second, pointer-only treatment to keep in step.
                    hover: crate::ui::screen::Hover::Focus,
                    // A row here commits on the press, exactly as its OK does (no pill dip to arm).
                    activate: crate::ui::screen::Activate::Immediate,
                },
            );
        }
    }
}

// ---- draw --------------------------------------------------------------------------------------

/// One label-over-value pair at a cursor y.
fn pair_at(
    p: Painter,
    pr: &crate::ui::track_facts::Pair,
    x: f32,
    y: f32,
    w: f32,
) -> f32 {
    if let Ok(cs) = CString::new(pr.label) {
        Label::new(cs.as_ptr(), theme::size::CAPTION, theme::TEXT_TERTIARY)
            .v(VAlign::CapTop)
            .h(theme::alert::TEXT_ALIGN)
            .draw(p, Rect::new(x, y, w, LABEL_H));
    }
    let vy = y + LABEL_H + PAIR_GAP;
    if let Ok(cs) = CString::new(pr.value.as_str()) {
        Label::new(cs.as_ptr(), theme::size::LABEL, theme::TEXT_HEADING)
            .bold()
            .v(VAlign::CapTop)
            .h(theme::alert::TEXT_ALIGN)
            .draw(p, Rect::new(x, vy, w, VALUE_H));
    }
    vy + VALUE_H
}

/// A caps section head.
fn head_at(p: Painter, text: &str, x: f32, y: f32, w: f32) {
    if let Ok(cs) = CString::new(text.to_uppercase()) {
        Label::new(cs.as_ptr(), theme::size::CAPTION, theme::TEXT_TERTIARY)
            .bold()
            .v(VAlign::CapTop)
            .h(theme::alert::TEXT_ALIGN)
            .draw(p, Rect::new(x, y, w, HEAD_H));
    }
}

impl PrePlaybackScreen {
    fn paint(
        &mut self,
        d: &Detail,
        appear: f32,
        measure: &dyn crate::ui::machine::Measure,
        field: Option<&crate::ui::underlay::UnderlayField>,
    ) {
        self.seed(d);
        let r = panel_rect();
        let slide = RISE * (1.0 - appear);
        let p = Painter::root().alpha(appear).translate(0.0, slide);
        widgets::panel_ground(p, r, theme::ALERT_PANEL_RAD, field);

        let main = main_rect();
        let opts = options_rect();
        self.paint_main(p, d, main, measure);
        self.paint_options(p, d, opts, measure);

        // ---- footer: the one thing this window answers besides OK ------------------------------
        let hint = widgets::KeyHint::translated(
            crate::i18n::msg::widgets_hint_return("\u{fffc}"),
            c"BACK",
        );
        let cy = r.y + PANEL_H - PAD - widgets::KeyHint::height() * 0.5;
        hint.draw(p, r.x + PANEL_W - PAD - hint.width(measure), cy, measure);
    }

    /// **The scores line**: the audience's five stars, then the critics' percentage under the
    /// provider's own name. Returns the height it consumed INCLUDING the gap under it, and `0.0`
    /// when the server sent neither score, the facts then move up into the space rather than
    /// sitting under an empty band.
    ///
    /// The two are separate facts from separate fields (`CommunityRating` and `CriticRating`), and
    /// **each is drawn only when the server sent it**: one number under two names is the
    /// attribution #317 refused, and it stays refused (#368).
    fn paint_scores(
        &self,
        p: Painter,
        d: &Detail,
        x: f32,
        y: f32,
        clip: Rect,
        measure: &dyn crate::ui::machine::Measure,
    ) -> f32 {
        let audience = audience_score(d);
        let critic = d.critic_rating;
        if audience.is_none() && critic.is_none() {
            return 0.0;
        }
        let cy = y + RATING_ROW_H * 0.5;
        let mut bx = x;
        if let Some(value) = audience {
            // No caption: five stars ARE a score out of five, and the caption would cost the width
            // the critics' provider name needs on this line.
            bx += widgets::star_score(p, clip, bx, cy, value) + theme::space::LG;
        }
        if let Some(percent) = critic {
            let text = crate::ui::fmt::percent(percent);
            let cells = [widgets::RatingCell {
                mark: &[],
                value: &text,
                suffix: "",
            }];
            widgets::rating_group(
                p,
                bx,
                cy,
                crate::metadata::RatingArt::Critic.provider(),
                &cells,
                measure,
            );
        }
        RATING_ROW_H + GAP_RATINGS_FACTS
    }

    /// The left column: the backdrop beside the title and the facts, then genres, synopsis and
    /// credits under them.
    ///
    /// The whole column is CLIPPED to its box. Every block below is capped (four synopsis lines,
    /// two credit lines, six facts), so the arithmetic fits — but a cap is a promise about one
    /// language's strings, and a translation that wraps one line further must not paint over the
    /// options column or off the sheet.
    fn paint_main(
        &self,
        p: Painter,
        d: &Detail,
        col: Rect,
        measure: &dyn crate::ui::machine::Measure,
    ) {
        p.clip(col);
        let mut y = col.y;

        // ---- the backdrop, cover-cropped into its own box ------------------------------------
        //
        // A HARD rect, not a rounded one: the texture is drawn under a scissor (this renderer has
        // no rounded-image shader), so a rounded skeleton behind it would show square corners
        // poking out of the arc.
        let band = Rect::new(col.x, y, ART_W, ART_H);
        p.rect(band, 0.0, theme::CONTROL_IDLE_FILL, theme::CONTROL_IDLE_FILL, 0.0);
        if !d.art.is_empty() {
            let (tex, w, h) = widgets::resolve_tex_wh_on(d.sid, &d.art, 1280, 720, 0);
            if tex != 0 {
                p.clip(band);
                p.tex_uv(
                    tex,
                    crate::gfx::UV_FULL,
                    band.cover(w, h),
                    0.0,
                    theme::with_a(theme::TINT_WHITE, 0.92),
                );
                p.clip(col);
            }
        }

        // ---- title and facts, beside the backdrop --------------------------------------------
        let text_x = band.x + band.w + GAP_ART_TEXT;
        let text_w = col.w - ART_W - GAP_ART_TEXT;
        let mut ty = col.y;
        if let Ok(cs) = CString::new(crate::text::elide_by(&d.title, text_w, false, |t| {
            measure.width_str(t, theme::size::TITLE, true)
        })) {
            Label::new(cs.as_ptr(), theme::size::TITLE, theme::TEXT_PRIMARY)
                .bold()
                .v(VAlign::CapTop)
                .h(theme::alert::TEXT_ALIGN)
                .draw(p, Rect::new(text_x, ty, text_w, TITLE_H));
        }
        ty += TITLE_H + GAP_TITLE_FACTS;
        ty += self.paint_scores(p, d, text_x, ty, col, measure);

        let now_min = crate::ui::fmt::wall_minutes().unwrap_or(0);
        let facts = fact_rows(d, self.audio, now_min);
        const FACT_COLS: usize = 2;
        let fcw = (text_w - COL_GAP) / FACT_COLS as f32;
        for (i, pr) in facts.iter().enumerate() {
            let cx = text_x + (i % FACT_COLS) as f32 * (fcw + COL_GAP);
            let cy = ty + (i / FACT_COLS) as f32 * (LABEL_H + PAIR_GAP + VALUE_H + ROW_GAP);
            pair_at(p, pr, cx, cy, fcw);
        }
        let fact_rows_n = facts.len().div_ceil(FACT_COLS).max(1);
        ty += fact_rows_n as f32 * (LABEL_H + PAIR_GAP + VALUE_H + ROW_GAP);
        y = ty.max(band.y + band.h) + SECTION_GAP;

        // ---- genres --------------------------------------------------------------------------
        let genres = genre_line(d);
        if !genres.is_empty() {
            if let Ok(cs) = CString::new(crate::text::elide_by(&genres, col.w, false, |t| {
                measure.width_str(t, theme::size::CAPTION, false)
            })) {
                Label::new(cs.as_ptr(), theme::size::CAPTION, theme::TEXT_SECONDARY)
                    .v(VAlign::CapTop)
                    .h(theme::alert::TEXT_ALIGN)
                    .draw(p, Rect::new(col.x, y, col.w, GENRE_LEAD));
            }
            y += GENRE_LEAD + SECTION_GAP;
        }

        // ---- cast and crew, measured FIRST so the synopsis knows what is left -----------------
        let credits = credits_line(d);
        let credits_h = if credits.is_empty() {
            0.0
        } else {
            HEAD_H + theme::space::XS + CREDIT_LEAD * 2.0
        };

        // ---- synopsis ------------------------------------------------------------------------
        if !d.summary.trim().is_empty() {
            let head_h = HEAD_H + theme::space::XS;
            let lines = main_synopsis_lines(col.y + col.h - y, credits_h, !credits.is_empty());
            head_at(p, crate::i18n::msg::widgets_preplay_overview(), col.x, y, col.w);
            y += head_h;
            let syn = TextView::new(&d.summary, theme::size::LABEL, theme::TEXT_READING)
                .h(theme::alert::TEXT_ALIGN)
                .leading(SYN_LEAD)
                .max_lines(lines);
            syn.draw(p, Rect::new(col.x, y, col.w, 0.0));
            y += syn.measure_h(col.w) + SECTION_GAP;
        }

        // ---- cast and crew -------------------------------------------------------------------
        if !credits.is_empty() {
            head_at(p, crate::i18n::msg::browse_detail_cast(), col.x, y, col.w);
            y += HEAD_H + theme::space::XS;
            let body = TextView::new(&credits, theme::size::CAPTION, theme::TEXT_SECONDARY)
                .h(theme::alert::TEXT_ALIGN)
                .leading(CREDIT_LEAD)
                .max_lines(2);
            body.draw(p, Rect::new(col.x, y, col.w, 0.0));
        }
        p.clip_clear();
    }

    /// The right column: the item's own menu control, then the two language choices and the primary
    /// action.
    fn paint_options(
        &self,
        p: Painter,
        d: &Detail,
        col: Rect,
        measure: &dyn crate::ui::machine::Measure,
    ) {
        self.paint_more(p, col);
        let audio_rows = audio_choices(d);
        let audio_text = audio_rows
            .get(self.audio)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| crate::i18n::msg::widgets_tracks_unknown().to_string());
        let subs = subtitle_choices(d);
        let sub_text = match self.sub.and_then(|i| subs.get(i)) {
            Some((_, label)) => label.clone(),
            None => crate::i18n::msg::widgets_tracks_off().to_string(),
        };

        // The headings sit above their capsules at the ladder's own gaps, so the pills and the
        // stops come from ONE walk (`option_rects`).
        let audio = option_rect(OptionRow::Audio);
        head_at(
            p,
            crate::i18n::msg::widgets_tracks_audio(),
            col.x,
            audio.y - theme::space::XS - HEAD_H,
            col.w,
        );
        self.option_row(
            p, OptionRow::Audio, &audio_text, audio, !audio_rows.is_empty(), measure,
        );

        let subs_rect = option_rect(OptionRow::Subtitles);
        head_at(
            p,
            crate::i18n::msg::widgets_tracks_subtitles(),
            col.x,
            subs_rect.y - theme::space::XS - HEAD_H,
            col.w,
        );
        self.option_row(
            p, OptionRow::Subtitles, &sub_text, subs_rect, !subs.is_empty(), measure,
        );

        // ---- the primary action ------------------------------------------------------------
        let action = if self.arg.restart || crate::metadata::resume_ns(d.resume_ms, d.dur_ms) <= 0 {
            crate::i18n::msg::browse_detail_play()
        } else {
            crate::i18n::msg::browse_detail_resume()
        };
        let focused = self.row == OptionRow::Action;
        let rect = option_rect(OptionRow::Action);
        let (fill, ink) = if focused {
            (theme::ACCENT, theme::ACCENT_INK)
        } else {
            (theme::CONTROL_IDLE_FILL, theme::CONTROL_IDLE_INK)
        };
        p.rrect(rect, ACT_H * 0.5, ACT_H * 0.5, fill);
        let ty = crate::text::text_vcenter_y(theme::size::LABEL, 1, rect.y + rect.h * 0.5);
        let label = crate::text::elide_by(action, rect.w - theme::space::MD * 2.0, false, |t| {
            measure.width_str(t, theme::size::LABEL, true)
        });
        if let Ok(cs) = CString::new(label) {
            p.text(
                cs.as_ptr(),
                rect.x + rect.w * 0.5,
                ty,
                theme::size::LABEL,
                ink,
                1,
                1,
            );
        }
    }

    /// The three-dot control at the head of the options column (#369): the overflow glyph in a
    /// capsule that takes the focus fill like every other control in this window. It is the first
    /// rung of the cursor ladder, so UP from the audio row reaches it.
    fn paint_more(&self, p: Painter, col: Rect) {
        let rect = more_rect(col);
        let focused = self.row == OptionRow::More;
        p.rrect(
            rect,
            MORE_H * 0.5,
            MORE_H * 0.5,
            if focused {
                theme::OVERLAY_FOCUS_PILL
            } else {
                theme::CONTROL_IDLE_FILL
            },
        );
        const GLYPH: f32 = 26.0;
        crate::ui::icons::draw(
            p,
            Icon::More,
            Rect::new(
                rect.cx() - GLYPH * 0.5,
                rect.y + (rect.h - GLYPH) * 0.5,
                GLYPH,
                GLYPH,
            ),
            if focused {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_SECONDARY
            },
        );
    }

    /// One language row: a capsule, its value, and the LEFT/RIGHT chevrons when it holds the
    /// cursor.
    #[allow(clippy::too_many_arguments)]
    fn option_row(
        &self,
        p: Painter,
        row: OptionRow,
        value: &str,
        rect: Rect,
        has_choices: bool,
        measure: &dyn crate::ui::machine::Measure,
    ) {
        let focused = self.row == row;
        p.rrect(
            rect,
            OPT_H * 0.5,
            OPT_H * 0.5,
            if focused {
                theme::OVERLAY_FOCUS_PILL
            } else {
                theme::CONTROL_IDLE_FILL
            },
        );
        let ink = if focused { theme::TEXT_PRIMARY } else { theme::TEXT_SECONDARY };
        let inset = theme::space::MD;
        // The chevrons own a fixed band on the right when this row can be changed at all.
        const GLYPH: f32 = 22.0;
        let glyph_band = if has_choices { GLYPH * 2.0 + theme::space::XS } else { 0.0 };
        if focused && has_choices {
            let gy = rect.y + (rect.h - GLYPH) * 0.5;
            for (i, icon) in [Icon::ChevronLeft, Icon::Chevron].into_iter().enumerate() {
                crate::ui::icons::draw(
                    p,
                    icon,
                    Rect::new(
                        rect.x + rect.w - inset - glyph_band + i as f32 * (GLYPH + theme::space::XS),
                        gy,
                        GLYPH,
                        GLYPH,
                    ),
                    theme::TEXT_PRIMARY,
                );
            }
        }
        let tw = rect.w - inset * 2.0 - glyph_band;
        let ty = crate::text::text_vcenter_y(theme::size::LABEL, 1, rect.y + rect.h * 0.5);
        if let Ok(cs) = CString::new(crate::text::elide_by(value, tw, false, |t| {
            measure.width_str(t, theme::size::LABEL, true)
        })) {
            p.text(cs.as_ptr(), rect.x + inset, ty, theme::size::LABEL, ink, 1, 0);
        }
    }
}

// ---- the Screen contract ------------------------------------------------------------------------

impl<H: crate::screens::registry::AppLike + crate::screens::registry::MetadataLike> crate::ui::machine::Machine<H> for PrePlaybackScreen {
    type Ev = crate::ui::screen::ScreenEvent<H>;
    fn step(
        &mut self,
        ev: &Self::Ev,
        cx: &crate::ui::machine::Cx<'_, H>,
        fx: &mut crate::ui::machine::Effects<'_, H>,
    ) -> crate::ui::machine::Handled {
        use crate::ui::machine::{Edge, Fx, Handled, InputKind, Key, NavOp};
        use crate::ui::screen::ScreenEvent;
        match ev {
            ScreenEvent::Input(input) => match input.kind {
                InputKind::Key {
                    key: Key::Back,
                    edge: Edge::Down,
                    ..
                } => {
                    // BACK leaves the page exactly as it was and starts nothing.
                    fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
                    Handled::Yes
                }
                InputKind::Key {
                    key: Key::Ok,
                    edge: Edge::Down,
                    ..
                } => {
                    // OK is the primary action from every LANGUAGE row: a remote's OK means "do
                    // it", and a window whose OK did nothing on two of its rows would be the dead
                    // key the read-only alerts are careful to avoid. The play request carries the
                    // tracks on screen; the loop dismisses this surface before the player is pushed.
                    //
                    // The one exception is the three-dot control at the head of the column, whose
                    // OK opens the item's own menu, a real surface, so still not a dead key.
                    let meta = H::metadata(cx);
                    let Some(d) = meta.current() else {
                        // Nothing landed to act on: close rather than leave a dead OK on a window
                        // with no item under it.
                        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
                        return Handled::Yes;
                    };
                    self.seed(d);
                    if self.row == OptionRow::More {
                        self.open_menu(fx);
                    } else {
                        self.commit(d, fx);
                    }
                    fx.invalidate(crate::ui::present::Provenance::Input);
                    Handled::Yes
                }
                InputKind::Key {
                    sym,
                    edge: Edge::Down | Edge::Repeat,
                    ..
                } => {
                    let meta = H::metadata(cx);
                    let Some(d) = meta.current() else { return Handled::Yes };
                    self.seed(d);
                    // UP/DOWN are the ENGINE's: this window's four rows are its focus model's four
                    // elements (`Focusable::neighbour`), so a key and a hover land the same way and
                    // the row below is always the row the pointer would park. Only the sideways
                    // gesture is the screen's own, because it changes a VALUE rather than the seat.
                    let moved = match sym as u32 {
                        SDLK_LEFT => self.cycle_value(d, true),
                        SDLK_RIGHT => self.cycle_value(d, false),
                        _ => false,
                    };
                    if moved {
                        fx.invalidate(crate::ui::present::Provenance::Input);
                    }
                    Handled::Yes
                }
                InputKind::Click { .. } | InputKind::Pointer { .. } => Handled::Yes,
                _ => Handled::No,
            },
            // The row the ENGINE landed on is the row this window draws lit: a D-pad press, a
            // pointer hover (`Hover::Focus` on the four stops) and a restore all arrive here, so
            // there is one focus face and no second hover style to keep in step. A key that is not
            // one of the four rows is chrome the container composes (the shared Back control), and
            // it lights ITSELF: the rows keep the cursor they had.
            ScreenEvent::FocusMoved { to, .. } => {
                if let Some(row) = OptionRow::ALL.get(to.elem as usize) {
                    self.row = *row;
                    fx.invalidate(crate::ui::present::Provenance::Input);
                }
                Handled::Yes
            }
            // A click on a row runs it, exactly as OK does on the row it parks.
            ScreenEvent::Activate(elem) => {
                // Only a ROW activates here. `from_index`'s clamp would turn a foreign key (the
                // shared Back control's) into the primary action, which is the one thing a click on
                // the way OUT must never do.
                let Some(row) = OptionRow::ALL.get(*elem as usize).copied() else {
                    return Handled::No;
                };
                let meta = H::metadata(cx);
                let Some(d) = meta.current() else {
                    fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
                    return Handled::Yes;
                };
                self.seed(d);
                self.row = row;
                if self.row == OptionRow::More {
                    self.open_menu(fx);
                } else {
                    self.commit(d, fx);
                }
                fx.invalidate(crate::ui::present::Provenance::Input);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

/// **The window's focus model: its four rows, as four elements of one column.** The rows used to be
/// a cursor the screen kept to itself over a single `Document` element covering the whole panel,
/// which is what made the window dead to the Magic Remote's pointer: a hit map can only park focus
/// on a stop whose key the focus model knows, and it knew one key for the entire sheet. With one
/// element per row, the D-pad, a pointer hover (`Hover::Focus` on the stops `record_stops`
/// registers) and a restore all move the same focus, so the row the pointer parks IS the row the
/// arrows would have reached and the face it lights is the one focus face this window has.
impl<H: crate::screens::registry::AppLike> crate::ui::screen::Focusable<H> for PrePlaybackScreen {
    fn groups(&self, _cx: &crate::ui::machine::Cx<'_, H>, out: &mut Vec<crate::ui::screen::GroupSpec>) {
        out.push(crate::ui::screen::GroupSpec {
            id: crate::ui::machine::GroupId(0),
            kind: crate::ui::screen::GroupKind::Column,
            seat: crate::ui::screen::Seat::Remembered,
            reachable: crate::ui::screen::AxisMask::BOTH,
            // Every side is a wall at the ends: the ladder clamps, and the sideways gesture is the
            // language rows' own (it changes a value), so it never leaves this column.
            edge: [crate::ui::screen::EdgeRule::Stop; 4],
            extent: options_rect(),
            len: OptionRow::ALL.len(),
            elem: crate::ui::screen::ElemKind::Control,
        });
    }
    fn group_of(&self, key: &u32, _cx: &crate::ui::machine::Cx<'_, H>) -> Option<crate::ui::machine::GroupId> {
        ((*key as usize) < OptionRow::ALL.len()).then_some(crate::ui::machine::GroupId(0))
    }
    fn neighbour(
        &self,
        key: crate::ui::machine::FocusKey<u32>,
        dir: crate::ui::screen::Dir,
        _cx: &crate::ui::machine::Cx<'_, H>,
    ) -> crate::ui::screen::Step<u32> {
        let down = match dir {
            crate::ui::screen::Dir::Up => false,
            crate::ui::screen::Dir::Down => true,
            crate::ui::screen::Dir::Left | crate::ui::screen::Dir::Right => {
                return crate::ui::screen::Step::Edge
            }
        };
        let from = OptionRow::from_index(key.elem as usize);
        let to = from.stepped(down);
        if to == from {
            crate::ui::screen::Step::Edge
        } else {
            crate::ui::screen::Step::Move(crate::ui::machine::FocusKey {
                entry: key.entry,
                elem: to.index() as u32,
            })
        }
    }
    fn place(
        &self,
        key: &u32,
        _cx: &crate::ui::machine::Cx<'_, H>,
        _at: crate::ui::screen::At,
    ) -> Option<crate::ui::screen::Placed> {
        let row = OptionRow::ALL.get(*key as usize)?;
        let rect = option_rect(*row);
        Some(crate::ui::screen::Placed {
            rect,
            rest_rect: rect,
            clip: panel_rect(),
            index: Some(row.index() as u32),
        })
    }
    fn reconcile(
        &self,
        want: crate::ui::machine::FocusKey<u32>,
        _cx: &crate::ui::machine::Cx<'_, H>,
    ) -> crate::ui::machine::FocusKey<u32> {
        // A key that is not one of the four rows bounces back to the row this window is drawing
        // lit, which is the row the engine last landed on.
        if (want.elem as usize) < OptionRow::ALL.len() {
            want
        } else {
            crate::ui::machine::FocusKey {
                entry: self.entry,
                elem: self.row.index() as u32,
            }
        }
    }
    fn seat(
        &self,
        _g: crate::ui::machine::GroupId,
        _from: crate::ui::screen::Placed,
        _cx: &crate::ui::machine::Cx<'_, H>,
    ) -> crate::ui::machine::FocusKey<u32> {
        crate::ui::machine::FocusKey {
            entry: self.entry,
            elem: self.row.index() as u32,
        }
    }
}

impl crate::ui::machine::LogicalState for PrePlaybackScreen {
    fn write(&self, c: &mut crate::ui::machine::Canon) {
        c.u32(self.row.index() as u32)
            .u32(self.audio as u32)
            .u32(self.sub.map_or(u32::MAX, |i| i as u32));
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay");
    }
}

impl<H: crate::screens::registry::AppLike + crate::screens::registry::MetadataLike> crate::ui::screen::Screen<H>
    for PrePlaybackScreen
{
    fn name(&self) -> &'static str {
        "preplay"
    }
    fn state(&self) -> &dyn crate::ui::machine::LogicalState {
        self
    }
    fn crumb(&self, _cx: &crate::ui::machine::Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut crate::ui::frame::Budget, _cx: &crate::ui::machine::Cx<'_, H>) {}
    /// The modal dim, asked for rather than drawn, exactly as the three read-only sheets ask for
    /// it: this window replaces the middle of the frame and holds no element under it that the dim
    /// must spare.
    fn scrim(&self) -> crate::ui::screen::Scrim {
        crate::ui::screen::Scrim::dim(theme::underlay::DIM_PANEL)
    }
    fn draw(&mut self, f: &mut crate::ui::screen::DrawFrame<'_, '_, H>) {
        let meta = H::metadata(f.cx);
        let Some(d) = meta.current() else { return };
        let appear = f.page_alpha;
        let measure = f.measure;
        let field = f.underlay;
        crate::ui::profile::phase("dt.preplay", || self.paint(d, appear, measure, field));
        self.record_stops(f);
    }
    fn render(&self) -> crate::ui::screen::RenderStrategy {
        crate::ui::screen::RenderStrategy::Page
    }
    fn focus_source(&self) -> crate::ui::screen::FocusSource {
        crate::ui::screen::FocusSource::Engine
    }
    fn hit_source(&self) -> crate::ui::screen::HitSource {
        crate::ui::screen::HitSource::Engine
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Cast, Stream};

    fn audio(lang: &str, codec: &str, layout: &str, ch: i64, kbps: i64) -> Stream {
        Stream {
            lang: lang.into(),
            codec: codec.into(),
            layout: layout.into(),
            channels: ch,
            bitrate: kbps,
            ..Default::default()
        }
    }

    fn sub(lang: &str, codec: &str, id: i64) -> Stream {
        Stream {
            id,
            lang: lang.into(),
            codec: codec.into(),
            ..Default::default()
        }
    }

    fn item() -> Detail {
        let mut english = audio("English", "eac3", "5.1(side)", 6, 768);
        english.id = 2;
        english.selected = true;
        let mut russian = audio("Русский", "ac3", "5.1(side)", 6, 448);
        russian.id = 1;
        let mut forced = sub("English", "subrip", 5);
        forced.forced = true;
        Detail {
            sid: crate::registry::ServerId::UNSET,
            rk: "movie".into(),
            kind: "movie".into(),
            title: "A Film".into(),
            part: "/library/parts/1/file.mkv".into(),
            vcodec: "hevc".into(),
            acodec: "eac3".into(),
            container: "mkv".into(),
            size: 34_659_545_780,
            width: 3840,
            height: 1602,
            dur_ms: 9_610_336,
            bitrate: 28_852,
            summary: "A synopsis.".into(),
            genres: vec!["Drama".into(), "Science Fiction".into()],
            audio: vec![russian, english],
            subs: vec![sub("English", "subrip", 4), forced],
            cast: vec![
                Cast { tag: "Ada Actor".into(), role: "Lead".into(), thumb: String::new(), id: 0, tag_key: String::new() },
                Cast { tag: "Bob Actor".into(), role: "Second".into(), thumb: String::new(), id: 0, tag_key: String::new() },
            ],
            crew: vec![Cast { tag: "Cy Director".into(), role: "Director".into(), thumb: String::new(), id: 0, tag_key: String::new() }],
            ..Default::default()
        }
    }

    /// The window offers the file's OWN tracks, in the server's order — audio ungrouped (the
    /// bitrate is what tells two same-language tracks apart) and subtitles only the embedded ones,
    /// because a sidecar has no container ordinal this client can render on a direct play.
    #[test]
    fn the_window_offers_the_files_own_tracks() {
        let d = item();
        let audio = audio_choices(&d);
        assert_eq!(audio.len(), 2);
        assert_eq!(audio[0].1, "AC3 5.1 \u{b7} 448 kbps");
        assert_eq!(audio[1].1, "EAC3 5.1 \u{b7} 768 kbps");
        let subs = subtitle_choices(&d);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].1, "English \u{b7} SUBRIP");
        assert_eq!(subs[1].1, "English \u{b7} SUBRIP \u{b7} Forced");
        // …and an external track is not offered: a sidecar cannot be carried by this path.
        let mut external = sub("English", "srt", 9);
        external.external = true;
        let with_sidecar = Detail { subs: vec![external], ..item() };
        assert!(subtitle_choices(&with_sidecar).is_empty());
    }

    /// The window opens on what the playback would have used anyway: the server's selected audio,
    /// else the file's flagged default, and the server's subtitle when it is one this client can
    /// render. Opening on a different track than the ladder's would make the window's mere
    /// appearance change the play.
    #[test]
    fn the_window_opens_on_the_tracks_the_ladder_would_have_picked() {
        let d = item();
        assert_eq!(initial_audio(&d), 1, "the selected English track");
        assert_eq!(initial_subtitle(&d), None, "nothing selected is Off");
        let mut picked = item();
        picked.subs[1].selected = true;
        assert_eq!(initial_subtitle(&picked), Some(1), "the forced track the server selected");
        // no server selection at all: the first audio track
        let mut plain = item();
        plain.audio[1].selected = false;
        assert_eq!(initial_audio(&plain), 0);
    }

    /// LEFT/RIGHT walks the choices and wraps at both ends; the subtitle row's slot 0 is Off, so
    /// a film with two subtitle tracks has three answers and Off is reachable in both directions.
    #[test]
    fn the_language_rows_cycle_and_wrap() {
        assert_eq!(cycle(0, 2, false), 1);
        assert_eq!(cycle(1, 2, false), 0);
        assert_eq!(cycle(0, 2, true), 1);
        assert_eq!(cycle(0, 0, false), 0, "an item with no track has nowhere to go");

        assert_eq!(cycle_subtitle(None, 2, false), Some(0));
        assert_eq!(cycle_subtitle(Some(0), 2, false), Some(1));
        assert_eq!(cycle_subtitle(Some(1), 2, false), None, "wraps back to Off");
        assert_eq!(cycle_subtitle(None, 2, true), Some(1), "and backwards from Off");
        assert_eq!(cycle_subtitle(None, 0, false), None, "only Off exists");
    }

    /// The chosen indices become the stream ids the playback starts with, and Off is `0` — the
    /// state `StartTracks` reads as "the viewer said no", not as "nobody said anything".
    #[test]
    fn the_choice_becomes_the_stream_ids_the_playback_gets() {
        let d = item();
        assert_eq!(audio_sid_at(&d, 0), 1);
        assert_eq!(audio_sid_at(&d, 1), 2);
        assert_eq!(audio_sid_at(&d, 9), 0, "no such track is no pick");
        assert_eq!(subtitle_sid_at(&d, None), 0);
        assert_eq!(subtitle_sid_at(&d, Some(0)), 4);
        assert_eq!(subtitle_sid_at(&d, Some(1)), 5);
        assert_eq!(subtitle_sid_at(&d, Some(9)), 0);
    }

    /// The facts are the item's own record through `tracks_panel`'s formatters — resolution,
    /// codec with the CHOSEN audio beside it, container, size, length and the wall-clock end —
    /// and the end time is the one fact that moves with the clock.
    #[test]
    fn the_facts_state_the_files_own_numbers_and_the_time_it_ends() {
        let d = item();
        let rows = fact_rows(&d, 1, 21 * 60 + 5); // 21:05
        let get = |label: &str| rows.iter().find(|r| r.label == label).map(|r| r.value.as_str());
        assert_eq!(get("Resolution"), Some("3840 \u{d7} 1602"));
        assert_eq!(get("Codec"), Some("HEVC \u{b7} EAC3"), "the chosen audio sits beside the video codec");
        assert_eq!(get("Container"), Some("MKV"));
        assert_eq!(get("Size"), Some("32.28 GB"));
        assert_eq!(get("Duration"), Some("2:40:10"));
        assert_eq!(get("Ends"), Some("23:45"));
        // …and choosing the other audio track changes the codec fact, not the film.
        let other = fact_rows(&d, 0, 21 * 60 + 5);
        assert_eq!(
            other.iter().find(|r| r.label == "Codec").map(|r| r.value.as_str()),
            Some("HEVC \u{b7} AC3")
        );
        // An item with no duration states no end rather than a wrong one.
        let silent = Detail { dur_ms: 0, ..item() };
        let rows = fact_rows(&silent, 0, 0);
        assert!(!rows.iter().any(|r| r.label == "Ends" || r.label == "Duration"));
    }

    /// Genres and credits are one line each, in the server's order: the cast first, then the crew,
    /// each `Name (Role)`.
    #[test]
    fn genres_and_credits_read_as_one_line_in_server_order() {
        let d = item();
        assert_eq!(genre_line(&d), "Drama \u{b7} Science Fiction");
        assert_eq!(
            credits_line(&d),
            "Ada Actor (Lead) \u{b7} Bob Actor (Second) \u{b7} Cy Director (Director)"
        );
        // An item with no genres and no credits draws no line rather than a stray separator.
        let bare = Detail { genres: Vec::new(), cast: Vec::new(), crew: Vec::new(), ..item() };
        assert!(genre_line(&bare).is_empty());
        assert!(credits_line(&bare).is_empty());
    }

    /// The cursor walks the four rows and stops at both ends: a window that wrapped from the
    /// action back to the audio would make one DOWN press skip the whole column. The three-dot
    /// control is the ladder's TOP rung, which is what gives the remote a way to reach it. The
    /// ladder is the ENGINE's (`Focusable::neighbour` answers with this same step), so this walks
    /// the real `FocusEngine` over the real screen rather than a private cursor.
    #[test]
    fn the_cursor_walks_the_rows_and_stops_at_the_ends() {
        use crate::ui::focus::{FocusEngine, Outcome};
        use crate::ui::machine::{FocusKey, GroupId, InputOwner};
        use crate::ui::screen::Dir;

        let mut row = OptionRow::Action;
        assert_eq!(row, OptionRow::Action, "the primary action holds the cursor");
        row = row.stepped(false);
        assert_eq!(row, OptionRow::Subtitles);
        row = row.stepped(false);
        assert_eq!(row, OptionRow::Audio);
        row = row.stepped(false);
        assert_eq!(row, OptionRow::More, "UP from the audio row reaches the menu control");
        assert_eq!(row.stepped(false), row, "clamped at the top");
        row = row.stepped(true);
        assert_eq!(row, OptionRow::Audio);
        row = row.stepped(true);
        row = row.stepped(true);
        assert_eq!(row, OptionRow::Action);
        assert_eq!(row.stepped(true), row, "clamped at the bottom");

        // …and the engine's walk over the screen answers the same ladder, which is what the
        // dispatcher hands a D-pad press and a pointer hover alike.
        let screen = PrePlaybackScreen::new(ENTRY, PrePlaybackArg { restart: false });
        let cx = fixture_cx(Some(OptionRow::Action.index() as u32));
        let owner = InputOwner::Entry(ENTRY);
        let mut engine: FocusEngine<u32> = FocusEngine::new();
        let key = |row: OptionRow| FocusKey { entry: ENTRY, elem: row.index() as u32 };
        engine.set(owner, key(OptionRow::Action), Some(GroupId(0)), crate::ui::screen::By::Restore);
        for want in [OptionRow::Subtitles, OptionRow::Audio, OptionRow::More] {
            let out = engine.move_dir(owner, &screen, &[], Dir::Up, &cx);
            assert_eq!(
                match out { Outcome::Moved { to, .. } => Some(to), _ => None },
                Some(key(want)),
                "UP walks the ladder one rung"
            );
        }
        assert!(
            matches!(engine.move_dir(owner, &screen, &[], Dir::Up, &cx), Outcome::Nothing),
            "the top of the column is a wall, not a wrap"
        );
        assert!(
            matches!(engine.move_dir(owner, &screen, &[], Dir::Left, &cx), Outcome::Nothing),
            "LEFT belongs to the language row's value, so the ladder has no sideways rung"
        );
    }

    /// The synopsis is the one block with no fixed height, so its line budget is what keeps the
    /// column inside the sheet: a full column gets the cap, a tight one gets fewer lines, and a
    /// column with no room left still gets ONE rather than a heading over nothing.
    #[test]
    fn the_synopsis_budget_shrinks_with_the_room_and_never_reaches_zero() {
        let roomy = main_synopsis_lines(900.0, 0.0, false);
        assert_eq!(roomy, SYN_LINES_MAX);
        let tight = main_synopsis_lines(140.0, 0.0, false);
        assert!(tight < roomy, "less room, fewer lines");
        assert!(tight >= 1);
        // Credits below the prose take their own room AND their gap out of the budget.
        let with_credits = main_synopsis_lines(300.0, 96.0, true);
        let without = main_synopsis_lines(300.0, 96.0, false);
        assert!(with_credits <= without, "reserved credits cannot buy lines");
        assert_eq!(main_synopsis_lines(0.0, 96.0, true), 1, "the floor is one line");
    }

    /// The two scores are SEPARATE facts from separate fields, and each is drawn only when the
    /// server sent it: the stars come from the community score, the percentage from the critics'
    /// field, and an item with neither states neither (#368, and #317 for why one number under two
    /// names is refused).
    #[test]
    fn the_scores_are_the_two_the_server_sent_and_neither_is_invented() {
        let plain = item();
        assert_eq!(audience_score(&plain), None);
        assert_eq!(plain.critic_rating, None);

        let scored = Detail {
            ratings: vec![crate::metadata::Rating {
                art: crate::metadata::RatingArt::Community,
                value: 7.4,
                critic: false,
            }],
            critic_rating: Some(85),
            ..item()
        };
        assert_eq!(audience_score(&scored), Some(7.4));
        assert_eq!(scored.critic_rating, Some(85));
        // …and the stars the window draws are that score, at half-star precision.
        assert_eq!(
            crate::ui::widgets::star_fills(audience_score(&scored).unwrap()),
            [1.0, 1.0, 1.0, 0.5, 0.0]
        );
    }

    /// The cursor is seeded ONCE: a refresh landing under an open window must not move the
    /// viewer's choice back to the server's default.
    #[test]
    fn the_seed_is_taken_once_and_never_again() {
        let mut s = PrePlaybackScreen::new(crate::ui::machine::EntryId(1), PrePlaybackArg { restart: false });
        let d = item();
        s.seed(&d);
        assert_eq!(s.audio, 1);
        s.audio = 0;
        s.seed(&d);
        assert_eq!(s.audio, 0, "a second seed would overwrite the viewer's choice");
    }

    // ---- the pointer (#359) --------------------------------------------------------------------

    struct HostFixture;
    impl crate::ui::machine::Host for HostFixture {
        type Arg = crate::ui::fixture::FixtureArg;
        type Fx = crate::screens::registry::AppFx;
        type Msg = crate::screens::registry::AppMsg;
        type Elem = u32;
        type Views<'a> = ();
        type Init = crate::ui::fixture::FixtureArg;
        type Memory = ();
    }

    thread_local! {
        // TEST ONLY, `screens::detail::tests`' pattern: the store has to live somewhere for the
        // free helpers below, and a per-owner one would be a second mechanism for reaching it.
        static TEST_METADATA: std::cell::UnsafeCell<crate::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(crate::stores::metadata::MetadataStore::default());
    }

    fn test_store() -> &'static mut crate::stores::metadata::MetadataStore {
        TEST_METADATA.with(|cell| unsafe { &mut *cell.get() })
    }

    impl crate::screens::registry::MetadataLike for HostFixture {
        fn metadata<'a>(_cx: &crate::ui::machine::Cx<'a, Self>) -> crate::metadata::MetadataView<'a> {
            test_store().view()
        }
    }

    fn install(d: Detail) -> crate::testlock::Serial {
        let guard = crate::testlock::serial();
        crate::metadata::set_current_for_test(test_store().state_mut(), Some(d));
        guard
    }

    fn fixture_cx(focus: Option<u32>) -> crate::ui::machine::Cx<'static, HostFixture> {
        crate::ui::machine::Cx {
            views: (),
            tick: crate::ui::machine::Tick::default(),
            measure: &crate::ui::fixture::FixtureMeasure,
            press: crate::ui::machine::PressRead::default(),
            focus: crate::ui::machine::FocusRead {
                current: focus.map(|elem| crate::ui::machine::FocusKey { entry: ENTRY, elem }),
                ..Default::default()
            },
            owner: crate::ui::machine::InputOwner::Entry(ENTRY),
        }
    }

    const ENTRY: crate::ui::machine::EntryId = crate::ui::machine::EntryId(4);

    fn step(
        screen: &mut PrePlaybackScreen,
        event: &crate::ui::screen::ScreenEvent<HostFixture>,
        focus: Option<u32>,
    ) -> (crate::ui::machine::Handled, Vec<crate::ui::machine::Stamped<HostFixture>>) {
        let context = fixture_cx(focus);
        let mut effects = Vec::new();
        let mut present = crate::ui::present::Present::new();
        let handled = {
            let mut sink = crate::ui::machine::Effects::new(
                &mut effects,
                crate::ui::machine::MachineId::Instance(crate::ui::machine::InstanceId(1)),
                &mut present,
            );
            crate::ui::machine::Machine::<HostFixture>::step(screen, event, &context, &mut sink)
        };
        (handled, effects)
    }

    /// **The pointer's half of rule 11 on this window** (#359): every row is a stop that HOVERS like
    /// a table row does (`Hover::Focus`, the same face the arrows give it) and ACTIVATES like its own
    /// OK does, and each stop's rectangle is the capsule that row is painted in. Before this the
    /// window registered no stop at all, so the Magic Remote's pointer was dead over the whole sheet
    /// and a click on a row fell on the alert's ground.
    #[test]
    fn every_row_is_a_stop_on_the_capsule_it_is_drawn_in() {
        use crate::ui::hit::{HitMap, PointerKind};
        use crate::ui::screen::{Activate, At, DrawFrame, Focusable, Hover};

        let screen = PrePlaybackScreen::new(ENTRY, PrePlaybackArg { restart: false });
        let cx = fixture_cx(None);
        let mut f = DrawFrame::new(&cx, crate::ui::Painter::root());
        screen.record_stops(&mut f);
        let stops = f.into_stops();
        assert_eq!(stops.len(), OptionRow::ALL.len(), "one stop per rung of the ladder");
        for (i, stop) in stops.iter().enumerate() {
            assert_eq!(stop.key.elem, i as u32, "the stop's key is its row");
            assert!(matches!(stop.hover, Hover::Focus), "a row hovers: the pointer parks focus");
            assert!(matches!(stop.activate, Activate::Immediate), "a click runs the row at once");
            let placed = <PrePlaybackScreen as Focusable<HostFixture>>::place(&screen, &(i as u32), &cx, At::Drawn)
                .expect("every row is a focusable element");
            let same = |a: Rect, b: Rect| [a.x, a.y, a.w, a.h] == [b.x, b.y, b.w, b.h];
            assert!(same(placed.rect, stop.rect), "the stop covers the capsule the row is drawn in");
            assert!(same(placed.rect, option_rect(OptionRow::from_index(i))));
        }

        let mut map: HitMap<u32> = HitMap::new();
        map.fill(stops);
        map.swap();
        // Two rows far apart in the column, so a geometry bug that merged them would still show.
        let audio = option_rect(OptionRow::Audio);
        let action = option_rect(OptionRow::Action);
        assert!(audio.y + audio.h <= action.y, "the rows are separate bands");
        assert_eq!(
            map.resolve(Some(ENTRY), PointerKind::Move, audio.cx(), audio.cy(), None).focus.map(|k| k.elem),
            Some(OptionRow::Audio.index() as u32),
            "hovering the audio row parks the audio row"
        );
        assert_eq!(
            map.resolve(Some(ENTRY), PointerKind::Move, action.cx(), action.cy(), None).focus.map(|k| k.elem),
            Some(OptionRow::Action.index() as u32),
            "…and a different row parks a different row"
        );
        // The facts column is not a control: a pointer there parks nothing and clicks nothing.
        let facts = main_rect();
        assert!(map.resolve(Some(ENTRY), PointerKind::Move, facts.cx(), facts.cy(), None).focus.is_none());
        assert!(map.resolve(Some(ENTRY), PointerKind::Click, facts.cx(), facts.cy(), None).activate.is_none());
        // A click on a row asks for that row, with the row's own activation policy.
        let click = map.resolve(Some(ENTRY), PointerKind::Click, action.cx(), action.cy(), None);
        assert_eq!(
            click.activate.map(|(k, act)| (k.elem, act)),
            Some((OptionRow::Action.index() as u32, Activate::Immediate))
        );
    }

    /// A hover that lands on a row IS this window's cursor: the engine's `FocusMoved` moves the row
    /// the pills are drawn lit from, so hover and the arrow keys land the same way, and a click on a
    /// row runs exactly what OK on that row runs.
    #[test]
    fn the_engine_lands_the_row_and_a_click_runs_it() {
        use crate::ui::machine::{Fx, Handled};
        use crate::ui::screen::{By, ScreenEvent};

        let _guard = install(item());
        let mut screen = PrePlaybackScreen::new(ENTRY, PrePlaybackArg { restart: false });
        assert_eq!(screen.row, OptionRow::Action, "the window opens on its primary action");

        // A pointer move over the audio row parks element 1; the window follows it.
        let moved = ScreenEvent::FocusMoved {
            from: Some(crate::ui::machine::FocusKey { entry: ENTRY, elem: 3 }),
            to: crate::ui::machine::FocusKey { entry: ENTRY, elem: 1 },
            by: By::Pointer,
        };
        let (handled, _) = step(&mut screen, &moved, Some(1));
        assert_eq!(handled, Handled::Yes);
        assert_eq!(screen.row, OptionRow::Audio, "the hovered row is the row drawn lit");

        // A click on the primary action starts the film with the tracks on screen.
        let (handled, fx) = step(&mut screen, &ScreenEvent::Activate(3), Some(3));
        assert_eq!(handled, Handled::Yes);
        assert!(
            fx.iter().any(|s| matches!(
                &s.fx,
                Fx::App(crate::screens::registry::AppFx::Content(
                    crate::screens::registry::ContentReq::Play { .. }
                ))
            )),
            "a click on the action row asks for the playback, exactly as its OK does"
        );

        // The container's own chrome is NOT a row: the shared Back control's element must never be
        // clamped into "the primary action", or the way OUT would start the film.
        let crumb = crate::ui::table_screen::CRUMB_ELEM;
        let (handled, fx) = step(&mut screen, &ScreenEvent::Activate(crumb), Some(crumb));
        assert_eq!(handled, Handled::No, "the window does not own the Back control");
        assert!(fx.is_empty(), "a click on the way out starts nothing");
        let stray = ScreenEvent::FocusMoved {
            from: None,
            to: crate::ui::machine::FocusKey { entry: ENTRY, elem: crumb },
            by: By::Pointer,
        };
        let (handled, _) = step(&mut screen, &stray, Some(crumb));
        assert_eq!(handled, Handled::Yes);
        assert_eq!(screen.row, OptionRow::Action, "chrome lighting itself leaves the cursor alone");
    }
}
