//! The **neutral collection tile** — what a collection with no artwork of its own draws in place of
//! a poster: the placeholder ground, the collection mark, and the collection's name under it.
//!
//! A collection whose server sent no `thumb` has nothing to resolve, so without this the tile is a
//! bare skeleton that reads as "still loading" forever and, beside its named neighbours, as a
//! broken image. The name is the one fact that tells two such tiles apart, so it is drawn ON the
//! tile rather than left to the focused caption, which only one tile at a time wears.
//!
//! **One widget for every surface that lists collections.** It is reached through the shared card
//! composite (`widgets::card`'s poster arm hands a thumb-less collection row here), so the Library
//! grid, a Home shelf and Search draw the same tile without asking. It names no application type:
//! the caller decides that a row is a thumb-less collection and passes the name.
//!
//! The name is set by ONE fitting policy, [`fit_name`]: sized, cased and wrapped from the tile's
//! RESTING width and memoised, so a focus pop moves the laid-out lines rather than re-wrapping them.
//! It measures through the live-font `widgets::LegacyMeasure` (`card` is drawn by callers that hold
//! no `Measure`, and the card composite skips recording passes, so no recorded layout depends on
//! this text); tests hand it the shipped faces' real advances.
use crate::ui::icons::{self, Icon};
use crate::ui::label::{HAlign, Label, VAlign};
use crate::ui::machine::Measure;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_int;
use std::rc::Rc;
use crate::ui::{theme, Painter, Rect};

/// The narrowest mock tile that sets the name at full size: C1's 200-wide header. The mock writes
/// the name's type in fixed px (`.art b { font: 700 24px/1.08; left:18px; right:18px;
/// bottom:22px }`) on both its 200 and 250 tiles, so the name is the SAME size on both; only a tile
/// narrower than this scales it down.
const TYPE_FULL_W: f32 = 200.0;

/// How a tile sets a name, in mock px (full size from [`TYPE_FULL_W`] up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NameStyle {
    /// The normal size, every name's first try (never grown to fill a short name).
    pub size: f32,
    /// The one smaller size a name that overflows its band steps down to.
    pub step: f32,
    /// The least a single over-wide word is shrunk to before it is elided.
    pub floor: f32,
    /// Side inset of the text column.
    pub side: f32,
    /// Line pitch over the size (CSS `line-height`).
    pub lead: f32,
    /// Upper-case the name (Unicode-aware; a caseless script passes through).
    pub upper: bool,
    /// Clear air at the top of the name's band, in lines of its own pitch (the fan keeps half a
    /// line between its front member and the block).
    pub gap: f32,
}

/// G1's `.ety div`: `font: 700 24px/1.12`, the neutral tile's 10% column inset, as written.
pub(crate) const NEUTRAL_NAME: NameStyle =
    NameStyle { size: 24.0, step: 20.0, floor: 18.0, side: 25.0, lead: 1.12, upper: false, gap: 0.0 };
/// `bottom:22px` — the least air under the name block, in mock px.
const NAME_BOTTOM: f32 = 22.0;

/// One line of a fitted name: its text and the size it is drawn at (a shrunk over-wide word is
/// smaller than its block).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FittedLine {
    pub text: String,
    pub sz: c_int,
}

/// A name set for one resting tile: its lines, the block's size, and its line pitch.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FittedName {
    pub lines: Vec<FittedLine>,
    pub sz: c_int,
    pub pitch: f32,
    /// The text column's width.
    pub column: f32,
}

impl FittedName {
    /// The line boxes' height (CSS: lines × line-height).
    pub(crate) fn height(&self) -> f32 {
        self.lines.len() as f32 * self.pitch
    }

    /// The INK's height: line 0's cap top to the last line's baseline.
    pub(crate) fn ink(&self, m: &dyn Measure) -> f32 {
        if self.lines.is_empty() { return 0.0; }
        (self.lines.len() - 1) as f32 * self.pitch + m.cap_h(self.sz)
    }
}

