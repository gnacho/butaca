//! **The server picker's switch, moved off the frame thread.**
//!
//! Pressing OK on a row of `screens::servers` asks the loop to make that entry active, and the
//! work behind that request is [`super::boot::sign_in`]: one blocking LAN round trip (adopt the
//! stored token, or mint one with the password). Run from the loop's request drain it parked the
//! FRAME thread for the length of the answer - up to `net::API`'s 25 s whole-transfer deadline,
//! longer if the name lookup hangs - with no spinner, no heartbeat and no BACK. A dead address in
//! the picker therefore looked exactly like a wedged app, on the screen the owner reaches when a
//! server needs adding or changing.
//!
//! It runs on a worker now and the CLIENT lands on the frame thread, which is the only thread
//! allowed to install it and reset the stores (`app::boot::activate_server_owned`). The picker
//! keeps the remote while the switch is out: BACK pops it, the next frame still draws, and a
//! switch that never answers costs nothing but the switch.
//!
//! Single-flight on purpose. Two rows pressed in a row would otherwise race two authentications
//! against one installed-client slot, and the last writer would win by luck rather than by
//! ordering; [`start`] refuses the second request while the first is out, and says so in the log.
//! The latch is BOUNDED: a round trip that outlives [`SWITCH_BUDGET`] (a name lookup that never
//! resolves, the one thing `net::API` cannot time out) is abandoned and the picker accepts the
//! next press again. Its late answer carries the old generation and is discarded, so an abandoned
//! switch can never install over the one that replaced it.

use super::boot::ServerEntry;
use super::client::JfClient;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the picker waits for one switch before it gives up on it. Long enough for a slow but
/// alive server (a healthy one answers in tens of milliseconds, `net::API` caps a transfer at
/// 25 s), short enough that a name lookup stuck in `getaddrinfo` does not retire the feature.
pub(crate) const SWITCH_BUDGET: Duration = Duration::from_secs(30);

/// What the worker leaves for the frame thread: which generation and row it was, and the
/// signed-in client when the server answered. The client travels in the mailbox rather than being
/// installed by the worker: an install is a store-wide event, and a worker that installed a server
/// after a sign-out landed would undo it from the wrong thread.
struct Outcome {
    generation: u32,
    index: usize,
    client: Option<JfClient>,
}

static MAIL: Mutex<Option<Outcome>> = Mutex::new(None);
static OUT: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU32 = AtomicU32::new(0);
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);

/// Sign in against the entry at `index` on a worker. `false` means the request was NOT started -
/// the OS refused the thread, or a switch is already out - and the caller has nothing to undo.
pub(crate) fn start(index: usize, entry: ServerEntry) -> bool {
    start_with(index, move || super::boot::sign_in(&entry))
}

/// [`start`] with the round trip supplied, which is how a host test drives the seam without a
/// server: the worker's only contract is "call this once, hand the answer back".
fn start_with(
    index: usize,
    sign_in: impl FnOnce() -> Option<JfClient> + Send + 'static,
) -> bool {
    if OUT.swap(true, Ordering::SeqCst) {
        crate::log("jellyfin: a server switch is already out - this press is ignored");
        return false;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut started) = STARTED.lock() {
        *started = Some(Instant::now());
    }
    let spawned = crate::task::spawn_small("jf-switch-server", move || {
        let client =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(sign_in)).unwrap_or(None);
        // An abandoned generation must not overwrite the switch that replaced it, and must not
        // wake a settled screen either.
        if generation == GENERATION.load(Ordering::SeqCst) {
            // A settled screen stops presenting, so the landing needs a frame to happen in. Done
            // BEFORE publishing: a reader that sees the answer knows the wake already happened.
            crate::ui::idle::invalidate();
            if let Ok(mut slot) = MAIL.lock() {
                *slot = Some(Outcome { generation, index, client });
            }
        }
    });
    if !spawned {
        OUT.store(false, Ordering::SeqCst);
        if let Ok(mut started) = STARTED.lock() {
            *started = None;
        }
    }
    spawned
}

