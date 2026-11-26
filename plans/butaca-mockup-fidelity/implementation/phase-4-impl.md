---
type: planning
entity: implementation-plan
plan: "butaca-mockup-fidelity"
phase: 4
status: draft
created: "2026-11-23"
updated: "2026-11-23"
---

# Implementation Plan: Phase 4 - Real audio and subtitle preferences

> Implements [Phase 4](../phases/phase-4.md) of [butaca-mockup-fidelity](../plan.md)

## Approach

Add the three preferences to the existing session record and Settings page, then resolve one track intent for both direct play and transcode. Reuse `StartTracks`, the existing series memory and the current server/file-default ladders instead of creating a second playback policy. Fresh explicit choices, retry selections and series choices outrank the installed preferences. The installed preferences outrank server defaults, and an explicit subtitle Off remains distinct from an unanswered subtitle choice.

Use the mockup's initial values and options: Spanish or Original audio, Always, forced-only or Off subtitles, and Spanish or English subtitle language. Keep the migration additive and the route worker pure. Read the cached session on the main thread when `ResolveEnv` is captured, so no new process globals, boot-restoration path or credential-schema version is needed.

Do not change the playback engine, Player HUD, Phase 3's authorized transport composition, subtitle appearance settings or sidecar fetching.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/server/session.rs` and `session_{compat,persistence}_tests.rs` | modify | Add the flattened, soft-migrating track-preference fields and canonical split/join coverage. |
| `rust-modules/src/screens/settings/page.rs`, `screens/registry.rs` and `app/preferences.rs` | modify | Append real fields, values, options and durable preference commands to the existing Settings flow. |
| `rust-modules/src/screens/settings_*tests.rs` | modify | Prove row/action/value pairing, picker contents, focus restoration and emitted commands. |
| `locales/{en,es,be}/settings.json` | modify | Reuse the existing preference labels and add their short detail lines. |
| `rust-modules/src/route/plan.rs` and `route/plan_track_selection_tests.rs` | modify | Resolve explicit, remembered, installed and server-default track intents for direct play and transcode. |
| `rust-modules/src/route/decision.rs` and `route/decision_resolve_route_tests.rs` | modify | Capture preferences, preserve retries and explicit Off, and carry current tracks through transcode rebuilds. |
| `rust-modules/src/jellyfin/profile.rs` | modify | Pin the distinct omitted, explicit-Off and positive subtitle indices in `PlaybackInfo`. |
| `rust-modules/src/dev/scenarios.rs`, `tests/screenshots/scenes.json` and `tests/fixtures/replay/` | modify | Add the three real picker captures and the intended Settings replay migration. |

## Required Context

| File | Why |
|------|-----|
| `plans/butaca-mockup-fidelity/{plan,phases/phase-4}.md` | Gated preferences, precedence, exclusions and acceptance criteria. |
| `plans/butaca-mockup-fidelity/implementation/phase-1-impl.md` | Settings composition and anchored-picker contract to consume without rebuilding it. |
| `plans/butaca-mockup-fidelity/implementation/phase-3-impl.md` | `StartTracks` continuity and the separate authorized Player composition that this phase must not address. |
| Supplied `butaca-rediseno-mockup.zip` | Binding option order and initial values in `idiomaAudio`, `modoSubs` and `idiomaSubs`. |
| `rust-modules/src/server/CLAUDE.md` | Canonical persistence boundaries and the recorded-state digest rule. |
| `rust-modules/src/ui/CLAUDE.md`, `screens/CLAUDE.md` and `docs/DESIGN.md` | Existing Settings focus, popover, localization and legibility invariants. |
| `rust-modules/src/player/CLAUDE.md` | Client-rendered versus burned subtitles and the boundary of playback-engine work. |
| `rust-modules/src/server/session.rs` | `Session`, `CanonicalSessionPreferences`, soft parsers, cached `peek`, durable updates and default construction. |
| `rust-modules/src/server/session_{compat,persistence}_tests.rs` | Literal legacy JSON and real read/write regression patterns. |
| `rust-modules/src/screens/settings/page.rs` | Appended `Field` discriminants, `Value`, `options`, row triples, picker commits and pending status. |
| `rust-modules/src/screens/registry.rs` and `app/preferences.rs` | `PreferenceCmd` and its storage-worker executor. |
| `rust-modules/src/screens/settings_composed_tests.rs` and `settings_test_support.rs` | Real Settings engine path and command-capture harness. |
| `rust-modules/src/route/plan.rs` | `StartTracks`, `ResolveEnv`, `Plan`, audio/subtitle ladders and Jellyfin direct/transcode branches. |
| `rust-modules/src/route/decision.rs` | Preference snapshot, retry context, `apply_plan`, sidecar restoration, seek and retranscode request bodies. |
| `rust-modules/src/route/{plan_track_selection_tests,decision_resolve_route_tests}.rs` | Existing selection, explicit-Off, retry and transcode anchors. |
| `rust-modules/src/jellyfin/profile.rs` | `playback_info_body[_for]` and its additive request-shape tests. |
| `rust-modules/src/metadata.rs` and `player/sidecar.rs` | Stream language/forced/external facts and the existing server-selected sidecar behavior to preserve outside explicit overrides. |
| `rust-modules/src/dev/scenarios.rs`, `tests/screenshots/scenes.json` and `tests/fixtures/replay/README.md` | Existing named-picker capture path and supported replay migration rules. |