/// The name's type scale on a tile `rest_w` wide at rest: 1 from [`TYPE_FULL_W`] up.
fn type_k(rest_w: f32) -> f32 {
    (rest_w / TYPE_FULL_W).min(1.0)
}

/// Greedy word wrap at `w`: a word wider than `w` takes a line of its own, unsplit.
fn greedy(words: &[&str], w: f32, sz: c_int, m: &dyn Measure) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in words {
        if cur.is_empty() {
            cur.push_str(word);
            continue;
        }
        let trial = format!("{cur} {word}");
        if m.width_str(&trial, sz, true) <= w {
            cur = trial;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// `text-wrap: balance`: the narrowest whole-pixel column that keeps greedy's line count, so no
/// lone orphan word hangs under a full line when a more even break exists.
fn balanced(words: &[&str], column: f32, sz: c_int, m: &dyn Measure) -> Vec<String> {
    let lines = greedy(words, column, sz, m);
    if lines.len() < 2 {
        return lines;
    }
    let widest = words.iter().map(|w| m.width_str(w, sz, true)).fold(0.0f32, f32::max);
    let (mut lo, mut hi) = (widest.ceil() - 1.0, column.floor());
    while hi - lo > 1.0 {
        let mid = ((lo + hi) * 0.5).floor();
        if greedy(words, mid, sz, m).len() <= lines.len() { hi = mid } else { lo = mid }
    }
    greedy(words, hi, sz, m)
}

/// Elide `text` to `w` at `sz` with a trailing ellipsis.
fn elide(text: &str, w: f32, sz: c_int, m: &dyn Measure) -> String {
    crate::text::elide_by(text, w, false, |t| m.width_str(t, sz, true))
}

/// Set line `text` at `sz`, or — a single word wider than the column — shrink it toward `floor`
/// and elide what still does not fit. Never wider than the column.
fn set_line(text: String, sz: c_int, floor: c_int, column: f32, m: &dyn Measure) -> FittedLine {
    let w = m.width_str(&text, sz, true);
    if w <= column {
        return FittedLine { text, sz };
    }
    let mut small = sz;
    while small > floor && m.width_str(&text, small, true) > column {
        small -= 1;
    }
    let text = if m.width_str(&text, small, true) > column { elide(&text, column, small, m) } else { text };
    FittedLine { text, sz: small }
}

/// **The one name-fitting policy** of the neutral tile, for a tile `rest_w` wide at rest whose
/// name may use a band `band` tall:
///
/// 1. the type is the mock's fixed px (24/1.08, 18px side insets) — the same on a 200 and a 250
///    tile, as the mock sets it — scaled down only on a tile narrower than [`TYPE_FULL_W`];
/// 2. a name that fits on one line stays one centred line at the NORMAL size — never grown;
/// 3. a longer one is balance-wrapped (`text-wrap: balance`) into as many lines as the band holds
///    at that size — its ink plus the style's clear `gap` above and below — with no fixed line cap;
/// 4. only if that overflows the band: the ONE smaller size, again as many lines as fit;
/// 5. only if that still overflows: the last line that fits ends in an ellipsis;
/// 6. a single word wider than the column shrinks toward the style's floor, then elides;
/// 7. upper-case is Unicode's (`be` Cyrillic works; a caseless script passes through);
/// 8. an empty or blank name sets nothing. A " Collection" suffix is kept.
pub(crate) fn fit_name(name: &str, style: &NameStyle, rest_w: f32, band: f32,
    m: &dyn Measure) -> FittedName {
    let k = type_k(rest_w);
    let px = |v: f32| (v * k).round().max(1.0) as c_int;
    let (normal, step, floor) = (px(style.size), px(style.step), px(style.floor).min(px(style.step)));
    let column = (rest_w - 2.0 * style.side * k).max(1.0).floor();
    let text = if style.upper { name.to_uppercase() } else { name.to_owned() };
    let words: Vec<&str> = text.split_whitespace().collect();
    let pitch = |sz: c_int| sz as f32 * style.lead;
    // Lines that fit: the block's INK (line 0's cap top to the last baseline) plus the clear gap
    // above AND below it — it is centred — within the band.
    let room = |sz: c_int| {
        let spare = band - 2.0 * style.gap * pitch(sz) - m.cap_h(sz);
        ((spare / pitch(sz)).floor().max(0.0) as usize + 1).max(1)
    };
    if words.is_empty() {
        return FittedName { lines: Vec::new(), sz: normal, pitch: pitch(normal), column };
    }
    for sz in [normal, step] {
        if greedy(&words, column, sz, m).len() <= room(sz) {
            let lines = balanced(&words, column, sz, m).into_iter()
                .map(|l| set_line(l, sz, floor, column, m)).collect();
            return FittedName { lines, sz, pitch: pitch(sz), column };
        }
    }
    let mut wrapped = greedy(&words, column, step, m);
    let rest = wrapped.split_off(room(step) - 1).join(" ");
    let last = elide(&rest, column, step, m);
    wrapped.push(if last.is_empty() { "\u{2026}".into() } else { last });
    let lines = wrapped.into_iter().map(|l| set_line(l, step, floor, column, m)).collect();
    FittedName { lines, sz: step, pitch: pitch(step), column }
}

/// The band the neutral tile's name may use: the tile less the mark, its gap and the mock's inset
/// above and below.
fn neutral_band(rest: Rect) -> f32 {
    (rest.h - glyph_px(rest.w) - rest.w * GAP_RATIO - 2.0 * NAME_BOTTOM * type_k(rest.w)).max(0.0)
}

thread_local! {
    /// Per (name, style, resting size): the fitted name. Settled once per resting width, so a frame
    /// pays one lookup per tile and a focus pop never re-wraps.
    static FITTED: RefCell<HashMap<u64, Rc<FittedName>>> = RefCell::new(HashMap::new());
}
/// A bound on [`FITTED`]; past it the memo starts over (a library's collections are far fewer).
const FITTED_CAP: usize = 512;

fn fitted(name: &str, style: &NameStyle, rest: Rect, band: f32) -> Rc<FittedName> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (name, style.upper, style.side.to_bits(), style.lead.to_bits(), rest.w.to_bits(),
        rest.h.to_bits(), band.to_bits()).hash(&mut h);
    let id = h.finish();
    if let Some(hit) = FITTED.with(|m| m.borrow().get(&id).cloned()) {
        return hit;
    }
    let fit = Rc::new(fit_name(name, style, rest.w, band, &crate::ui::widgets::LegacyMeasure));
    FITTED.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= FITTED_CAP {
            m.clear();
        }
        m.insert(id, Rc::clone(&fit));
    });
    fit
}

