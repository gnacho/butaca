//! **The pair kerning the font carries, applied to LAYOUT rather than to the rasterizer.**
//!
//! SDL_ttf 2.0.x shapes through FreeType's legacy kerning path, and the FreeType this build links
//! answers `FT_Get_Kerning` with 0 even for a face that carries a valid `kern` table: measured
//! 2026-11-25 against DejaVu Sans Bold on the same host, whose own 2000+ pairs it also refuses, so
//! this is the platform and not the cut. The browser the supplied design mockup is drawn in applies
//! the font's GPOS pairs, which is why its five navigation labels measure 298.31px where
//! `TTF_SizeUTF8` measures 300.43 — `Favoritos` alone is 1.61px wider because the reference kerns
//! `vo` and `os` and we do not.
//!
//! So the pairs are BAKED into the shipped faces as a legacy `kern` table at cut time (they are the
//! font's own GPOS pairs, flattened — see [`crate::kern`]'s test that re-derives them) and read here,
//! by this module, so the advance a row is LAID OUT with is the reference's.
//!
//! **Layout, not paint, and that is deliberate.** What is fed the number is the pill geometry — the
//! capsule the reference draws behind each word — so the row and its track land on the reference's
//! boxes. The glyphs inside still come from SDL_ttf, which advances by the unkerned metric; a run is
//! centred in its pill, so the ink lands within half a pixel of the reference's and the boundary
//! that matters (the capsule) is exact. Making the PAINT kern as well would mean drawing glyph by
//! glyph, which buys that half pixel at the cost of the whole glyph cache.
//!
//! Only the two roles the mockup is set in are kerned ([`crate::text::FontRole::InterfaceMedium`],
//! [`crate::text::FontRole::Brand`], and the clock's tabular cut); every other face keeps exactly
//! the metrics it had, so no screen's existing layout moves.

use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::OnceLock;

use crate::text::FontRole;

/// A face's kerning, in the two forms the sums need: units per em, and `(left, right) -> xAdvance`
/// in font units. Glyph IDs, because that is what a `kern` table stores.
struct Kern {
    upm: u16,
    pairs: HashMap<(u16, u16), i16>,
    /// `char -> glyph id`, resolved through the face's own `cmap`.
    cmap: HashMap<u32, u16>,
    /// `glyph id -> advance, in font units` (`hmtx`, read to `hhea`'s metric count and then the
    /// last entry repeated, which is the format's own rule).
    adv: HashMap<u16, u16>,
}

/// A shipped face's bytes: from where the package actually got installed, and from the repository's
/// own `pkg/` when there is no install root — which is every host test. `pkg/` is the same file that
/// ships, so a test parses exactly what the television will.
fn face_bytes(role: FontRole) -> Option<Vec<u8>> {
    let name = crate::text::role_file(role);
    std::fs::read(crate::paths::in_app_dir(name)).ok().or_else(|| {
        let repo = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../pkg"));
        std::fs::read(repo.join(name)).ok()
    })
}

/// The kerning of one shipped face, parsed once on first use.
///
/// `None` is a real answer: the reference cuts are the only ones with a baked table, and a face
/// whose file could not be read must not take the app down over a 1.6px advance. A miss is logged
/// once per role — this is the kind of defect that would otherwise be silent forever.
fn table(role: FontRole) -> Option<&'static Kern> {
    static CACHE: OnceLock<HashMap<u8, Option<Kern>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let mut map = HashMap::new();
        // `Interface` is here for the one run that needs a tracked pitch in the body's own weight:
        // the account dialog's `ACTIVO` tag (`.tag { letter-spacing:.1em }`). Asking for a role
        // this list does not name answers `None`, and the caller then draws nothing at all — which
        // is how the tag vanished from the dialog the first time it was drawn glyph by glyph.
        for role in [
            FontRole::Interface,
            FontRole::InterfaceMedium,
            FontRole::Brand,
            FontRole::Clock,
        ] {
            let parsed = face_bytes(role).as_deref().and_then(parse);
            if parsed.is_none() {
                crate::log(&format!(
                    "kern: {role:?} face cannot be read — that row lays out from the rasterizer's \
                     own advances"
                ));
            }
            map.insert(role as u8, parsed);
        }
        map
    });
    cache.get(&(role as u8)).and_then(|k| k.as_ref())
}