## Implementation Steps

### Step 1: Pin migration, Settings and complete track precedence with failing regressions

- **What**: Add literal legacy-session cases for absent fields, explicit Original, explicit Off and malformed values. Add Settings cases for the three rows, their exact options, saved-option seating, parent restoration and emitted commands. Extend route cases to cover language aliases, forced versus ordinary subtitles, direct-play and transcode choices, fresh explicit choices, series memory, retry selections, explicit Off versus unset, unsupported direct-play tracks and external subtitles. Add request-body cases proving that an unset subtitle omits `SubtitleStreamIndex`, explicit Off sends `-1` and a positive id names the track.
- **Where**: `server/session_{compat,persistence}_tests.rs`, `screens/settings_*tests.rs`, `route/plan_track_selection_tests.rs`, `route/decision_resolve_route_tests.rs` and `jellyfin/profile.rs` tests.
- **Authorized By**: Phase 4 Includes, Deliverables 1-2 and Acceptance Criteria 1-4, plus the repository rule that behavior fixes begin with a failing regression.
- **Why**: Current code already has three-state `StartTracks`, but the Jellyfin transcode branch drops explicit Off and `apply_plan` can restore a server sidecar afterward. Tests must pin those distinctions before the preference rung is inserted.
- **Considerations**: Keep current server/file-default, codec-fallback, subtitle-offset and route-ownership tests as anchors. Use synthetic tracks and request bodies, never real-account playback or writes.

### Step 2: Add the smallest additive session migration

- **What**: Introduce `TrackLanguage::{Spanish, English}`, `SubtitleMode::{Always, Forced, Off}` and a `TrackPreferences` value flattened into both `Session` and `CanonicalSessionPreferences`. Give it the mockup's defaults: `Some(Spanish)`, `Forced` and `Spanish`. Persist Original audio as explicit `preferred_audio_language: null`, not as a missing field, and persist every subtitle mode by stable serde names. Soft-parse malformed values per field so credentials survive. Thread the value through `CanonicalSessionPreferences::default`, `split_public`, `join_canonical` and `public_session`, with `Session::{track_preferences,with_*}` accessors and durable setters using `update_with_outcome`.
- **Where**: `server/session.rs` and its compat/persistence suites.
- **Authorized By**: Phase 4 Includes "Persisted fields and a forward-compatible session migration" and Acceptance Criterion 1.
- **Why**: Both legacy JSON and the DB8 public-preference payload must produce the same domain value. A nested flattened value lets the existing derived `Session::default` carry the reference defaults without rewriting every session field, while explicit null preserves Original across restart.
- **Considerations**: Do not bump `CanonicalSessionAuth.version`, touch `install_preferences.rs`, or add these fields to `SessionInit::write_persisted`. Controlled replay has no playback consumer for them, and the existing preference-digest exclusion keeps unrelated anchors stable. Do not skip serializing explicit null or Off.

### Step 3: Connect the existing Settings Field/Value/options flow

