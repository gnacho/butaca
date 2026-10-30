//! **The OLED idle screen** (issue #66): a true-black field with a dim clock that relocates once a
//! minute, so a television left on a settled menu or a paused frame does not burn its static UI
//! into the panel.
//!
//! # Why this is a whole-screen field, not a screen
//!
//! Every ordinary screen in this app rests at [`theme::SURFACE_APP`] or above - a medium grey that
//! an OLED left for hours WILL retain. The idle screen is not a screen but a field drawn OVER
//! everything, on every route, because the thing it protects against is exactly "the same picture
//! for a long time" and a screen cannot know how long the picture it is drawing has been on the
//! panel. It draws true black ([`theme::IDLE_OLED_BLACK`]) - zero emission - and one dim clock,
//! and the clock moves once a minute so the clock itself cannot burn a glyph into the panel.
//!
//! # The rule (owner's numbers)
//!
//! * active playback (the video plane presenting frames) NEVER counts as idle;
//! * a paused frame counts as idle after **3 minutes**;
//! * a menu (anything else) counts as idle after **2 minutes**;
//! * after **20 minutes** idle the app closes.
//!
//! [`stage`] is the pure decision; the app calls it once a frame with the wall-clock `now` and the
//! last-input stamp, invalidates the present gate when the clock's position changes, and closes
//! (sets `running = false`) on [`Stage::Close`]. The clock's POSITION is derived from the idle
//! time, so it changes on its own once a minute without any stored phase.

use std::ffi::CString;

use crate::ui::{consts, theme, Painter, Rect};

/// No input for this long on a menu (not the player) draws the idle screen.
pub(crate) const MENU_IDLE_MS: u32 = 120_000;
/// No input for this long while paused draws the idle screen.
pub(crate) const PAUSE_IDLE_MS: u32 = 180_000;
/// No input for this long closes the app.
pub(crate) const CLOSE_MS: u32 = 1_200_000;
/// The clock relocates once per this many idle milliseconds.
const MOVE_MS: u32 = 60_000;
/// The clock's type size: a `size::DISPLAY` rung, dim enough not to matter but legible at the
/// couch. It is the one thing on an otherwise black panel, so it may be large.
const CLOCK_SZ: std::os::raw::c_int = theme::size::DISPLAY;

/// Where the idle clock is right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stage {
    /// Not idle: the ordinary picture.
    None,
    /// Idle: draw true black and the clock at the given position.
    Clock { pos: usize },
    /// Idle long enough: close the app.
    Close,
}

/// The pure idle decision. `playing` is "actively presenting frames" (and therefore NEVER idle);
/// `paused` is "on the player route with the transport paused" (the 3-minute branch). Everything
/// else is a menu (the 2-minute branch).
pub(crate) fn stage(now: u32, last_input: u32, playing: bool, paused: bool) -> Stage {
    if playing {
        return Stage::None;
    }
    let elapsed = now.wrapping_sub(last_input);
    if elapsed >= CLOSE_MS {
        return Stage::Close;
    }
    let threshold = if paused { PAUSE_IDLE_MS } else { MENU_IDLE_MS };
    if elapsed < threshold {
        return Stage::None;
    }
    Stage::Clock { pos: position_index(elapsed - threshold) }
}

/// Which position the clock sits at after `idle_ms` of idling - one step per [`MOVE_MS`], cycling
/// through the position set so no single spot holds the glyph for more than a minute.
fn position_index(idle_ms: u32) -> usize {
    (idle_ms / MOVE_MS) as usize % POSITIONS.len()
}

/// The clock's resting positions, as a fraction of the panel (centre of the glyph). Spread across
/// the four quadrants, well clear of the edges, so each minute's glyph occupies fresh pixels.
const POSITIONS: &[(f32, f32)] = &[(0.5, 0.30), (0.5, 0.70), (0.30, 0.50), (0.70, 0.50)];

fn position(pos: usize) -> (f32, f32) {
    let (fx, fy) = POSITIONS[pos % POSITIONS.len()];
    (consts::SCR_W as f32 * fx, consts::SCR_H as f32 * fy)
}

/// "H:MM", with the minute zero-padded - the idle clock's whole face.
pub(crate) fn format_clock(hour: u32, minute: u32) -> String {
    format!("{}:{:02}", hour % 24, minute % 60)
}

