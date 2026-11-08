//! **Identify**, replacing an item's metadata match with another the providers offered (#369).
//!
//! Two server calls and one model:
//!
//! * `POST /Items/RemoteSearch/Movie` answers the candidates a metadata provider (TMDB, OMDb, …)
//!   can match an item to. It is a POST with a typed body, not the GET older documentation shows:
//!   verified against Jellyfin's own OpenAPI, the same shape serves 10.9 through 12.x, and the
//!   client's `remote_search_movie` sends it.
//! * `POST /Items/RemoteSearch/Apply/{itemId}` takes one candidate back, re-reads its provider ids
//!   and refreshes the item from them. **The endpoint requires elevation**, so an ordinary viewer's
//!   account gets a refusal, which this module reports as a plain `false` rather than dressing up.
//!
//! **Only a MOVIE can be identified from this client.** Jellyfin's lookup routes exist for
//! `Movie`, `Series`, `BoxSet`, `MusicAlbum`, `MusicArtist`, `MusicVideo`, `Person`, `Trailer` and
//! `Book`, and **not** for `Episode`, which is the other leaf the pre-playback window can stand on.
//! An episode's match belongs to its series, i.e. to a different item than the one the window is
//! about, so the window's menu offers no Identify there (see `screens::preplay_menu`).
//!
//! This module is the WORKER's half: it holds no state. The loaded item, the candidates and the
//! phase live in `crate::metadata`'s Identify store, which is what a screen reads.

use crate::registry::ServerId;

/// One candidate, as a row of the Identify list reads it, plus the object the server expects back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IdentifyHit {
    /// The candidate exactly as the server sent it. It is posted back VERBATIM on Apply: the server
    /// looks the item up by the provider ids inside it, and rebuilding that payload here would be
    /// this client inventing a query the viewer never picked.
    pub(crate) raw: super::dto::RemoteSearchResultDto,
    /// The candidate's title, what the row's label says. Empty only if a provider answered with no
    /// name at all, which the row then shows as its year alone.
    pub(crate) name: String,
    /// The candidate's year, 0 when the provider sent none (the row drops the year then).
    pub(crate) year: i32,
    /// WHICH provider offered it ("TheMovieDb", "The Open Movie Database"). Two providers routinely
    /// return the same film and only this tells the rows apart, so it is the row's read-out.
    pub(crate) provider: String,
}

impl IdentifyHit {
    fn from_dto(raw: super::dto::RemoteSearchResultDto) -> Self {
        Self {
            name: raw.name.clone().unwrap_or_default(),
            year: raw.production_year.unwrap_or(0),
            provider: raw.search_provider_name.clone().unwrap_or_default(),
            raw,
        }
    }
}

/// The candidates for one movie. `None` is a transport/auth/parse failure, the list and "the
/// server did not answer" are different answers and the store keeps them apart.
pub(crate) fn search(sid: ServerId, item_id: &str, name: &str, year: i32) -> Option<Vec<IdentifyHit>> {
    let client = super::client()?;
    let hits = client.remote_search_movie(item_id, name, year)?;
    crate::log(&format!(
        "identify: rk={item_id} sid={sid:?} - {} candidate(s)",
        hits.len()
    ));
    Some(hits.into_iter().map(IdentifyHit::from_dto).collect())
}

/// Apply one candidate. `false` covers every refusal the endpoint can give, transport, a non-admin
/// account (403), an item the server no longer has (404), and the surface says so in one line.
pub(crate) fn apply(sid: ServerId, item_id: &str, hit: &IdentifyHit) -> bool {
    let Some(client) = super::client() else {
        return false;
    };
    let ok = client.apply_remote_search(item_id, &hit.raw);
    crate::log(&format!(
        "identify: rk={item_id} sid={sid:?} provider={} apply ok={ok}",
        hit.provider
    ));
    ok
}

/// TEST ONLY: one candidate with the fields a row reads, so a store test can post the answer a
/// worker would have produced without a server. Compiled out of the shipped binary.
#[cfg(test)]
pub(crate) fn hit_for_test(name: &str, year: i32, provider: &str) -> IdentifyHit {
    IdentifyHit {
        raw: super::dto::RemoteSearchResultDto {
            name: Some(name.to_string()),
            production_year: Some(year),
            search_provider_name: Some(provider.to_string()),
            ..Default::default()
        },
        name: name.to_string(),
        year,
        provider: provider.to_string(),
    }
}
