//! Subtitle appearance: the `ass_set_style_overrides` list the player hands libass for a text
//! track the app composed (#86). AUTHORED ASS/SSA tracks never reach this seam: their own style
//! is the author's, so nothing here can override it.

use super::*;

fn steps(scale: u8, position: u8, background: u8) {
    restore_subtitle_scale_step(scale);
    restore_subtitle_position_step(position);
    restore_subtitle_background_step(background);
}

#[test]
fn the_factory_look_asks_for_no_overrides() {
    let _serial = crate::testlock::serial();
    steps(0, 0, 0);
    // An untouched preference must draw exactly what shipped, and an EMPTY list is how libass is
    // told there is nothing to override.
    assert_eq!(subtitle_style_overrides(), "");
}

#[test]
fn a_size_step_moves_the_font_size_and_leaves_the_rest_alone() {
    let _serial = crate::testlock::serial();
    steps(3, 0, 0);
    let list = subtitle_style_overrides();
    assert!(list.contains("FontSize=30"), "{list}");
    assert!(list.contains("MarginV=0"), "{list}");
    assert!(list.contains("BorderStyle=1"), "{list}");
}

#[test]
fn a_position_step_walks_the_vertical_margin() {
    let _serial = crate::testlock::serial();
    steps(0, 4, 0);
    assert!(subtitle_style_overrides().contains("MarginV=90"));
}

#[test]
fn a_background_step_boxes_the_line() {
    let _serial = crate::testlock::serial();
    steps(0, 0, 1);
    let half = subtitle_style_overrides();
    assert!(half.contains("BorderStyle=3"), "{half}");
    assert!(half.contains("BackColour=&H80000000"), "{half}");
    steps(0, 0, 2);
    let solid = subtitle_style_overrides();
    assert!(solid.contains("BackColour=&HC0000000"), "{solid}");
}

#[test]
fn every_step_clamps_to_the_top_of_its_scale() {
    let _serial = crate::testlock::serial();
    steps(9, 9, 9);
    assert_eq!(subtitle_scale_step(), 4);
    assert_eq!(subtitle_position_step(), 4);
    assert_eq!(subtitle_background_step(), 2);
    assert!(subtitle_style_overrides().contains("FontSize=38"));
}
