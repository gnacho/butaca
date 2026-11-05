//! `butaca-sim` — the desktop UI simulator.
//!
//! The same application core the television runs, linked against a desktop SDL2 and desktop GL
//! instead of LG's. It draws the real interface, against a real Jellyfin server, on a laptop.
//!
//! **What it is for.** The television serializes the entire dev loop: one set, one app instance,
//! and two `tests/run.py` jobs kill each other's app. That makes every UI change a queue. Several
//! simulators can run at once — each with its own instance root (`BUTACA_RUNTIME_DIR`), so each
//! has its own trigger namespace, its own remote FIFO and its own event log — which is what lets
//! independent UI and data-layer work proceed in parallel.
//!
//! **What it is NOT.** There is no video. The 29-symbol media seam does not exist off-device, and
//! `player::ffi`'s host arm reports the same "no video path" failure a television with no usable
//! ACB binding reports, so Play lands on the app's real failure read-out. Nor is it a substitute
//! for the device on anything the GPU decides: the `--fps` gates are calibrated to the SM9000's
//! Mali, and text rasterization here goes through a different FreeType. A green run on this binary
//! is evidence about a Mac. Layout, focus, navigation, and every byte of the data layer are
//! the parts that do transfer — and they are most of the UI work.
//!
//! Usage:
//!   butaca-sim [--write-synthetic-initial <path> <seed> <port> [flow12|settings=…]]
//! Environment:
//!   BUTACA_SERVER_HOST / BUTACA_SERVER_PORT   accepted and ignored: older scripts still pass
//!                                             them, but the app reads its server from the
//!                                             Jellyfin config, never from here
//!   BUTACA_RUNTIME_DIR                     this instance's trigger/FIFO/log root
//!   BUTACA_APP_DIR                         where appfont*.ttf and the icons live (repo `pkg/`)

use butaca_modules::butaca_run;
use std::os::raw::c_int;

fn main() {
    // Browser assets and runtime state live in Emscripten's virtual filesystem. Set these before
    // the shared app helpers resolve their directories; JS preRun code cannot mutate libc's
    // process environment because Emscripten keeps ENV inside its runtime closure.
    #[cfg(target_os = "emscripten")]
    {
        std::env::set_var("BUTACA_APP_DIR", "/app");
        std::env::set_var("BUTACA_RUNTIME_DIR", "/persist/butaca-emscripten");
    }

    // SIGPIPE needs no handling here, unlike `src/main.c:105`: that file installs `SIG_IGN` by hand
    // precisely because a C `main` skips Rust's `std::rt::init`. This IS a Rust main, so std has
    // already done it, and the first server socket closed mid-write cannot kill the process.

    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .or_else(|| std::env::var("BUTACA_SERVER_HOST").ok())
        .unwrap_or_default();
    if first == "--write-synthetic-initial" {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let result = (|| -> Result<(), &'static str> {
            let path = args.next().ok_or("missing output path")?;
            let seed = args.next().and_then(|value| value.parse::<u32>().ok()).ok_or("invalid seed")?;
            let port = args.next().and_then(|value| value.parse::<u16>().ok()).ok_or("invalid port")?;
            let settings = match args.next() {
                None => None,
                Some(value) if value == "flow12" || value.starts_with("flow12=") => Some(value),
                Some(value) if value.starts_with("settings=") =>
                    Some(value.trim_start_matches("settings=").to_string()),
                Some(_) => return Err("unsupported synthetic initial argument"),
            };
            if args.next().is_some() { return Err("unexpected argument"); }
            let encoded = butaca_modules::synthetic_home_initial(seed,port,settings)?;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
                .open(path).map_err(|_| "cannot create initial input file")?;
            file.write_all(encoded.as_bytes()).map_err(|_| "cannot write initial input file")
        })();
        if let Err(reason) = result { eprintln!("synthetic initial input: {reason}"); std::process::exit(2); }
        return;
    }
    // Older scripts still pass `[host] [port]`. The app takes neither: it reads its server from
    // the Jellyfin config, so the pair is consumed and ignored rather than rejected.
    let _ = args.next().or_else(|| std::env::var("BUTACA_SERVER_PORT").ok());

    // The instance root must exist before anything writes into it, and the event log is truncated
    // per launch — `src/main.c` does the same on the television, and `tests/run.py` relies on the
    // log starting empty to date its first line.
    let root = butaca_modules::sim_runtime_dir();
    if let Err(e) = std::fs::create_dir_all(&root) {
        eprintln!(
            "butaca-sim: cannot create runtime dir {}: {e}",
            root.display()
        );
        std::process::exit(1);
    }
    let events = butaca_modules::sim_events_log();
    // The banner is the FIRST line of the log, so a log read from the middle, tailed, or pasted
    // into an issue still declares what produced it. Every heartbeat additionally carries `sim=1`
    // (see `app.rs`'s SIM_TAG) — one marker at the top is easy to scroll past.
    let banner = "sim: THIS IS THE DESKTOP SIMULATOR, not a television. No video pipeline; \
                  frame rates here describe this Mac's GPU and must never be read as an fps gate.\n";
    if let Err(e) = std::fs::write(&events, banner) {
        eprintln!("butaca-sim: cannot truncate {}: {e}", events.display());
        std::process::exit(1);
    }

    eprintln!(
        "butaca-sim: server from the Jellyfin config runtime={} log={}",
        root.display(),
        events.display()
    );

    let rc = butaca_run(std::ptr::null());
    exit_without_atexit_teardown(rc)
}

/// Leave the process WITHOUT running libc's `atexit` handlers or shared-library destructors.
///
/// `butaca_run` returns with detached workers still alive — the sign-in PIN request, the poster
/// fetchers, anything else parked in libcurl — because nothing in the app joins them, and on the
/// television nothing has to. `std::process::exit` would call libc `exit`, whose handlers include
/// OpenSSL 3's `OPENSSL_cleanup`: it frees libcrypto's global tables underneath a worker that is
/// mid-handshake, and the simulator dies with SIGSEGV after it has already done its job. That is
/// what failed the Linux simulator CI launch intermittently (core dump: main thread in
/// `OPENSSL_cleanup` from `exit`, worker in `X509_STORE_load_file_ex` under `auth::mint_pin`).
///
/// Nothing that must survive the process lives in those handlers: the event log is appended
/// unbuffered, the app's own shutdown (telemetry flush included) already ran inside `butaca_run`,
/// and the two std streams are flushed here by hand.
fn exit_without_atexit_teardown(rc: c_int) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: `_exit` is async-signal-safe and takes no pointers; it ends every thread at once,
    // which is exactly the point.
    unsafe { libc::_exit(rc) }
}
