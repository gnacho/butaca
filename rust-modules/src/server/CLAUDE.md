# server/ - the server-side vocabulary the app still needs

This directory holds the parts of the former wire layer that the Jellyfin-only app still uses: the
account's persisted preferences, the identity strings the server and the diagnostics report, the
pinned libraries the browse table reads, and the small probes and links the session keeps.

The wire client itself is gone. The registry in `crate::registry` names servers and the installed
Jellyfin client in `crate::jellyfin` carries the requests, so there is no second transport here.

## What is here

- `session.rs`: the playback preferences store (quality, audio, subtitle tone, up-next policy, pins
  for the shared browse table) and the persisted account record. Jellyfin flows read and write it.
- `session/`: the stores `session.rs` loads and saves through (`persistence.rs`,
  `async_persistence.rs`, `install_preferences.rs`).
- `pins.rs`: pinned libraries, consumed by the browse table.
- `identity.rs`: the client identity strings, including `VERSION` and the headers the diagnostics
  panel reports.
- `serverinfo.rs`: the Server row of the diagnostics panel.
- `probe.rs`: endpoint probing tiers (`configured_tier`, the persisted tier canon).
- `link.rs`: the share-link vocabulary the session keeps.
- `session_*_tests.rs`: the suites split out of `session.rs` (cache, compat, persistence, profile
  cache, publication, roster).

## Rules that still hold

- Nothing outside this directory may reach into the internals: fields and helpers are `pub(super)`,
  and callers go through the module re-exports.
- Recorded replay state pins the persisted shapes (`SessionInit::write` digest). Do not reorder or
  remove write-only fields without a re-record of the replay fixtures.
- The campaign slices amend this document when they cut something. An outdated claim here means a
  slice forgot to update it, not that the code came back.
