//! client — the typed Jellyfin server client.
//!
//! One `JfClient` is one server: an [`Origin`], the access token [`authenticate_by_name`] won,
//! and the user id that token acts as. It mirrors the shape of `plex::Client` on purpose — same
//! "built path carries the token, and the built path is also the cache key" discipline — so the
//! stores that were written against the Plex client (the poster LRU above all) read this one
//! without new concepts.
//!
//! ## Where the token rides
//!
//! Control requests carry it inside the `Authorization: MediaBrowser Token=...` HEADER (the
//! legacy `X-Emby-Token` line no longer authenticates on Jellyfin 12); built media/image paths carry it as
//! the `api_key` query parameter, because those paths double as byte-identical cache keys and
//! fetch paths, exactly as Plex's `X-Plex-Token`-suffixed paths do. Both spellings are on
//! `crate::http`'s credential gate (`http::credential_carried`): plaintext is allowed only to a
//! LAN address. **The token is never logged** — parse failures name the endpoint, which is the
//! part of the path before the query string.

use super::dto::{AuthResponse, BaseItemDto, ItemsResult, ViewsResult};
use crate::http;
use crate::plex::Origin;
use serde::de::DeserializeOwned;
use std::collections::BTreeSet;
use std::sync::{Mutex, RwLock};

/// The one login failure taxonomy the onboarding needs: wrong credentials must be told apart
/// from "the server did not answer", because the first is fixable on the on-screen keyboard and
/// the second is not. `Unauthorized` is exactly HTTP 401/403; every other completed response and
/// every transport failure folds into `Unreachable` — a 500 from the server is no more
/// typeable-around than a refused connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthError {
    Unauthorized,
    Unreachable,
}

/// What a successful login leaves behind.
#[derive(Debug)]
pub(crate) struct AuthOk {
    /// The Jellyfin access token — the credential every later request carries.
    pub(crate) token: String,
    /// The user's GUID, which every user-scoped endpoint (`/Users/{id}/Items`, played-state
    /// writes, playback progress) is addressed by.
    pub(crate) user_id: String,
    /// Display name, for the account menu's avatar row.
    pub(crate) user_name: String,
    /// The server's own id — see [`AuthResponse::server_id`].
    pub(crate) server_id: String,
}

/// The Library grid's full query shape — the Jellyfin counterpart of `plex::SectionQuery`. One
/// page of one view, with the sort/filter menus applied. All borrows, built at the spawn site and
/// moved into the worker as a unit.
#[derive(Clone, Copy)]
pub(crate) struct BrowseQuery<'a> {
    /// The view's GUID — `jellyfin::browse`'s side table resolved the synthetic i64 key back.
    pub(crate) view_id: &'a str,
    pub(crate) start: i64,
    pub(crate) limit: i64,
    /// Jellyfin's `IncludeItemTypes` vocabulary ("Movie", "Series") — the section kind, in the
    /// server's own spelling.
    pub(crate) include_types: &'a str,
    /// Jellyfin's `SortBy` vocabulary ("SortName", "PremiereDate", "DateCreated", …). Never
    /// empty: the adapter substitutes the Title default, so the listing always matches the chip.
    pub(crate) sort_by: &'a str,
    pub(crate) sort_desc: bool,
    /// `IsPlayed=false`. For a Series the server rolls the flag up from its episodes (played iff
    /// ALL are played), which makes this read as Plex's `unwatchedLeaves` — "has anything
    /// unwatched" — rather than as "never started".
    pub(crate) unwatched: bool,
    /// Genre GUIDs, comma-joined already; "" = unfiltered.
    pub(crate) genre_ids: &'a str,
}

pub(crate) struct JfClient {
    origin: Origin,
    /// Per-install stable id, minted once by the boot flow — Jellyfin's authorized-devices list
    /// keys on it the way plex.tv's keys on `X-Plex-Client-Identifier`.
    device_id: String,
    /// Set exactly once, at login. A `RwLock` rather than the Plex client's token-generation
    /// machinery because nothing here rotates: Jellyfin has no token refresh, only re-login.
    token: RwLock<Option<TokenState>>,
    /// The PlaySessionIds this client has opened with `/Sessions/Playing` and not yet closed —
    /// what turns a playback's FIRST `playing` tick into the session-opening POST and every later
    /// one into a Progress. Playback session ids are minted fresh per playback
    /// (`route::new_sess`), so entries never collide across films and each drains on Stopped.
    announced: Mutex<BTreeSet<String>>,
}

struct TokenState {
    token: String,
    user_id: String,
}

impl JfClient {
    pub(crate) fn new(origin: Origin, device_id: String) -> JfClient {
        JfClient {
            origin,
            device_id,
            token: RwLock::new(None),
            announced: Mutex::new(BTreeSet::new()),
        }
    }

    pub(crate) fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The `MediaBrowser` identity, as the standard `Authorization` header the login wants, and
    /// also the value Jellyfin shows in its own device list. Reuses the ONE product/version/
    /// device identity the Plex backend reports (`plex::identity`), so an install never
    /// describes itself two ways to two servers.
    ///
    /// Jellyfin 12 removed the legacy `X-Emby-Authorization` line this client used to send:
    /// the login POST answers a blanket 400 to it, identity or no identity (verified against
    /// this LAN's 12.0.0 server). The `MediaBrowser` scheme of `Authorization` was already
    /// accepted by every 10.x server, so this is the one shape that works across the split.
    fn identity_header(&self) -> String {
        format!(
            "Authorization: MediaBrowser Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
            crate::plex::identity::PRODUCT,
            crate::plex::identity::DEVICE,
            self.device_id,
            crate::plex::identity::VERSION,
        )
    }