- **What**: Append `AudioLanguage`, `SubtitleMode` and `SubtitleLanguage` after `SkipMarkers` in `Field`, preserving every existing discriminant. Add matching `Value` variants and `PreferenceCmd` arms. Build the rows first in the Audio & subtitles group, before size, position and background, using the reference order: Spanish, English, Original for audio; Always, Forced only, Off for mode; Spanish, English for subtitle language. Reuse `metadata::language_name` and the existing `settings.audio.*` labels, adding only missing detail strings. Hold the current `TrackPreferences` in `RootPage::State`, refresh it from `server::session::peek` on rebuild, include its three values in `LogicalState::write`, and advance `RootPage::SHAPE` to `SettingsHomeV2`. Execute writes through the existing storage-worker reply/status path.
- **Where**: `screens/settings/page.rs`, `screens/registry.rs`, `app/preferences.rs`, `screens/settings_*tests.rs` and `locales/{en,es,be}/settings.json`.
- **Authorized By**: Phase 4 Includes "Settings rows and native pickers connected to those fields" and Acceptance Criterion 1.
- **Why**: `RootPage` already owns row/action/value pairing, saved-option seating, anchored pickers, busy state and failure reporting. Extending that path is sufficient and avoids a new screen or preference store. The three values belong in its recorded logical state because they change what the page displays and what a replay must reproduce.
- **Considerations**: Consume Phase 1's anchored `Popover` without changing its geometry or focus behavior. Extend `Field::is_subtitle` so section membership and flattened actions stay aligned. No optimistic process-global copy is needed because a durable session update installs the cached value read by the page. Keep the `SessionInit` digest unchanged, since controlled replay has no playback consumer for these fields.

### Step 4: Resolve one explicit track intent in the pure route plan

- **What**: Add `track_preferences: TrackPreferences` to `ResolveEnv` and populate it from the cached session in `ResolveEnv::snapshot`. In `route::plan`, resolve an effective `StartTracks` from the fetched streams. Treat fresh `StartTracks`, the retry selection in `ResolveEnv::{audio_sid,subtitle_override}` and existing `SeriesTracks` as explicit choices. Only when those are unanswered, apply the installed audio language or subtitle rule, then fall back to the existing server-selected/file-default ladders. Original audio skips the installed audio rung. Subtitle Always selects a preferred-language track, Forced requires `Stream.forced`, and Off produces `StartTracks::subtitle() == Some(0)`. If a preferred language has no matching track, use the server default rather than another language.
- **Where**: `route/plan.rs`, `route/decision.rs::ResolveEnv::snapshot` and `route/plan_track_selection_tests.rs`.
- **Authorized By**: Phase 4 Includes the documented precedence and Acceptance Criteria 2-3.
- **Why**: `StartTracks` already represents an absent answer, explicit Off and a positive subtitle id. Resolving the effective intent once keeps direct play and transcode from assigning different winners, while owned preference data preserves the resolve worker's purity.
- **Considerations**: Do not reapply `request.start_tracks` on retry. Use `subtitle_override.is_some()` to identify the retry's live selection, including zero Off, and do not let a previous item's `audio_sid` leak into a fresh request. Preserve the existing series Off representation and ISO matching through `metadata::lang_matches`.

### Step 5: Apply that intent to direct play, transcode and retries without losing Off

- **What**: In `build_stream_jellyfin`, use the effective audio id before the existing direct-play default ladder. A named track that direct play cannot carry takes the transcode branch and is written as `AudioStreamIndex`. For subtitles, carry the effective answer into `Plan.subtitle_override`, `sub_sid` and `sub_render_ordinal`. An external or otherwise non-client-renderable named subtitle requires transcode burn-in. An unresolved explicit id stays Off rather than silently substituting another track. When there is no explicit answer, retain the current server-selected embedded/sidecar behavior. In `apply_plan`, call `restore_server_selection` only when `Plan.subtitle_override` is `None`, so explicit Off or a named-but-unrenderable choice cannot resurrect the server selection. In the transcode body, map `None` to an omitted `SubtitleStreamIndex`, `Some(0)` to `-1` and a positive id to itself. Carry the installed `cur_audio_sid`/`cur_sub_sid` through `jellyfin_transcode_seek` and `jellyfin_retranscode_as`, omitting audio when its id is zero and preserving actual subtitle Off with `-1`.
- **Where**: `route/plan.rs::build_stream_jellyfin`, `route/decision.rs::{apply_plan,request_play_inner,jellyfin_transcode_seek,jellyfin_retranscode_as}` and `jellyfin/profile.rs`.
- **Authorized By**: Phase 4 Includes explicit Off, retry and transcode behavior, Deliverable 2 and Acceptance Criteria 2-4.
- **Why**: Today the transcode branch filters zero out of the body and the sidecar restoration cannot distinguish Off from no embedded ordinal. The existing retry fields are also not consumed by track selection. These are the real paths where the new precedence would otherwise be lost.
- **Considerations**: Keep `request_play_inner`'s rule that a retry clears `start_tracks` and uses `RetryContext`. Do not change decoders, `ff`, Starfish/ACB, HUD state, track-menu commits, route leases or timeline behavior. Treat external subtitles as transcode-only for these new explicit paths rather than repairing the existing sidecar fetcher.