/// The current wall-clock time as "H:MM", local time when the platform can resolve it and UTC
/// otherwise. Display-only: it reads the wall clock directly rather than the frame clock because
/// the idle screen is the one surface whose meaning is "what time is it in the room", and it is
/// only ever called while the idle screen is up - never on a recorded replay, whose short,
/// input-driven frames keep [`stage`] at [`Stage::None`].
fn local_clock() -> String {
    let mut tv = libc::timeval { tv_sec: 0, tv_usec: 0 };
    // SAFETY: gettimeofday writes a caller-owned struct and reads no memory of ours.
    if unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) } != 0 {
        return String::new();
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r fills the caller-owned `tm` from `tv_sec`.
    let local = unsafe { libc::localtime_r(&tv.tv_sec, &mut tm) };
    if local.is_null() {
        // No local-time zone data: fall back to the epoch's UTC hour and minute.
        let secs = tv.tv_sec.rem_euclid(86_400);
        return format_clock((secs / 3_600) as u32, ((secs % 3_600) / 60) as u32);
    }
    format_clock(tm.tm_hour as u32, tm.tm_min as u32)
}

/// Draw the idle screen: true black over everything, then the dim clock at `pos`.
pub(crate) fn draw(p: Painter, pos: usize) {
    if p.is_recording() {
        return;
    }
    p.rrect(Rect::FULL, 0.0, 0.0, theme::IDLE_OLED_BLACK);
    let clock = local_clock();
    let (cx, cy) = position(pos);
    if let Ok(cs) = CString::new(clock) {
        // The cap-band centre, not a magic y: descenders (there are none in digits, but the rule
        // is the rule) hang below the optical centre instead of dragging the whole face up.
        let y = crate::text::text_vcenter_y(CLOCK_SZ, 0, cy);
        p.text(cs.as_ptr(), cx, y, CLOCK_SZ, theme::IDLE_CLOCK_INK, 1, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_playback_is_never_idle() {
        // Any elapsed, even well past the close threshold, stays None while frames present.
        for elapsed in [0u32, MENU_IDLE_MS, CLOSE_MS, CLOSE_MS + 60_000] {
            assert_eq!(
                stage(elapsed, 0, true, false),
                Stage::None,
                "active playback must never count as idle at {elapsed}ms"
            );
        }
    }

    #[test]
    fn a_menu_idles_at_two_minutes_and_closes_at_twenty() {
        assert_eq!(stage(MENU_IDLE_MS - 1, 0, false, false), Stage::None);
        assert_eq!(stage(MENU_IDLE_MS, 0, false, false), Stage::Clock { pos: 0 });
        assert!(matches!(stage(CLOSE_MS - 1, 0, false, false), Stage::Clock { .. }),
            "one tick before the close the idle screen is still up");
        assert_eq!(stage(CLOSE_MS, 0, false, false), Stage::Close);
    }

    #[test]
    fn a_paused_frame_idles_at_three_minutes() {
        assert_eq!(stage(PAUSE_IDLE_MS - 1, 0, false, true), Stage::None);
        assert_eq!(stage(PAUSE_IDLE_MS, 0, false, true), Stage::Clock { pos: 0 });
        assert_eq!(stage(MENU_IDLE_MS, 0, false, true), Stage::None,
            "the 2-minute menu threshold does not reach a paused frame");
        assert_eq!(stage(CLOSE_MS, 0, false, true), Stage::Close);
    }

    #[test]
    fn the_clock_relocates_once_a_minute_and_cycles() {
        let base = MENU_IDLE_MS;
        let positions: Vec<usize> = (0..6)
            .map(|m| match stage(base + m * MOVE_MS, 0, false, false) {
                Stage::Clock { pos } => pos,
                other => panic!("expected Clock, got {other:?}"),
            })
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 0, 1], "the clock steps once a minute and wraps");
        // The four positions are distinct and inside the panel.
        for p in 0..POSITIONS.len() {
            let (x, y) = position(p);
            assert!((0.0..consts::SCR_W as f32).contains(&x) && (0.0..consts::SCR_H as f32).contains(&y));
        }
        let mut set = std::collections::BTreeSet::new();
        for p in 0..POSITIONS.len() {
            set.insert((position(p).0 as i32, position(p).1 as i32));
        }
        assert_eq!(set.len(), POSITIONS.len(), "no two positions coincide");
    }

    #[test]
    fn the_clock_face_is_hh_mm() {
        assert_eq!(format_clock(0, 0), "0:00");
        assert_eq!(format_clock(9, 5), "9:05");
        assert_eq!(format_clock(14, 30), "14:30");
        assert_eq!(format_clock(23, 59), "23:59");
        assert_eq!(format_clock(24, 60), "0:00", "out-of-range values wrap, never print 24 or 60");
    }

    #[test]
    fn the_tick_wrap_reads_as_short_elapsed() {
        // `wrapping_sub` across the ~49-day tick wrap reads a small elapsed, never ~4 billion ms.
        assert_eq!(stage(5, u32::MAX - 10, false, false), Stage::None);
        assert_eq!(stage(MENU_IDLE_MS.wrapping_add(u32::MAX - 10), u32::MAX - 10, false, false),
            Stage::Clock { pos: 0 });
    }
}
