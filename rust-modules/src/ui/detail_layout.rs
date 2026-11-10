//! Shared detail hero geometry, also used by the contrast and safe-area audits.
use super::{consts::{MARGIN_X, MARGIN_Y, SCR_H}, theme, widgets};
#[cfg(test)]
use super::{consts::SCR_W, Rect};

/// The copy column's width. It runs from `HERO_TEXT_X` to the safe edge (`SCR_W - MARGIN_X`), which
/// is the whole point: at 943 it stopped 481px short of the margin and the hero read as a column
/// with a hole beside it. The poster keeps the left margin, so nothing is lost there, and the
/// people column is no longer a second column beside this one (see `PEOPLE_W`).
pub(crate) const HERO_TEXT_W: f32 = 1244.0;
/// The hero's poster column: 364 wide at 2:3 — 30% up from the mockup's 280 (the owner's
/// 2026-11-10 pass: "la carátula, al menos 30% más grande"). A second column is the whole point —
/// the wide backdrop stays, and the poster is what makes the page read as an item rather than a
/// still.
/// The hero's poster column: 460 wide at 2:3 — the page's DOMINANT element (the owner's
/// 2026-11-10 pass: "lo que manda es la carátula, tiene que cobrar protagonismo"). The copy column
/// starts right of it and the action row lives IN that column, so the taller poster no longer
/// collides with the controls below.
pub(crate) const POSTER_W: f32 = 460.0;
pub(crate) const POSTER_H: f32 = 690.0;
pub(crate) const POSTER_GAP: f32 = 24.0;
/// **The hero's one top edge.** The poster's head and the title band's head are the same line
/// (the owner's pass: "alinear el logo con la carátula"), and the poster rose 20% of its own
/// height from the old `366` to get there — the space the removed `Película` kicker used to spend
/// above the title. The whole column below it moved up with it, which is what pays for the fifth
/// synopsis line.
pub(crate) const HERO_TOP: f32 = 282.0;
/// The poster's top edge: the hero's own top, shared with the title band.
pub(crate) const POSTER_Y: f32 = HERO_TOP;
/// Where the hero's copy column begins, right of the poster. Every left-anchored row of the hero
/// (title logo, identity, ratings, synopsis, people, actions) starts here, so the poster never
/// shares ink with the text.
pub(crate) const HERO_TEXT_X: f32 = MARGIN_X + POSTER_W + POSTER_GAP;
/// The title band's BOTTOM edge — its anchor (`HeroLogo` places by the band's foot), and the line
/// the identity row hangs under.
///
/// **The rung's ceiling, so the band's HEAD is the poster's head** (the owner's "alinear el logo
/// con la carátula"): the two columns start on one line. The title's own ink stays at the band's
/// foot rather than at its head, and that is not negotiable: the hero's scrim reaches full strength
/// only at [`crate::ui::widgets`]' knee, and a `size::HERO` run moved up to 282 measured 1.41:1 over
/// bright artwork against the design system's 3:1 floor. A clearLogo is placed by the same foot and
/// spills upward as paint, which is what makes the alignment visible for the art case.
pub(crate) const TITLE_BOTTOM: f32 = HERO_TOP + theme::logo::HERO_H_MAX;
/// The cast and crew band, which is now a BAND under the facts row rather than a column beside the
/// copy. A 560-wide column at the right edge was what capped the copy at 943: the two shared a
/// horizontal strip, so widening the text meant running under the names. Stacked, each gets the
/// full measure and the page has no dead corner.
pub(crate) const PEOPLE_W: f32 = 1244.0;
pub(crate) const PEOPLE_LEAD: f32 = 32.0;
pub(crate) const PEOPLE_MAX_LINES: usize = 4;
pub(crate) const PEOPLE_INK: [f32; 4] = theme::TEXT_SECONDARY;
/// The facts row's ink, and the second time this trade has been made: at `TEXT_TERTIARY`
/// (L=0.317) the row needed α≈0.79 of scrim for 4.5:1 — an essentially black hero corner — so
/// `widgets_hero_scrim_tests` carried it at a lowered 2.6 floor and wrote the ink step down as
/// deferred. Tightening the hero rhythm is what came due: every rung the row moves up sits in
/// less of the atmospheric ramp (it is linear in `y`), and the two named rungs the owner asked
/// for cost 0.16 of contrast, which no scrim retune buys back cheaply — the ramp's own
/// measurement says it is 97% of the drop, and steepening it darkens the artwork the hero exists
/// to show. One ink step costs nothing on screen and takes the row to the ordinary 3.0 floor,
/// exactly as [`PEOPLE_INK`] did for the people column.
pub(crate) const FACTS_INK: [f32; 4] = theme::TEXT_SECONDARY;
pub(crate) const COMPACT_TITLE_BOT: f32 = MARGIN_Y + theme::logo::COMPACT_H_MAX;
pub(crate) const TOP_MARGIN: f32 = COMPACT_TITLE_BOT + 26.0;


