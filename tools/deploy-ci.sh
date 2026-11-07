#!/usr/bin/env bash
# Deploy the package CI built for a commit, on a host that cannot cross-build one.
#
# The webOS NDK ships for darwin-arm64, darwin-x86_64 and linux-aarch64 only (see the Makefile's
# NDK bootstrap block), so on x86_64 Linux `make deploy` cannot run at all: it dies on a missing
# arm-webos-linux-gnueabi-gcc. CI builds on ubuntu-24.04-arm for that exact reason, and its .ipk is
# the payload `make deploy` would have produced from the same commit. This is `make install` with
# the build replaced by a download, and it re-verifies the sha the television is actually running.
#
# Usage: tools/deploy-ci.sh <tv> <app-id> [<sha>]      (SHA defaults to HEAD)
set -euo pipefail

TV=${1:?usage: deploy-ci.sh <tv> <app-id> [<sha>]}
APPID=${2:?usage: deploy-ci.sh <tv> <app-id> [<sha>]}
SHA=${3:-$(git rev-parse HEAD)}

# `gh repo view` answers with the UPSTREAM here (the fork's parent), which is not where CI runs, so
# the slug comes from `origin` — the one remote whose runs produce the artifact we want.
ORIGIN=$(git remote get-url origin)
REPO=$(printf '%s' "$ORIGIN" | sed -e 's#^git@github.com:##' -e 's#^https://github.com/##' -e 's#\.git$##')
[ -n "$REPO" ] || { echo "deploy-ci: cannot read the repo slug from origin ($ORIGIN)" >&2; exit 1; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/butaca-deploy-ci-XXXXXX")
trap 'rm -rf "$WORK"' EXIT

echo "deploy-ci: $REPO @ ${SHA:0:9} -> $APPID on $TV"

RUN=$(gh run list --repo "$REPO" --workflow ci.yml --limit 60 \
        --json databaseId,headSha,conclusion \
        --jq "[.[] | select(.headSha == \"$SHA\" and .conclusion == \"success\")][0].databaseId")
if [ -z "$RUN" ] || [ "$RUN" = "null" ]; then
  echo "deploy-ci: no successful ci.yml run for $SHA — push it, or wait for CI" >&2
  exit 1
fi
echo "deploy-ci: run $RUN"

gh run download "$RUN" --repo "$REPO" --dir "$WORK" >/dev/null
IPK=$(find "$WORK" -name '*.ipk' -type f | head -1)
[ -n "$IPK" ] || { echo "deploy-ci: the run produced no .ipk" >&2; exit 1; }
echo "deploy-ci: $(basename "$IPK") ($(wc -c <"$IPK") bytes)"

# The binary out of that package, kept as the LOCAL build: `tv-session.sh up` deploys when the
# on-device md5 differs from `pkg/butaca`, and there is no local cross-build to compare against.
mkdir -p "$WORK/x" && ( cd "$WORK/x" && ar x "$IPK" && tar xzf data.tar.gz \
  "usr/palm/applications/$APPID/butaca" 2>/dev/null || tar xzf data.tar.gz \
  "./usr/palm/applications/$APPID/butaca" )
BIN="$WORK/x/usr/palm/applications/$APPID/butaca"
[ -f "$BIN" ] || { echo "deploy-ci: the package has no $APPID/butaca" >&2; exit 1; }
WANT=$(grep -a -o 'sha=[0-9a-f]*' "$BIN" | head -1 | cut -d= -f2)
mkdir -p pkg && cp "$BIN" pkg/butaca
echo "deploy-ci: package binary sha=$WANT, written to pkg/butaca"

# The quoting is why this is a file and not a line of ssh: luna-send takes its payload as a JSON
# string, and `script -qc` takes the whole call as one more string.
cat > "$WORK/install.sh" <<EOF
#!/bin/sh
exec script -qc "luna-send -i -a com.webos.appInstallService luna://com.webos.appInstallService/dev/install '{\\"id\\":\\"$APPID\\",\\"ipkUrl\\":\\"/tmp/$(basename "$IPK")\\",\\"subscribe\\":true}'" /dev/null
EOF
chmod +x "$WORK/install.sh"
scp -q -o BatchMode=yes "$IPK" "$WORK/install.sh" "root@$TV:/tmp/"
# The remote command SUBSCRIBES and never returns on its own, so the ssh call is bounded on the
# remote side rather than by closing our end of a pipe: `head` on the local side only ends the
# stream if the remote keeps writing, and a quiet install leaves ssh waiting for ever (measured:
# the call hung until the caller's own timeout killed it).
# Bounded from the LOCAL side: the remote `timeout` on this television is not the coreutils one
# (it read `180` as the command and ran nothing, which made the sha check pass against the build
# that was already installed — a false positive). The remote still writes its own report, and the
# check below refuses a run whose report is empty.
timeout 120 ssh -o BatchMode=yes "root@$TV" \
  "chmod +x /tmp/install.sh && /tmp/install.sh >/tmp/install.out 2>&1" 2>/dev/null || true
REPORT=$(ssh -o BatchMode=yes "root@$TV" "grep -c appInstallService /tmp/install.out 2>/dev/null || echo 0")
if [ "${REPORT:-0}" -lt 1 ]; then
  echo "deploy-ci: the installer reported nothing — it did not run" >&2
  exit 1
fi
# Anchored on purpose: `pkill -f 'luna-send.*appInstallService'` matches the remote shell's own
# command line, because that line contains the pattern, and kills the session running it.
ssh -o BatchMode=yes "root@$TV" \
  "pkill -f '^luna-send ' || true; rm -f '/tmp/$(basename "$IPK")' /tmp/install.sh /tmp/install.out"

# What the television is ACTUALLY running, not what we asked it to run.
GOT=$(ssh -o BatchMode=yes "root@$TV" \
  "grep -a -o 'sha=[0-9a-f]*' /media/developer/apps/usr/palm/applications/$APPID/butaca | head -1 | cut -d= -f2")
if [ "$GOT" != "$WANT" ]; then
  echo "deploy-ci: deployed sha=$GOT, package said sha=$WANT — the install did not land" >&2
  exit 1
fi
echo "deploy-ci: verified, the television runs sha=$GOT"
echo "deploy-ci: next, 'tools/tv-session.sh up' to boot it (its md5 check will pass)"
