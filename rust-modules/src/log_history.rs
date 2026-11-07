//! **The event log's history, across a reboot** (issue #331).
//!
//! The event log itself lives under the runtime root, which on a television is `/tmp`: `main.c`
//! opens it `O_TRUNC` before a single line of Rust runs, so every launch starts from an empty file
//! and a reboot takes the whole thing with it. A playback that stopped by itself twenty-eight
//! minutes in left nothing behind but the server's own line, which is exactly the case this module
//! exists for.
//!
//! So the run that is about to be truncated is COPIED first, into a directory that survives a
//! reboot, and the copies are rotated. The copy has to happen before `main.c`'s `open`, which is
//! why [`butaca_archive_events_log`] is an `extern "C"` entry point: C asks, Rust resolves, and
//! there is still exactly one definition of where anything lives (see `paths::runtime_dir`'s doc).
//!
//! # Why the durable candidates and not the runtime root
//!
//! [`crate::paths::session_candidates`] explains the ordering at length. The short version for
//! logs: `/media/developer` is writable in the Developer Mode jail and survives a reinstall,
//! `/media/internal` is the only persistent writable mount in the production jail, the app
//! directory is third on the theory that a future layout makes it writable, and `/tmp` is offered
//! last because it is swept on reboot. A log kept in `/tmp` is precisely the bug, so the last
//! candidate is not a candidate here.
//!
//! # The caps
//!
//! [`KEEP`] runs, [`CAP_BYTES`] each. A measured 14-minute session is about 190 KB, so a full
//! history is a handful of evenings of viewing and cannot grow without bound on a device with a
//! small writable partition. A file over the cap keeps its TAIL, not its head: the interesting
//! part of a log is the end.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// How many previous runs are kept. `1` is the most recent.
pub(crate) const KEEP: usize = 4;

/// Per-file ceiling on the archived copy. 190 KB is a measured 14-minute session, so this is
/// roughly twenty of them, and four files at the ceiling is 16 MiB in the worst case.
pub(crate) const CAP_BYTES: u64 = 4 * 1024 * 1024;

/// `<dir>/butaca-events.<i>.log`, `1` being the most recent archived run.
fn slot(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("butaca-events.{i}.log"))
}

/// **The directory the history lives in**, first writable durable candidate, or `None`.
///
/// Writability is proved, not assumed: the directory is created and a probe file is written and
/// removed. A candidate that exists but refuses a write is skipped rather than trusted, which is
/// the failure a jailed television actually produces (`/media/internal` is a read-only mount in
/// some profiles, and the app directory is `root:5000 0755`).
pub(crate) fn history_dir() -> Option<PathBuf> {
    let id = crate::paths::app_id();
    let candidates = [
        PathBuf::from(format!("/media/developer/{id}-logs")),
        PathBuf::from(format!("/media/internal/.{id}-logs")),
        crate::paths::in_app_dir("logs"),
    ];
    candidates.into_iter().find(|dir| writable(dir))
}