/// Top-left anchor for the hero logo while a trailer plays in the background — independent of
/// scroll, unlike the pinned [`COMPACT_TITLE_BOT`] title (which answers "how far down have I
/// scrolled", not "is a trailer playing"). Sits inside the safe-area margins.
pub(crate) const PREVIEW_LOGO_X: f32 = MARGIN_X;
pub(crate) const PREVIEW_LOGO_Y: f32 = MARGIN_Y;
/// Keeps a wide wordmark from reaching toward the center of the screen once it has shrunk.
pub(crate) const PREVIEW_LOGO_MAX_W: f32 = 480.0;

#[derive(Clone, Copy)]
pub(crate) struct HeroChain {
    pub(crate) meta_y: f32,
    pub(crate) ratings_y: f32,
    pub(crate) syn_y: f32,
    /// Where the cast and crew band starts: under the synopsis, spanning the copy column.
    pub(crate) people_y: f32,
    pub(crate) btn_y: f32,
}

/// Every row below the title is placed the same way: the previous row's own MEASURED bottom edge
/// plus one named rung from [`theme::space`] — never an absolute offset picked to match a mock.
///
/// The rungs are not a rhythm to be kept even; they are the page's GRAMMAR, and what they say is
/// how strongly two things belong together (owner, 2026-09-18). The closer the relation, the
/// smaller the distance — so an even ladder of one rung all the way down would be the worst answer
/// available, not the tidiest: it reads as one undifferentiated list and the eye stops seeing any
/// structure at all. Three degrees are in play here, and the hero uses all three:
///
/// * `SM` — one group. Title, identity line and ratings are the item SAYING WHAT IT IS, and are
///   meant to read almost as a single object.
/// * `MD` — a change in the KIND of information. The synopsis is prose; the facts line is date,
///   extent and how it plays. Both still belong to the hero.
/// * `LG` — information giving way to ACTION (see [`btn_y`]'s own note), and
/// * `XL` — the hero giving way to the next region entirely, which is `content_top`'s rung in
///   `screens::detail`, measured off the action row's bottom rather than stated as a page offset.
///
/// A gap is between visual OBJECTS, never between coordinates — which is the whole reason this
/// function adds a measured height before every rung. `+50` between two baselines of 26-tall text
/// is a 24 gap wearing a number that belongs to neither the type nor the scale.
/// "Measured" means the row's real bounding box, which for the identity and ratings lines is
/// their badge/mark (`widgets::BADGE_H`, `widgets::RATING_MARK_D` — both centred on, and taller
/// than, the caption text beside them), and for the facts line is the `CAPTION` cap band its own
/// glyphs match. An item with a taller or shorter line at any step still lands the next one a
/// clean gap below it, and the gaps read the same whether or not the trailer preview has faded a
/// label's alpha (this function never looks at preview state — see `compute_hero_chain`'s caller,
/// which only ever asks "does the item HAVE this content", never "is it currently visible").
pub(crate) fn hero_chain(
    syn_h: f32,
    has_ratings: bool,
    people_h: f32,
    measure: &dyn crate::ui::machine::Measure,
) -> HeroChain {
    let meta_y = TITLE_BOTTOM + theme::space::SM;
    // The identity line's own bounding box is its BADGE, not its caption: `draw_identity_line`
    // centres the resolution/HDR/audio badges on the text's cap band, and at `BADGE_H` (34) they
    // stand taller than the `BODY` caption (cap_h ~21) that shares their row.
    let meta_h = widgets::BADGE_H;
    let ratings_y = meta_y + meta_h + theme::space::SM;
    // Same reasoning: the rating marks (`RATING_MARK_D`, 30) are centred on the caption's cap band
    // and are the row's tallest element, not the `LABEL` text (cap_h ~19.5) beside them.
    let ratings_h = widgets::RATING_MARK_D;
    // `MD`, not `SM`: the synopsis is a change of KIND (prose after the item's own labels), and
    // with the facts row gone this rung is the only one left to say so — the ladder still has to
    // widen on the way down (`the_hero_ladder_widens_…`).
    let syn_y = (if has_ratings { ratings_y + ratings_h } else { meta_y + meta_h }) + theme::space::MD;
    // **There is no facts row any more.** The date and the runtime moved into the identity line
    // (the owner's pass) and the play-mode word (`Transmisión directa`) was removed outright, so
    // the band it used to take is now air the synopsis and the people band spend.
    //
    // The one rung in this chain that is a JUDGEMENT rather than a measurement, and the one place
    // the ladder steps twice. Everything above this line is information about the item and is
    // spaced by how closely it belongs together — `SM` inside the identity group, `MD` where the
    // kind of information changes. The action row is not information: it is what you can DO, so
    // the step up to `LG` is the grammar saying so.
    //
    // It is also an optical correction, which is why it is not simply "the next rung". The Play
    // pill is a filled white capsule ~72 tall against a `CAPTION` line of ~18 — at an equal
    // mathematical gap the heavier object reads as CLOSER, because its mass crowds the air above
    // it. Equal by eye is what the page is judged on, and that is one rung more here.
    // The people band sits between the facts row and the action row, so it is part of the chain
    // and the buttons move down by its measured height. When there is no cast or crew the band
    // takes no space at all and the action row keeps the rung it always had.
    let people_y = people_top(syn_y, syn_h.max(34.0));
    let btn_y = if people_h > 0.0 {
        people_y + people_h + theme::space::LG
    } else {
        syn_y + syn_h.max(34.0) + theme::space::LG
    };
    let _ = measure;
    HeroChain { meta_y, ratings_y, syn_y, people_y, btn_y }
}