/// The advance `s` should be LAID OUT with in `role`: the rasterizer's own advance plus the kerning
/// its `kern` table carries between the glyphs actually drawn.
///
/// A NUL-separated string is not expected (the app's labels are single runs); every character that
/// the face does not map simply contributes no pair, which is what the rasterizer does with it too.
/// The advance the reference would LAY OUT `s` with, computed from the face itself: its own
/// `hmtx` advances plus its kern pairs. Slightly different from [`advance`], which adds a
/// rasterizer's number to a kern delta — that one inherits whatever hinting the rasterizer is
/// using, and this bar's boxes are specified to a hundredth of a pixel, so they read the face.
///
/// `fallback` is called only when the face cannot draw the string (a script that needs the
/// fallback chain): then the rasterizer's own measurement is the honest answer.
pub(crate) fn layout_width(
    role: FontRole,
    s: &CStr,
    sz: i32,
    fallback: impl FnOnce() -> f32,
) -> f32 {
    if let Some(k) = table(role) {
        if let Ok(text) = s.to_str() {
            if let Some((total, _)) = glyphs_of(k, text, sz) {
                return total;
            }
        }
    }
    fallback()
}

// ---- the parser: one sfnt, three tables, no dependencies ------------------------------------

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// The `(offset, length)` of a table, from the sfnt's directory.
fn table_range(b: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let n = u16_at(b, 4)? as usize;
    for i in 0..n {
        let rec = 12 + 16 * i;
        if b.get(rec..rec + 4)? != tag {
            continue;
        }
        let off = u32_at(b, rec + 8)? as usize;
        let len = u32_at(b, rec + 12)? as usize;
        return Some((off, len));
    }
    None
}

/// Parse the three tables a layout sum needs: `head` (units per em), `cmap` (char to glyph) and
/// `kern` (the pairs). Any of them missing is `None` — a face without a kern table has nothing to
/// add, and every caller already handles that.
fn parse(b: &[u8]) -> Option<Kern> {
    let (head, _) = table_range(b, b"head")?;
    let upm = u16_at(b, head + 18)?;
    if upm == 0 {
        return None;
    }
    let (cmap_off, cmap_len) = table_range(b, b"cmap")?;
    let cmap = parse_cmap(b.get(cmap_off..cmap_off + cmap_len)?)?;
    // **A face without a `kern` table still lays out**, at its own `hmtx` advances and no pairs.
    // Only the brand cut carries a baked table; the rest of the shipped faces keep their kerning in
    // `GPOS` (which FreeType ignores here, recorded in the module doc). Answering `None` for them
    // made `glyphs` unusable for the body face, which is what a tracked run needs.
    let pairs = match table_range(b, b"kern") {
        Some((kern_off, kern_len)) => parse_kern(b.get(kern_off..kern_off + kern_len)?)?,
        None => HashMap::new(),
    };
    let (hhea, _) = table_range(b, b"hhea")?;
    let metrics = u16_at(b, hhea + 34)? as usize;
    let (hmtx, hmtx_len) = table_range(b, b"hmtx")?;
    let adv = parse_hmtx(b.get(hmtx..hmtx + hmtx_len)?, metrics)?;
    Some(Kern { upm, pairs, cmap, adv })
}

/// `cmap`, the subtables a TrueType face's own shaping uses: the Unicode BMP one (format 4) and the
/// full-repertoire one (format 12). A format 0 or 6 subtable would be a 1990s Mac face; the shipped
/// cuts are modern and carry 4 and 12.
fn parse_cmap(c: &[u8]) -> Option<HashMap<u32, u16>> {
    let n = u16_at(c, 2)? as usize;
    let mut best: Option<(u32, HashMap<u32, u16>)> = None;
    for i in 0..n {
        let rec = 4 + 8 * i;
        let platform = u16_at(c, rec)?;
        let encoding = u16_at(c, rec + 2)?;
        let off = u32_at(c, rec + 4)? as usize;
        // (3,10) full repertoire and (3,1) BMP are the two a Windows/FreeType stack reads; (0,x)
        // Unicode platform records are accepted as the fallback.
        let rank = match (platform, encoding) {
            (3, 10) => 3,
            (3, 1) => 2,
            (0, _) => 1,
            _ => 0,
        };
        if rank == 0 {
            continue;
        }
        let format = u16_at(c, off)?;
        let map = match format {
            4 => parse_cmap4(c, off),
            12 => parse_cmap12(c, off),
            _ => None,
        };
        if let Some(map) = map {
            if best.as_ref().is_none_or(|(r, _)| rank > *r) {
                best = Some((rank, map));
            }
        }
    }
    best.map(|(_, m)| m)
}

