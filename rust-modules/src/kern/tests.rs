//! The five navigation advances, against the reference's own DOM numbers.
//!
//! The expected values are not "what our table says": they are what the supplied mockup's browser
//! measures for its own `#mainnav button` labels at `font: 600 17px Manrope`, read off
//! `getBoundingClientRect()` at 1920x1080 on 2026-11-25. The raw (unkerned) advance of each label
//! sits beside it, so a change in the cut or in the table shows up as a number that moved.
use super::*;

fn delta_of(role: FontRole, s: &str, sz: i32) -> f32 {
    let k = table(role).expect("the reference cut ships a kern table");
    delta(k, s, sz)
}

#[test]
fn the_navigation_advances_are_the_references_own() {
    for (label, want, raw) in [
        ("Inicio", 43.72f32, 43.98f32),
        ("Películas", 73.09, 73.22),
        ("Series", 51.41, 51.41),
        ("Favoritos", 74.09, 75.70),
        ("Buscar", 56.00, 56.12),
    ] {
        let d = delta_of(FontRole::InterfaceMedium, label, 17);
        let got = raw + d;
        assert!(
            (got - want).abs() < 0.03,
            "{label}: {raw} + {d:.3} = {got:.3}, the reference measures {want}"
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

/// **The wordmark is unkerned in Fraunces, and the two-run split depends on it.** `buta` advances by
/// exactly what the rasterizer says, which is why `bridge` places the second run (`ca`) at
/// `stem_w + tracking + gap` and lands on the reference's 192.33. If this ever gains a pair, that
/// placement has to read this module instead of the painted width.
#[test]
fn the_wordmark_runs_are_unkerned_at_layout_time() {
    for s in ["buta", "ca", "butaca"] {
        assert_eq!(delta_of(FontRole::Brand, s, 30), 0.0, "{s} gained a kern pair");
    }
}

#[test]
fn the_clock_digits_are_tabular() {
    let k = table(FontRole::Clock).expect("clock cut");
    assert!(k.cmap.get(&('0' as u32)).is_some() && k.cmap.get(&('9' as u32)).is_some());
    for s in ["21:34", "00:00", "07:58", "11:11"] {
        assert!(delta(k, s, 17).abs() < 4.0, "{s} kerning out of range");
    }
}
