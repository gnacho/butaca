#!/usr/bin/env python3
"""Re-record a committed replay fixture against today's mock Jellyfin server.

**Why this exists.** A recording is pinned to the build lineage that took it, and the gate that
holds it (`make check-replay`) fails the day the app legitimately moves under it. The specs answer
that with re-recording, and the harness that used to do it is retired: `tests/focusfp.sh` exits 2
with a note saying its world went away with the Plex wire. Nothing else drove a flow and armed the
recorder, so a fixture that diverges has had no way back to green except hand-editing hashes, which
is not a rebaseline, it is a lie.

**What it does.** Boots the simulator in the recorder's controlled mode against `tests/mock_jf.py`
in catalog mode, writes the typed synthetic initial the boot needs, drives the flow through the
remote FIFO, and lets the app stop itself (`BUTACA_SHOT_SETTLE`/`BUTACA_SHOT_EXIT`) so the recorder
flushes through its ordinary shutdown. Then it verifies the result with `tools/butaca-rec check`.

**What it does NOT do.** It never rebaselines. Which fixture a new recording replaces, and why, is
a judgement the tool cannot make, and `tools/butaca-rec rebaseline` already demands the replay's
own divergence record plus a written reason for an anchor. This driver hands you a recording that
passes `check` and the command line to act on it.

**The world is the GENERATED one, never the demo catalog.** `tools/butaca-rec check` holds a
committed recording to a closed alphabet (`tests/fixtures/replay/ALPHABET.json`), and the demo
catalog would fail it on the first real film title. `tests/mock_jf.py`'s seeded world speaks that
alphabet by construction (`sname`), which is what makes a re-recorded fixture committable at all.

**The world is not the original world.** Fixture 12's item id (`57f4111...`) belongs to the
synthetic server of the day, which no longer exists, so a re-record is the same FLOW SHAPE against
today's generated world: boot flow 12 on a seeded film's id and press the same keys. The presses are
the fixture's own (read out of its `in` records), not invented here.

Usage:

    python3 tests/record_fixture.py                      # the default flow
    python3 tests/record_fixture.py --movie 3            # another film of the same seed
    python3 tests/record_fixture.py --keys down,down,ok,back,back --keep

The instance root is printed, with the recording under `<root>/butaca-recordings/latest` and the
event log beside it. On success the exit code is 0 and the last line is the `rebaseline` command
to run once the divergence has been read and accepted.
"""

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
import mock_jf  # noqa: E402  (the repo's own server, imported by path like tools/screenshots.py)

DEFAULT_SEED = 1
DEFAULT_MOVIE = 0
# The fixture's own presses, in its own order: `down, down, ok, back, back`. Each key is a DOWN and
# an UP edge on the wire; the FIFO token is the pair, so one token per press.
DEFAULT_KEYS = "down,down,ok,back,back"
FIXTURE = "12-filmography-detail-return"
# **The fixture's OWN cadence**, read out of its `in` records (637, 1703, 2703, 3738, 4719 ms): the
# first press lands just after the detail arm opens the page at 500 ms, and the presses are ~1 s
# apart. It is not a copy for its own sake: a press that arrives before the page it acts on exists
# is a press on a different screen, and on Home a SECOND Back at the root is a `BackAtRoot` request,
# which a controlled world refuses.
FIRST_PRESS_S = 0.64
PRESS_GAP_S = 1.03
# **The app stops itself on a FRAME COUNT, not on idleness.** A recording boot presents every frame
# (`rec=` in the heartbeat), so `ui::idle`'s "nothing changed" signal never arrives and a settle
# trigger would run forever. A frame is a fact the boot controls; a wall clock is not.
STOP_FRAME = 720


def fail(msg):
    print(f"record-fixture: {msg}", file=sys.stderr)
    sys.exit(1)