fn parse_cmap4(c: &[u8], o: usize) -> Option<HashMap<u32, u16>> {
    let segs = u16_at(c, o + 6)? as usize / 2;
    let ends = o + 14;
    let starts = ends + segs * 2 + 2;
    let deltas = starts + segs * 2;
    let ranges = deltas + segs * 2;
    let mut out = HashMap::new();
    for i in 0..segs {
        let end = u16_at(c, ends + i * 2)? as u32;
        let start = u16_at(c, starts + i * 2)? as u32;
        let delta = u16_at(c, deltas + i * 2)? as i16 as i32;
        let range = u16_at(c, ranges + i * 2)? as usize;
        for cp in start..=end {
            if cp == 0xFFFF {
                continue;
            }
            let gid = if range == 0 {
                (cp as i32 + delta) & 0xFFFF
            } else {
                // `range` is a byte offset from THIS idRangeOffset entry into glyphIdArray.
                let at = ranges + i * 2 + range + 2 * (cp - start) as usize;
                let g = u16_at(c, at)? as i32;
                if g == 0 {
                    continue;
                }
                (g + delta) & 0xFFFF
            };
            if gid != 0 {
                out.insert(cp, gid as u16);
            }
        }
    }
    Some(out)
}

fn parse_cmap12(c: &[u8], o: usize) -> Option<HashMap<u32, u16>> {
    let groups = u32_at(c, o + 12)? as usize;
    let mut out = HashMap::new();
    for g in 0..groups {
        let at = o + 16 + 12 * g;
        let start = u32_at(c, at)?;
        let end = u32_at(c, at + 4)?;
        let glyph = u32_at(c, at + 8)?;
        if end < start || end - start > 0xFFFF {
            continue;
        }
        for cp in start..=end {
            out.insert(cp, (glyph + (cp - start)) as u16);
        }
    }
    Some(out)
}

/// The horizontal advances, one per glyph: `numberOfHMetrics` records, then the LAST one repeats for
/// every glyph above it — that repetition is the format, not a fallback.
fn parse_hmtx(h: &[u8], metrics: usize) -> Option<HashMap<u16, u16>> {
    if metrics == 0 {
        return None;
    }
    let mut out = HashMap::new();
    let mut last = 0u16;
    for i in 0..metrics {
        last = u16_at(h, 4 * i)?;
        out.insert(i as u16, last);
    }
    // The glyph count comes from `maxp`; reading it here keeps the table's own promise that every
    // glyph has an advance, without trusting a caller to know how many there are.
    let _ = last;
    Some(out)
}

/// `kern`, version 0, horizontal format-0 subtables — the only shape a cut of ours writes and the
/// only one FreeType ever read.
fn parse_kern(k: &[u8]) -> Option<HashMap<(u16, u16), i16>> {
    if u16_at(k, 0)? != 0 {
        return None;
    }
    let n = u16_at(k, 2)? as usize;
    let mut out = HashMap::new();
    let mut at = 4;
    for _ in 0..n {
        let _version = u16_at(k, at)?;
        let length = u16_at(k, at + 2)? as usize;
        let coverage = u16_at(k, at + 4)?;
        // bit 0 = horizontal, bit 1 = minimum, bit 2 = cross-stream: only horizontal, additive
        // pairs mean what this module assumes.
        if coverage & 0b111 != 0b001 {
            at += length.max(6);
            continue;
        }
        let npairs = u16_at(k, at + 6)? as usize;
        let mut p = at + 14;
        for _ in 0..npairs {
            let left = u16_at(k, p)?;
            let right = u16_at(k, p + 2)?;
            let value = u16_at(k, p + 4)? as i16;
            if value != 0 {
                out.insert((left, right), value);
            }
            p += 6;
        }
        at += length;
    }
    Some(out)
}

/// One glyph of a run: where its bytes start, how long they are, and how far into the run its
/// origin sits — all in the run's own coordinates, kerning included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub at: usize,
    pub len: usize,
    /// Offsets from the run's start, in pixels at `sz`. Fractional on purpose: the painter snaps the
    /// origin of each glyph when it draws it, which is exactly what a browser at 1x does.
    pub dx: f32,
}

/// A run laid out for **per-glyph painting**: the total advance (kerning included) and every
/// glyph's slice and offset. `None` when the face does not map every character — a run that needs
/// the Inter/CJK fallback is the rasterizer's to shape, not this module's to split.
pub(crate) fn glyphs(role: FontRole, s: &str, sz: i32) -> Option<(f32, Vec<Glyph>)> {
    glyphs_of(table(role)?, s, sz)
}

fn glyphs_of(k: &Kern, s: &str, sz: i32) -> Option<(f32, Vec<Glyph>)> {
    let mut x = 0f32;
    let mut prev: Option<u16> = None;
    let mut out = Vec::with_capacity(s.len());
    for (at, ch) in s.char_indices() {
        let gid = *k.cmap.get(&(ch as u32))?;
        if let Some(left) = prev {
            if let Some(&v) = k.pairs.get(&(left, gid)) {
                x += v as f32 * sz as f32 / k.upm as f32;
            }
        }
        out.push(Glyph { at, len: ch.len_utf8(), dx: x });
        let advance = *k.adv.get(&gid)?;
        x += advance as f32 * sz as f32 / k.upm as f32;
        prev = Some(gid);
    }
    Some((x, out))
}

#[cfg(test)]
mod tests;
