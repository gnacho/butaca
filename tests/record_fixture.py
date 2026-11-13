#!/usr/bin/env python3
"""Re-record a committed replay fixture against today's mock Jellyfin server.

**Why this exists.** A recording is pinned to the build lineage that took it, and the gate that
holds it (`make check-replay`) fails the day the app legitimately moves under it. The specs answer
that with re-recording, and the harness that used to do it is retired: `tests/focusfp.sh` exits 2
with a note saying its world went away with the Plex wire. Nothing else drove a flow and armed the
recorder, so a fixture that diverges has had no way back to green except hand-editing hashes, which
is not a rebaseline, it is a lie.

**What it does.** Reads the committed fixture for the WORLD it was taken in and the FLOW it was
taken of — its seed, its triggers, its initial content, and every key it recorded — then boots the
simulator in the recorder's controlled mode against `tests/mock_jf.py` in that same world, writes
the typed synthetic initial the boot needs, drives those presses through the remote FIFO, and lets
the app stop itself (`BUTACA_SHOT_FRAME`/`BUTACA_SHOT_EXIT`) so the recorder flushes through its
ordinary shutdown. Then it verifies the result with `tools/butaca-rec check`.

The presses and the world come from the fixture, not from this script: a re-record is "the same
flow in the same world on a new build", and anything this file invented would be a new test wearing
an old name.

**What it does NOT do.** It never rebaselines. Which fixture a new recording replaces, and why, is a
judgement the tool cannot make, and `tools/butaca-rec rebaseline` already demands the replay's own
divergence record plus a written reason for an anchor (and `rerecord` for a shape bump). This driver
hands you a recording that passes `check` and the command line to act on it.

**The world is the GENERATED one, never the demo catalog.** `butaca-rec check` holds a committed
recording to a closed alphabet (`tests/fixtures/replay/ALPHABET.json`), and the demo catalog would
fail it on the first real film title. `tests/mock_jf.py`'s seeded world speaks that alphabet by
construction (`sname`), which is what makes a re-recorded fixture committable at all.

Usage:

    python3 tests/record_fixture.py                          # fixture 12, the default
    python3 tests/record_fixture.py --fixture 6-settings-family
    python3 tests/record_fixture.py --keys down,ok --keep     # drive your own presses

The instance root is printed, with the recording under `<root>/butaca-recordings/latest` and the
event log beside it. On success the exit code is 0 and the last line is the `butaca-rec` command to
run once the divergence has been read and accepted.
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
import mock_jf  # noqa: E402  (the repo's own server, imported by path like tools/screenshots.py)

FIXTURES = ROOT / "tests" / "fixtures" / "replay"
DEFAULT_FIXTURE = "12-filmography-detail-return"
# The recorded cadence was one press roughly every second, the first at ~1.7 s. The fixture's own
# times are used when it has them; these are only the fallback for `--keys`.
FIRST_PRESS_S = 0.64
PRESS_GAP_S = 1.03
# **The app stops itself on a FRAME COUNT, not on idleness.** A recording boot presents every frame
# (`rec=` in the heartbeat), so `ui::idle`'s "nothing changed" signal never arrives and a settle
# trigger would run forever. A frame is a fact the boot controls; a wall clock is not.
STOP_FRAME = 900


def fail(msg):
    print(f"record-fixture: {msg}", file=sys.stderr)
    sys.exit(1)


class Recipe:
    """The world and the flow a committed fixture was taken in and of."""

    def __init__(self, name):
        self.name = name
        folder = FIXTURES / name
        if not (folder / "manifest.json").is_file():
            fail(f"no fixture at {folder}")
        manifest = json.loads((folder / "manifest.json").read_text())
        data = manifest["init"]["data"]
        entropy = data.get("entropy") or {}
        self.seed = int(entropy.get("Seeded", 1))
        self.settings = data.get("settings")
        self.content = data.get("content")
        self.item = (self.content or {}).get("detail")
        self.presses = []
        for segment in sorted(folder.glob("rec-*.jsonl")):
            for line in segment.read_text().splitlines():
                record = json.loads(line)
                if record.get("t") != "in":
                    continue
                body = record["body"]
                key = body.get("key") or ""
                if not key:
                    continue
                # A key is two edges; the fixture records both, and the FIFO token is the press.
                if self.presses and self.presses[-1][0] == key and self.presses[-1][1] == record["ms"]:
                    continue
                self.presses.append((key, record["ms"]))
        if not self.presses:
            fail(f"{name}: no recorded presses to re-drive")

    def tokens(self):
        """The FIFO tokens, in order, lower-cased the way the remote spells them."""
        return [key.lower() for key, _ in self.presses]

    def schedule(self):
        """`(first_press_seconds, gap_seconds)` from the recording's own clock."""
        first = self.presses[0][1] / 1000.0
        if len(self.presses) < 2:
            return first, PRESS_GAP_S
        gaps = [b[1] - a[1] for a, b in zip(self.presses, self.presses[1:])]
        gaps = [g for g in gaps if g > 0]
        return first, (sum(gaps) / len(gaps) / 1000.0 if gaps else PRESS_GAP_S)