/// Can this directory hold a file? Creates it if needed, then writes and removes a probe.
fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".probe");
    let ok = write_0600(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// **Rotate one run into `dir`.** Shifts every slot down, drops the oldest, and copies `src` into
/// slot 1 keeping at most `cap` bytes of its tail.
///
/// Split out from the entry point so the policy can be tested against a temporary directory: the
/// real one names `/media/...`, which does not exist off a television.
pub(crate) fn rotate(dir: &Path, src: &Path, keep: usize, cap: u64) -> std::io::Result<()> {
    if keep == 0 {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(slot(dir, keep));
    for i in (1..keep).rev() {
        let from = slot(dir, i);
        if from.exists() {
            // A rename that fails must not take the whole rotation with it: the older slot is
            // already gone, so the alternative is losing this run as well.
            let _ = std::fs::rename(&from, slot(dir, i + 1));
        }
    }
    let bytes = std::fs::read(src)?;
    let tail: &[u8] = if bytes.len() as u64 > cap {
        let skip = bytes.len() - cap as usize;
        // Start at a line boundary so the file opens on a whole record, and never at a byte that
        // continues a multi-byte character.
        let mut at = skip;
        while at < bytes.len() && bytes[at] != b'\n' {
            at += 1;
        }
        &bytes[(at + 1).min(bytes.len())..]
    } else {
        &bytes
    };
    write_0600(&slot(dir, 1), tail)
}

/// Write a file at the same 0600 the live log is created with.
///
/// `std::fs::write` creates 0666 & ~umask, which on a television is 0644 in a world-readable
/// `/tmp`, and the archived lines carry the same scrubbed-but-still-sensitive content as the live
/// ones (server name, episode titles, item identifiers). The incident that put 0600 on the live
/// log is documented in `src/main.c`; a copy must not reopen it.
fn write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

/// **Archive the run that is about to be truncated**, then get out of the way.
///
/// Called by `src/main.c` immediately before it opens the event log `O_TRUNC`. Every failure is
/// swallowed on purpose: this runs before the app can report anything, and a device that cannot
/// write a log history must still start.
///
/// It must not log (see `paths::butaca_runtime_path`'s doc on re-entering the resolver), and it
/// allocates nothing global.
#[no_mangle]
pub extern "C" fn butaca_archive_events_log() {
    let src = crate::paths::in_runtime_dir(crate::paths::runtime_file::EVENTS);
    if !src.exists() {
        return;
    }
    if let Some(dir) = history_dir() {
        let _ = rotate(&dir, &src, KEEP, CAP_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("butaca-loghistory-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// The point of the whole module: the run that `main.c` is about to truncate is still readable
    /// afterwards, from a directory that is not the runtime root.
    #[test]
    fn the_previous_run_survives_the_truncation_that_follows() {
        let dir = scratch("survives");
        let src = dir.join("butaca-events.log");
        std::fs::write(&src, b"boot\nplay\nstop\n").unwrap();
        rotate(&dir, &src, KEEP, CAP_BYTES).unwrap();
        // What `main.c` does next.
        std::fs::write(&src, b"").unwrap();
        let kept = std::fs::read_to_string(slot(&dir, 1)).unwrap();
        assert_eq!(kept, "boot\nplay\nstop\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rotation is a shift, not a pile: four runs kept, the fifth pushes the oldest out, and slot
    /// 1 is always the newest.
    #[test]
    fn the_oldest_run_is_dropped_and_slot_one_is_the_newest() {
        let dir = scratch("rotate");
        for run in 1..=5 {
            let src = dir.join("butaca-events.log");
            std::fs::write(&src, format!("run {run}\n")).unwrap();
            rotate(&dir, &src, 3, CAP_BYTES).unwrap();
        }
        let kept: Vec<String> = (1..=3)
            .map(|i| std::fs::read_to_string(slot(&dir, i)).unwrap())
            .collect();
        assert_eq!(kept, vec!["run 5\n", "run 4\n", "run 3\n"]);
        assert!(!slot(&dir, 4).exists(), "only the declared slots exist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Over the cap, what is kept is the END of the log, on a whole line: the failure being
    /// diagnosed is the last thing that happened.
    #[test]
    fn an_oversized_run_keeps_its_tail_on_a_line_boundary() {
        let dir = scratch("cap");
        let src = dir.join("butaca-events.log");
        std::fs::write(&src, b"first\nsecond\nthird\n").unwrap();
        rotate(&dir, &src, KEEP, 13).unwrap();
        let kept = std::fs::read_to_string(slot(&dir, 1)).unwrap();
        assert!(kept.ends_with("third\n"), "the tail is what survives: {kept:?}");
        assert!(!kept.starts_with("second"), "the cut lands on a line start: {kept:?}");
        assert!(kept.len() <= 13);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A device that refuses the write must not stop the app from starting, and a missing source
    /// is not an error to report either.
    #[test]
    fn a_failed_archive_is_silent() {
        let dir = scratch("silent");
        let missing = dir.join("nothing-here.log");
        assert!(rotate(&dir, &missing, KEEP, CAP_BYTES).is_err());
        // And the entry point itself never panics on that shape.
        butaca_archive_events_log();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The copy is as private as the original: 0600, not 0644 in a world-readable `/tmp`.
    #[test]
    fn the_archived_run_is_private_like_the_live_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("private");
        let src = dir.join("butaca-events.log");
        std::fs::write(&src, b"server: x\n").unwrap();
        rotate(&dir, &src, KEEP, CAP_BYTES).unwrap();
        let mode = std::fs::metadata(slot(&dir, 1)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the archive must not be readable by others");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The declared caps are the ones the docs state.
    #[test]
    fn the_declared_caps_are_four_runs_of_four_mebibytes() {
        assert_eq!(KEEP, 4);
        assert_eq!(CAP_BYTES, 4 * 1024 * 1024);
    }
}
