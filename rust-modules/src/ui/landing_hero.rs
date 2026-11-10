//! Shared geometry and atmospheric curve for a TOP-anchored hero: the home billboard's title,
//! synopsis and action row. Keeping these pure lets the renderer, route narrative and legibility
//! tests use one definition.

use super::{consts::SCR_H, theme, widgets::HERO_BASE_SCRIM_Y0};

/// **The hero text block's HEAD - the top edge of the title band, and the owner's "título y
/// sinopsis arriba" (2026-11-10).**
///
/// It is not a taste number: it is the highest line a hero title can be placed on, and the term
/// that binds it is the CLEARLOGO's own spill. A logo is placed by its band's FOOT and paints
/// UPWARD out of it (`hero_logo`'s layout-vs-paint rule), up to
/// [`theme::logo::HERO_H_MAX`] tall against the band's [`theme::logo::HERO_H_MIN`] - so the band's
/// foot may not rise above the top chrome by more than that difference. Below that line a square
/// emblem would paint over the tab track, which is one of the three things LG's overscan item #2
/// names.
///
/// The block used to be bottom-anchored at 692, which put the title at 390 with the whole upper
/// third of the panel empty and the shelves paying for it. Anchoring it here moves the title up
/// 112px and the first shelf with it.
pub(crate) const TEXT_TOP: f32 = crate::ui::widgets::TOP_BAR_BOTTOM
    + (theme::logo::HERO_H_MAX - theme::logo::HERO_H_MIN);
/// The copy column's width. Wide enough for a three-line blurb at the hero's rung, and it stops
/// well short of the panel's centre so the artwork keeps its side.
pub(crate) const COL_W: f32 = 660.0;
pub(crate) const KNEE_Y: f32 = 0.65 * SCR_H;
pub(crate) const MID_WEIGHT: f32 = 0.55;
/// How much a bound preview multiplies the hero scrim curve. The shape stays the wedge, the
/// bottom ramp and the feathered corner. 1.35 is the video-bound row the legibility table grades.
pub(crate) const PREVIEW_FIELD: f32 = 1.35;

/// The block's measured BOTTOM: its head plus the three rows it stacks and the rungs between them
/// (`MD` inside the identity group, `SM` before the blurb). The action row hangs off this by
/// [`theme::space::LG`] - the ladder's step from information to action - and it is a function
/// rather than a constant because the blurb's height is measured, not quoted.
pub(crate) fn stack_bottom(title_h: f32, meta_h: f32, synopsis_h: f32) -> f32 {
    TEXT_TOP + title_h + theme::space::MD + meta_h + theme::space::SM + synopsis_h
}

pub(crate) fn base_scrim_bottom_a(hero_a: f32) -> f32 { 0.30 + 0.64 * hero_a.clamp(0.0, 1.0) }

pub(crate) fn base_scrim_ramp(hero_a: f32) -> [f32; 4] {
    let foot = base_scrim_bottom_a(hero_a);
    [HERO_BASE_SCRIM_Y0, KNEE_Y, foot * MID_WEIGHT, foot]
}

pub(crate) fn base_scrim_a(y: f32, hero_a: f32) -> f32 {
    let foot = base_scrim_bottom_a(hero_a);
    let mid = foot * MID_WEIGHT;
    if y <= HERO_BASE_SCRIM_Y0 { 0.0 }
    else if y < KNEE_Y { mid * (y - HERO_BASE_SCRIM_Y0) / (KNEE_Y - HERO_BASE_SCRIM_Y0) }
    else { mid + (foot - mid) * ((y - KNEE_Y) / (SCR_H - KNEE_Y)).clamp(0.0, 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The head is exactly the clearance the tallest logo needs, so the band cannot drift into the
    /// tab track - and the text's own cap top lands UNDER the wedge's knee, which is what lets
    /// `hero_scrim_a` grade it as a function of x alone.
    #[test]
    fn the_block_head_is_the_logos_own_clearance_and_the_title_clears_the_knee() {
        assert_eq!(
            TEXT_TOP,
            crate::ui::widgets::TOP_BAR_BOTTOM + (theme::logo::HERO_H_MAX - theme::logo::HERO_H_MIN)
        );
        let band = crate::ui::hero_logo::band_h(crate::ui::hero_logo::LogoRung::Hero);
        assert_eq!(band, theme::logo::HERO_H_MIN, "band_h is the rung's floor");
        // a square emblem's ink reaches the top chrome's own bottom edge and not one pixel past
        let ink_top = TEXT_TOP + band - theme::logo::HERO_H_MAX;
        assert!(ink_top >= crate::ui::widgets::TOP_BAR_BOTTOM - 0.01);
        // the TEXT fallback's cap top sits at or below the wedge's knee
        // one HERO-72 bold cap band, quoted like the rest of the host suite's font metrics
        // (the host links no SDL_ttf; `widgets_test_support::HERO_CAP_H` is the same 52)
        let cap_top = TEXT_TOP + band - 52.0;
        assert!(
            cap_top >= crate::ui::widgets::HERO_SCRIM_KNEE,
            "cap top {cap_top} must be under the knee {}",
            crate::ui::widgets::HERO_SCRIM_KNEE
        );
    }

    #[test]
    fn the_stack_bottom_adds_the_three_rows_and_their_rungs() {
        let (t, m, s) = (120.0, 34.0, 108.0);
        assert_eq!(
            stack_bottom(t, m, s),
            TEXT_TOP + t + theme::space::MD + m + theme::space::SM + s
        );
    }

    #[test]
    fn base_scrim_preserves_the_legacy_foot_and_clamps_inputs() {
        assert_eq!(base_scrim_a(0.0, 1.0), 0.0);
        assert_eq!(base_scrim_a(SCR_H, 1.0), 0.94);
        assert_eq!(base_scrim_a(SCR_H + 100.0, 2.0), 0.94);
        // a fully faded hero still stands on the page's own ground: the curve's foot is
        // `0.30 + 0.64 * hero_a`, so zero hero is a third of a scrim and never none.
        assert_eq!(base_scrim_a(SCR_H, -1.0), 0.30);
    }
}
