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
// WHICH servers exist and which one is current. The SERVER REGISTRY itself now lives in
// `crate::registry` (backend-neutral: `ServerId`, `ServerFacts` and the per-slot state machine are
// shared vocabulary — the Jellyfin backend names its one server in the same slot model), and the
// private alias below keeps the in-layer `super::servers::…` spellings resolving unchanged.
// `client()`/`client_opt()` still mean "the current server"; `client.rs` is just the type.
use crate::registry as servers;

// Op files below only add `impl Client { … }` blocks (Rust allows multiple impls of one
// type across a crate) — declared here so those methods compile onto `Client`.
mod library;
/// Whole-file text subtitles are bounded before transport allocation and parsing.
pub(crate) const SIDECAR_MAX_BYTES: usize = 4 * 1024 * 1024;
// The relay link policy (`link_policy`): pure policy over `probe::Location`. The transcoder
// request builders it used to govern went with the Plex wire layer.
mod link;

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
// The typed request params (`TranscodeSpec`, `StreamSelection`, `TimelineReport`) and the
// universal-transcoder ops died with the Plex wire layer; the backend-neutral nouns they
// carried had already moved to `crate::playback` (#189). The i64 wire-fold helper the params
// doc mentioned is `crate::plex::i64_fold`'s sibling and stays with its own users.
// The registry's test-fixture surface (`register_with_client_id`, `reset_for_test`,
// `write_held_for_test`, `register_pinned_with_client_id*`) and the registration entry points
// (`register_origin`, `register_captured_origin_with_connection`) moved to `crate::registry`
// with the registry itself — callers import them there.
// `link_policy` is the one symbol the wire layer left behind that outside callers still name:
// the detail page's playback preview grades the live link tier through it, and the routing
// tests grade it beside `route::quality_policy`. `LinkPolicy` itself moved to
// `crate::playback`; `DP_AUDIO_CODECS`/`is_dp_audio_track` moved to `crate::devcaps` (their
// implementation home), and the subtitle list went local to `route/plan.rs` with its render
// consumer.
pub use link::link_policy;
