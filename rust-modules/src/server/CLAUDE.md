# plex/ - the surviving Plex wire layer

This directory is the remainder of the Plex data layer. The app is Jellyfin-only now: the onboarding,
the account flow, the profile picker and the legacy session resume are gone, so no product path can
produce a Plex session any more. What is left is the PMS wire client and the vocabulary its surviving
callers still use through the backend-neutral dispatch. The whole directory is queued for deletion as
that dispatch is collapsed; treat it as frozen unless a campaign slice (tracking issue #166) says
otherwise.

## What is here

- `client.rs`: request building for PMS endpoints. `with_token` is the only place `X-Plex-Token` is
  appended; the shared headers (including `X-Plex-Language`) live here, and the TLS/`plex.direct`
  origin rules are documented on `TlsOrigin`.
- `params.rs`: query builders for the wire endpoints (`TranscodeSpec`, `StreamSelection`,
  `TimelineReport`, section queries).
- `transcoder.rs`: `LinkPolicy`, the direct-play predicates (`is_dp_audio_track`, `is_dp_subtitle`,
  the codec lists) and the transcode/relay URL construction.
- `timeline.rs`: timeline reporting, machine identity and the play-queue payloads.
- `library.rs`: the reads that still have live callers — the guid lookup (the alt-sources panel),
  item metadata (the playing-tracks store), the sidecar subtitle download, and the two part-level
  playback ops (stream selection, direct-play target). The hub fetches and the section listings
  (children, related, extras, the batched metadata read) went with the Plex read layer (issue #166).
- `identity.rs`: the client identity strings, including `VERSION` and the `X-Plex-*` headers the
  diagnostics panel reports.
- `serverinfo.rs`: the Server row of the diagnostics panel and `subscription_of` (the Plex Pass
  verdict; for a Jellyfin session it answers `Unknown`).
- `probe.rs`: endpoint probing tiers (`configured_tier`, the persisted tier canon).
- `session.rs`: the playback preferences store (quality, audio, subtitle tone, up-next policy, pins
  for the shared browse table). Jellyfin flows read and write it, which is why it outlives the rest.
- `pins.rs`: pinned libraries, consumed by the browse table that both backends share.
- `persistence.rs`, `install_preferences.rs`: the stores `session.rs` loads and saves through.

## What is gone (deleted by the campaign - do not look for it)

- `account.rs` and the plex.tv API client: PIN sign-in, QR wait, home-user roster, profile switching.
- The session owner's sign-in machine: ready handoff, `RegistryPlan::Install`, the profile cache and
  the protected-profile registry.
- The DevPms harness (`BootstrapAuthority::DevPms`, `activate_dev`, the synthetic Plex home world).
- The metadata vocabulary: it moved to `crate::media` (models.rs is no longer here), and the
  playback vocabulary to `crate::playback`. The server registry lives in `crate::registry`.

## Rules that still hold

- Nothing outside this directory may reach into the wire internals: fields and helpers are
  `pub(super)`, and callers go through the module re-exports.
- Recorded replay state pins the persisted shapes (`SessionInit::write` digest). Do not reorder or
  remove write-only fields without a re-record of the replay fixtures.
- The campaign slices amend this document when they cut something; an outdated claim here means a
  slice forgot to update it, not that the code came back.