### Step 6: Capture the real pickers and complete the shape-governed replay migration

- **What**: Add `settings-audio-language`, `settings-subtitle-mode` and `settings-subtitle-language` scenes through Phase 1's real named-row D-pad/OK path, alongside the refreshed Settings root. Add new labels to `ALPHABET.json`. After the `SettingsHomeV2` census is observed refusing the old recordings, use the supported `tools/butaca-rec rerecord` path for all three shape-incompatible anchors, preserving their existing journeys and anchor flags. Fill only measurements still missing after that migration and run strict replay. Review all four 1920x1080 captures.
- **Where**: `dev/scenarios.rs`, `tests/screenshots/scenes.json`, `tests/fixtures/replay/ALPHABET.json`, all three replay fixtures and `tests/replay_fixtures.py`.
- **Authorized By**: Phase 4 Includes Settings and migration regressions, Deliverable 3 and the plan's capture/replay strategy, which permits re-recording when the fixture shape requires it.
- **Why**: The new rows change Settings behavior and their values are now part of its canonical state. A measurement fill cannot migrate the old recordings, while the observed shape refusal is exactly the condition supported by `rerecord`.
- **Considerations**: Re-record only after the actual shape refusal, never to re-anchor behavior alone. Do not edit hashes by hand, broaden replay into playback I/O or add real-account data. `SessionInit` and every screen census except Settings remain unchanged.

## Testing Plan

Run this once on the committed, clean tree after the supported shape-governed replay migration, then restore any `make check` self-test debris and open `settings.jpg`, `settings-audio-language.jpg`, `settings-subtitle-mode.jpg` and `settings-subtitle-language.jpg` for review.

**Primary Verify Command**:
```sh
make RUST_NIGHTLY=nightly-2026-07-02 check && \
CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 check --manifest-path rust-modules/Cargo.toml --lib --no-default-features && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=settings,settings-audio-language,settings-subtitle-mode,settings-subtitle-language SHOT_OUT=/tmp/opencode/butaca-449-phase4 SHOT_CHECK=1 && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make check-replay
```

### Additional Checks

- Under the repository TV lock, use the established mock or guest identity to edit all three preferences, relaunch and verify their values, then inspect synthetic direct-play and transcode route evidence for preferred audio, forced subtitles and explicit Off. Do not test the Player HUD, press OK on real catalog items, scrobble or make real-account writes. CI must then prove ARM and Simulator green on the same commit.

## Reality Check

- **Non-blocking**: `StartTracks` already has the required three subtitle states, but `build_stream_jellyfin` currently filters explicit Off out of `PlaybackInfo` and `Plan` has no corresponding override for sidecar restoration. Step 5 fixes those exact seams rather than changing `StartTracks::NONE` or treating zero as unset everywhere.
- **Non-blocking**: `ResolveEnv.subtitle_override` already records retry Off versus a positive id, but current Jellyfin track selection does not consume it. Step 4 uses that existing contract and leaves `request.start_tracks` cleared on retry, as `request_play_inner` requires.
- **Non-blocking**: Production `player::sidecar::select` currently installs a fetch callback that returns `None`, while `restore_server_selection` remains the incumbent server-default path. Repairing sidecar delivery is outside this phase. New explicit external-subtitle choices therefore go through transcode burn-in, and explicit Off only suppresses the existing restoration call.
- **Non-blocking**: `jellyfin_transcode_seek` currently sends the plain body and `jellyfin_retranscode_as` sends only the audio index. Carrying the current ids and explicit Off is necessary to preserve the selected route across those rebuilds. It does not authorize changes to the Player HUD or the separate Phase 3 transport decision.