/// Draw `fit`'s lines centred on `cx`, line 0's cap top at `top`; a shrunk line shares its block
/// line's baseline.
fn draw_lines(p: Painter, fit: &FittedName, cx: f32, top: f32, col: [f32; 4]) {
    let cap = crate::ui::machine::Measure::cap_h(&crate::ui::widgets::LegacyMeasure, fit.sz);
    for (i, line) in fit.lines.iter().enumerate() {
        let Ok(text) = CString::new(line.text.as_str()) else { continue };
        let baseline = top + i as f32 * fit.pitch + cap;
        Label::new(text.as_ptr(), line.sz, col).bold().h(HAlign::Center).v(VAlign::Baseline)
            .draw(p, Rect::new(cx - fit.column * 0.5, baseline, fit.column, 0.0));
    }
}

/// The mark's size as a fraction of the tile's width.
const GLYPH_RATIO: f32 = 0.3;
/// Air between the mark and the name's cap band, as a fraction of the tile's width.
const GAP_RATIO: f32 = 0.08;

/// The mark's raster size for a tile `w` wide — quantized to 4px so a focus pop reuses a handful
/// of cached masks instead of rasterizing one per rounded pixel (the person glyph's discipline).
pub(crate) fn glyph_px(w: f32) -> f32 {
    ((w * GLYPH_RATIO) / 4.0).round() * 4.0
}

