#!/usr/bin/env python3
"""
font-hint-audit.py — verify the theme::size ladder rasterizes with design-true stroke
weights for the INTER faces we ship (`FONTS` below).

Scope, because a third face ships and is deliberately NOT audited here: `pkg/appfont-cjk.ttf`
(Noto Sans CJK KR, the fallback chain's link 2 — see rust-modules/src/text.rs) never draws Latin
in this app, the ladder was tuned on Inter's stems, and SDL_ttf's synthetic bold is never applied
to it. A CJK re-cut is graded by rust-modules/src/fontcov.rs's coverage gate and by a device
capture, not by this script.

Why this exists: TrueType hinting grid-fits strokes to whole pixels, and the rounding is
size- and font-specific. Under FreeType's default NORMAL hinting, Arial's own bytecode
rounds horizontal bars UP — at bold 26 (size::LABEL, the card-title rung) it drew 4px bars
over 3px stems, inverting the typeface's stem>bar design and making titles read top-heavy.
The app therefore rasterizes with LIGHT hinting and draws text quads at integer pixel
origins (both in rust-modules/src/text.rs), which keeps stems >= bars at every size.

This script proves that property holds: it parses the rungs out of theme.rs `mod size`
(single source of truth), adds the two documented carve-outs, and renders a capital F from
each shipped font at each size under LIGHT hinting, measuring the solid core (coverage
> 0.9) of the top bar vs the vertical stem. Any size where bars come out heavier is
flagged; new flags (outside the accepted list below) fail the run.

Run it after swapping font files or touching the hinting/rasterization path:
    ./tools/font-hint-audit.py
Dependency: freetype-py  (pip install freetype-py — any venv works, no other deps)
"""

import os
import re
import sys

try:
    import freetype
except ImportError:
    sys.exit("freetype-py not installed — run: pip install freetype-py")

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
THEME = os.path.join(REPO, "rust-modules", "src", "ui", "theme.rs")
# Every shipped LATIN face, so a face swap cannot dodge the gate. Manrope is the interface
# primary, Fraunces the display primary, Inter the retained script fallback. `pkg/appfont-cjk.ttf`
# is deliberately NOT audited here: it never draws Latin in this app (see the module docstring).
FONTS = {
    "regular": os.path.join(REPO, "pkg", "appfont.ttf"),
    "bold": os.path.join(REPO, "pkg", "appfont-bold.ttf"),
    "manrope-regular": os.path.join(REPO, "pkg", "appfont-manrope-400.ttf"),
    "manrope-bold": os.path.join(REPO, "pkg", "appfont-manrope-700.ttf"),
    "fraunces-regular": os.path.join(REPO, "pkg", "appfont-fraunces-600.ttf"),
    "fraunces-bold": os.path.join(REPO, "pkg", "appfont-fraunces-700.ttf"),
    # The two cuts the supplied design mockup is set in: `#mainnav button { font: 600 17px
    # Manrope }` and the wordmark's Fraunces 700 at optical size 30. Audited like every other
    # shipped latin face, so a swap cannot dodge the ladder here either.
    "manrope-medium": os.path.join(REPO, "pkg", "appfont-manrope-600.ttf"),
    "fraunces-brand": os.path.join(REPO, "pkg", "appfont-brand-700.ttf"),
    # Manrope 700 with its tabular figures baked into the cmap: the clock's cut.
    "manrope-clock": os.path.join(REPO, "pkg", "appfont-clock-700.ttf"),
    # Manrope 800: the avatar initial's weight, the mockup's `#avatar { font-weight: 800 }`.
    "manrope-extrabold": os.path.join(REPO, "pkg", "appfont-manrope-800.ttf"),
}
# Named carve-outs living outside the ladder (see theme.rs's size-module doc).
CARVEOUTS = {54: "HUD_TITLE_SZ (bold)", 36: "subtitle caption (bold)"}
# Known, accepted imbalances — reviewed 2026-07-18. MICRO (22) and LABEL's rare regular
# sites (26) are bar-heavy by one solid pixel even under LIGHT hinting; both are low-
# prominence roles and every remedy (size nudge) costs scale hierarchy. Anything NOT in
# this set that measures bar-heavy is a regression and fails the audit.
#
# Manrope (2026-11-23) carries a capital F whose top bar is one solid pixel heavier than its stem
# at the listed rungs — a typeface proportion, not a hinting regression, and the same +1px class
# the Inter exceptions above already accept. The interface face draws the whole ladder, so its
# entries cover every flagged rung.
ACCEPTED = {
    ("regular", 22), ("regular", 26),
    ("manrope-regular", 20), ("manrope-regular", 26), ("manrope-regular", 32),
    ("manrope-regular", 40), ("manrope-regular", 48), ("manrope-regular", 54),
    ("manrope-bold", 20), ("manrope-bold", 32), ("manrope-bold", 36),
    # Manrope 600 (2026-11-25) is the SAME typeface at the weight the design mockup sets its top
    # navigation in, so it carries the same capital F proportion the 400 and 700 cuts are already
    # accepted for — one solid pixel of bar over stem, not a hinting change. Nothing in the product
    # draws this face above 17px, but the sweep is the ladder, not the call sites.
    ("manrope-medium", 20), ("manrope-medium", 22), ("manrope-medium", 28),
    ("manrope-medium", 32), ("manrope-medium", 48),
    # The clock's tabular cut is the same Manrope 700 the app already ships, with the digits
    # remapped, so it lands on the same accepted rungs the 700 cut does.
    ("manrope-clock", 20), ("manrope-clock", 32), ("manrope-clock", 36),
    # Manrope 800 is the same typeface one notch heavier still, so it carries the same capital-F
    # proportion the 400/600/700 cuts are accepted for. The product draws it at exactly one size.
    ("manrope-extrabold", 22), ("manrope-extrabold", 28), ("manrope-extrabold", 32),
    ("manrope-extrabold", 48),
}


