#!/usr/bin/env bash
# tests/focusfp.sh — RETIRED: the focus-fingerprint flows have no synthetic world to run against.
#
# What this was: the harness that booted `butaca-sim` once per flow into a fresh instance root with
# `butaca-focus` armed, drove it through the remote FIFO with the tokens a television session uses,
# and kept every `focus …` fingerprint line (`crate::focusprobe`, logged only on CHANGE) plus every
# `route=` heartbeat transition. One `.fp` file per flow under $OUT: the (screen x key)
# characterisation of the app's focus behaviour, taken against pinned synthetic data, so it could be
# diffed across commits and committed as a fixture (restructure spec §15.4).
#
# Why it cannot run: every flow injected a `butaca-token` and named its item by a numeric
# ratingKey. The boot gate no longer reads either — it speaks Jellyfin (`jellyfin/boot.rs`: a
# `butaca-jellyfin` config, item ids that are the server's own GUIDs) — and the synthetic mock
# server the flows were recorded against was deleted with the Plex wire. The committed replay
# fixtures still encode that wire as well, so the replay half is no greener than the live one.
#
# What a re-recorded harness needs: `tests/mock_jf.py` as the server (catalog mode for real titles,
# `--seed` for the closed alphabet), a `butaca-jellyfin` config in each instance root instead of
# `butaca-token`, and `--write-synthetic-initial ... flow12=<item id>` seeded with the GUID that
# server hands out rather than `1001`. Nothing else about a flow's shape changes.
#
# The flows it covered, kept here as the record of the intended coverage:
#
#   1  boot-home-chip-grid        boot -> Home -> chip -> grid (the picker is suppressed by a token)
#   2  grid-detail-back           the grid's card -> Detail -> BACK
#   3  home-library-detail-back   Library -> card -> Detail -> BACK to the same card
#   4  search-shelf-detail-back   a seeded query -> result shelf -> Detail -> BACK
#   5  detail-person-detail       Detail -> Person -> Detail -> BACK -> Person -> BACK -> Detail
#   6  settings-family            the Settings overlay: Legal -> About -> Playback -> BACK
#   7  firstrun-consent-onboard   first-run consent -> onboarding -> Home
#   8  detail-hold-itemmenu       Detail -> hold on Related -> item menu -> BACK
#   9  player-readout             Play -> the player's failure read-out (no video off-device)
#   10 root-back                  SKIP on the simulator: a root BACK is a television flow
#   11 pointer-stops              pointer clicks on every stop class, one clipped, one on the track
#   12 filmography-detail-return  a credit opens Detail, BACK restores Filmography and the Person
#
# Runs nothing, and exits 2 so a script that still calls it fails loudly instead of quietly passing.
set -euo pipefail

cat >&2 <<'REFUSED'
focusfp: retired — its flows inject a `butaca-token` and name items by numeric ratingKey, and the
app's boot gate reads neither any more (it speaks Jellyfin, whose ids are the server's GUIDs). The
synthetic server they were recorded against went with the Plex wire. Nothing was started and no
.fp file was written. tests/mock_jf.py is the server a re-recorded harness would run against; see
this file's header for what else changes.
REFUSED
exit 2
