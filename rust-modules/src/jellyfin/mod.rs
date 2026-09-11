//! jellyfin/ — the Jellyfin data layer (typed Jellyfin server client)
//!
//! This is the **second backend** of the application, sitting where [`crate::plex`] sits for
//! Plex: one typed HTTP client ([`client::JfClient`]), the serde DTOs it parses
//! ([`dto`]), and the converters that turn those DTOs into the app's own catalog rows
//! ([`convert`]).
//!
//! ## The adapter principle: the UI's structs are the contract
//!
//! The screens do not read this module's DTOs. They read the SAME internal structs they always
//! have — `pms::PmsMovie` for grids and shelves, `metadata::Detail` for the detail page — because
//! those structs are already backend-neutral in everything but name: a title, a year, a duration,
//! a resume point, an image path, a watched flag. What is backend-specific is how those fields
//! are FETCHED and PARSED, and that is the only thing this module replaces. A Jellyfin
//! `BaseItemDto` goes in one side of [`convert::movie_from_dto`] and a `PmsMovie` comes out of
//! the other, indistinguishable from one parsed off a PMS.
//!
//! That choice is deliberate, and the alternative was considered and rejected: introducing a
//! `trait MediaBackend` and migrating the UI onto it would have touched ~20 UI files before a
//! single Jellyfin byte could be validated on hardware. The adapter validates the backend FIRST;
//! a trait, if one is ever wanted, can name itself over code that is known to work.
//!
//! ## What "a server" means here
//!
//! Plex has a cloud account: servers are discovered through plex.tv, and a shared server arrives
//! with its own token. Jellyfin has none of that — a server is a URL the user types, and the
//! login is that server's own user and password. The PoC therefore registers exactly ONE
//! Jellyfin server, in the same registry slot model the Plex sources use ([`SERVER_ID`]), and the
//! onboarding asks for address + username + password instead of showing a plex.tv QR. The
//! multi-source machinery (`browse`'s `(sid, section)` addressing, `posters`'s per-server keying)
//! is flavor-blind and works unchanged with one Jellyfin source in it.
//!
//! ## The credential-transport rule this backend lives under
//!
//! Plex's `X-Plex-Token` may ride only TLS, because every PMS is reachable through a
//! `*.plex.direct` certificate. Jellyfin has no equivalent: a home Jellyfin server is plain HTTP
//! on a LAN address in the overwhelming majority of installs, so `crate::http`'s credential gate
//! was extended with a Jellyfin-only relaxation — its credential shapes (the
//! `Authorization: MediaBrowser` header, and the `api_key=` query that media/image/stream
//! URLs still carry) may cross plaintext ONLY to an address that cannot route off the LAN
//! (`crate::http::is_lan_host`); everywhere else stays refused. That is a considered weakening of
//! the transport floor for this backend alone, and it is why the onboarding asks for an IP
//! literal rather than a hostname.
//!
//! ## Units, spelled out once
//!
//! Jellyfin measures times in **ticks of 100 ns** (`RunTimeTicks`, `PlaybackPositionTicks`); the
//! app measures durations in NANOSECONDS and resume points in MILLISECONDS (Plex's units, baked
//! into `PmsMovie`). The conversions live in [`convert`] — multiply ticks by 100 for `dur_ns`,
//! divide by 10_000 for `resume_ms` — and nowhere else. A tick value must never reach a struct
//! field unconverted.

// PoC scaffolding: person pages and parts of the DTO surface are still unwired, so nothing
// outside the module consumes them yet, and warnings-are-errors would refuse the crate.
// Module-wide and TEMPORARY on purpose — the last consumer to land removes this attribute.
#![allow(dead_code, unused_imports)]

pub(crate) mod boot;
pub(crate) mod browse;
pub(crate) mod client;
mod convert;
pub(crate) mod detail;
mod dto;
pub(crate) mod profile;
pub(crate) mod search;
// The sign-in flow only exists on the flavor build: its persistence half (`boot::save_config`,
// `device_id_persistent`) is cfg-gated to it, and a Plex build has no form to drive it.
#[cfg(feature = "jellyfin")]
pub(crate) mod signin;

pub(crate) use client::{AuthError, AuthOk, JfClient};
pub(crate) use convert::movie_from_dto;
pub(crate) use dto::{BaseItemDto, ItemsResult, ViewDto};

use crate::plex::ServerId;

/// The registry slot the PoC's one Jellyfin server answers to. Jellyfin item ids are GUIDs and
/// cannot collide with a Plex ratingKey the way two Plex servers' integer keys do, but the row
/// identity rule is kept anyway — `(sid, rk)` stays the address of a catalog row, so a future
/// second Jellyfin server slots in without a structural change.
pub(crate) const SERVER_ID: ServerId = ServerId::from_raw(0);

/// `RwLock<Option<&'static …>>` rather than a `OnceLock`: sign-out must be able to RETIRE the
/// client mid-run (a OnceLock cannot), and the reference is leaked for the Plex registry's own
/// reason — a worker that captured it mid-request keeps a live client however the sign-out
/// lands. The lock is the only write path; `client()` is a read.
static CLIENT: std::sync::RwLock<Option<&'static JfClient>> = std::sync::RwLock::new(None);

/// Install the authenticated client. Called by the boot flow and by the sign-in form's worker,
/// after [`JfClient::authenticate_by_name`] has succeeded. A SECOND install (sign out → a
/// different server) replaces the first — the leaked predecessor stays alive for any worker
/// still holding it and is simply never handed out again.
pub(crate) fn install(client: JfClient) {
    let leaked: &'static JfClient = Box::leak(Box::new(client));
    *CLIENT.write().unwrap_or_else(|e| e.into_inner()) = Some(leaked);
}

/// Retire the installed client — the sign-out path. After this, every `sid == SERVER_ID` guard
/// in the stores fails closed until the next install.
pub(crate) fn uninstall() {
    *CLIENT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed client, `None` until the boot flow authenticates (and again after sign-out).
pub(crate) fn client() -> Option<&'static JfClient> {
    *CLIENT.read().unwrap_or_else(|e| e.into_inner())
}