/// The frame thread's read of the mailbox: the switch that finished, once. `None` while one is
/// still out, after the answer has been taken, and when an out switch outlived [`SWITCH_BUDGET`].
pub(crate) fn take() -> Option<(usize, Option<JfClient>)> {
    take_at(Instant::now(), SWITCH_BUDGET)
}

fn take_at(now: Instant, budget: Duration) -> Option<(usize, Option<JfClient>)> {
    let mut slot = MAIL.lock().ok()?;
    if let Some(out) = slot.take() {
        if out.generation != GENERATION.load(Ordering::SeqCst) {
            crate::log("jellyfin: an abandoned server switch answered - discarded");
            return None;
        }
        OUT.store(false, Ordering::SeqCst);
        if let Ok(mut started) = STARTED.lock() {
            *started = None;
        }
        return Some((out.index, out.client));
    }
    if !OUT.load(Ordering::SeqCst) {
        return None;
    }
    let expired = STARTED
        .lock()
        .ok()
        .and_then(|started| *started)
        .is_some_and(|started| now.saturating_duration_since(started) >= budget);
    if !expired {
        return None;
    }
    // Retire the generation so the abandoned worker's answer cannot land as the next switch's.
    GENERATION.fetch_add(1, Ordering::SeqCst);
    OUT.store(false, Ordering::SeqCst);
    if let Ok(mut started) = STARTED.lock() {
        *started = None;
    }
    crate::log("jellyfin: server switch abandoned after its budget - the picker is live again");
    None
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

    fn client() -> JfClient {
        let origin = crate::origin::Origin::parse("http://10.0.0.2:8096").unwrap();
        JfClient::synthetic(
            origin,
            "dev".to_string(),
            "tok".to_string(),
            "uid".to_string(),
            "someone".to_string(),
        )
    }

    /// Wait for the worker's answer with a deadline, so a broken latch fails the test instead of
    /// hanging the suite.
    fn landed() -> (usize, Option<JfClient>) {
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
        assert!(start_with(3, || Some(client())));
        let (index, client) = landed();
        assert_eq!(index, 3);
        assert!(client.is_some(), "the signed-in client travels to the frame thread");
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
            Some(client())
        }));
        assert!(is_out());
        assert!(!start_with(1, || Some(client())), "the second request is refused");
        let _ = tx.send(());
        let (index, _) = landed();
        assert_eq!(index, 0, "the row that was accepted is the one that lands");
    }

    /// A panicking round trip is a refused switch, not a process that loses its latch.
    #[test]
    fn a_panicking_round_trip_answers_refused_and_releases_the_latch() {
        let _guard = crate::testlock::serial();
        let _ = take();
        assert!(start_with(7, || panic!("the server answered with nonsense")));
        let (index, client) = landed();
        assert_eq!(index, 7);
        assert!(client.is_none(), "a refused switch installs nothing");
        assert!(!is_out());
    }

    /// **A switch that never answers cannot retire the picker.** The latch is released at the
    /// budget, a later press is accepted, and the abandoned worker's answer (old generation) is
    /// discarded instead of landing as the new switch's.
    #[test]
    fn an_out_switch_expires_and_its_late_answer_is_discarded() {
        let _guard = crate::testlock::serial();
        let _ = take();
        let (tx, rx) = mpsc::channel();
        assert!(start_with(0, move || {
            let _ = rx.recv_timeout(Duration::from_secs(5));
            Some(client())
        }));
        assert!(is_out());

        let after_budget = Instant::now() + SWITCH_BUDGET + Duration::from_secs(1);
        assert!(take_at(after_budget, SWITCH_BUDGET).is_none(), "the switch is abandoned");
        assert!(!is_out(), "and the picker accepts a press again");

        // A replacement starts while the abandoned worker is still out.
        let (tx2, rx2) = mpsc::channel();
        assert!(start_with(1, move || {
            let _ = rx2.recv_timeout(Duration::from_secs(5));
            Some(client())
        }));
        // The abandoned worker answers now; its generation is older than the live one.
        let _ = tx.send(());
        std::thread::sleep(Duration::from_millis(20));
        assert!(take().is_none(), "the abandoned answer is not mistaken for the live switch");
        let _ = tx2.send(());
        let (index, _) = landed();
        assert_eq!(index, 1, "the replacement is the one that lands");
    }
}