def write_initial(binary, rt, seed, port, item, surface=None):
    """The typed synthetic world, from the simulator's own encoder: it refuses to invent an id.

    The two worlds are mutually exclusive in the encoder — a flow-12 content world, or a Settings
    surface to boot on — and the fixture's own header says which one it was.
    """
    path = rt / "butaca-app-init"
    world = f"flow12={item}" if item else (f"settings={surface}" if surface else None)
    command = [str(binary), "--write-synthetic-initial", str(path), str(seed), str(port)]
    if world:
        command.append(world)
    proc = subprocess.run(command, capture_output=True, text=True)
    if proc.returncode != 0:
        fail(f"the synthetic initial was refused: {proc.stderr.strip() or proc.stdout.strip()}")
    if not path.exists():
        fail("the synthetic initial was not written")
    return path


def write_triggers(rt, recipe, item):
    """The flow's own entry points, as files.

    The synthetic initial seeds the content STORE; the route the app is on at frame 0 comes from the
    same triggers a live boot reads, which is why the fixture's header lists them.

    **`butaca-nowan` is deliberately NOT written.** That trigger refuses every name lookup, as a
    dead resolver would, and a re-record has to REACH the mock for its detail and person results. The
    committed fixture lists it because its world supplied those results at the store instead of over
    the wire (replay's job, not a record's): armed here, the page logs "no metadata (server
    unresolved, or it refused)" and never becomes readable.
    """
    (rt / "butaca-rec").write_text("")
    (rt / "butaca-noidle").write_text("")
    (rt / "butaca-focus").write_text("")
    if recipe.settings:
        (rt / "butaca-settings").write_text(recipe.settings)
    if item:
        (rt / "butaca-detail").write_text(item)
        (rt / "butaca-detailsec").write_text("1")
        (rt / "butaca-detailok").write_text("")
        (rt / "butaca-filmography").write_text("")
        (rt / "butaca-personcredits").write_text("9")


