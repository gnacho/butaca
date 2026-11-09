//! **The server picker's switch, moved off the frame thread.**
//!
//! Pressing OK on a row of `screens::servers` asks the loop to make that entry active, and the
//! work behind that request is [`super::boot::authenticate_server`]: one blocking LAN round trip
//! (adopt the stored token, or mint one with the password). Run from the loop's request drain it
//! parked the FRAME thread for the length of the answer - up to `net::API`'s 25 s whole-transfer
//! deadline, longer if the name lookup hangs - with no spinner, no heartbeat and no BACK. A dead
//! address in the picker therefore looked exactly like a wedged app, on the screen the owner
//! reaches when a server needs adding or changing.
//!
//! It runs on a worker now and the answer lands on the frame thread, which is the only thread
//! allowed to touch the stores (`app::boot::activate_server_owned`). The picker keeps the remote
//! while the switch is out: BACK pops it, the next frame still draws, and a switch that never
//! answers costs nothing but the switch.
//!
//! Single-flight on purpose. Two rows pressed in a row would otherwise race two authentications
//! against one installed-client slot, and the last writer would win by luck rather than by
//! ordering; [`start`] refuses the second request while the first is out, and says so in the log.

use super::boot::ServerEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// What the worker leaves for the frame thread: which row it was, and whether it signed in.
struct Outcome {
    index: usize,
    signed_in: bool,
}

static MAIL: Mutex<Option<Outcome>> = Mutex::new(None);
static OUT: AtomicBool = AtomicBool::new(false);

/// Sign in against the entry at `index` on a worker. `false` means the request was NOT started -
/// the OS refused the thread, or a switch is already out - and the caller has nothing to undo.
pub(crate) fn start(index: usize, entry: ServerEntry) -> bool {
    start_with(index, move || super::boot::authenticate_server(&entry))
}

/// [`start`] with the round trip supplied, which is how a host test drives the seam without a
/// server: the worker's only contract is "call this once, hand the answer back".
fn start_with(index: usize, sign_in: impl FnOnce() -> bool + Send + 'static) -> bool {
    if OUT.swap(true, Ordering::SeqCst) {
        crate::log("jellyfin: a server switch is already out — this press is ignored");
        return false;
    }
    let spawned = crate::task::spawn_small("jf-switch-server", move || {
        let signed_in = std::panic::catch_unwind(std::panic::AssertUnwindSafe(sign_in))
            .unwrap_or(false);
        if let Ok(mut slot) = MAIL.lock() {
            *slot = Some(Outcome { index, signed_in });
        }
        // A settled screen stops presenting, so the landing needs a frame to happen in.
        crate::ui::idle::invalidate();
    });
    if !spawned {
        OUT.store(false, Ordering::SeqCst);
    }
    spawned
}

/// The frame thread's read of the mailbox: the switch that finished, once. `None` while one is
/// still out, and after the answer has been taken.
pub(crate) fn take() -> Option<(usize, bool)> {
    let out = MAIL.lock().ok()?.take()?;
    OUT.store(false, Ordering::SeqCst);
    Some((out.index, out.signed_in))
}

/// Whether a switch is out. Read by the picker's own tests and by nothing on a frame path.
#[cfg(test)]
pub(crate) fn is_out() -> bool {
    OUT.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Wait for the worker's answer with a deadline, so a broken latch fails the test instead of
    /// hanging the suite.
    fn landed() -> (usize, bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(out) = take() {
                return out;
            }
            assert!(Instant::now() < deadline, "the switch never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A switch that answers lands once, names its row, and releases the single-flight latch.
    #[test]
    fn a_switch_lands_once_and_names_its_row() {
        let _guard = crate::testlock::serial();
        let _ = take();
        assert!(start_with(3, || true));
        assert_eq!(landed(), (3, true));
        assert!(take().is_none(), "the mailbox hands the answer over once");
        assert!(!is_out(), "and the latch is released for the next press");
    }

    /// A second press while one is out is refused rather than racing two authentications.
    #[test]
    fn a_second_press_while_one_is_out_is_refused() {
        let _guard = crate::testlock::serial();
        let _ = take();
        let (tx, rx) = mpsc::channel();
        assert!(start_with(0, move || {
            let _ = rx.recv_timeout(Duration::from_secs(5));
            true
        }));
        assert!(is_out());
        assert!(!start_with(1, || true), "the second request is refused");
        let _ = tx.send(());
        assert_eq!(landed(), (0, true), "the row that was accepted is the one that lands");
    }

    /// A panicking round trip is a refused switch, not a process that loses its latch.
    #[test]
    fn a_panicking_round_trip_answers_refused_and_releases_the_latch() {
        let _guard = crate::testlock::serial();
        let _ = take();
        assert!(start_with(7, || panic!("the server answered with nonsense")));
        assert_eq!(landed(), (7, false));
        assert!(!is_out());
    }
}
