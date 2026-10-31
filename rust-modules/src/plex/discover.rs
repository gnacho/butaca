//! Wire DTOs for a person's biography and filmography.
//!
//! This module USED to be the plex.tv metadata provider (`discover.provider.plex.tv`) client:
//! `AccountClient::person_profile`/`person_credits` fetched the biography half of a person the
//! local PMS does not have, plus the global filmography. Those fetch paths are GONE (the app is
//! Jellyfin-only; `person.rs`'s Plex spawn arm was removed with them). What survives is the DTO
//! vocabulary the person screen is shaped around — `PersonProfile`, `CreditGroup`, `Credit`,
//! `CreditItem`, `CreditType` — which the Jellyfin backend's pure mappers (`person.rs`'s
//! `jf_profile_from`, `jf_shelf`, `jf_roles`) build from `BaseItemDto` answers and the mailbox
//! landings carry. The wire facts in the field docs below describe the plex.tv responses these
//! types were modelled from; they remain the reference for why each field is optional and what
//! each id space meant, and are kept verbatim so re-reading a captured response stays possible.
//!
//! Two facts worth keeping (each cost a probe when the provider was live):
//! * Only the `tagKey` guid addressed a person here; the numeric `Tag::id` PMS accepts elsewhere
//!   answered `404` on this endpoint. The two id spaces are the local library's and plex.tv's.
//! * `Accept: application/json` was load-bearing: without it the provider answered XML, exactly
//!   like PMS does.
use serde::Deserialize;

use super::models::de_i64;

// ---- serde DTOs (only the fields the page consumes; all optional to tolerate shape drift) ----

/// One person as plex.tv knows them. Every field is optional: a living person has no
/// [`died_at`](PersonProfile::died_at), a person plex.tv has no record for has nothing at all, and
/// the page is composed to read as finished in both cases.
///
/// Deliberately NOT modelled: `slug` / `ratingKey` / `metadataId` / `Image[]` / `External[]`.
/// `External[]` is the social handles (`facebook`/`instagram`/`twitter`) — see the person page's
/// module docs for why a TV does not show them. Serde ignores unknown fields, so adding one back is
/// a field, not a migration.
#[derive(Deserialize, Default)]
#[derive(serde::Serialize)]
pub struct PersonProfile {
    #[serde(default)]
    pub title: String,
    /// The biography — the whole reason this endpoint is called. Several paragraphs of prose,
    /// `\n\n`-separated, usually ending in a Wikipedia CC-BY-SA attribution sentence.
    #[serde(default)]
    pub summary: String,
    /// ISO `YYYY-MM-DD`. Absent → unknown, never "not born".
    #[serde(rename = "bornAt", default)]
    pub born_at: String,
    /// ISO `YYYY-MM-DD`, present only for someone who has died.
    #[serde(rename = "diedAt", default)]
    pub died_at: String,
    #[serde(rename = "birthPlace", default)]
    pub birth_place: String,
    /// The single department Plex leads with ("Acting"). The roles LINE comes from
    /// [`credit_types`](PersonProfile::credit_types) instead — it is the full list.
    #[serde(rename = "knownFor", default)]
    pub known_for: String,
    /// Headshot on `metadata-static.plex.tv`. Usually the same URL the local `Role[]` row carries,
    /// so the page keeps drawing the one it was handed rather than swapping textures mid-fetch.
    #[serde(default)]
    pub thumb: String,
    /// The person's departments, wire order = most credits first: `[{type:"actor",title:"Actor"},
    /// {type:"producer",title:"Producer"}, …]`. This is Plex's "Actor, Producer" roles line, and
    /// (via a `count` we deliberately do not model yet) the counts its Filmography tabs show.
    #[serde(rename = "CreditType", default)]
    pub credit_types: Vec<CreditType>,
}

/// One department a person is credited in. `title` is the display form — except when the provider
/// has no display name for a department, where it repeats the raw `type` verbatim
/// (`"costume-makeup"`, seen live on Peter Sallis). `crate::person::roles_line` is what cleans that
/// up; do not print `title` raw.
#[derive(Deserialize, Default)]
#[derive(serde::Serialize)]
pub struct CreditType {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
}

// ---- the filmography ---------------------------------------------------------------------------