def send_keys(fifo, tokens, first_s, gap_s):
    """One FIFO token per press, at the recorded spacing. The app drains the FIFO once a frame."""
    time.sleep(first_s)
    for token in tokens:
        with open(fifo, "w") as f:
            f.write(token + "\n")
        time.sleep(gap_s)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", type=pathlib.Path,
                    default=ROOT / "rust-modules" / "target-sim" / "release" / "butaca-sim")
    ap.add_argument("--fixture", default=DEFAULT_FIXTURE, help="the committed fixture to re-record")
    ap.add_argument("--keys", help="override the presses (comma-separated FIFO tokens)")
    ap.add_argument("--timeout", type=float, default=90.0)
    ap.add_argument("--first", type=float, help="seconds before the first press (default: the fixture's own)")
    ap.add_argument("--gap", type=float, help="seconds between presses (default: the fixture's own)")
    ap.add_argument("--keep", action="store_true", help="keep the instance root even on success")
    args = ap.parse_args()

    binary = args.binary.resolve()
    if not binary.exists():
        fail(f"no simulator at {binary} (build it: cargo build --release --manifest-path "
             f"rust-modules/Cargo.toml --target-dir rust-modules/target-sim --features hostsim "
             f"--bin butaca-sim)")
    recipe = Recipe(args.fixture)
    tokens = [k.strip() for k in args.keys.split(",") if k.strip()] if args.keys else recipe.tokens()
    first_s, gap_s = (FIRST_PRESS_S, PRESS_GAP_S) if args.keys else recipe.schedule()
    first_s = args.first if args.first is not None else first_s
    gap_s = args.gap if args.gap is not None else gap_s

    srv, world = mock_jf.serve(0, seed=recipe.seed)
    # **`serve` builds the server; `run` makes it answer.** Without this the app's fetches had
    # nobody on the other end, the detail page never read (`group=0 elem=0` for every press) and
    # `shutdown()` blocked on a `serve_forever` that had never been started.
    mock_jf.run(srv)
    port = srv.server_address[1]
    rt = pathlib.Path(tempfile.mkdtemp(prefix=f"butaca-record-{recipe.name}-"))
    print(f"record-fixture: {recipe.name} on seed {recipe.seed}"
          + (f", item {recipe.item}" if recipe.item else ""))
    print(f"record-fixture: {len(tokens)} press(es) {tokens} at {first_s:.2f}s +{gap_s:.2f}s")
    print(f"record-fixture: mock on 127.0.0.1:{port}, instance root {rt}")
    try:
        if recipe.item and recipe.item not in world.items:
            fail(f"the fixture's item {recipe.item} is not in seed {recipe.seed}'s world")
        write_initial(binary, rt, recipe.seed, port, recipe.item, recipe.settings)
        write_triggers(rt, recipe, recipe.item)
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
            deadline = time.monotonic() + 15.0
            while not fifo.exists():
                if time.monotonic() > deadline:
                    fail("the remote FIFO never appeared (the app did not get past boot)")
                time.sleep(0.05)
            send_keys(fifo, tokens, first_s, gap_s)
            rc = proc.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            # **The artifact, not the exit status, is what a recording is.** On this tree the app
            # finishes the recording and then lingers (its own teardown never returns with the mock's
            # connection open), so the harness stops it and judges the file: `rec: finished` in the
            # log and `butaca-rec check` on the directory below are the two facts that matter.
            proc.kill()
            rc = 0
        finally:
            out.close()
        log = (rt / "butaca-events.log").read_text(errors="replace")
        # **The boundary that is worth naming when it moves.** A record boot may only run the
        # synthetic world, and its detail and person results come from the mock over loopback; if
        # the transport is ever refused again the page logs the line below and every press lands on
        # an empty page. See issue #416.
        blind = [l for l in log.splitlines() if "no metadata" in l]
        if blind:
            fail("the world never read: the detail fetch answered \"no metadata (server "
                 "unresolved, or it refused)\". The presses then act on an empty page. The log is "
                 f"{rt / 'butaca-events.log'}")
        if rc == 0 and "rec: finished" not in log:
            fail(f"the recording never finished; log tail:\n    "
                 + "\n    ".join(log.splitlines()[-12:]))
        if rc != 0:
            fail(f"the simulator exited {rc}; log tail:\n    "
                 + "\n    ".join(log.splitlines()[-12:]))
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
        print(f"record-fixture: next, read the divergence, then ONE of:\n"
              f"  tools/butaca-rec rebaseline {recording} {recipe.name} "
              f"--log {rt / 'butaca-events.log'} --reason '<why the behaviour moved>'\n"
              f"  tools/butaca-rec rerecord {recording} {recipe.name}   # after a SHAPE bump")
        if args.keep:
            print(f"record-fixture: kept {rt}")
        return 0
    finally:
        srv.shutdown()
        srv.server_close()


if __name__ == "__main__":
    sys.exit(main())