/// Where the mark and the name block sit in `r`: the pair is centred vertically as one block,
/// the mark above, sized from the tile so a scaled (popped) tile scales its layout with it.
pub(crate) fn layout(r: Rect, name_w: f32, name_h: f32) -> (Rect, Rect) {
    let d = glyph_px(r.w);
    let gap = r.w * GAP_RATIO;
    let top = r.y + (r.h - (d + gap + name_h)) * 0.5;
    let glyph = Rect::new(r.cx() - d * 0.5, top, d, d);
    let name = Rect::new(r.cx() - name_w * 0.5, top + d + gap, name_w, name_h);
    (glyph, name)
}

/// Where CSS puts the cap top inside a ~1.1 line box, in em: the half-leading plus the font's
/// ascent-over-cap-height, about 0.15.
const NAME_CAP_DROP: f32 = 0.15;

/// Draw the neutral tile at `r` (the rect it is drawn at, popped when focused) with corner radius
/// `rad`; `rest` is its RESTING rect, which the name is fitted to. The caller has already decided
/// the row is a collection with no artwork (`thumb` empty) — an unresolved texture with a path
/// behind it is merely still loading and keeps the ordinary skeleton.
pub(crate) fn draw(p: Painter, rest: Rect, r: Rect, rad: f32, name: &str) {
    p.rrect_sheened(r, rad, theme::CARD_PLACEHOLDER);
    let fit = fitted(name, &NEUTRAL_NAME, rest, neutral_band(rest));
    let (glyph, column) = layout(r, fit.column, fit.height());
    icons::draw(p, Icon::Collection, glyph, theme::TEXT_TERTIARY);
    draw_lines(p, &fit, column.cx(), column.y + NAME_CAP_DROP * fit.sz as f32, theme::TEXT_SECONDARY);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fontcov::advances::ShippedMeasure;

    fn tile(w: f32) -> Rect { Rect::new(0.0, 0.0, w, w * 1.5) }
    fn fit(name: &str, w: f32) -> FittedName {
        fit_name(name, &NEUTRAL_NAME, w, neutral_band(tile(w)), &ShippedMeasure)
    }
    fn texts(f: &FittedName) -> Vec<&str> { f.lines.iter().map(|l| l.text.as_str()).collect() }
    fn fits_column(f: &FittedName) -> bool {
        f.lines.iter().all(|l| ShippedMeasure.width_str(&l.text, l.sz, true) <= f.column)
    }
    fn elided(f: &FittedName) -> bool { f.lines.iter().any(|l| l.text.ends_with('\u{2026}')) }

    #[test]
    fn the_mark_and_name_are_one_block_centred_on_the_tile() {
        let r = Rect::new(100.0, 200.0, 250.0, 375.0);
        let (glyph, name) = layout(r, 200.0, 64.0);
        assert_eq!(glyph.w, 76.0, "a grid poster's mark quantizes to 76px");
        assert!((glyph.cx() - r.cx()).abs() < 0.01);
        let top_air = glyph.y - r.y;
        let bottom_air = r.y + r.h - (name.y + name.h);
        assert!((top_air - bottom_air).abs() < 0.01, "the block is centred: {top_air} vs {bottom_air}");
        assert!(name.y > glyph.y + glyph.h, "the name sits under the mark");
    }

    #[test]
    fn a_pop_reuses_quantized_mark_sizes() {
        let sizes: std::collections::BTreeSet<i32> = (0..=20)
            .map(|step| glyph_px(250.0 * (1.0 + step as f32 * 0.004)) as i32)
            .collect();
        assert!(sizes.len() <= 2, "a 1.08 pop spans at most two cached masks: {sizes:?}");
        assert!(sizes.iter().all(|px| px % 4 == 0));
    }

    /// The mock's type is fixed px and only a tile narrower than the header scales it down.
    #[test]
    fn the_name_is_the_mocks_fixed_type() {
        for w in [200.0, 250.0] {
            let f = fit("Up", w);
            assert_eq!(f.sz, 24, "{w}");
        }
        assert_eq!(fit("Up", 150.0).sz, 18, "a tile narrower than the header scales down");
    }

    /// A typical name sets in BALANCED lines: no narrower column keeps the line count.
    #[test]
    fn a_typical_name_balances_its_lines() {
        let f = fit("Harbor Lights Mysteries", 250.0);
        assert_eq!(f.lines.len(), 2, "{:?}", texts(&f));
        assert!(!elided(&f));
        assert!(fits_column(&f));
        let widths: Vec<f32> = f.lines.iter().map(|l| ShippedMeasure.width_str(&l.text, l.sz, true)).collect();
        let greedy = greedy(&["Harbor", "Lights", "Mysteries"], f.column, f.sz, &ShippedMeasure);
        let greedy_max = greedy.iter().map(|l| ShippedMeasure.width_str(l, f.sz, true)).fold(0.0, f32::max);
        assert!(widths.iter().cloned().fold(0.0, f32::max) <= greedy_max, "balanced is never wider than greedy");
    }

    /// A long name uses every line the band holds — there is no line cap — and only then ends the
    /// last line that fits in an ellipsis, after the one step down. The band is sized so the
    /// normal size cannot hold the name but the step-down can hold most of it.
    #[test]
    fn an_overflowing_name_steps_down_once_then_elides() {
        let long = "The Complete and Utterly Definitive Chronological Anthology of Every Starfall Film Ever Made In Any Format";
        let band = 3.0 * 24.0 * NEUTRAL_NAME.lead;
        let f = fit_name(long, &NEUTRAL_NAME, 250.0, band, &ShippedMeasure);
        assert_eq!(f.sz, 20);
        assert!(f.lines.last().unwrap().text.ends_with('\u{2026}'), "{:?}", texts(&f));
        assert!(fits_column(&f));
    }

    /// A single word wider than the column shrinks toward the floor, then elides — never wider.
    #[test]
    fn an_overwide_word_shrinks_to_the_floor_then_elides() {
        let f = fit("Donaudampfschifffahrtsgesellschaft", 250.0);
        assert_eq!(f.lines.len(), 1);
        assert!(f.lines[0].sz < f.sz && f.lines[0].sz >= 18, "{:?}", f.lines);
        assert!(fits_column(&f));
        let f = fit("Rindfleischetikettierungsüberwachungsaufgabenübertragungsgesetz", 250.0);
        assert_eq!(f.lines[0].sz, 18);
        assert!(f.lines[0].text.ends_with('\u{2026}') && fits_column(&f));
    }

    #[test]
    fn a_blank_name_sets_nothing() {
        for w in [200.0, 250.0] {
            assert!(fit("", w).lines.is_empty());
            assert!(fit("  \t ", w).lines.is_empty());
        }
    }

    /// Unicode casing: the neutral tile keeps the name's own case, and a Cyrillic name fits.
    #[test]
    fn a_cyrillic_name_is_fitted_and_a_latin_one_keeps_its_case() {
        for w in [200.0, 250.0] {
            let f = fit("Калекцыя Зорнага Падзення", w);
            assert!(texts(&f).join(" ") == "Калекцыя Зорнага Падзення", "{w}: {:?}", texts(&f));
            assert!(fits_column(&f));
            let kept = fit("Starfall Saga Collection", w);
            assert_eq!(texts(&kept).join(" "), "Starfall Saga Collection");
        }
    }
}