/// One DEPARTMENT of a person's filmography — `actor`, `appeared`, `other`, `writer`, … in the
/// provider's own order, which is most-credits-first.
///
/// `title` is the display form and repeats the raw `type` slug when the provider has no display
/// name for a department (`"costume-makeup"`, live on Peter Sallis) — exactly the trap
/// [`CreditType`] carries, and it is cleaned by the same [`crate::person::pretty_department`].
#[derive(Deserialize, Default)]
#[derive(serde::Serialize)]
pub struct CreditGroup {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// The group's own row count where the provider states one, `0` where it does not — in which
    /// case `Credit.len()` IS the count. Never [`CreditType`]'s number; see the module doc.
    #[serde(default, deserialize_with = "de_i64")]
    pub size: i64,
    #[serde(rename = "Credit", default)]
    pub credits: Vec<Credit>,
}

/// One credit inside a department: what they were in it, and the item itself.
#[derive(Deserialize, Default)]
#[derive(serde::Serialize)]
pub struct Credit {
    /// The billing order the provider sorted the cast by. Modelled because it is on the wire and
    /// costs nothing; the screen sorts by YEAR, so nothing reads it yet.
    #[serde(default, deserialize_with = "de_i64")]
    pub order: i64,
    /// The character or the job — `"Wallace (voice)"`, `"Self"`, `"Soundtrack"`. Empty where the
    /// provider names none, and the row then carries no second line rather than an empty one.
    #[serde(default)]
    pub role: String,
    /// **A single object on the wire, not an array**, which is why this is an `Option`: a credit is
    /// one item. A credit that somehow carries none is dropped by the store rather than drawn as a
    /// row with no title.
    #[serde(rename = "Metadata", default)]
    pub item: Option<CreditItem>,
}

/// The item a credit is FOR — a **Discover** row, not a library one. Its `rating_key` is a plex.tv
/// id and its `thumb` an `image.tmdb.org` URL, so neither addresses anything on a PMS; what does
/// cross is [`guid`](CreditItem::guid), which is the metadata provider's global id and the key both
/// sides of the availability join speak (`plex::Metadata::guid` on the library side).
#[derive(Deserialize, Default)]
#[derive(serde::Serialize)]
pub struct CreditItem {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// `0` is the ONLY absent value PMS and the provider have for a year, so "announced" and "the
    /// metadata is missing" are the same wire fact. The screen states neither: an undated credit
    /// reads as an em dash and sorts to the bottom.
    #[serde(default, deserialize_with = "de_i64")]
    pub year: i64,
    /// **THE join key, and it is NOT a guid** — a bare catalog ID, `5d7768295af944001f1f7477`.
    ///
    /// This endpoint carries **no `guid` field at all** (measured 2026-09-06; the item's whole key
    /// set is `art`/`key`/`originallyAvailableAt`/`publicPagesURL`/`ratingKey`/`slug`/`thumb`/
    /// `title`/`type`/`year`). PMS states the same identity as `plex://movie/<this>`, so the join
    /// is against the TAIL of a local row's guid — `person::guid_tail`. A `guid: String` was
    /// modelled here at first, defaulted to empty on every row, and the availability join it fed
    /// therefore matched NOTHING while every count around it looked healthy: `joinable=5`, 544 rows
    /// drawn, not one of them markable. If a `guid` ever does appear on this wire, prefer it —
    /// but do not model it again without a response in front of you.
    #[serde(rename = "ratingKey", default)]
    pub rating_key: String,
    /// **An ABSOLUTE URL on somebody else's host** — `https://image.tmdb.org/…` or
    /// `https://metadata-static.plex.tv/…`, never a PMS path. That is not a reason to skip the
    /// artwork: `app::adapters::poster::built_key` URL-encodes exactly such a URL into a
    /// `/photo/:/transcode?url=…` request and the SERVER fetches it, which is the same path the
    /// Search screen's `actor` headshots already take (`app/adapters/poster.rs`'s own tests pin it). This
    /// module's first draft asserted the opposite in a comment — "a tmdb URL no PMS transcoder will
    /// serve" — and hard-coded an empty plate on the strength of it, which is why the Filmography
    /// shipped with no posters at all. Nothing was measured before that claim was written down.
    #[serde(default)]
    pub thumb: String,
}