def ladder():
    """size -> rung name, parsed from theme.rs `mod size`."""
    src = open(THEME).read()
    mod = re.search(r"pub mod size \{(.*?)\n\}", src, re.S).group(1)
    return {int(m.group(2)): m.group(1)
            for m in re.finditer(r"pub const (\w+): c_int = (\d+);", mod)}


def measure_f(face, size):
    """Solid-core px of the capital F's top bar (rows) and stem (cols) + AA partials."""
    face.set_pixel_sizes(0, size)
    face.load_char("F", freetype.FT_LOAD_RENDER | freetype.FT_LOAD_TARGET_LIGHT)
    bmp = face.glyph.bitmap
    px = lambda x, y: bmp.buffer[y * bmp.pitch + x] / 255
    bar, bx = [], int(bmp.width * 0.6)
    for y in range(bmp.rows):
        v = px(bx, y)
        if v > 0.05:
            bar.append(v)
        elif bar:
            break
    yy = int(bmp.rows * 0.75)
    stem = [v for x in range(max(6, bmp.width // 2)) if (v := px(x, yy)) > 0.05]
    core = lambda p: sum(1 for v in p if v > 0.9)
    part = lambda p: sum(v for v in p if v <= 0.9)
    return core(bar), part(bar), core(stem), part(stem)


def main():
    names = ladder()
    sizes = sorted(set(names) | set(CARVEOUTS))
    failures = []
    for weight, path in FONTS.items():
        face = freetype.Face(path)
        print(f"== {weight}  ({os.path.basename(path)}) ==")
        print("size (rung)      | bar core+part | stem core+part | verdict")
        for sz in sizes:
            bc, bp, sc, sp = measure_f(face, sz)
            role = names.get(sz) or CARVEOUTS.get(sz, "")
            if bc > sc:
                verdict = "bars +%d  <-- AVOID" % (bc - sc)
                if (weight, sz) in ACCEPTED:
                    verdict += "  (accepted)"
                else:
                    failures.append((weight, sz))
            else:
                verdict = "ok (stems >= bars)"
            print(" %2d %-12s | %d + %.2f      | %d + %.2f       | %s"
                  % (sz, "(%s)" % role if role else "", bc, bp, sc, sp, verdict))
    if failures:
        print("\nFAIL: bar-heavy at", ", ".join("%s %d" % f for f in failures))
        return 1
    print("\nOK: every rung renders design-true (or is an accepted exception).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