    /// `POST /Users/AuthenticateByName`. On success the token is INSTALLED on this client —
    /// the boot flow constructs the client unauthenticated precisely so this method is the one
    /// place a token comes into existence.
    pub(crate) fn authenticate_by_name(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthOk, AuthError> {
        let body = serde_json::json!({
            "Username": username,
            "Pw": password,
        });
        // The identity header is a credential shape under the http gate's Jellyfin arm even
        // before it carries a token, and that is the correct reading: on a public address it
        // still must not ride plaintext.
        let reply = match http::request_post_json(
            &self.origin,
            "/Users/AuthenticateByName",
            &[&self.identity_header()],
            body.to_string().as_bytes(),
        ) {
            Some(r) => r,
            None => return Err(AuthError::Unreachable),
        };
        if reply.status == 401 || reply.status == 403 {
            return Err(AuthError::Unauthorized);
        }
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: AuthenticateByName answered {}",
                reply.status
            ));
            return Err(AuthError::Unreachable);
        }
        let parsed: AuthResponse = match serde_json::from_slice(&reply.body) {
            Ok(p) => p,
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: AuthenticateByName answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                return Err(AuthError::Unreachable);
            }
        };
        let ok = AuthOk {
            token: parsed.access_token,
            user_id: parsed.user.id,
            user_name: parsed.user.name,
            server_id: parsed.server_id,
        };
        *self.token.write().unwrap() = Some(TokenState {
            token: ok.token.clone(),
            user_id: ok.user_id.clone(),
        });
        Ok(ok)
    }

    /// The authed headers for a control request: token + identity in the ONE `Authorization`
    /// header. `None` before login — callers are all post-auth by construction, so `None`
    /// surfaces as a failed request, which is what the stores already do with an unreachable
    /// server.
    ///
    /// Jellyfin 12 removed BOTH legacy carriers at once: `X-Emby-Token` and the `api_key` query
    /// parameter no longer authenticate any control route (each answers 401; verified against
    /// this LAN's 12.0.0 server). The token therefore rides inside the `MediaBrowser`
    /// authorization next to the identity — accepted by 10.x and 12 alike — and the second
    /// slot states what every one of these calls wants anyway. Media/image/stream URLs built
    /// with `api_key=` are deliberately untouched: those routes still honour it.
    fn authed_headers(&self) -> Option<[String; 2]> {
        let token = {
            let state = self.token.read().unwrap();
            state.as_ref()?.token.clone()
        };
        let identity = self.identity_header();
        // identity_header is `Authorization: MediaBrowser Client=..., ...`; splice the token in
        // as the first parameter of that same scheme rather than emitting a second
        // Authorization line, which HTTP forbids and servers would fight over.
        let with_token = identity.replacen(
            "Authorization: MediaBrowser ",
            &format!("Authorization: MediaBrowser Token=\"{token}\", "),
            1,
        );
        Some([with_token, "Accept: application/json".to_string()])
    }

    /// One authed GET, parsed. `None` covers transport failure, non-2xx and unparseable JSON
    /// alike — the failure granularity the Plex stores were written against (`Option` from
    /// `fetch_built`), so the adapters land without new error plumbing. The log line names the
    /// ENDPOINT ONLY: `path` carries no token here (it rides in the header) but does carry the
    /// query a user typed when the endpoint is search.
    pub(crate) fn get_json<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(&self.origin, path, http::Method::Get, &refs)?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: GET {} answered {}",
                path.split('?').next().unwrap_or(path),
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice::<T>(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: GET {} answered {} bytes that will not parse — {e}",
                    path.split('?').next().unwrap_or(path),
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// The library list — Jellyfin's counterpart of `/library/sections`.
    pub(crate) fn views(&self) -> Option<ViewsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!("/Users/{user_id}/Views"))
    }

    /// The fields every LISTING asks for. MediaSources rides along because the direct-play gate
    /// and the codec badges both read it off the row; Overview feeds the hero's blurb. Kept as
    /// one constant so no two listing queries can drift into requesting different row shapes.
    const LISTING_FIELDS: &'static str = "Overview,MediaSources,OfficialRating,CommunityRating";

    /// Home's Continue Watching: `/Items/Resume` — in-progress movies and episodes, server-sorted
    /// by last-played, which is the deck's own order.
    pub(crate) fn resume(&self, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items/Resume?Limit={limit}&MediaTypes=Video\
             &Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        ))
    }

    /// One library's Recently Added shelf: `/Items/Latest` scoped to the view. Jellyfin's Latest
    /// already collapses episodes to their series for a tvshows view, which is the shelf shape
    /// Home wants (a row of seventeen episodes of one show is a bug report, not a shelf).
    ///
    /// Unlike every other listing here, `/Items/Latest` answers a BARE JSON ARRAY on current
    /// servers — but the DTO doc and one deployment each claimed the opposite, so this accepts
    /// both the array and the `{Items:[…], TotalRecordCount}` envelope instead of betting on
    /// which the television's server speaks.
    pub(crate) fn latest(&self, parent_id: &str, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        let path = format!(
            "/Users/{user_id}/Items/Latest?ParentId={parent_id}&Limit={limit}\
             &Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        );
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(&self.origin, &path, http::Method::Get, &refs)?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: GET {} answered {}",
                path.split('?').next().unwrap_or(&path),
                reply.status
            ));
            return None;
        }
        // Sniff the first non-space byte: '[' is the bare array (what current servers answer),
        // '{' the paged envelope (the shape the rest of this file's listings use).
        let trimmed: &[u8] = {
            let mut i = 0;
            while i < reply.body.len() && reply.body[i].is_ascii_whitespace() {
                i += 1;
            }
            &reply.body[i..]
        };
        if trimmed.first() == Some(&b'[') {
            match serde_json::from_slice::<Vec<BaseItemDto>>(trimmed) {
                Ok(items) => {
                    let total = items.len() as i64;
                    Some(ItemsResult { items, total })
                }
                Err(e) => {
                    crate::log(&format!(
                        "jellyfin: GET /Items/Latest (array) will not parse — {e}"
                    ));
                    None
                }
            }
        } else {
            serde_json::from_slice::<ItemsResult>(trimmed)
                .map_err(|e| {
                    crate::log(&format!(
                        "jellyfin: GET /Items/Latest (envelope) will not parse — {e}"
                    ));
                })
                .ok()
        }
    }

    /// One page of a library listing with the Library screen's whole query shape — the Jellyfin
    /// counterpart of `plex::Client::section_items_query`, and the ONLY listing method so no two
    /// callers can drift into requesting different row shapes. The caller (the browse adapter)
    /// owns the Plex-menu → Jellyfin-parameter translation; everything here rides the query
    /// string, which is Jellyfin's own convention for this endpoint.
    pub(crate) fn browse_page(&self, q: &BrowseQuery) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        let mut path = format!(
            "/Users/{user_id}/Items?ParentId={}&Recursive=true&IncludeItemTypes={}\
             &Fields={}&StartIndex={}&Limit={}&EnableImageTypes=Primary,Backdrop",
            q.view_id,
            q.include_types,
            Self::LISTING_FIELDS,
            q.start,
            q.limit
        );
        // SortBy/SortOrder ALWAYS go out (unlike Plex's "empty until the menus land"): Jellyfin's
        // server default is an implementation detail, while the toolbar's first chip says
        // "Title" — the listing must already be what the chip claims.
        path.push_str(&format!(
            "&SortBy={}&SortOrder={}",
            q.sort_by,
            if q.sort_desc { "Descending" } else { "Ascending" }
        ));
        if q.unwatched {
            path.push_str("&IsPlayed=false");
        }
        if !q.genre_ids.is_empty() {
            path.push_str(&format!("&GenreIds={}", q.genre_ids));
        }
        self.get_json(&path)
    }

    /// The genre value list of one library (`GET /Genres?ParentId=…`) — the filter menu's rows.
    /// Genres arrive as items with their own GUIDs, and the listing filter takes those ids back
    /// (`GenreIds`), so no name ever round-trips through a query string.
    pub(crate) fn genres(&self, view_id: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Genres?ParentId={view_id}&UserId={user_id}&SortBy=SortName&SortOrder=Ascending"
        ))
    }

    /// The media half of search: `GET /Items?searchTerm=` over the three playable kinds plus
    /// `BoxSet` (a collection opens a listing, not a detail page, so it rides the media page and
    /// the shelfing — `jellyfin::search` — projects it to a tag hit). The terms are
    /// percent-encoded HERE, at the wire boundary: they are user-typed text, and the one encoder
    /// the crate has (`pms::urlenc_str`, `QueryBuilder`'s choke point) is what encodes them.
    pub(crate) fn search_media(&self, terms: &str, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?searchTerm={}&Recursive=true\
             &IncludeItemTypes=Movie,Series,Episode,BoxSet\
             &Fields={}&Limit={limit}&EnableImageTypes=Primary,Backdrop",
            crate::pms::urlenc_str(terms),
            Self::LISTING_FIELDS
        ))
    }

    /// The people half of search (`GET /Persons?searchTerm=`), which `/Items` never returns.
    /// Kept a separate call so its failure degrades the Cast & Crew shelf to empty rather than
    /// failing the whole query — a person index is a nicety, the films are the search.
    pub(crate) fn search_people(&self, terms: &str, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Persons?searchTerm={}&UserId={user_id}&Limit={limit}&EnableImageTypes=Primary",
            crate::pms::urlenc_str(terms)
        ))
    }

    /// The server naming ITSELF (`GET /System/Info/Public`) — the Sources list's group header,
    /// Plex's `friendlyName` counterpart. The endpoint is public by design; this wrapper still
    /// rides the authed path like every call here, and discovery only ever asks post-login.
    pub(crate) fn server_name(&self) -> Option<String> {
        let info: super::dto::PublicInfo = self.get_json("/System/Info/Public")?;
        (!info.server_name.is_empty()).then_some(info.server_name)
    }

    /// GET raw bytes (the poster/artwork pipeline's fetch — the Jellyfin counterpart of
    /// `plex::Client::get_bytes`). The built image path carries its `api_key`, so no header is
    /// needed and none is sent: the path IS the credential, same as the Plex shape.
    pub(crate) fn get_bytes(&self, built_path: &str) -> Option<Vec<u8>> {
        let reply = http::request(&self.origin, built_path, http::Method::Get, &[])?;
        reply.ok().then_some(reply.body)
    }

    /// A person's filmography: `/Users/{uid}/Items` scoped by `PersonIds`, one page of one kind.
    /// `include_types` names the shelf ("Movie" or "Series" - or both, which is what the roles
    /// worker asks, since a credit can sit on either); `fields` rides the same vocabulary every
    /// listing uses, and is where "People" goes when the caller wants the per-item credit rows.
    /// SortBy=SortName pins an order the shelf spring does not have to re-derive.
    pub(crate) fn person_items(
        &self,
        person_id: &str,
        include_types: &str,
        fields: &str,
    ) -> Option<super::dto::ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?Recursive=true&PersonIds={person_id}\
             &IncludeItemTypes={include_types}&Fields={fields}&SortBy=SortName\
             &EnableImageTypes=Primary,Backdrop"
        ))
    }

    // ---- detail-page fetches -------------------------------------------------------------

    /// The fields the DETAIL page asks for, one constant for the same reason as
    /// [`Self::LISTING_FIELDS`]. Jellyfin answers a single-item GET as the bare `BaseItemDto`
    /// (no envelope), which is why this has its own method rather than sharing `items_page`'s.
    const DETAIL_FIELDS: &'static str =
        "MediaSources,Overview,Genres,People,Chapters,Taglines,ProductionLocations,OfficialRating,CommunityRating";

    /// One item's full record (`GET /Users/{uid}/Items/{id}`) — the detail page's main fetch,
    /// and also how a show borrows its hero episode's streams.
    pub(crate) fn item_detail(&self, item_id: &str) -> Option<super::dto::BaseItemDto> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items/{item_id}?Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::DETAIL_FIELDS
        ))
    }

    /// A show's seasons (`GET /Shows/{id}/Seasons`).
    pub(crate) fn seasons(&self, series_id: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Shows/{series_id}/Seasons?UserId={user_id}&Fields=Overview,OfficialRating\
             &EnableImageTypes=Primary,Backdrop"
        ))
    }

    /// One season's episodes (`GET /Shows/{id}/Episodes?SeasonId=…`). MediaSources rides along:
    /// the episode list's codec badges and per-episode direct-play answers read off the row.
    pub(crate) fn episodes(&self, series_id: &str, season_id: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Shows/{series_id}/Episodes?UserId={user_id}&SeasonId={season_id}\
             &Fields=Overview,MediaSources,OfficialRating&EnableImageTypes=Primary"
        ))
    }

    /// The episode the server says is next for this show (`GET /Shows/NextUp`) — the detail
    /// page's OnDeck. An empty result is a show never started or finished, NOT a failure: the
    /// caller distinguishes by `Items` emptiness, and only a transport/HTTP failure is `None`.
    pub(crate) fn next_up(&self, series_id: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Shows/NextUp?UserId={user_id}&SeriesId={series_id}&Limit=1\
             &Fields=Overview,MediaSources,OfficialRating&EnableImageTypes=Primary"
        ))
    }

    /// The Related shelf (`GET /Items/{id}/Similar`).
    pub(crate) fn similar(&self, item_id: &str, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Items/{item_id}/Similar?UserId={user_id}&Limit={limit}\
             &Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        ))
    }

    /// Intro/credits ranges (`GET /MediaSegments/{id}`, 10.9+). A 404 — older server, or no
    /// segment data — is folded to EMPTY HERE rather than by the caller: "this item has no
    /// markers" and "this server cannot say" draw the same UI (no Skip pill), and only the log
    /// owes the difference a line.
    pub(crate) fn media_segments(&self, item_id: &str) -> Option<super::dto::SegmentsResult> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(
            &self.origin,
            &format!("/MediaSegments/{item_id}"),
            http::Method::Get,
            &refs,
        )?;
        if reply.status == 404 {
            return Some(super::dto::SegmentsResult { items: Vec::new() });
        }
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: GET /MediaSegments answered {}",
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: GET /MediaSegments answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// Build an image request path — the Jellyfin counterpart of
    /// `plex::Client::image_transcode_path`, and like it BOTH the poster store's LRU key and the
    /// bytes it fetches. `src_path` is the stored image path (`/Items/{id}/Images/Primary?tag=…`,
    /// see [`super::convert`]); the server scales, so the size the tile wants rides as
    /// `fillWidth`/`fillHeight`, the direct analogue of Plex's `/photo/:/transcode` box.
    ///
    /// `png` is accepted for signature parity and deliberately ignored: Jellyfin's Primary
    /// images are JPEG and its Logo/Art types keep their own transparency — the PNG flag exists
    /// on the Plex side for clearLogos, which this backend reads through their own image types.
    pub(crate) fn image_path(&self, src_path: &str, w: i64, h: i64, png: bool) -> Option<String> {
        if png {
            // Not a failure — a Logo/Art request must be built from its own path by the caller.
            crate::log("jellyfin: image_path called with png=1 (clearLogo path); serving JPEG box");
        }
        let token = self.token.read().unwrap().as_ref()?.token.clone();
        // `src_path` is sometimes bare (`/Items/{id}/Images/Backdrop/0`, built by
        // [`super::convert`]) and sometimes already query-bearing (`…/Primary?tag=…`), so the
        // sizing/auth query hangs off whichever separator is right. Appending `&` to a path with
        // no `?` yields `…/Backdrop/0&fillWidth=…`, which the server rejects with 400 and which
        // left every backdrop — hero, detail page, player ground — blank.
        let sep = if src_path.contains('?') { '&' } else { '?' };
        Some(format!(
            "{src_path}{sep}fillWidth={w}&fillHeight={h}&quality=90&api_key={token}"
        ))
    }

    /// The direct-play URL for an item: the file itself, unaltered, with the auth riding as
    /// `api_key` like every other built path. `static=true` is Jellyfin's "no remux, no
    /// transcode" switch — the byte-exact counterpart of Plex's direct-play `part` URL.
    /// `media_source_id` pins the VERSION (Jellyfin's `part`): empty means the server's default
    /// source, which is also the right answer when the detail fetch found exactly one.
    pub(crate) fn direct_stream_url(&self, item_id: &str, media_source_id: &str) -> Option<String> {
        let token = self.token.read().unwrap().as_ref()?.token.clone();
        let ms = if media_source_id.is_empty() {
            String::new()
        } else {
            format!("&MediaSourceId={media_source_id}")
        };
        Some(format!(
            "{}/Videos/{item_id}/stream?static=true{ms}&api_key={token}",
            self.origin.base()
        ))
    }

    /// The Media Decision Engine: `POST /Items/{id}/PlaybackInfo` with this television's
    /// DeviceProfile (built by [`crate::jellyfin::profile`] off the same devcaps table the local
    /// gate reads). Called only when the LOCAL gate has already refused direct play — so the
    /// answer we want back is a `TranscodingUrl`, and a source without one is a refusal, not a
    /// second opinion. `None` = unreachable / unparseable, folded into the caller's failed plan
    /// exactly like a `/decision` that never answered.
    pub(crate) fn playback_info(
        &self,
        item_id: &str,
        media_source_id: &str,
        body: &serde_json::Value,
    ) -> Option<super::dto::PlaybackInfoResult> {
        let user_id = self.user_id()?;
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let mut path = format!("/Items/{item_id}/PlaybackInfo?UserId={user_id}");
        if !media_source_id.is_empty() {
            path.push_str(&format!("&MediaSourceId={media_source_id}"));
        }
        // UserId rides BOTH the query (Jellyfin's routing reads it there) and the body (the
        // session binder reads it there); the builder leaves it out on purpose so no caller
        // ever assembles credential-adjacent state — fill it here, at the last moment.
        let mut body = body.clone();
        body["UserId"] = serde_json::Value::String(user_id);
        let reply = http::request_post_json(
            &self.origin,
            &path,
            &refs,
            body.to_string().as_bytes(),
        )?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: POST PlaybackInfo answered {}",
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice::<super::dto::PlaybackInfoResult>(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: PlaybackInfo answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// Absolutize a `TranscodingUrl` with the auth riding as `api_key`, the same shape as every
    /// built path — the HLS playlist and every segment it lists are fetched by URL alone, with
    /// no header slot to carry a token in.
    pub(crate) fn transcode_url(&self, transcoding_url: &str) -> Option<String> {
        let token = self.token.read().unwrap().as_ref()?.token.clone();
        let sep = if transcoding_url.contains('?') { "&" } else { "?" };
        Some(format!(
            "{}{}{}api_key={token}",
            self.origin.base(),
            transcoding_url,
            sep
        ))
    }

    // ---- playback session reporting ---------------------------------------------------------
    //
    // Jellyfin's progress protocol is three JSON POSTs where Plex's is one query-stringed POST:
    // `/Sessions/Playing` opens the session, `/Sessions/Playing/Progress` carries every later
    // tick (a pause included, as `IsPaused`), and `/Sessions/Playing/Stopped` closes it — and
    // the STOP is what commits the resume point (and, past the server's own threshold, the
    // watched flag) to UserData. The body below is the subset of jellyfin-web's reportPlayback*
    // shape this app can speak truthfully: ItemId/PositionTicks/IsPaused/PlaySessionId/CanSeek.
    // PlayMethod and MediaSourceId feed only the dashboard's Now Playing detail and are left out
    // on purpose — nothing functional reads them. All four endpoints were verified on the wire
    // against a live 10.11 server: each answers 204, and the Stopped position shows up in
    // `/Items/Resume` immediately after.
    //
    // `play_session_id` arrives ALREADY resolved by the caller: the transcode's id when one
    // exists, the playback's own logical session id otherwise — the rule `jellyfin::profile`'s
    // doc states, which `route`'s timeline projection and scrobble both implement.

    /// ms → Jellyfin ticks (100 ns units). The OUTBOUND half of the module's units rule —
    /// `convert` owns the inbound half — and it sits at this wire boundary for the same reason:
    /// no caller of these methods ever sees a tick.
    fn ms_to_ticks(ms: i64) -> i64 {
        ms.saturating_mul(10_000)
    }

    fn session_body(
        item_id: &str,
        position_ms: i64,
        paused: bool,
        play_session_id: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "ItemId": item_id,
            "PositionTicks": Self::ms_to_ticks(position_ms),
            "IsPaused": paused,
            "CanSeek": true,
            "PlaySessionId": play_session_id,
        })
    }

    /// A `playing`/`paused` timeline tick — the whole of `route::report_timeline`'s job on this
    /// backend. The first tick of a session OPENS it with `/Sessions/Playing` (which records a
    /// position too); every later one is a Progress. The distinction is tracked here rather than
    /// in the caller because the reporter thread has no playback lifecycle of its own — it is
    /// spawned after the plan lands and must not re-open a session that is already open. An open
    /// the server did not take is un-marked, so the next tick retries it instead of settling for
    /// Progress on a session the server never saw.
    pub(crate) fn session_progress(
        &self,
        item_id: &str,
        position_ms: i64,
        paused: bool,
        play_session_id: &str,
    ) -> bool {
        let first = {
            let mut ann = self.announced.lock().unwrap_or_else(|e| e.into_inner());
            ann.insert(play_session_id.to_string())
        };
        let path = if first {
            "/Sessions/Playing"
        } else {
            "/Sessions/Playing/Progress"
        };
        let ok = self.post_status(
            path,
            &Self::session_body(item_id, position_ms, paused, play_session_id),
        );
        if first && !ok {
            self.announced
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(play_session_id);
        }
        ok
    }

    /// The end-of-playback report — the one that COMMITS the resume point (and past the server's
    /// own threshold the watched flag) to UserData, so it always goes out as `/Stopped`,
    /// announced or not: a stop for a session whose open never landed is still the position the
    /// user left at.
    pub(crate) fn session_stopped(
        &self,
        item_id: &str,
        position_ms: i64,
        play_session_id: &str,
    ) -> bool {
        self.announced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(play_session_id);
        self.post_status(
            "/Sessions/Playing/Stopped",
            &Self::session_body(item_id, position_ms, false, play_session_id),
        )
    }

    /// Kill a server-side transcode: `DELETE /Videos/ActiveEncodings?playSessionId=…`, what
    /// jellyfin-web calls when HLS playback ends. Best-effort for the same reason Plex's
    /// transcode stop is: the server reaps the encoder on its own once the playlist goes quiet,
    /// so a false here costs one log line and nothing else.
    pub(crate) fn stop_active_encodings(&self, play_session_id: &str) -> bool {
        self.request_status(
            &format!(
                "/Videos/ActiveEncodings?playSessionId={play_session_id}&deviceId={}",
                self.device_id
            ),
            http::Method::Delete,
        )
    }

    // ---- view-state writes ------------------------------------------------------------------
    //
    // The three `viewstate` writes in Jellyfin spellings. Watched/Unwatched map exactly:
    // POST/DELETE `/Users/{uid}/PlayedItems/{id}`, and the DELETE clears PlayCount AND the resume
    // point — precisely the "an unscrobble throws two away" `viewstate`'s module doc prices.
    // Deck removal has NO exact counterpart: Jellyfin's Continue Watching is not a list an item
    // can be hidden from, it is the set of items holding a resume point — so the honest mapping
    // discards the point (a UserData merge with `PlaybackPositionTicks: 0`, Played untouched,
    // verified against 10.11), where Plex's removeFromContinueWatching keeps it. The deck row
    // disappears the same; what differs is that coming back later restarts the film.

    /// Mark played: `POST /Users/{uid}/PlayedItems/{id}` (bodyless — the endpoint takes no
    /// payload, and the id rides the path).
    pub(crate) fn mark_played(&self, item_id: &str) -> bool {
        let Some(user_id) = self.user_id() else {
            return false;
        };
        self.request_status(
            &format!("/Users/{user_id}/PlayedItems/{item_id}"),
            http::Method::Post,
        )
    }

    /// Mark unplayed: `DELETE /Users/{uid}/PlayedItems/{id}` — clears the flag, the count and
    /// the resume point, the exact semantics `viewstate` documents for an unscrobble.
    pub(crate) fn mark_unplayed(&self, item_id: &str) -> bool {
        let Some(user_id) = self.user_id() else {
            return false;
        };
        self.request_status(
            &format!("/Users/{user_id}/PlayedItems/{item_id}"),
            http::Method::Delete,
        )
    }

    /// Remove from Continue Watching: `POST /Users/{uid}/Items/{id}/UserData` as a MERGE — only
    /// `PlaybackPositionTicks` is sent, and the server keeps Played/PlayCount as they are. See
    /// the section header for what this costs versus Plex's hide-but-keep-resume.
    pub(crate) fn clear_resume(&self, item_id: &str) -> bool {
        let Some(user_id) = self.user_id() else {
            return false;
        };
        self.post_status(
            &format!("/Users/{user_id}/Items/{item_id}/UserData"),
            &serde_json::json!({ "PlaybackPositionTicks": 0 }),
        )
    }

    // ---- status-only write plumbing -----------------------------------------------------------

    /// One JSON POST whose whole answer is the status code: true on any 2xx (Jellyfin answers
    /// 204 to the session calls, 200 with a UserItemDataDto to the user-data one — both are
    /// "taken"), false on transport failure or refusal. Fail-closed before login like every
    /// method here, and the failure line names the ENDPOINT only.
    fn post_status(&self, path: &str, body: &serde_json::Value) -> bool {
        let Some(headers) = self.authed_headers() else {
            return false;
        };
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        match http::request_post_json(&self.origin, path, &refs, body.to_string().as_bytes()) {
            Some(r) if r.ok() => true,
            Some(r) => {
                crate::log(&format!(
                    "jellyfin: POST {} answered {}",
                    path.split('?').next().unwrap_or(path),
                    r.status
                ));
                false
            }
            None => {
                crate::log(&format!(
                    "jellyfin: POST {} — no answer",
                    path.split('?').next().unwrap_or(path)
                ));
                false
            }
        }
    }

    /// The bodyless twin of [`post_status`] — Jellyfin's DELETEs and its played-mark POST take
    /// their parameters in the path/query, so `http::request` (which sends an empty body for a
    /// POST and none for a DELETE) is the right door.
    fn request_status(&self, path: &str, method: http::Method) -> bool {
        let Some(headers) = self.authed_headers() else {
            return false;
        };
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        match http::request(&self.origin, path, method, &refs) {
            Some(r) if r.ok() => true,
            Some(r) => {
                crate::log(&format!(
                    "jellyfin: {} {} answered {}",
                    method_name(method),
                    path.split('?').next().unwrap_or(path),
                    r.status
                ));
                false
            }
            None => {
                crate::log(&format!(
                    "jellyfin: {} {} — no answer",
                    method_name(method),
                    path.split('?').next().unwrap_or(path)
                ));
                false
            }
        }
    }

    fn user_id(&self) -> Option<String> {
        Some(self.token.read().unwrap().as_ref()?.user_id.clone())
    }
}

