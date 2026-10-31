//! The `/tmp` developer-trigger surface, behind one door.
//!
//! This app is driven headlessly by ~44 files under `/tmp/butaca-*`: which screen to boot to,
//! which item to play, which URL to stream, whether to auto-press OK, which PMS token to use.
//! That is how `tests/run.py` and every capture scene work, and it is not going away.
//!
//! It must not exist in a public build. `/tmp` is the SHARED system `/tmp` in the production jail
//! too (mode 1777, both jail profiles), so on an ordinary user's TV every one of those files is a
//! behaviour switch any co-resident process can throw. Three are outright takeovers:
//! `butaca-token` beats the signed-in session (`app.rs`'s boot gate), `butaca-servers` hands
//! the app a whole additional server — an address AND the token to trust it with (see [`servers`])
//! — and `butaca-url` replaces the stream the player feeds.
//!
//! So every read goes through here, and here is `#[cfg]`-gated on the `devtriggers` feature. In a
//! `--no-default-features` build [`flag`] is `false` and [`read`] is `None` at COMPILE time, so
//! no trigger can be armed. Storage and diagnostics still use runtime files; none are developer
//! triggers.
//!
//! That does NOT keep a trigger's NAME out of the binary. A branch behind `flag`/`read` usually
//! folds away, but not reliably: once the answer is carried through a struct field (the
//! `nobudget` flag on `DevFlags`), the optimizer may keep the branch and its string literals
//! in a release build, and `ci/check-package.py` fails the package because it greps the shipped
//! bytes for every trigger name this module lists. So every statement whose literal names a trigger
//! (a log line saying `/tmp/butaca-…`) carries its own `#[cfg(feature = "devtriggers")]`. Never
//! rely on constant folding for this.
//!
//! Two rules for anything added later:
//!
//! 1. **Never open a `/tmp` path directly.** The grep that audits this (`/tmp/butaca-` outside
//!    this module and the unconditional log sinks) is the only thing keeping the property
//!    true. The two profiler logs are dev-only and listed in [`DIAG`] below.
//! 2. **A gate is not always a path.** `any_trigger_present` scans the whole directory and names
//!    no file at all — it was the one surface a literal-replacement sweep would have missed, and
//!    it silently changes which screen the app boots to. Structural surfaces that take no name
//!    (the capture listener's `INADDR_ANY` socket, the remote FIFO's `mkfifo`) are gated at their
//!    call sites in `app.rs` for the same reason.
//!
//! The unconditional LOG sinks are deliberately NOT here and stay in every build: they are creates, not
//! reads, they are how on-device crash triage works at all, and writing them is not a way for
//! another process to steer this one.
//!
//! **Since UI restructure phase 10, [`scenarios`] is where a read gets ACTED on.** This module is
//! still the one door onto `/tmp` itself (`flag`/`read`, below); `dev::scenarios` gathers every
//! ARM — the app-core code that calls through this door and reacts — that used to be scattered
//! across `app/boot.rs`, `app/run.rs`, `app/content.rs` and `app/mod.rs`, plus the per-arm state
//! (oscillator phases, retry latches) those arms used to keep on `App` itself. Read that module's
//! doc before adding a new trigger that `app/` consumes.

pub(crate) mod scenarios;

/// Files that are pure diagnostics rather than automation — see [`any_trigger_present`].
///
/// Every log this app writes belongs here, not just its trigger. `butaca-anim` was listed and
/// `butaca-anim.log` was not, while arming the overlay creates exactly that file and nothing
/// ever removes it (`make run` clears only the event log; `tests/run.py` spares every `*.log` by
/// design) — so a single historical anim session skipped the who's-watching picker on every later
/// boot, interactive ones included.
// `test` as well as the feature: `any_trigger_present` is the only caller and it is cfg'd out of a
// release build, but the test below asserts this list's contents and runs with default features.
#[cfg(any(feature = "devtriggers", test))]
const DIAG: [&str; 31] = [
    "butaca-diag.log",
    "butaca-events.log",
    "butaca-stderr.log",
    "butaca-crash.log",
    "butaca-anim.log",
    "butaca-profile",
    "butaca-gputime.jsonl",
    "butaca-hwcnt",
    "butaca-hwcnt.jsonl",
    // The render thread's own per-phase clock ([`crate::ui::profile`]'s CPU mode). DIAG for the
    // reason the two GPU profilers are: it observes a screen, and must not move the boot away
    // from the screen it was armed to observe.
    "butaca-cpuprof",
    "butaca-anim",
    "butaca-remote",
    "butaca-capture",
    "butaca-noidle",
    // The focus fingerprint ([`crate::focusprobe`]). Diagnostic for `noidle`'s reason and one of
    // its own: it only READS focus and writes a log line, and a (route × key) characterization
    // harness has to be able to observe the who's-watching picker, which a non-DIAG trigger would
    // suppress — the observer would remove the screen it was armed to watch.
    "butaca-focus",
    // The two OVERDRAW surfaces and the hero-ground fold ([`crate::ui::overdraw`],
    // `docs/backdrop-blur-profiling.md` Part 5). All three are measurement knobs whose whole
    // method is an A/B against an unmasked control leg — and a non-DIAG trigger suppresses the
    // who's-watching picker, so the control leg and the masked leg would boot to DIFFERENT
    // SCREENS and the difference between them would be the screen, not the class being priced.
    // That is the exact failure this list exists to stop, and it is invisible in the numbers.
    "butaca-overdraw",
    "butaca-drawmask",
    "butaca-heroground",
    // The FRAME BUDGET's A/B control leg (`ui/frame/budget.rs`, spec §8.1): admission as it was
    // before phase 11 — quota only, no time ceiling, no solo rule. DIAG for exactly the argument
    // the three above make: its whole method is an A/B against an unmasked control leg, and a
    // non-DIAG trigger would boot the two legs to DIFFERENT SCREENS, so what the numbers measured
    // would be the screen and not the admission rule.
    "butaca-nobudget",
    // LG's own GStreamer logging ([`arm_gst_logging`]) and the file it writes. Both are DIAG for
    // the same reason `butaca-profile` is: the whole point is to observe a playback that would
    // otherwise be unobservable, and a non-DIAG trigger would silently move the boot screen out
    // from under the very session being measured.
    "butaca-gstlog",
    "butaca-gst.log",
    // The forced Stats-for-nerds overlay. It only changes presentation, and grading a playback
    // case without the read-out risks a failure whose evidence was never put on screen.
    "butaca-stats",
    // A deliberate crash happens before the picker could ever be reached, so whether it suppresses
    // one is moot — but leaving it out of this list would be a silent inconsistency for the next
    // reader, and the honest reading is that it changes no screen.
    "butaca-crashtest",
    // The FRAME-DROP DETECTOR (`app.rs`, the `FRAMEDROP` line and `worstframe=`). It observes the
    // frame it is armed on and changes no screen; the harness's `worst_ceiling_ms` /
    // `stall_ceiling_ms` gates arm it under every fps scene, and a scene whose gate moved the boot
    // away from the screen it grades would fail as "never entered this screen".
    "butaca-framedrop",
    "butaca-imagecache-stats",
    "butaca-imagecache-bypass",
    // The deterministic RECORDER and its replay trigger (`ui/rec.rs`, NOT YET IN THE TREE — reserved
    // here first so the recorder cannot land as a non-DIAG trigger and move the boot screen out
    // from under the session it records; restructure spec §5.3). Both observe or reproduce a session and must not decide
    // which screen it starts on — a recording of the who's-watching picker has to be possible.
    // `recplay` is a NEW name: `butaca-replay[=N]` is the EOS replay COUNTER, non-DIAG, and
    // stays exactly as it is.
    "butaca-rec",
    "butaca-recplay",
    "butaca-guard", // diagnostic policy only; never changes the boot screen
    // Boot-latched Dolby Vision capability A/B. They alter only the platform answer used by the
    // route policy, so an experiment must not independently replace Home with the profile picker.
    "butaca-dvcaps0",
    "butaca-dvcaps1",
];

