//! The five navigation advances, against the reference's own DOM numbers.
//!
//! The expected values are not "what our table says": they are what the supplied mockup's browser
//! measures for its own `#mainnav button` labels at `font: 600 17px Manrope`, read off
//! `getBoundingClientRect()` at 1920x1080 on 2026-11-25. The raw (unkerned) advance of each label
//! sits beside it, so a change in the cut or in the table shows up as a number that moved.
use super::*;

fn width_of(role: FontRole, s: &str, sz: i32) -> f32 {
    let k = table(role).expect("the reference cut ships a kern table");
    glyphs_of(k, s, sz).expect("the face draws this string").0
}

#[test]
fn the_navigation_advances_are_the_references_own() {
    for (label, want) in [
        ("Inicio", 43.72f32),
        ("Películas", 73.09),
        ("Series", 51.41),
        ("Favoritos", 74.09),
        ("Buscar", 56.00),
    ] {
        let got = width_of(FontRole::InterfaceMedium, label, 17);
        assert!(
            (got - want).abs() < 0.03,
            "{label}: the face lays the run out {got:.3} wide, the reference measures {want}"
        );
    }
}

#[test]
fn the_reference_cuts_carry_pairs_and_the_others_do_not_need_to() {
    let k = table(FontRole::InterfaceMedium).expect("InterfaceMedium kern table");
    assert!(k.pairs.len() > 1000, "parsed only {} pairs", k.pairs.len());
    assert_eq!(k.upm, 2000, "Manrope's units per em moved");
    assert!(
        table(FontRole::Brand).is_some_and(|b| b.pairs.len() > 1000),
        "the wordmark's cut carries its own pairs"
    );
    assert!(!k.pairs.contains_key(&(0, 0)), "notdef is never a kern partner");
}

/// **The wordmark's runs carry no pairs in Fraunces**, which is what lets `bridge` place the second
/// run (`ca`) from the PAINTED advance of the first and still land on the reference's 192.33: with no
/// pair between them, the face's own layout sum and the rasterizer's advance are the same number.
#[test]
fn the_wordmark_runs_carry_no_pairs() {
    let k = table(FontRole::Brand).expect("the wordmark's cut");
    for word in ["buta", "ca", "butaca"] {
        let (total, glyphs) = glyphs_of(k, word, 30).expect("the face draws it");
        let sum: f32 = word
            .chars()
            .map(|ch| {
                let gid = k.cmap[&(ch as u32)];
                k.adv[&gid] as f32 * 30.0 / k.upm as f32
            })
            .sum();
        assert!(
            (total - sum).abs() < 0.02,
            "{word}: laid out {total}, its own advances sum {sum} — a pair appeared"
        );
        assert_eq!(glyphs.len(), word.chars().count());
    }
}

#[test]
fn the_clock_digits_are_tabular() {
    let k = table(FontRole::Clock).expect("clock cut");
    let _ = k;
    assert!(k.cmap.get(&('0' as u32)).is_some() && k.cmap.get(&('9' as u32)).is_some());
    let mut widths = Vec::new();
    for s in ["21:34", "00:00", "07:58", "11:11"] {
        widths.push(width_of(FontRole::Clock, s, 17));
    }
    assert!(
        widths.iter().all(|w| (w - 47.98).abs() < 0.03),
        "tabular digits make every HH:MM the reference's 47.98: {widths:?}"
    );
}

/// The dialog's tag is the one run drawn in the BODY weight at a tracked pitch, so `Interface` has
/// to be a parsed face like the reference cuts are. It answers `None` for a role this module never
/// parsed, and a caller that draws glyph by glyph then draws nothing at all.
#[test]
fn the_body_face_lays_out_a_tracked_run() {
    let (total, glyphs) = glyphs(FontRole::Interface, "ACTIVO", 12).expect("Interface kern table");
    assert_eq!(glyphs.len(), 6);
    assert!(total > 30.0 && total < 60.0, "six capitals at 12px: {total}");
    assert_eq!(glyphs[0].dx, 0.0);
    assert!(glyphs[5].dx > glyphs[0].dx);
}