/// `http::Method`'s token is private to the transport; the log line wants the same spelling.
fn method_name(m: http::Method) -> &'static str {
    match m {
        http::Method::Get => "GET",
        http::Method::Put => "PUT",
        http::Method::Post => "POST",
        http::Method::Delete => "DELETE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// One mock Jellyfin server on loopback: accepts exactly `responses.len()` connections
    /// (every request here is `Connection: close`), records each raw request, and answers with
    /// the queued body in order. Loopback is inside the http gate's LAN arm, so the credential
    /// policy being tested is the REAL one, not a bypass.
    struct MockServer {
        port: u16,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn start(responses: Vec<(i32, &'static str)>) -> MockServer {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let requests2 = requests.clone();
            let join = std::thread::spawn(move || {
                for (status, body) in responses {
                    let (mut socket, _) = listener.accept().expect("accept");
                    // Read head AND the Content-Length'd body — the whole point of the fixture
                    // is to see what the transport put on the wire, and leaving the body unread
                    // would RST the socket under the client's response read on some kernels.
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let mut content_length = None::<usize>;
                    let mut head_end = None::<usize>;
                    loop {
                        let n = socket.read(&mut chunk).expect("read");
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if head_end.is_none() {
                            if let Some(pos) = find(&buf, b"\r\n\r\n") {
                                head_end = Some(pos + 4);
                                let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                                content_length = head
                                    .lines()
                                    .find_map(|l| l.strip_prefix("content-length:"))
                                    .and_then(|v| v.trim().parse().ok());
                            }
                        }
                        if let (Some(he), Some(cl)) = (head_end, content_length) {
                            if buf.len() >= he + cl {
                                break;
                            }
                        } else if head_end.is_some() && content_length.is_none() {
                            break;
                        }
                    }
                    requests2
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf).into_owned());
                    let reason = if status == 200 { "OK" } else { "Error" };
                    write!(socket, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                        .expect("write");
                }
            });
            MockServer {
                port,
                requests,
                join: Some(join),
            }
        }

        fn finish(self) -> Vec<String> {
            let mut this = self;
            if let Some(j) = this.join.take() {
                j.join().expect("server thread");
            }
            let recorded = std::mem::take(&mut *this.requests.lock().unwrap());
            recorded
        }
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    const AUTH_OK: &str = r#"{"User":{"Id":"u-1","Name":"demo"},"AccessToken":"tok-1","ServerId":"srv-1","SessionInfo":null}"#;

    #[test]
    fn authenticate_posts_a_json_body_and_installs_the_token() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":0}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());

        let ok = client
            .authenticate_by_name("demo", "")
            .expect("auth succeeds");
        assert_eq!(ok.user_id, "u-1");
        assert_eq!(ok.server_id, "srv-1");

        // A later control request must carry the token inside the Authorization header.
        let views = client.views().expect("views parse");
        assert!(views.items.is_empty());

        let reqs = server.finish();
        assert_eq!(reqs.len(), 2);
        let auth = &reqs[0];
        assert!(auth.starts_with("POST /Users/AuthenticateByName HTTP/1.1"));
        assert!(auth.to_ascii_lowercase().contains("content-type: application/json"));
        assert!(auth.contains("Authorization: MediaBrowser Client=\"PlxNative\""));
        // Key order in the body is serde_json's (alphabetical without preserve_order) — assert
        // the two fields, not one literal, so the test does not hinge on serializer internals.
        assert!(auth.contains(r#""Username":"demo""#));
        assert!(auth.contains(r#""Pw":"""#));
        let views_req = &reqs[1];
        assert!(views_req.starts_with("GET /Users/u-1/Views HTTP/1.1"));
        assert!(views_req.contains("Authorization: MediaBrowser Token=\"tok-1\", "));
    }

    #[test]
    fn a_401_is_unauthorized_not_unreachable() {
        let server = MockServer::start(vec![(401, "{}")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        assert!(matches!(
            client.authenticate_by_name("demo", "wrong"),
            Err(AuthError::Unauthorized)
        ));
        server.finish();
    }

    #[test]
    fn a_failed_auth_installs_no_token_and_later_requests_fail_closed() {
        let server = MockServer::start(vec![(401, "{}")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        let _ = client.authenticate_by_name("demo", "wrong");
        // No token → no request is even attempted: the gate is in the client, not the server.
        assert!(client.views().is_none());
        assert!(client.image_path("/Items/x/Images/Primary", 100, 150, false).is_none());
        server.finish();
    }

    #[test]
    fn image_and_stream_paths_carry_the_size_and_the_token() {
        let server = MockServer::start(vec![(200, AUTH_OK)]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();
        let p = client
            .image_path("/Items/abc/Images/Primary?tag=t1", 300, 450, false)
            .unwrap();
        assert_eq!(
            p,
            "/Items/abc/Images/Primary?tag=t1&fillWidth=300&fillHeight=450&quality=90&api_key=tok-1"
        );
        let u = client.direct_stream_url("abc", "").unwrap();
        assert_eq!(
            u,
            format!("http://127.0.0.1:{}/Videos/abc/stream?static=true&api_key=tok-1", server.port)
        );
        // a picked version pins the MediaSource
        let u = client.direct_stream_url("abc", "ms-9").unwrap();
        assert_eq!(
            u,
            format!("http://127.0.0.1:{}/Videos/abc/stream?static=true&MediaSourceId=ms-9&api_key=tok-1", server.port)
        );
        server.finish();
    }

    #[test]
    fn a_queryless_image_path_starts_its_own_query() {
        let server = MockServer::start(vec![(200, AUTH_OK)]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();
        // The backdrop path convert.rs builds carries no query of its own, so the sizing/auth
        // query must OPEN with `?` — hanging it off `&` yields `…/Backdrop/0&fillWidth=…`, which
        // the server rejects with 400 and which left every backdrop (hero, detail, player ground)
        // blank.
        let p = client
            .image_path("/Items/abc/Images/Backdrop/0", 1280, 720, false)
            .unwrap();
        assert_eq!(
            p,
            "/Items/abc/Images/Backdrop/0?fillWidth=1280&fillHeight=720&quality=90&api_key=tok-1"
        );
        server.finish();
    }

    /// One playback's worth of session reports: the first tick OPENS (`/Sessions/Playing`), the
    /// second is a Progress carrying the pause, and the stop closes. The body fields and the
    /// ms→ticks conversion are asserted off the wire, not the unit's own word for them.
    #[test]
    fn a_session_opens_progresses_and_stops() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (204, ""),
            (204, ""),
            (204, ""),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        assert!(client.session_progress("mv-1", 60_000, false, "ps-1")); // opens
        assert!(client.session_progress("mv-1", 70_000, true, "ps-1")); // paused tick
        assert!(client.session_stopped("mv-1", 70_000, "ps-1"));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 4);
        let open = &reqs[1];
        assert!(open.starts_with("POST /Sessions/Playing HTTP/1.1"));
        assert!(open.contains("\"ItemId\":\"mv-1\""));
        assert!(open.contains("\"PositionTicks\":600000000")); // 60 s in 100 ns ticks
        assert!(open.contains("\"IsPaused\":false"));
        assert!(open.contains("\"PlaySessionId\":\"ps-1\""));
        assert!(open.contains("Authorization: MediaBrowser Token=\"tok-1\", "));
        let tick = &reqs[2];
        assert!(tick.starts_with("POST /Sessions/Playing/Progress HTTP/1.1"));
        assert!(tick.contains("\"IsPaused\":true"));
        assert!(tick.contains("\"PositionTicks\":700000000"));
        let stop = &reqs[3];
        assert!(stop.starts_with("POST /Sessions/Playing/Stopped HTTP/1.1"));
        assert!(stop.contains("\"PlaySessionId\":\"ps-1\""));
    }

    /// An open the server refused must not latch the session as announced: the next tick retries
    /// `/Sessions/Playing` rather than reporting Progress on a session the server never saw.
    #[test]
    fn a_refused_open_is_retried_as_an_open() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (500, ""),
            (204, ""),
            (204, ""),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        assert!(!client.session_progress("mv-1", 0, false, "ps-2")); // open refused
        assert!(client.session_progress("mv-1", 10_000, false, "ps-2")); // retried open
        assert!(client.session_progress("mv-1", 20_000, false, "ps-2")); // now a Progress

        let reqs = server.finish();
        assert_eq!(reqs.len(), 4);
        assert!(reqs[1].starts_with("POST /Sessions/Playing HTTP/1.1"));
        assert!(reqs[2].starts_with("POST /Sessions/Playing HTTP/1.1"));
        assert!(reqs[3].starts_with("POST /Sessions/Playing/Progress HTTP/1.1"));
    }

    /// A stop for a session whose open never landed still goes out as `/Stopped` — it is the
    /// report that commits the resume point, announced or not.
    #[test]
    fn a_stop_needs_no_prior_open() {
        let server = MockServer::start(vec![(200, AUTH_OK), (204, "")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        assert!(client.session_stopped("mv-1", 5_000, "ps-never-opened"));
        let reqs = server.finish();
        assert_eq!(reqs.len(), 2);
        assert!(reqs[1].starts_with("POST /Sessions/Playing/Stopped HTTP/1.1"));
        assert!(reqs[1].contains("\"PositionTicks\":50000000"));
    }

    /// The three view-state writes: played is a bodyless POST, unplayed a DELETE on the same
    /// path, and deck removal a UserData MERGE carrying only the zeroed position.
    #[test]
    fn view_state_writes_hit_the_user_scoped_endpoints() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, "{}"),
            (200, "{}"),
            (200, "{}"),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        assert!(client.mark_played("mv-1"));
        assert!(client.mark_unplayed("mv-1"));
        assert!(client.clear_resume("mv-1"));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 4);
        assert!(reqs[1].starts_with("POST /Users/u-1/PlayedItems/mv-1 HTTP/1.1"));
        assert!(reqs[1].contains("Authorization: MediaBrowser Token=\"tok-1\", "));
        assert!(reqs[2].starts_with("DELETE /Users/u-1/PlayedItems/mv-1 HTTP/1.1"));
        let clear = &reqs[3];
        assert!(clear.starts_with("POST /Users/u-1/Items/mv-1/UserData HTTP/1.1"));
        assert!(clear.contains("\"PlaybackPositionTicks\":0"));
        // the merge sends NOTHING else — Played/PlayCount are the server's to keep
        assert!(!clear.contains("\"Played\""));
    }

    /// The Library grid's query puts every menu on the query string: sort + direction always
    /// (the chip says "Title" from the first page), the unwatched filter as IsPlayed, and the
    /// genre as its GUID.
    #[test]
    fn a_browse_page_carries_the_whole_query() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":183}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let res = client
            .browse_page(&BrowseQuery {
                view_id: "view-1",
                start: 60,
                limit: 60,
                include_types: "Series",
                sort_by: "PremiereDate",
                sort_desc: true,
                unwatched: true,
                genre_ids: "g-1,g-2",
            })
            .expect("page parses");
        assert_eq!(res.total, 183);

        let reqs = server.finish();
        assert_eq!(reqs.len(), 2);
        let head = reqs[1].lines().next().unwrap();
        assert!(head.starts_with("GET /Users/u-1/Items?"));
        for want in [
            "ParentId=view-1",
            "IncludeItemTypes=Series",
            "StartIndex=60",
            "Limit=60",
            "SortBy=PremiereDate",
            "SortOrder=Descending",
            "IsPlayed=false",
            "GenreIds=g-1,g-2",
        ] {
            assert!(head.contains(want), "missing {want} in {head}");
        }
        assert!(reqs[1].contains("Authorization: MediaBrowser Token=\"tok-1\", "));
    }

    /// Genres and the server's own name ride the same authed GET path as every other read.
    #[test]
    fn genres_and_server_name_are_plain_authed_gets() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[{"Id":"g-1","Name":"Drama","Type":"Genre"}],"TotalRecordCount":1}"#),
            (200, r#"{"ServerName":"attic","Id":"srv-1","Version":"10.11.11"}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let genres = client.genres("view-1").expect("genres parse");
        assert_eq!(genres.items[0].name, "Drama");
        assert_eq!(client.server_name().as_deref(), Some("attic"));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 3);
        let g = reqs[1].lines().next().unwrap();
        assert!(g.starts_with("GET /Genres?"));
        assert!(g.contains("ParentId=view-1"));
        assert!(g.contains("UserId=u-1"));
        assert!(reqs[2].starts_with("GET /System/Info/Public HTTP/1.1"));
    }

    /// Search encodes the typed terms at the wire boundary, scopes media to the playable kinds
    /// plus BoxSet, and keeps people on their own endpoint.
    #[test]
    fn search_encodes_terms_and_splits_media_from_people() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":0}"#),
            (200, r#"{"Items":[],"TotalRecordCount":0}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        client.search_media("Amélie & Co", 12).expect("media parses");
        client.search_people("Amélie & Co", 12).expect("people parse");

        let reqs = server.finish();
        assert_eq!(reqs.len(), 3);
        let media = reqs[1].lines().next().unwrap();
        assert!(media.starts_with("GET /Users/u-1/Items?"));
        assert!(media.contains("searchTerm=Am%C3%A9lie%20%26%20Co"), "terms encoded: {media}");
        assert!(media.contains("IncludeItemTypes=Movie,Series,Episode,BoxSet"));
        let people = reqs[2].lines().next().unwrap();
        assert!(people.starts_with("GET /Persons?"));
        assert!(people.contains("searchTerm=Am%C3%A9lie%20%26%20Co"));
        assert!(people.contains("UserId=u-1"));
        assert!(reqs[1].contains("Authorization: MediaBrowser Token=\"tok-1\", "));
    }

    /// The transcode kill is a DELETE naming the PlaySessionId, and every write here fails
    /// closed before login.
    #[test]
    fn encodings_stop_is_a_delete_and_writes_fail_closed_without_a_token() {
        let server = MockServer::start(vec![(200, AUTH_OK), (200, "")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        assert!(!client.mark_played("mv-1"));
        assert!(!client.session_progress("mv-1", 0, false, "ps-x"));
        client.authenticate_by_name("demo", "").unwrap();

        assert!(client.stop_active_encodings("ps-9"));
        let reqs = server.finish();
        assert_eq!(reqs.len(), 2); // auth + the delete; the fail-closed calls never hit the wire
        assert!(reqs[1]
            .starts_with("DELETE /Videos/ActiveEncodings?playSessionId=ps-9&deviceId=dev-1 HTTP/1.1"));
    }
}