/// The triggers a CONTROLLED boot (`app::bootstrap`: the recorder, a replay, an explicit
/// `app-init`) may carry, as bare names. Anything else armed on a recording boot makes its typed
/// initial unsupported, so a replay cannot silently run under a trigger it never modelled.
///
/// One gated table rather than literals at each consumer, for the reason [`DIAG`] is gated: a
/// full trigger name in the release binary is exactly what `ci/check-package.py` grades as "dev
/// triggers compiled in", and `butaca-noidle` is its witness. A release build never arms a
/// trigger ([`armed_triggers`] is empty there), so it has no vocabulary to check against and the
/// accessors below answer "not supported" / "not listed" without naming one.
#[cfg(any(feature = "devtriggers", test))]
const CONTROLLED: &[&str] = &[
    "rec", "recplay", "focus", "noidle", "token", "app-init", "settings",
    "detail", "detailsec", "detailok", "filmography", "personcredits", "nowan",
];

/// Is the recorded trigger `trigger` (full `butaca-<name>` form, as [`armed_triggers`] lists
/// it) one a controlled boot supports? See [`CONTROLLED`].
#[cfg(any(feature = "devtriggers", test))]
pub(crate) fn controlled_trigger(trigger: &str) -> bool {
    trigger.strip_prefix("butaca-").is_some_and(|name| CONTROLLED.contains(&name))
}
#[cfg(not(any(feature = "devtriggers", test)))]
pub(crate) fn controlled_trigger(_trigger: &str) -> bool {
    false
}

/// Does a recorded trigger list (full names, as [`armed_triggers`] returns them) carry the
/// trigger `name` (bare)? The typed-initial counterpart of [`flag`]: it reads the list a boot was
/// captured with, never the filesystem. Always `false` in a release build, whose list is empty.
#[cfg(any(feature = "devtriggers", test))]
pub(crate) fn listed(triggers: &[String], name: &str) -> bool {
    triggers.iter().any(|trigger| trigger.strip_prefix("butaca-") == Some(name))
}
#[cfg(not(any(feature = "devtriggers", test)))]
pub(crate) fn listed(_triggers: &[String], _name: &str) -> bool {
    false
}

