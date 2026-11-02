//! Typed Plex API layer — one method per PMS operation the app uses.
//!
//! Replaces every hand-built Plex path/query string in the app with a typed `Client`
//! method (see `docs/plex-api-design.md` + `docs/plex-api-catalog.md`). Percent-encoding
//! (`crate::pms::urlenc_str`), the `X-Plex-Token` injection, and the origin-aware HTTP(S)
//! transport (`crate::http`) are centralised in `client.rs`, so no op file can bypass them.
//! Response bodies deserialize into `serde` DTOs (`crate::media`).
//!
//! The read layer (`pms`/`metadata`/`posters`/`detail`) and the playback layer (`route.rs`
//! decision/start/stop/selection, PlayQueue/identity, and the player's `/:/timeline`) use this
//! typed client (history: `docs/plex-api-migration.md`).

mod client;
pub(crate) mod identity; // ONE X-Plex-* identity for both transports (plex.tv headers + PMS query)
mod params;
// WHICH servers exist and which one is current. The SERVER REGISTRY itself now lives in
// `crate::registry` (backend-neutral: `ServerId`, `ServerFacts` and the per-slot state machine are
// shared vocabulary — the Jellyfin backend names its one server in the same slot model), and the
// private alias below keeps the in-layer `super::servers::…` spellings resolving unchanged.
// `client()`/`client_opt()` still mean "the current server"; `client.rs` is just the type.
use crate::registry as servers;

// Op files below only add `impl Client { … }` blocks (Rust allows multiple impls of one
// type across a crate) — declared here so those methods compile onto `Client`.
mod hubs;
mod library;
/// Whole-file text subtitles are bounded before transport allocation and parsing.
pub(crate) const SIDECAR_MAX_BYTES: usize = 4 * 1024 * 1024;
mod timeline;
mod transcoder;

// The server's self-description (version + Plex Pass tristate), refreshed by `install` on every
// session path. pub(crate) because it is a DIAGNOSTICS surface with readers outside this layer
// (`app::diagnostics`, `player::error_shape`) — and deliberately nothing else: see its module doc for
// why subscription state must never become a routing input.
pub(crate) mod serverinfo;

// The persisted session record and its one read-modify-write door. pub(crate): the boot code
// (app.rs), the preference editors and the stores' read paths all touch it — through the doors
// this module documents, never the fields directly.
pub(crate) mod session;

// WHERE a server is, as one value: scheme + host + port, parsed from a URL and never assembled
// from an address. The type every layer below the transport passes instead of a bare
// `(host, port)` pair — read its module doc before adding a call site, in particular the bracket
// invariant that keeps `host()` (the resolver's node) and `authority()` (URL serialization) from
// being confused for one another.
pub(crate) use crate::origin;

// Which of a server's advertised addresses are worth dialling, and in what order. PURE policy over
// an `account::Resource` — no socket, no thread — so the rules that decide reachability are gradeable
// on the host, which is the only tier that can grade them at all: the failures they prevent are an
// 8-second timeout and a probe that answers as the wrong machine.
pub(crate) mod probe;

// Compatibility alias for Plex internals. The credential authority and plaintext consent
// lifecycle live in crate::grant so shared HTTP transport code need not import through Plex.
// (Unused since the server registry moved to crate::registry — its `super::grant` calls went
// with it — so the alias is gone.)

// The ONE door the out-of-layer registry (crate::registry) has back into this layer's private
// surface: minting a slot `Client` (its constructor and the resolve-pin builder are `pub(super)`
// on purpose) and triggering the self-description refresh. Pure forwarding, same calls in the
// same order as the in-layer spellings — no behavior of its own.
pub(crate) mod registry_seam {
    use super::{Client, ServerId};
    use crate::origin::{Origin, ResolvePin};

    pub(crate) fn new_client(
        id: ServerId,
        machine_id: &str,
        origin: Origin,
        token: &str,
        client_id: &str,
        pin: Option<ResolvePin>,
    ) -> Client {
        Client::new(id, machine_id, origin, token, client_id).with_resolve_pin(pin)
    }

    pub(crate) fn refresh_serverinfo(id: ServerId) {
        super::serverinfo::refresh(id);
    }

    /// Test-only read of the slot client's token (`with_token` is `pub(super)` by design).
    #[cfg(test)]
    pub(crate) fn client_token_for_test(c: &Client) -> String {
        c.with_token("/x").rsplit('=').next().unwrap_or_default().to_owned()
    }
}

// Which libraries feed Home, PER PROFILE. Pure policy over the section table and one profile's
// persisted answer — no store, no screen — so the household default, the recorded answer and the
// never-empty floor are graded on the host rather than observed on a television.
pub(crate) mod pins;

// The re-exports are the public surface the call sites import.
pub(crate) use client::ArtFetch;
pub(crate) use client::JsonDeadlineOutcome;
// The one link/IP ⇄ u8 encode/decode pair — shared by `Client`'s own atomics and the removed
// report trace's packed attempt snapshot; with nothing left to read them, the re-export goes.
pub use client::{Client, IpVersion, StreamUrl};
// WHERE a server is, as one value. `Origin` is what `register_origin` takes and what a
// `Client` carries; `Scheme` is re-exported beside it because `dev::DevServer` deserializes one
// straight out of the `butaca-servers` trigger.
#[allow(unused_imports)]
pub(crate) use crate::origin::{plex_direct_literal, url_host, CredentialPolicy, Origin, ResolvePin, Scheme};
// The registry surface THIS LAYER still names through `super::…` / `crate::plex::…` — the
// crate-wide surface lives in `crate::registry` since the move out of plex/ (`super::servers::…`
// spellings resolve through the alias above).
#[allow(unused_imports)]
pub use servers::{
    client_for, current as current_server, count as server_count, same_item, ServerId,
};
#[allow(unused_imports)]
pub(crate) use servers::ConnectionFacts;
// The registry as a TEST FIXTURE, for suites outside this module (`route.rs` grades which server a
// `/:/timeline` POST reaches). `register_for_test` skips the `session::load` the public `register`
// does — that call mints and PERSISTS a device uuid, which a host test has no business writing —
// and `reset_servers_for_test` is what keeps the table a per-test fixture instead of a growing
// process-global holding clients whose loopback ports closed when their test returned. Both must be
// called under `crate::testlock::serial`.
// The i64 wire-fold helper is shared, not plex-private: the person screen's DTO vocabulary
// (crate::person, where the old discover.rs types now live) deserializes its optional i64s
// through it.
pub use params::*;
// The registry's test-fixture surface (`register_with_client_id`, `reset_for_test`,
// `write_held_for_test`, `register_pinned_with_client_id*`) and the registration entry points
// (`register_origin`, `register_captured_origin_with_connection`) moved to `crate::registry`
// with the registry itself — callers import them there.
// The projected play-queue row and the rest of the encode/timeline vocabulary moved to
// `crate::playback`; route/, `plex/session.rs` and `jellyfin/profile.rs` name it there now.
// DP_AUDIO_CODECS rides along for `devcaps`, which intersects it with the device's own codec
// table — normal routing and its profile read the same codec and channel limits.
// `is_dp_subtitle` is the subtitle twin: the profile's `subtitleCodec=` list and route's MDE
// `subtitleStreamID` gate, so a selected PGS cannot be advertised in one and omitted from the other.
// `is_dp_audio_track` checks normal codec/channel eligibility, while `link_policy` constrains
// the connection. Forced mode retains software feed limits and uses a separate PMS profile.
pub use transcoder::{DP_AUDIO_CODECS, LinkPolicy, is_dp_audio_track, is_dp_subtitle, link_policy};