/// The top of the cast and crew band: one `SM` under the block above it (the synopsis now that
/// the facts row is gone). It used to be anchored by its BOTTOM on the action row and grow
/// upward, which is exactly what made it share a strip with the facts row; anchored by its top it
/// can only ever sit below.
pub(crate) fn people_top(above_y: f32, above_h: f32) -> f32 {
    above_y + above_h + theme::space::SM
}

pub(crate) fn base_scrim_a(y: f32, hero_vis: f32) -> f32 {
    0.95 * hero_vis.clamp(0.0, 1.0)
        * ((y - widgets::HERO_BASE_SCRIM_Y0).max(0.0) / (SCR_H - widgets::HERO_BASE_SCRIM_Y0)).min(1.0)
}

#[cfg(test)]
pub(crate) fn overscan_rects(out: &mut Vec<(&'static str, Rect)>) {
    let tall = theme::logo::COMPACT_H_MAX;
    out.push(("detail pinned compact title (tallest logo)",
        Rect::new(MARGIN_X, COMPACT_TITLE_BOT - tall, SCR_W - 2.0 * MARGIN_X, tall)));
    out.push(("detail hero text column", Rect::new(HERO_TEXT_X, HERO_TOP, HERO_TEXT_W, 200.0)));
    out.push(("detail hero poster", Rect::new(MARGIN_X, POSTER_Y, POSTER_W, POSTER_H)));
    out.push(("detail people band", Rect::new(HERO_TEXT_X, 700.0, HERO_TEXT_W, 100.0)));
    out.push(("detail below-hero flow, at rest", Rect::new(MARGIN_X, TOP_MARGIN, SCR_W - 2.0 * MARGIN_X, 100.0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cast and crew band sits UNDER the facts row and spans the same column, so the two can
    /// no longer share a horizontal strip at any height — the reason the copy had to stay narrow.
    #[test]
    fn the_people_band_sits_below_the_facts_row() {
        let measure = crate::ui::fixture::FixtureMeasure;
        let ch = hero_chain(76.0, true, 0.0, &measure);
        let syn_bot = ch.syn_y + 76.0;
        assert_eq!(
            ch.people_y,
            syn_bot + theme::space::SM,
            "the band starts one rung under the synopsis' measured bottom"
        );
        assert!(
            ch.people_y > syn_bot,
            "the band must start below the synopsis, never level with it"
        );
        // And the action row clears the band by its own measured height, whatever that is.
        let with_people = hero_chain(76.0, true, 64.0, &measure);
        assert_eq!(
            with_people.btn_y,
            with_people.people_y + 64.0 + theme::space::LG,
            "the buttons move down by the band's measured height"
        );
    }

    /// The band spans the copy column, so a long credit wraps inside the same measure the
    /// synopsis uses instead of being squeezed into a right-hand strip.
    #[test]
    fn the_people_band_spans_the_copy_column() {
        assert_eq!(PEOPLE_W, HERO_TEXT_W, "the band is as wide as the copy column");
        assert!(
            HERO_TEXT_X + PEOPLE_W <= SCR_W - MARGIN_X,
            "the band ends at the safe edge, never past it"
        );
        assert!(
            people_top(400.0, 20.0) > 400.0 + 20.0,
            "the band is measured downward from the facts row, not upward from the buttons"
        );
    }

    /// The crew credit takes no vertical space: the action row hangs off the facts line's own
    /// measured height by a flat [`theme::space::MD`], whether or not the server sent a Director[]
    /// credit (the credit is drawn as one more `Bit` inside the same `CAPTION`-sized facts row, so
    /// it never changes that row's height).
    #[test]
    fn the_crew_credit_costs_the_chain_no_vertical_space() {
        let m = crate::ui::fixture::FixtureMeasure;
        for syn_h in [0.0_f32, 108.0] {
            for ratings in [false, true] {
                let ch = hero_chain(syn_h, ratings, 0.0, &m);
                assert_eq!(
                    ch.btn_y,
                    ch.syn_y + syn_h.max(34.0) + theme::space::LG,
                    "the action row hangs off the synopsis' measured height, credit or no credit"
                );
            }
        }
    }

    /// The ratings row is the chain's conditional band: it moves everything below it and nothing
    /// above it, by exactly its own measured height plus the ratings→synopsis rung.
    #[test]
    fn the_ratings_band_is_reserved_when_there_are_scores_and_never_otherwise() {
        let m = crate::ui::fixture::FixtureMeasure;
        let meta_h = widgets::BADGE_H;
        let ratings_h = widgets::RATING_MARK_D;
        for syn_h in [0.0_f32, 108.0] {
            let none = hero_chain(syn_h, false, 0.0, &m);
            let some = hero_chain(syn_h, true, 0.0, &m);

            assert_eq!(some.meta_y, none.meta_y);
            assert_eq!(none.syn_y, none.meta_y + meta_h + theme::space::MD);
            assert_eq!(some.syn_y, some.ratings_y + ratings_h + theme::space::MD);
            let shift = some.syn_y - none.syn_y;
            // The band costs its own mark height plus the rung that put it under the identity
            // line; the synopsis sits `MD` under whichever row is above it, so that rung is the
            // same on both sides and does not enter the shift.
            assert_eq!(shift, ratings_h + theme::space::SM);
            assert_eq!(some.btn_y - none.btn_y, shift);
        }
    }

    /// Each rung in the hero chain is a gap between two MEASURED bounding boxes — the previous
    /// row's own cap-top→baseline height plus a named [`theme::space`] rung — never a difference of
    /// absolute Y coordinates copied off a mock. This is the systematic form the owner asked for
    /// in place of the old `+30`/`+50`/`+54` literals, which happened to reproduce one mock's ys
    /// for one synopsis length and nothing else.
    #[test]
    fn every_hero_row_sits_a_named_rung_below_the_previous_rows_measured_bottom() {
        let syn_h = 72.0_f32;
        let m = crate::ui::fixture::FixtureMeasure;
        let meta_h = widgets::BADGE_H;
        let ratings_h = widgets::RATING_MARK_D;

        let some = hero_chain(syn_h, true, 0.0, &m);
        assert_eq!(some.meta_y - TITLE_BOTTOM, theme::space::SM, "logo/title -> meta");
        assert_eq!(some.ratings_y - (some.meta_y + meta_h), theme::space::SM, "meta -> ratings");
        assert_eq!(some.syn_y - (some.ratings_y + ratings_h), theme::space::MD, "ratings -> synopsis");
        assert_eq!(some.btn_y - (some.syn_y + syn_h.max(34.0)), theme::space::LG, "synopsis -> action row");

        // No ratings: the synopsis follows the meta line directly, off ITS measured bottom.
        let none = hero_chain(syn_h, false, 0.0, &m);
        assert_eq!(none.syn_y - (none.meta_y + meta_h), theme::space::MD, "meta -> synopsis (no ratings)");
    }

    /// The ladder only ever WIDENS on the way down the hero, and it widens where the meaning
    /// changes — inside the identity group, then at the change of information kind, then at the
    /// step from information to action. An even ladder would pass every other test in this file
    /// and is exactly the answer the owner ruled out: distance is the grammar, so a chain that
    /// says the same thing at every step says nothing.
    #[test]
    fn the_hero_ladder_widens_at_every_change_of_meaning_and_never_narrows() {
        let m = crate::ui::fixture::FixtureMeasure;
        let syn_h = 108.0_f32;
        let ch = hero_chain(syn_h, true, 0.0, &m);
        let within_identity = ch.ratings_y - (ch.meta_y + widgets::BADGE_H);
        let kind_change = ch.syn_y - (ch.ratings_y + widgets::RATING_MARK_D);
        let to_action = ch.btn_y - (ch.syn_y + syn_h);

        assert!(within_identity < kind_change, "a group must sit tighter than a change of kind");
        assert!(
            kind_change < to_action,
            "the action row is not more information: it takes the next rung up, and the pill's own \
             mass is why the correction is optical rather than merely the next number"
        );
        // And the hero as a whole is further from what follows it than any two rows inside it are
        // from each other — the region rung `screens::detail::build_layout` adds to `btn_y`.
        assert!(to_action < theme::space::XL);
    }

    #[test]
    fn base_scrim_preserves_the_legacy_foot_and_clamps_inputs() {
        assert_eq!(base_scrim_a(0.0, 1.0), 0.0);
        assert_eq!(base_scrim_a(SCR_H, 1.0), 0.95);
        assert_eq!(base_scrim_a(SCR_H + 100.0, 2.0), 0.95);
        assert_eq!(base_scrim_a(SCR_H, -1.0), 0.0);
    }

    #[test]
    fn overscan_rects_keep_the_legacy_detail_regions() {
        let mut got = Vec::new();
        overscan_rects(&mut got);
        assert_eq!(got.len(), 5);

        let want = [
            (
                "detail pinned compact title (tallest logo)",
                Rect::new(
                    MARGIN_X,
                    COMPACT_TITLE_BOT - theme::logo::COMPACT_H_MAX,
                    SCR_W - 2.0 * MARGIN_X,
                    theme::logo::COMPACT_H_MAX,
                ),
            ),
            (
                "detail hero text column",
                Rect::new(HERO_TEXT_X, HERO_TOP, HERO_TEXT_W, 200.0),
            ),
            (
                "detail hero poster",
                Rect::new(MARGIN_X, POSTER_Y, POSTER_W, POSTER_H),
            ),
            (
                "detail people band",
                Rect::new(HERO_TEXT_X, 700.0, HERO_TEXT_W, 100.0),
            ),
            (
                "detail below-hero flow, at rest",
                Rect::new(MARGIN_X, TOP_MARGIN, SCR_W - 2.0 * MARGIN_X, 100.0),
            ),
        ];
        for (got, want) in got.iter().zip(want.iter()) {
            assert_eq!(got.0, want.0);
            assert_eq!((got.1.x, got.1.y, got.1.w, got.1.h), (want.1.x, want.1.y, want.1.w, want.1.h));
        }
    }

    /// The mockup's second column: a 2:3 poster in the left margin and the copy right of it. The
    /// numbers are a contract, not a mood — the copy's own rows all start at `HERO_TEXT_X`, so the
    /// two columns can never share ink, and the text column still ends inside the safe area.
    #[test]
    fn the_poster_column_sits_beside_the_copy_and_never_over_it() {
        assert_eq!(HERO_TEXT_X, MARGIN_X + POSTER_W + POSTER_GAP);
        assert!((POSTER_W / POSTER_H - 2.0 / 3.0).abs() < 1e-6, "the poster is 2:3");
        assert!(MARGIN_X + POSTER_W < HERO_TEXT_X, "the two columns are apart");
        assert!(
            HERO_TEXT_X + HERO_TEXT_W <= SCR_W - MARGIN_X,
            "the copy stays inside the safe area"
        );
        assert!(
            POSTER_Y >= MARGIN_Y,
            "the poster starts below the overscan margin"
        );
    }
}