/// Is the trigger `name` (bare, without the `butaca-` prefix) present?
#[cfg(feature = "devtriggers")]
pub(crate) fn flag(name: &str) -> bool {
    path(name).exists()
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn flag(_name: &str) -> bool {
    false
}

/// [`flag`], answered ONCE for the whole process.
///
/// **A `dev::flag` is a `stat`, so a trigger read every frame is a syscall on the 60 fps path.**
/// Latching also fixes a correctness wrinkle that has nothing to do with cost: `tests/run.py`
/// clears `/tmp/butaca-*` between cases, so a later read can legitimately find the file gone
/// mid-run and a per-frame probe would change its answer half way through a case.
///
/// A macro rather than a function because the latch has to be a `static` per trigger, and a
/// function would need a map behind a lock — which is the thing being avoided. It lives HERE
/// because this module is the one door onto the `/tmp` surface; it was briefly a file-local macro
/// in `ui/widgets.rs`, which walled it off from the other per-frame `flag` callers
/// ([`crate::focusprobe::armed`] had already hand-rolled exactly this body, doc comment and all).
///
/// No `#[cfg]` arms, deliberately: [`flag`] is already `false` at COMPILE time without the
/// `devtriggers` feature, so a second gate here would only re-derive what the door behind it
/// guarantees.
///
/// ```ignore
/// crate::dev::latched_flag!(
///     /// `/tmp/butaca-flattabs` — the material off, for an A/B against the flat capsule.
///     fn flat_tabs_armed = "flattabs";
/// );
/// ```
macro_rules! latched_flag {
    ($(#[$m:meta])* $vis:vis fn $name:ident = $trigger:literal;) => {
        $(#[$m])*
        $vis fn $name() -> bool {
            static SEEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            *SEEN.get_or_init(|| $crate::dev::flag($trigger))
        }
    };
}
pub(crate) use latched_flag;

/// **Turn on the TELEVISION'S OWN GStreamer logging** — `/tmp/butaca-gstlog`.
///
/// This is the only instrument that can see inside LG's Dolby Vision chain. That chain is
/// `dvbin` → `h265parse` → `dvsplitter` → {`lxvideodec`, `dvmdparse`} → `dualsequencer`, all of it
/// closed, and the app's own logs stop at `Feed()`. Decompilation established that
/// `mediapipeline::PlayerFactory::create()` calls `gst_debug_is_active()` and, when it is, honours
/// `GST_DEBUG_FILE_OVERWRITE` / `GST_DEBUG_FILE` by installing its own log function — so these four
/// variables are read by libpf itself and need no cooperation from us beyond setting them.
///
/// **Timing is the whole reason this is here and not later.** Neither `libpf` nor `libplayerAPIs`
/// imports `gst_init`; they use LG's lazy `gst_cool_init_check`, which does not run until a player
/// is created. `plex_run` is therefore comfortably early — but anything that arms this AFTER the
/// first `Load` would be setting variables nobody reads again.
///
/// An empty trigger takes the five Dolby categories at level 6; content overrides the whole
/// `GST_DEBUG` spec, so `dvbin:9,dualsequencer:9` or `*:3` both work. The log goes to the runtime
/// directory beside the event log.
///
/// **Not free.** Level 6 on five categories is a lot of formatted I/O on an ARM TV and it is not a
/// setting to leave armed while measuring anything about frame pacing.
#[cfg(feature = "devtriggers")]
pub(crate) fn arm_gst_logging() {
    let Some(spec) = read("gstlog") else { return };
    let spec = if spec.is_empty() {
        "dvbin:6,dvsplitter:6,dvsplitter_algo:6,dvmdparse:6,dualsequencer:6".to_string()
    } else {
        spec
    };
    let log = crate::paths::in_runtime_dir(crate::paths::runtime_file::GST);
    // SAFETY: single-threaded here by construction — `plex_run` has not yet minted a worker, and
    // this runs before SDL init. `set_var` is only unsound against a concurrent reader.
    std::env::set_var("GST_DEBUG", &spec);
    std::env::set_var("GST_DEBUG_FILE", &log);
    std::env::set_var("GST_DEBUG_FILE_OVERWRITE", "enable");
    std::env::set_var("GST_DEBUG_NO_COLOR", "1");
    crate::log(&format!("gstlog: GST_DEBUG={spec} -> {}", log.display()));
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn arm_gst_logging() {}

/// The trigger's CONTENT, trimmed. `Some("")` for a trigger armed as an empty file — several
/// distinguish empty (take the default) from a value (`autoseek`, `library`, `marker`), so an
/// empty file must not read the same as an absent one.
#[cfg(feature = "devtriggers")]
pub(crate) fn read(name: &str) -> Option<String> {
    std::fs::read_to_string(path(name))
        .ok()
        .map(|s| s.trim().to_string())
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn read(_name: &str) -> Option<String> {
    None
}

/// Boot-latched main-thread checker escape hatch. File content must be exactly `log`.
#[cfg(feature = "threadcheck")]
pub(crate) fn guard_log_only() -> bool {
    static MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| read("guard").as_deref() == Some("log"))
}

/// **`/tmp/butaca-nowan` — refuse every name lookup, as a dead resolver would.**
///
/// The offline-mode reproduction. A household whose internet is down but whose LAN is up resolves
/// no public name at all: `plex.tv`, `discover.provider.plex.tv` and — the one that matters — the
/// `plex.direct` hostname the app persisted for its OWN server on the LAN. Nothing on a desk can
/// take the router's uplink away deterministically, so this trigger does it inside the app: while
/// armed, [`crate::net`], [`crate::curlio`] and [`crate::stream`] refuse any host that is not a
/// numeric literal, at the point where they would otherwise hand it to a resolver, and return the
/// same error a failed resolution returns. A name reaches the wire only when the request carries a
/// resolve pin ([`crate::origin::ResolvePin`]) — which is exactly what the fix provides, so the same
/// trigger shows the defect red and the fix green with no network condition arranged anywhere.
///
/// Content `slow` first sleeps the connect budget an API call would have spent waiting on a dead
/// resolver ([`crate::net::API`]'s `connect_s`), so a worker that would have stalled stalls here
/// too. Empty is the fast variant. Latched at first read like every per-frame trigger, and `None`
/// at COMPILE time without `devtriggers`, so a public binary carries no such switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NoWan {
    pub(crate) slow: bool,
}

pub(crate) fn no_wan() -> Option<NoWan> {
    static SEEN: std::sync::OnceLock<Option<NoWan>> = std::sync::OnceLock::new();
    *SEEN.get_or_init(|| {
        read("nowan").map(|s| NoWan {
            slow: s.trim() == "slow",
        })
    })
}

/// **Hold the Load-returned flag** — `butaca-holdload[=ms]`.
///
/// `Some(ms)` when armed (default 30000 for a bare/empty trigger, per its own `parse` fallback),
/// else `None`. `player::threads::load_thread` sleeps this many milliseconds right after the real
/// `sf_load` call returns and BEFORE `Shared::mark_native_load_returned` publishes that fact — so
/// issue #74 D.1's budget (the pump's `deferring` line, then, past `NATIVE_LOAD_BUDGET`, the
/// failure read-out) becomes observable on a real television on demand, rather than only on a
/// k5lp set that happens to hang there for real. Deliberately NOT `DIAG`: it changes playback
/// behaviour (a real Load attempt now waits), so arming it must suppress the who's-watching
/// picker like every other automation trigger.
#[cfg(feature = "devtriggers")]
pub(crate) fn holdload_delay_ms() -> Option<u64> {
    let raw = read("holdload")?;
    Some(if raw.is_empty() {
        30_000
    } else {
        raw.parse().unwrap_or(30_000)
    })
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn holdload_delay_ms() -> Option<u64> {
    None
}

/// Parse `/tmp/butaca-server=<slot>`, the optional server half of a direct-screen trigger.
///
/// A Plex `ratingKey` is only unique together with its server.  The original `butaca-play`
/// trigger predated the multi-server registry and therefore meant "that key on the current
/// server".  Keeping the slot in a separate trigger preserves that wire format for the regression
/// harness while allowing `tv-session --server N` to name the other half explicitly.
///
/// Bounds are checked here rather than left to `ServerId::from_raw`: that constructor also exists
/// for compact stores and deliberately accepts any `u16`; a hand-written dev trigger must not turn
/// an arbitrary number into a plausible identity and then silently fall back elsewhere.
#[cfg(any(feature = "devtriggers", test))]
fn parse_server_slot(s: &str) -> Result<u16, String> {
    let slot = s
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("{s:?} is not a server slot"))?;
    if (slot as usize) >= crate::plex::MAX_SERVERS {
        return Err(format!(
            "server slot {slot} is outside 0..{}",
            crate::plex::MAX_SERVERS
        ));
    }
    Ok(slot)
}

/// The optional registry slot selected for direct screen automation.
///
/// `None` means the old/current-server behaviour.  `Some(Err)` is intentionally distinct: a bad
/// explicit identity must fail closed rather than play the same numeric rating key from whichever
/// server happens to be current.
#[cfg(feature = "devtriggers")]
pub(crate) fn server_slot() -> Option<Result<u16, String>> {
    read("server").map(|s| parse_server_slot(&s))
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn server_slot() -> Option<Result<u16, String>> {
    None
}

/// **Crash the app on purpose** — `butaca-crashtest=<segv|abrt|bus|ill|trap|panic|unwind>`.
///
/// Not a feature. An INSTRUMENT for the instrument, and it exists because of the rule this repo
/// keeps re-learning: prove the instrument can see the thing before you read its silence, and
/// prove the recorder records before you trust an empty recording. The fallback C crash tracer is
/// the only witness: nothing else inspects the stopped process, and no reporting backend is
/// compiled into this build. Until
/// 2026-08-29 nothing had ever exercised it deliberately — which is how it went seven weeks with a
/// re-raise that did not re-raise, silently costing every crash its `WIFSIGNALED` status — and only
/// that: no crashd backtrace was lost, because this firmware writes no core and so produces none.
/// The log looked perfectly normal either way, which is the whole reason it went seven weeks.
///
/// `segv` is a genuine null write, so the kernel raises the signal from a faulting instruction and
/// the record carries a real faulting PC and a real `si_addr`. The rest go through `raise`, which
/// proves five of the seven `sigaction` calls took but cannot produce a meaningful PC.
///
/// `panic` is a Rust panic inside an `extern "C"` CALLBACK — the shape of `ff::read_cb` under
/// libav: the hook writes its `*** RUST PANIC` line, the unwind stops at the callback's own
/// boundary, and the process aborts with `plex_run`'s frame intact. That leaves BOTH a panic
/// record and a native SIGABRT envelope, the pair `telemetry::crashreport` must send as the one
/// panic.
///
/// `unwind` panics straight in here instead, so the unwind crosses `plex_run`'s own frame before
/// aborting at its `extern "C"` boundary. That used to drop the telemetry `Guard` on the way past
/// and stop the native backend before the abort landed (dev set, 2026-09-19), so no envelope was
/// written. `Drop for Guard` (`telemetry::native`) now checks `std::thread::panicking()` and
/// returns early instead of tearing the backend down, so the backend stays armed through the
/// unwind and the boundary abort still produces a native SIGABRT envelope (PR #168). `panic`
/// remains the shape worth exercising for a real panic under libav; `unwind` is the regression
/// test for the `Guard` fix itself.
///
/// **Compiled out of a release build** with the rest of `devtriggers`, so a shipped binary has no
/// path to it at all. Called after telemetry boot (so native capture can be armed) but before SDL
/// or any screen is created, so it remains reachable when the fault being chased prevents UI boot.
#[cfg(feature = "devtriggers")]
pub(crate) fn crash_on_purpose() {
    let Some(kind) = read("crashtest") else {
        return;
    };
    if kind == "panic" {
        extern "C" fn callback() {
            panic!("crashtest: deliberate panic");
        }
        crate::log("crashtest: DELIBERATE crash, kind=panic (aborts at an extern \"C\" callback)");
        callback();
        return;
    }
    if kind == "unwind" {
        crate::log(
            "crashtest: DELIBERATE crash, kind=unwind (unwinds plex_run to its extern \"C\" boundary)",
        );
        panic!("crashtest: deliberate unwinding panic");
    }
    // The signal numbers are Linux's, written out rather than taken from a libc crate: this crate
    // binds no libc, and these five have been stable in the Linux ABI since it had one.
    let sig = match kind.as_str() {
        "" | "segv" => 11,
        "abrt" => 6,
        "bus" => 7,
        "ill" => 4,
        "trap" => 5,
        other => {
            crate::log(&format!("crashtest: unknown kind {other:?} — not crashing"));
            return;
        }
    };
    // Logged BEFORE the fault, and flushed by `crate::log`'s own O_APPEND write, so the event log
    // says the death was deliberate. Without this line a deliberate crash is indistinguishable
    // from the real one somebody is hunting.
    crate::log(&format!(
        "crashtest: DELIBERATE crash, kind={kind} signal={sig}"
    ));
    if sig == 11 {
        // A real memory fault. `write_volatile` so the optimiser cannot decide a null write is
        // undefined and therefore removable — which it may, and then this proves nothing.
        unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
    }
    unsafe extern "C" {
        fn raise(sig: i32) -> i32;
    }
    unsafe { raise(sig) };
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn crash_on_purpose() {}

/// `butaca-softfloat`: the ARM half of `ui::motion`'s differential claim (spec §4.2). Runs the
/// 4,096-operand table through this binary's own arithmetic, logs its hash beside the host's
/// pinned one, and writes the words to `butaca-softfloat.tbl` in the runtime root so a
/// divergence can be diffed word by word. `make softfloat-probe` is the recipe.
#[cfg(feature = "devtriggers")]
pub(crate) fn softfloat_probe() {
    if !flag("softfloat") {
        return;
    }
    let host = crate::ui::motion::DIFFERENTIAL_HASH_HOST;
    let here = crate::ui::motion::differential_hash();
    crate::log(&format!(
        "softfloat: n={} hash={here:#018x} host={host:#018x} {}",
        crate::ui::motion::DIFFERENTIAL_N,
        if here == host { "MATCH" } else { "DIVERGE" }
    ));
    let mut t = Vec::new();
    crate::ui::motion::differential_table(&mut t);
    let body: String = t.iter().map(|w| format!("{w:08x}\n")).collect();
    let path = crate::paths::runtime_dir().join("butaca-softfloat.tbl");
    if let Err(e) = std::fs::write(&path, body) {
        crate::log(&format!("softfloat: table write failed: {e}"));
    }
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn softfloat_probe() {}

/// A test-only playback policy override from `butaca-quality`.
///
/// The server matrix grades established direct-play/remux/transcode routes. It must not silently
/// become an Auto-HLS matrix because the television happened to persist that user preference in
/// an earlier run. This is deliberately an in-memory boot override: writing the session would
/// make a test change the owner's real preference. Unknown and empty values fail closed by
/// producing no override.
pub(crate) fn playback_quality_override() -> Option<crate::plex::session::PlaybackQuality> {
    let value = read("quality")?;
    parse_playback_quality(&value)
}

/// The inverse of [`parse_playback_quality`], and it lives here so the two cannot drift.
///
/// The log line a test greps must carry the SAME string the trigger accepts. `Quality::label()`
/// is display text ("1080p \u{b7} 8 Mbps") and would make a case state its rung twice, in two
/// spellings, with nothing keeping them in step — which is the shape that rots.
pub(crate) fn quality_wire_name(q: crate::plex::session::PlaybackQuality) -> &'static str {
    use crate::plex::session::PlaybackQuality as Q;
    match q {
        Q::Auto => "auto",
        Q::Original => "original",
        Q::P1080High => "1080p_20_mbps",
        Q::P1080 => "1080p_8_mbps",
        Q::P720 => "720p_4_mbps",
        Q::P720Low => "720p_2_mbps",
        Q::P480 => "480p_720_kbps",
    }
}

fn parse_playback_quality(value: &str) -> Option<crate::plex::session::PlaybackQuality> {
    use crate::plex::session::PlaybackQuality;
    match value {
        "auto" => Some(PlaybackQuality::Auto),
        "original" => Some(PlaybackQuality::Original),
        "1080p_20_mbps" => Some(PlaybackQuality::P1080High),
        "1080p_8_mbps" => Some(PlaybackQuality::P1080),
        "720p_4_mbps" => Some(PlaybackQuality::P720),
        "720p_2_mbps" => Some(PlaybackQuality::P720Low),
        "480p_720_kbps" => Some(PlaybackQuality::P480),
        _ => None,
    }
}

/// **Switch the playback quality MID-PLAYBACK** — `butaca-qualityswitch=[gap=<ms>,]<q>[,<q>…]`.
///
/// The one thing [`playback_quality_override`] above cannot do. That is a BOOT override: it decides
/// what the playback starts as and is read once. What a person actually does at the television is
/// start something, watch it, and then change the quality while it plays — which re-asks the
/// routing question against a stream already on screen, reloads if the answer moved, and (on the
/// way out of Auto) tears down a running ABR controller. None of that is reachable from a boot
/// value, and none of it was reachable from a test at all.
///
/// Same vocabulary as `butaca-quality`, deliberately — one spelling of a rung across both
/// triggers, so a case cannot name a quality one of them accepts and the other silently ignores.
/// Same grammar as `butaca-autoseek`: an optional leading `gap=<ms>` then comma-separated steps
/// fired one per gap. There is no default gap and none is invented: with a single step no cadence
/// exists to state, and a case wanting several states its own, in the manifest, where a reader can
/// see it beside the assertions it enables.
///
/// **This writes the stored preference, and that is not incidental.** `route::set_quality` persists
/// through `plex::session::update`, because a person picking a rung means it. A test therefore
/// leaves the value behind — on the `debug` flavour's own session, never the install you watch
/// with, and `tests/run.py` writes `butaca-quality` on every server case, so the next boot
/// overrides whatever this left. Both halves are load-bearing; neither alone would make it safe.
fn parse_quality_switch_script(
    raw: &str,
) -> Option<(u32, Vec<crate::plex::session::PlaybackQuality>)> {
    let mut steps: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let mut gap_ms = None;
    if let Some(g) = steps.first().and_then(|f| f.strip_prefix("gap=")) {
        gap_ms = Some(g.parse().ok()?);
        steps.remove(0);
    }
    // A sequence needs an authored cadence. Without one, several valid rungs would all fire on
    // successive render loops: technically ordered, but not the mid-playback interactions the
    // trigger claims to reproduce.
    if steps.len() > 1 && gap_ms.is_none() {
        return None;
    }
    // Fail the WHOLE script when one rung is unparseable. Running a valid subset would still
    // change playback to a sequence nobody requested, merely without inventing a default rung.
    let qs: Vec<_> = steps
        .iter()
        .map(|t| parse_playback_quality(t))
        .collect::<Option<Vec<_>>>()?;
    if qs.is_empty() {
        None
    } else {
        Some((gap_ms.unwrap_or(0), qs))
    }
}

pub(crate) fn quality_switch_script() -> Option<(u32, Vec<crate::plex::session::PlaybackQuality>)> {
    parse_quality_switch_script(&read("qualityswitch")?)
}

/// One synchronized user Pause, optionally followed by Resume —
/// `butaca-autopause=[delay=<ms>,][at=<ms>,][hold=<ms>]`.
///
/// An empty file preserves the original paused-HUD capture contract: pause at the player's
/// ordinary six-second dev gate and stay paused. A non-empty script may delay that edge and name a
/// finite accepted hold. `at` also holds the edge until the PUBLISHED playhead has reached that
/// media position (and, in the simulator, stops the clock sink exactly on it, so the pause freezes
/// that position: `ffi_host.rs::stop_clock_at`): a wall-clock delay pauses wherever the host's
/// scheduling has got playback to, which moves from run to run, and the documentation's player
/// figure wants the same frame and the same clock every time. Unknown/duplicate/invalid fields fail the whole trigger closed; silently
/// substituting a duration would exercise a different interleaving from the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PauseScript {
    pub(crate) delay_ms: u32,
    pub(crate) hold_ms: Option<u32>,
    pub(crate) at_ms: Option<u32>,
}

fn parse_pause_script(raw: &str) -> Option<PauseScript> {
    if raw.trim().is_empty() {
        return Some(PauseScript {
            delay_ms: 0,
            hold_ms: None,
            at_ms: None,
        });
    }
    let mut delay_ms = None;
    let mut hold_ms = None;
    let mut at_ms = None;
    for field in raw
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
    {
        if let Some(value) = field.strip_prefix("delay=") {
            if delay_ms.is_some() {
                return None;
            }
            delay_ms = Some(value.parse().ok()?);
        } else if let Some(value) = field.strip_prefix("at=") {
            if at_ms.is_some() {
                return None;
            }
            at_ms = Some(value.parse().ok()?);
        } else if let Some(value) = field.strip_prefix("hold=") {
            if hold_ms.is_some() {
                return None;
            }
            let value: u32 = value.parse().ok()?;
            if value == 0 {
                return None;
            }
            hold_ms = Some(value);
        } else {
            return None;
        }
    }
    Some(PauseScript {
        delay_ms: delay_ms.unwrap_or(0),
        hold_ms,
        at_ms,
    })
}

pub(crate) fn pause_script() -> Option<PauseScript> {
    parse_pause_script(&read("autopause")?)
}

/// **Pin Auto's HLS ladder to one actuator, by request rate** — `butaca-abrpin=<kbps>`.
///
/// Measurement-only, for step M4 of `docs/adaptive-playback-plan.md`: reading a settled reserve at
/// a given rung means holding that rung for minutes, and nothing in the app could do that.
/// [`playback_quality_override`] above cannot serve, for two independent reasons — a non-Auto
/// quality returns `None` from `route::hls_abr_control` before a controller is ever constructed,
/// so it measures a different transport path entirely; and [`crate::plex::session::PlaybackQuality`]
/// has no mid-1080p points, while the ladder this pins has eight of them.
///
/// The value is the actuator's REQUEST rate (`Rung::kbps`) — 320, 720, 2000, 4000, 6000, 8000,
/// 10000, 12000, 14000, 16000, 18000, 20000, 22000 — because that is the number PMS is given and
/// the one identity in the catalog that does not move when somebody re-measures the server.
/// An unrecognised or empty value pins nothing, deliberately: a typo must leave Auto alone rather
/// than silently park playback on the bottom rung for a whole measurement session.
///
/// Compiled out with `devtriggers`, so a release build cannot be pinned at all.
pub(crate) fn abr_pin() -> Option<crate::abr::Rung> {
    let raw = read("abrpin")?;
    let kbps: u32 = raw.trim().parse().ok()?;
    crate::abr::Rung::from_request_kbps(kbps)
}

/// A raw dev payload in the runtime root, by bare NAME — only `sample.h264` and `sample.h265`,
/// which predate the `butaca-` prefix and feed the player a local Annex-B sample instead of a
/// stream. Everything else here is `butaca-<name>`; these two are the exception, so they get
/// their own door rather than a prefix they do not have.
///
/// It took an ABSOLUTE path until the flavour split, which left them as the last two runtime
/// surfaces still pinned to a shared `/tmp` while every other one had moved — harmless in itself
/// (two installs reading one sample is fine) but a hole in the rule that every runtime surface
/// resolves through [`crate::paths::in_runtime_dir`], and rules with holes stop being checkable.
/// NB the file now goes in the install's own root: `$(make -s print-rundir)/sample.h264`.
#[cfg(feature = "devtriggers")]
pub(crate) fn read_sample(name: &str) -> Option<Vec<u8>> {
    std::fs::read(crate::paths::in_runtime_dir(name)).ok()
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn read_sample(_name: &str) -> Option<Vec<u8>> {
    None
}

/// A stream to play and **the Load payload declaration to play it with**, with no library item
/// behind it — `/tmp/butaca-playurl`, the player-PIPELINE test tier's one entry point.
///
/// This is the trigger that makes the pipeline testable without Plex. `butaca-url` already
/// hands the engine a URL, and everything downstream of it — `stream.rs`, `ff.rs`, `aq.rs`, the
/// pump's `Feed()`, the ACB bind — is byte-identical to a real playback. What it CANNOT do is say
/// what the stream *is*: the Starfish `Load` payload takes its codecs from `route::stream_vcodec`
/// / `stream_acodec`, and its Dolby nodes from `stream_dovi` / `stream_immersive`, all five of
/// which are normally installed together by `route::apply_plan` from a PMS decision and replaced
/// together by later route transitions. So a URL-fed 4K HEVC file was declared to the television
/// as whatever the route happened to hold — in a fresh boot, the empty string, which falls through
/// [`crate::player::engine`]'s `_ =>` arm to `"AC3"` and an H264 payload. The declaration is
/// precisely what governs HEVC-vs-H264 payload selection, the `"AC3 PLUS"` naming trap, and both
/// Dolby nodes, so a tier that cannot set it cannot test them.
///
/// JSON, whole-file, one object. Chosen over a `key=value` line for three reasons: the DV node is
/// nested, `serde_json` is already a dependency and [`DevServer`] is the established precedent for
/// exactly this shape, and JSON contains no apostrophes — which matters because `tests/run.py`
/// writes triggers through a single-quoted `printf` with no escaping.
///
/// ```jsonc
/// {"url":"http://192.0.2.10:8020/pipe_hevc_eac3_4k_dovi_p8.mkv",
///  "vcodec":"hevc", "acodec":"eac3", "fps":23.976,
///  "dovi":{"profile":8,"bl_compat":1,"el_present":false},
///  "atmos":false}
/// ```
///
/// Deliberately **not** `Debug`, for [`DevServer`]'s reason one step removed: `url` is a free
/// string, and while the pipeline tier's own URLs carry no credentials, the field is the same
/// shape as `route::url()` — which for a real playback carries `X-Plex-Token` in its query. A
/// derived `Debug` is how that reaches a log the day someone points this trigger at a PMS part.
#[derive(serde::Deserialize, Clone)]
pub(crate) struct PlayUrl {
    /// `http://<dotted-quad>:<port>/<path>`. **A dotted quad, not a hostname** — `stream.rs` does
    /// no DNS on this path, so a name is a flat failure to open with nothing to read it by.
    #[serde(default)]
    pub(crate) url: String,
    /// The Load payload's video codec: `"hevc"` selects the H265 payload, anything else H264.
    #[serde(default)]
    pub(crate) vcodec: String,
    /// The Load payload's audio codec, in FFmpeg's spelling (`"eac3"`, not `"AC3 PLUS"`) — the
    /// engine does the LG-side renaming, which is the trap this tier exists to keep testing.
    #[serde(default)]
    pub(crate) acodec: String,
    /// Source frame rate for the Load `esInfo`; 0 omits it, exactly as a transcode does.
    #[serde(default)]
    pub(crate) fps: f64,
    /// Dolby Vision layering, for the payload's `contents.DolbyHdrInfo` node. Absent = none.
    #[serde(default)]
    pub(crate) dovi: PlayDovi,
    /// Dolby Atmos, for the payload's `contents.immersive` node.
    #[serde(default)]
    pub(crate) atmos: bool,
    /// Pipeline-tier Auto watchdog seam: whole Original wire bitrate. Zero leaves the ordinary
    /// Plex-free one-shot playback unchanged.
    #[serde(default)]
    pub(crate) auto_source_kbps: u32,
    /// Same-origin fixture HLS root used after the synthetic Original becomes unsustainable.
    /// Present only in debug/test artifacts; production playback obtains replacement URLs from
    /// PMS through `HlsAbrControl`.
    #[serde(default)]
    pub(crate) auto_hls_base: String,
    /// **Start in HLS instead of arriving there through a starvation.** The pipeline tier's ABR
    /// cases exist to exercise the HLS controller, and until 2026-08-27 their only way in was to
    /// declare an Original source rate no link could carry (900 000 kbps) and let the starvation
    /// horizon fire. That worked only because the horizon fired without checking whether the
    /// reserve was actually draining — on an unshaped link it was FILLING — so the entry depended
    /// on a defect, and it stopped working the moment the defect was fixed
    /// (`docs/measurements/local-original-blind.md`, `docs/measurements/orig-first-window-fallback.md`).
    /// With this set, `route::arm_auto_fixture` installs the post-fallback state directly and the
    /// controller runs from the first segment. `pipe_auto_original_slow_recover` deliberately does
    /// NOT set it: the transition is what that case grades.
    #[serde(default)]
    pub(crate) auto_start_hls: bool,
    /// **The SOURCE raster, which decides whether the 4K actuator is feasible at all.**
    ///
    /// `route::arm_auto_fixture` hardcoded `1920x1080`, and its comment gave the honest reason:
    /// an unknown source raster is treated as UNBOUNDED by
    /// [`HlsActuatorCatalog::limited_to`](crate::abr::HlsActuatorCatalog::limited_to), which makes
    /// the Uhd rung feasible — and `tests/serve_fixtures.py` served no 22000 rung, so a candidate
    /// there would 404 and read on the television as a rejected encoder. That is a fixture gap
    /// standing in for a policy, and it is what kept the plan's I9 blocked: with every
    /// `auto_network` case pinned to a 1080p source, `admits` deletes Uhd and the two entries the
    /// production table calls empirical are the two no case can reach.
    ///
    /// `[w, h]`. Absent or malformed keeps the 1080p default, so every existing case is unchanged
    /// by construction. The server now answers 22000 with a real 4K clip, so declaring 4K here
    /// selects a rung that exists rather than one that 404s.
    #[serde(default)]
    pub(crate) source_raster: Option<[u16; 2]>,
}

/// The four DV fields the Load payload actually decides on — [`crate::metadata::Dovi`]'s
/// decision half. The three descriptive fields (level, version, bl/rpu present) are read by the
/// tracks panel and by nothing on the playback path, so this trigger does not carry them.
#[derive(serde::Deserialize, Clone, Copy, Default)]
pub(crate) struct PlayDovi {
    /// 5 / 7 / 8. **Zero means no Dolby Vision at all** — it is what drives `present` below,
    /// rather than a separate flag that could disagree with it.
    #[serde(default)]
    pub(crate) profile: i64,
    /// `DOVIBLCompatID` — 0 none (P5) / 1 HDR10 / 2 SDR / 4 HLG.
    #[serde(default)]
    pub(crate) bl_compat: i64,
    /// An enhancement layer is present (P7).
    #[serde(default)]
    pub(crate) el_present: bool,
}

impl PlayDovi {
    /// The engine-facing record. `present` is DERIVED from a non-zero profile rather than carried
    /// separately: two fields that can disagree is a way to declare "Dolby Vision, profile 0",
    /// which is not a thing, and the harness would have to keep them in step by hand in every case.
    pub(crate) fn to_dovi(self) -> crate::metadata::Dovi {
        crate::metadata::Dovi {
            present: self.profile > 0,
            profile: self.profile,
            bl_compat: self.bl_compat,
            el_present: self.el_present,
            ..crate::metadata::Dovi::NONE
        }
    }
}

/// Parse the `playurl` trigger's content. Pure, so the host suite can pin it.
///
/// An `Err` rather than a defaulted object on malformed input, so a harness typo is visible: a
/// run whose declaration was silently dropped grades as "the payload is wrong", when the fault is
/// a typo in the harness. An empty `url` is an `Err` too — an all-defaults object would send the
/// engine looking for `butaca-url` instead and the case would play something else entirely.
#[cfg(any(feature = "devtriggers", test))]
fn parse_playurl(s: &str) -> Result<PlayUrl, String> {
    let p: PlayUrl = serde_json::from_str(s).map_err(|e| e.to_string())?;
    if p.url.is_empty() {
        return Err("no `url`".to_string());
    }
    Ok(p)
}

/// This boot's URL-and-declaration, if one was armed — `/tmp/butaca-playurl`.
///
/// `None` = not armed; `Some(Err)` = armed but unreadable, which the caller logs.
///
/// **Not memoized**, unlike [`servers`]: every `start_bufferfeed` re-reads it, because a seek that
/// escalates to a full reload tears the engine down and builds the payload again, and a
/// declaration that applied only to the first `Load` would make the second one silently wrong.
/// `servers` is memoized for the opposite reason — credentials are a property of the boot.
#[cfg(feature = "devtriggers")]
pub(crate) fn playurl() -> Option<Result<PlayUrl, String>> {
    read("playurl").map(|s| parse_playurl(&s))
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn playurl() -> Option<Result<PlayUrl, String>> {
    None
}

/// Is ANY non-diagnostic trigger armed? Used to skip the boot who's-watching picker, so that a
/// headless run lands on a deterministic Home.
///
/// This is the surface with no path literal: it `read_dir`s the runtime root and matches by
/// prefix, so in a release build it would still have run — and still have changed the boot
/// screen from a squatted file — after every named read had been compiled out.
///
/// **A trigger is a FILE.** Nothing else here names a path, so nothing else could have caught a
/// non-file entry whose name happens to match: a directory called `butaca-anything` sitting in
/// the runtime root would read as an armed trigger and permanently suppress the boot picker,
/// silently changing which screen this install comes up on with no line in any log. That became
/// reachable the moment two installs could share `/tmp` — the obvious name for a second install's
/// runtime root is exactly `butaca-<flavour>`, which is why `paths::resolve_runtime_dir` spells
/// it with a DOT instead. Two independent reasons is the right number for a failure this quiet.
#[cfg(feature = "devtriggers")]
pub(crate) fn any_trigger_present() -> bool {
    std::fs::read_dir(crate::paths::runtime_dir())
        .ok()
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| is_armed_trigger(&e)))
        .unwrap_or(false)
}

/// Every `butaca-*` FILE in the runtime root, by name (DIAG entries included), sorted — the
/// recorder's header lists them so a replay can say what the recording boot had armed. Names
/// only: a trigger's CONTENT can be a query or a path and never enters a recording.
#[cfg(feature = "devtriggers")]
pub(crate) fn armed_triggers() -> Vec<String> {
    armed_triggers_in(crate::paths::runtime_dir())
}
#[cfg(feature = "devtriggers")]
fn armed_triggers_in(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    // Runtime logs share the prefix and are not triggers
                    (n.starts_with("butaca-") && !n.ends_with(".log")).then_some(n)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn armed_triggers() -> Vec<String> {
    Vec::new()
}

#[cfg(feature = "devtriggers")]
fn is_armed_trigger(entry: &std::fs::DirEntry) -> bool {
    if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
        return false;
    }
    let name = entry.file_name();
    let name = name.to_string_lossy();
    name.starts_with("butaca-") && !DIAG.contains(&name.as_ref())
}
#[cfg(not(feature = "devtriggers"))]
pub(crate) fn any_trigger_present() -> bool {
    false
}

/// `true` when this build reads `/tmp` at all — for the one boot log line that says so, and for
/// call sites gating a whole subsystem (the capture listener, the remote FIFO) rather than a read.
pub(crate) const ENABLED: bool = cfg!(feature = "devtriggers");

/// The trigger's absolute path. `/tmp/butaca-<name>` on the television; see
/// [`crate::paths::runtime_dir`] for why a host build may put the whole namespace elsewhere.
///
/// `test` is in the cfg beside the feature, and only for a compile reason: the test below writes
/// through this door rather than through a literal, and it guards itself at RUNTIME on
/// [`ENABLED`] — but a runtime guard cannot stop a call from being compiled, so without this the
/// whole crate failed to build under `--no-default-features --test` (E0425, "cannot find function
/// `path` in module `super`"). A shipping release build is unchanged: `cfg(test)` is false there,
/// and the fn is gone exactly as before.
#[cfg(any(feature = "devtriggers", test))]
fn path(name: &str) -> std::path::PathBuf {
    crate::paths::in_runtime_dir(&format!("butaca-{name}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_slot_trigger_accepts_only_a_registry_slot() {
        assert_eq!(super::parse_server_slot("0"), Ok(0));
        assert_eq!(super::parse_server_slot("15"), Ok(15));
        for invalid in ["", "-1", "16", "1.0", "server-1"] {
            assert!(
                super::parse_server_slot(invalid).is_err(),
                "{invalid:?} must not silently target another server"
            );
        }
    }

    #[test]
    fn quality_trigger_accepts_only_persisted_policy_spellings() {
        use crate::plex::session::PlaybackQuality;

        assert_eq!(
            super::parse_playback_quality("auto"),
            Some(PlaybackQuality::Auto)
        );
        assert_eq!(
            super::parse_playback_quality("original"),
            Some(PlaybackQuality::Original)
        );
        assert_eq!(
            super::parse_playback_quality("720p_4_mbps"),
            Some(PlaybackQuality::P720)
        );
        for invalid in ["", "Auto", "720p", "unlimited"] {
            assert_eq!(super::parse_playback_quality(invalid), None, "{invalid}");
        }
    }

    /// The DIAG list must name every log the app writes, or that log permanently suppresses the
    /// boot picker. This asserts the property against the paths the code actually opens rather
    /// than against a copy of the list, so adding another log sink without listing it fails here.
    #[test]
    fn diag_names_every_log_this_app_writes() {
        for log in crate::paths::runtime_file::LOGS {
            assert!(super::DIAG.contains(&log), "{log} is written by this app but absent from DIAG — it would suppress the boot picker forever");
        }
    }

    #[cfg(feature = "devtriggers")]
    #[test]
    fn storage_diagnostics_is_never_an_armed_trigger() {
        let _serial = crate::testlock::serial();
        let root = std::env::temp_dir().join(format!(".plx-diag-trigger-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let _cleanup = Cleanup(root.clone());
        std::fs::write(root.join(crate::storage::diagnostics::NAME), b"schema=1\n").unwrap();
        let entry = std::fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(!super::is_armed_trigger(&entry));
        assert!(super::armed_triggers_in(&root).is_empty());
        std::fs::write(root.join("butaca-home"), b"").unwrap();
        assert_eq!(super::armed_triggers_in(&root), vec!["butaca-home"]);
    }

    /// A DIRECTORY whose name matches the trigger prefix must not read as an armed trigger.
    ///
    /// The failure it prevents is silent and permanent: `any_trigger_present` suppresses the boot
    /// who's-watching picker, so a squatted entry changes which screen this install comes up on
    /// with nothing logged anywhere. It became reachable when two installs started sharing `/tmp`
    /// — the second install's runtime root is a directory sitting right there.
    // `is_armed_trigger` itself is compiled out entirely under `--no-default-features` (it is
    // `devtriggers`-only, not merely dead code behind the runtime `ENABLED` check below), so this
    // test cannot exist in that build at all rather than just skip at runtime.
    #[cfg(feature = "devtriggers")]
    #[test]
    fn a_directory_is_not_an_armed_trigger() {
        if !super::ENABLED {
            return; // a release build reads nothing
        }
        // Test the exact entry rather than scanning the whole host /tmp. Developers legitimately
        // keep captured TV artifacts there, and their names are intentionally outside DIAG.
        let _g = crate::testlock::serial();
        // Per PROCESS, not a fixed name: the runtime dir is the host's /tmp here, shared with every
        // other `cargo test` on this Mac, and two suites running at once (a second checkout's
        // `make check`) removed each other's entry between the create and the read_dir.
        let d = crate::paths::in_runtime_dir(&format!(
            "butaca-notatrigger-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let entry = std::fs::read_dir(d.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.path() == d)
            .unwrap();
        let armed = super::is_armed_trigger(&entry);
        let _ = std::fs::remove_dir_all(&d);
        assert!(
            !armed,
            "a directory named {} read as an armed trigger",
            d.display()
        );
    }

    /// An empty trigger file and an absent one mean different things to several call sites
    /// (`autoseek` empty = one seek to 140s; `navosc` empty = Home <-> the first library section).
    #[test]
    fn empty_trigger_is_some_not_none() {
        if !super::ENABLED {
            return; // a release build reads nothing; nothing to distinguish
        }
        // Arms a real trigger in the shared runtime root, which is what
        // `a_directory_is_not_an_armed_trigger` scans — they must not overlap.
        let _g = crate::testlock::serial();
        // Write through `path()` itself, NOT a literal and NOT `env::temp_dir()`. The literal was
        // right when the namespace was always `/tmp/butaca-…`, but it stops meeting the read as
        // soon as an instance root is in effect; `env::temp_dir()` never met it at all, since on
        // the dev Mac that is a per-user `/var/folders/…/T/` path. Going through the same door the
        // code under test uses keeps the write and the read together wherever the root points.
        let p = super::path("devtest-empty");
        std::fs::write(&p, "").unwrap();
        let got = super::read("devtest-empty");
        let _ = std::fs::remove_file(p);
        assert_eq!(
            got.as_deref(),
            Some(""),
            "an empty trigger must not read as absent"
        );
    }

    // ---- butaca-playurl (the pipeline test tier's one trigger) ----

    /// The payload `tests/run.py` writes, verbatim. Pinned as a literal for `the same cross-language
    /// reason: nothing links the two languages at build time, so if this has to change, the
    /// harness's writer changes with it.
    #[test]
    fn the_harness_payload_parses_to_the_declaration_it_names() {
        let p = super::parse_playurl(
            r#"{"url":"http://192.0.2.10:8020/pipe_hevc_eac3_4k_dovi_p8.mkv","vcodec":"hevc",
                "acodec":"eac3","fps":23.976,
                "dovi":{"profile":8,"bl_compat":1,"el_present":false},"atmos":false}"#,
        )
        .unwrap();
        assert_eq!(
            p.url,
            "http://192.0.2.10:8020/pipe_hevc_eac3_4k_dovi_p8.mkv"
        );
        assert_eq!((p.vcodec.as_str(), p.acodec.as_str()), ("hevc", "eac3"));
        assert!((p.fps - 23.976).abs() < 1e-9);
        assert!(!p.atmos);
        let dv = p.dovi.to_dovi();
        assert!(dv.present, "profile 8 must derive present=true");
        assert_eq!((dv.profile, dv.bl_compat, dv.el_present), (8, 1, false));
    }

    /// The baseline case declares two fields and omits the rest, which must mean "no Dolby
    /// anything" — not a parse failure and not an inherited value.
    #[test]
    fn omitted_fields_default_to_a_silent_declaration() {
        let p = super::parse_playurl(
            r#"{"url":"http://10.0.0.2:8020/a.mkv","vcodec":"h264","acodec":"ac3"}"#,
        )
        .unwrap();
        assert_eq!(p.fps, 0.0);
        assert!(!p.atmos);
        let dv = p.dovi.to_dovi();
        assert!(!dv.present);
        assert_eq!(
            dv,
            crate::metadata::Dovi::NONE,
            "an absent dovi node must be silence itself"
        );
    }

    /// The synthetic Auto case carries only public fixture coordinates and a declared source
    /// rate. Keep that cross-language seam pinned separately from the ordinary declaration: a
    /// parser that silently defaults either field would play Original forever and make the TV
    /// network-profile case grade the wrong path.
    #[test]
    fn the_auto_fixture_fields_reach_the_runtime_verbatim() {
        let p = super::parse_playurl(
            r#"{"url":"http://192.0.2.10:8020/original.mp4","vcodec":"h264","acodec":"aac",
                "auto_source_kbps":8000,"auto_hls_base":"http://192.0.2.10:8020/__abr"}"#,
        )
        .unwrap();
        assert_eq!(p.auto_source_kbps, 8_000);
        assert_eq!(p.auto_hls_base, "http://192.0.2.10:8020/__abr");
    }

    /// `present` is DERIVED, so it cannot disagree with the profile in either direction.
    #[test]
    fn dovi_presence_follows_the_profile() {
        let none = super::PlayDovi {
            profile: 0,
            bl_compat: 1,
            el_present: true,
        }
        .to_dovi();
        assert!(
            !none.present,
            "profile 0 is not Dolby Vision whatever else is set"
        );
        let p7 = super::PlayDovi {
            profile: 7,
            bl_compat: 6,
            el_present: true,
        }
        .to_dovi();
        assert!(p7.present);
        assert_eq!((p7.profile, p7.bl_compat, p7.el_present), (7, 6, true));
    }

    /// An empty `url` must be an Err, not an all-defaults object: on `Ok` the engine would take
    /// the empty URL, fall through to `butaca-url` or a local sample, and PLAY SOMETHING ELSE —
    /// a case grading a stream it was never pointed at. The same cross-language contract as the
    /// ordering trap, in different clothes.
    #[test]
    fn an_empty_url_is_refused_rather_than_defaulted() {
        assert!(super::parse_playurl(r#"{"vcodec":"hevc"}"#).is_err());
        assert!(super::parse_playurl(r#"{"url":""}"#).is_err());
        assert!(super::parse_playurl("").is_err());
        assert!(super::parse_playurl("not json at all").is_err());
    }

    /// The trigger decides WHAT THE APP PLAYS. Listing it in DIAG would leave a headless pipeline
    /// run booting to the who's-watching picker — with no session, to the sign-in screen — instead
    /// of into the player.
    #[test]
    fn playurl_trigger_is_not_diagnostic() {
        assert!(!super::DIAG.contains(&"butaca-playurl"));
    }

    /// The harness writes triggers through a single-quoted `printf` with NO escaping
    /// (`tests/run.py::apply_triggers`), so an apostrophe anywhere in the payload would end the
    /// quoting and hand the rest to the TV's shell. JSON has no apostrophe in its syntax; this
    /// pins that the fields we generate carry none either.
    #[test]
    fn the_harness_payload_carries_no_apostrophe() {
        let payload = r#"{"url":"http://192.0.2.10:8020/pipe_h264_ac3_1080p.mkv","vcodec":"h264","acodec":"ac3","fps":24.0}"#;
        assert!(
            !payload.contains('\''),
            "would break apply_triggers' single-quoted printf"
        );
        assert!(super::parse_playurl(payload).is_ok());
    }

    /// **One vocabulary for a rung, in both directions.** `butaca-quality` and
    /// `butaca-qualityswitch` accept these strings and `quality: switch → …` prints them, so a
    /// case states its rung once and asserts on the same word. A one-way table would let the log
    /// drift from what the trigger accepts and the drift would show up as a case that arms
    /// correctly and then matches nothing — indistinguishable from the feature not working.
    #[test]
    fn every_quality_wire_name_parses_back_to_itself() {
        use crate::plex::session::PlaybackQuality as Q;
        for q in [
            Q::Auto,
            Q::Original,
            Q::P1080High,
            Q::P1080,
            Q::P720,
            Q::P720Low,
            Q::P480,
        ] {
            let name = super::quality_wire_name(q);
            assert_eq!(
                super::parse_playback_quality(name),
                Some(q),
                "{name} does not round-trip"
            );
        }
    }

    /// The script grammar, including the two ways it must FAIL CLOSED. A typo that resolved to a
    /// default would switch the playback to something the case never asked for — the same hazard
    /// `butaca-abrpin` documents for its own value.
    #[test]
    fn a_quality_script_fails_closed_and_never_substitutes() {
        use crate::plex::session::PlaybackQuality as Q;
        let parse = super::parse_quality_switch_script;
        assert_eq!(
            parse("720p_4_mbps"),
            Some((0, vec![Q::P720])),
            "one step needs no cadence"
        );
        assert_eq!(
            parse("gap=9000,1080p_8_mbps,auto"),
            Some((9_000, vec![Q::P1080, Q::Auto])),
            "a leading gap is consumed, not treated as a rung",
        );
        assert_eq!(
            parse("gap=9000,nonsense"),
            None,
            "a typo must arm NOTHING, not a default"
        );
        assert_eq!(
            parse("gap=oops,auto"),
            None,
            "an invalid cadence must not become zero"
        );
        assert_eq!(
            parse("720p_4_mbps,auto"),
            None,
            "a multi-step script must state its cadence",
        );
        assert_eq!(parse(""), None);
        assert_eq!(
            parse("nonsense,auto"),
            None,
            "one invalid rung must not leave a valid subset running",
        );
    }

    #[test]
    fn a_pause_script_preserves_the_legacy_hold_and_fails_closed() {
        let parse = super::parse_pause_script;
        assert_eq!(
            parse(""),
            Some(super::PauseScript {
                delay_ms: 0,
                hold_ms: None,
                at_ms: None,
            }),
            "the empty screenshot trigger remains a permanent Pause",
        );
        assert_eq!(
            parse("delay=25000,hold=6000"),
            Some(super::PauseScript {
                delay_ms: 25_000,
                hold_ms: Some(6_000),
                at_ms: None,
            }),
        );
        assert_eq!(
            parse("at=432000"),
            Some(super::PauseScript {
                delay_ms: 0,
                hold_ms: None,
                at_ms: Some(432_000),
            }),
            "a position-gated pause: the published playhead, not a wall-clock delay",
        );
        assert_eq!(parse("at=1,at=2"), None);
        assert_eq!(parse("at=soon"), None);
        assert_eq!(parse("hold=0"), None);
        assert_eq!(parse("delay=10,delay=20"), None);
        assert_eq!(parse("hold=oops"), None);
        assert_eq!(parse("resume=6000"), None);
    }
}
