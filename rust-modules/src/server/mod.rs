//! The server layer's remaining surfaces: the client's identity, which advertised addresses are
//! worth dialling, the link policy, the persisted session record, the server's self-description
//! and the per-profile library pins.
//!
//! The typed client this module used to head (`Client`, one method per operation, with
//! percent-encoding and credential injection in `client.rs`) went with the wire layer (#166).
//! The read layer asks `crate::jellyfin` instead, and playback keeps its own route code.

pub(crate) mod identity; // ONE credential identity for both transports (account + server query)

// WHICH servers exist and which one is current. The SERVER REGISTRY itself now lives in
// `crate::registry` (backend-neutral: `ServerId`, `ServerFacts` and the per-slot state machine are
// shared vocabulary), and the private alias below keeps the in-layer `super::servers::…`
// spellings resolving unchanged. The registry's client slots went with the wire layer (#166);
// what remains there is the neutral slot vocabulary.
use crate::registry as servers;

// Whole-file text subtitles are bounded before transport allocation and parsing. The typed
// client that enforced it went with the wire layer; the bound survives because
// `player::sidecar` grades the same ceiling.
pub(crate) const SIDECAR_MAX_BYTES: usize = 4 * 1024 * 1024;

// The relay link policy (`link_policy`): pure policy over `probe::Location`. The transcoder
// request builders it used to govern went with the retired wire layer.
mod link;

// The server's self-description (version + paid-tier tristate), refreshed by `install` on every
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

// The credential authority and plaintext consent lifecycle live in `crate::grant`, so shared
// HTTP transport code need not import through this layer. The compatibility alias that used to
// re-export it went with the server registry's move to `crate::registry`.

// The ONE door the out-of-layer registry (crate::registry) had back into this layer's private
// surface — minting a slot `Client` and triggering the self-description refresh — went with the
// wire layer (#166): the registry no longer holds clients, so there is nothing to forward.

// Which libraries feed Home, PER PROFILE. Pure policy over the section table and one profile's
// persisted answer — no store, no screen — so the household default, the recorded answer and the
// never-empty floor are graded on the host rather than observed on a television.
pub(crate) mod pins;

// The registry surface THIS LAYER still names through `super::…` / `crate::server::…`. The typed
// client and its vocabulary (`Client`, `IpVersion`, `ArtFetch`, the registration seams) went
// with the wire layer (#166); `StreamUrl` moved to `crate::origin`, whose splitter the media
// plane already read. `link_policy` is the one symbol the wire layer left behind that outside
// callers still name: the detail page's playback preview grades the live link tier through it,
// and the routing tests grade it beside `route::quality_policy`. `LinkPolicy` itself lives in
// `crate::playback`; `DP_AUDIO_CODECS`/`is_dp_audio_track` moved to `crate::devcaps` (their
// implementation home), and the subtitle list went local to `route/plan.rs` with its render
// consumer.
pub use link::link_policy;