def write_initial(binary, rt, port, item_id):
    """The typed synthetic world, from the simulator's own encoder: it refuses to invent an id."""
    path = rt / "butaca-app-init"
    proc = subprocess.run(
        [str(binary), "--write-synthetic-initial", str(path), "1", str(port), f"flow12={item_id}"],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        fail(f"the synthetic initial was refused: {proc.stderr.strip() or proc.stdout.strip()}")
    if not path.exists():
        fail("the synthetic initial was not written")
    return path
    return path


def send_keys(fifo, keys):
    """One FIFO token per press, spaced. The app drains the FIFO once per frame."""
    time.sleep(FIRST_PRESS_S)
    for token in keys:
        with open(fifo, "w") as f:
            f.write(token + "\n")
        time.sleep(PRESS_GAP_S)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", type=pathlib.Path,
                    default=ROOT / "rust-modules" / "target-sim" / "release" / "butaca-sim")
    ap.add_argument("--seed", type=int, default=DEFAULT_SEED, help="the generated world's seed")
    ap.add_argument("--movie", type=int, default=DEFAULT_MOVIE, help="which film of it to open")
    ap.add_argument("--keys", default=DEFAULT_KEYS, help="comma-separated FIFO tokens")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--keep", action="store_true", help="keep the instance root even on success")
    args = ap.parse_args()

    binary = args.binary.resolve()
    if not binary.exists():
        fail(f"no simulator at {binary} (build it: cargo build --release --manifest-path "
             f"rust-modules/Cargo.toml --target-dir rust-modules/target-sim --features hostsim "
             f"--bin butaca-sim)")
    item_id = mock_jf.sid(args.seed, "movie", args.movie)
    keys = [k.strip() for k in args.keys.split(",") if k.strip()]

    srv, _ = mock_jf.serve(0, seed=args.seed)
    threading.Thread(target=mock_jf.run, args=(srv,), daemon=True).start()
    port = srv.server_address[1]
    rt = pathlib.Path(tempfile.mkdtemp(prefix=f"butaca-record-s{args.seed}m{args.movie}-"))
    print(f"record-fixture: {FIXTURE} on seed {args.seed} movie {args.movie} ({item_id})")
    print(f"record-fixture: mock on 127.0.0.1:{port}, instance root {rt}")
    try:
        write_initial(binary, rt, port, item_id)
        # The recorder's trigger file: EMPTY is the record mode (`Preflight::detect`). The other two
        # are what make a controlled boot repeatable: no idle osc, and the focus fingerprints the
        # fixture's own header lists.
        (rt / "butaca-rec").write_text("")
        (rt / "butaca-noidle").write_text("")
        (rt / "butaca-focus").write_text("")
        # **The flow's own entry points, as files.** The synthetic initial seeds the content STORE;
        # the route the app is on at frame 0 comes from the same triggers a live boot reads, and the
        # fixture's header lists exactly these (`bootstrap::ContentInitial` mirrors their values):
        # `butaca-detail=<rk>` opens the page, and `detailsec`/`detailok`/`filmography` are the
        # steps the scene then takes.
        (rt / "butaca-detail").write_text(item_id)
        (rt / "butaca-detailsec").write_text("1")
        (rt / "butaca-detailok").write_text("")
        (rt / "butaca-filmography").write_text("")
        (rt / "butaca-personcredits").write_text("9")
        # **NOT `butaca-nowan`.** That trigger refuses every name lookup, as a dead resolver would,
        # and a re-record has to REACH the mock for its detail and person results. The committed
        # fixture lists it because its world supplied those results at the store instead of over the
        # wire (replay's job, not a record's): armed here, the page logs "no metadata (server
        # unresolved)" and never becomes readable, which is what the first run of this harness did.
        fifo = rt / "butaca-remote"

        env = {k: v for k, v in os.environ.items() if not k.startswith("BUTACA_")}
        env.update({
            "BUTACA_RUNTIME_DIR": str(rt),
            "BUTACA_APP_DIR": str(ROOT / "pkg"),
            "BUTACA_SHOT": str(rt / "shot.png"),
            "BUTACA_SHOT_FRAME": str(STOP_FRAME),
            "BUTACA_SHOT_EXIT": "1",
        })
        out = open(rt / "sim.out", "w")
        proc = subprocess.Popen([str(binary), "127.0.0.1", str(port)], env=env,
                                stdout=out, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 10.0
            while not fifo.exists():
                if time.monotonic() > deadline:
                    fail("the remote FIFO never appeared (the app did not get past boot)")
                time.sleep(0.05)
            send_keys(fifo, keys)
            rc = proc.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            rc = None
        finally:
            out.close()
        log = (rt / "butaca-events.log").read_text(errors="replace")
        # **The boundary this harness cannot cross, named.** A record boot is refused a live
        # server ("unsupported replay bootstrap authority"), so it runs the synthetic DevJf world,
        # and that world is `nowan: true` by contract (the app refuses `nowan: false` with
        # "unsupported initial content input"). Its detail and person results therefore never
        # arrive: the page logs the line below and every press lands on an empty page. Shipping a
        # re-record past this needs the recorder's transport guard to allow the mock, which is a
        # product decision, not a harness one. See issue #416.
        blind = [l for l in log.splitlines() if "no metadata" in l]
        if blind:
            fail("the world never read: the detail fetch answered \"no metadata (server "
                 "unresolved, or it refused)\" because a record boot may not reach a server. "
                 "The presses then act on an empty page. See issue #416; the log is "
                 f"{rt / 'butaca-events.log'}")
        if rc is None:
            fail(f"the run never reached frame {STOP_FRAME}; log tail:\n    "
                 + "\n    ".join(log.splitlines()[-12:]))
        if rc != 0:
            fail(f"the simulator exited {rc}; log tail:\n    " + "\n    ".join(log.splitlines()[-12:]))
        refused = [l for l in log.splitlines() if "REFUSED" in l or "BADTRIGGER" in l]
        if refused:
            fail("the boot refused a trigger: " + refused[0])
        if srv.unknown:
            fail(f"the mock could not answer: {sorted(set(srv.unknown))[:5]}")

        recording = rt / "butaca-recordings" / "latest"
        if not (recording / "manifest.json").exists():
            fail(f"no recording at {recording}; log tail:\n    "
                 + "\n    ".join(log.splitlines()[-12:]))
        check = subprocess.run([str(ROOT / "tools" / "butaca-rec"), "check", str(recording)],
                               capture_output=True, text=True)
        info = subprocess.run([str(ROOT / "tools" / "butaca-rec"), "info", str(recording)],
                              capture_output=True, text=True)
        print(info.stdout.strip())
        if check.returncode != 0:
            fail(f"the recording does not pass `butaca-rec check`:\n{check.stdout}{check.stderr}")
        print(f"record-fixture: recording passes check: {recording}")
        print(f"record-fixture: next, read the divergence, then:\n"
              f"  tools/butaca-rec rebaseline {recording} {FIXTURE} "
              f"--log {rt / 'butaca-events.log'} --reason '<why the behaviour moved>'")
        if args.keep:
            print(f"record-fixture: kept {rt}")
        return 0
    finally:
        srv.shutdown()
        srv.server_close()


if __name__ == "__main__":
    sys.exit(main())
