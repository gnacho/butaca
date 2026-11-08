//! client — the typed Jellyfin server client.
//!
//! One `JfClient` is one server: an [`Origin`], the access token [`authenticate_by_name`] won,
//! and the user id that token acts as. It mirrors the shape of `server::Client` on purpose — same
//! "built path carries the token, and the built path is also the cache key" discipline — so the
//! stores that were written against the retired vendor client (the poster LRU above all) read
//! this one without new concepts.
//!
//! ## Where the token rides
//!
//! Control requests carry it inside the `Authorization: MediaBrowser Token=...` HEADER (the
//! legacy `X-Emby-Token` line no longer authenticates on Jellyfin 12); built media/image paths carry it as
//! the `api_key` query parameter, because those paths double as byte-identical cache keys and
//! fetch paths, exactly as the retired wire's token-suffixed paths did. Both spellings are on
//! `crate::http`'s credential gate (`http::credential_carried`): plaintext is allowed only to a
//! LAN address. **The token is never logged** — parse failures name the endpoint, which is the
//! part of the path before the query string.

use super::dto::{AuthResponse, AuthUser, BaseItemDto, ItemsResult, QuickConnectResult, TrickplayInfoDto, ViewsResult};
use crate::http;
use crate::origin::Origin;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, RwLock};

/// What a stored token resolves to: the account it was minted for. Narrower than [`AuthOk`]
/// because `GET /Users/Me` names no server id — the token was minted against one and the boot arm
/// only needs the user id every scoped endpoint is addressed by, plus the name the chip shows.
#[derive(Debug, Clone)]
pub(crate) struct TokenUser {
    pub(crate) user_id: String,
    pub(crate) user_name: String,
}

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

/// The Library grid's full query shape — the Jellyfin counterpart of `server::SectionQuery`. One
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
    /// ALL are played), which makes this read as the old server's `unwatchedLeaves` ("has anything
    /// unwatched") rather than as "never started".
    pub(crate) unwatched: bool,
    /// Genre GUIDs, comma-joined already; "" = unfiltered.
    pub(crate) genre_ids: &'a str,
}

pub(crate) struct JfClient {
    origin: Origin,
    /// Per-install stable id, minted once by the boot flow — Jellyfin's authorized-devices list
    /// keys on it the way the retired account service keyed on its client identifier.
    device_id: String,
    /// Set exactly once, at login. A `RwLock` rather than the retired client's token-generation
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
    /// The display name the sign-in answered (`AuthOk.user_name`) — the Home chip reads it
    /// (`Session::account`'s jellyfin arm), so a signed-in user never sees "Sign in".
    user_name: String,
}

/// Parse a dotted `Major.Minor[.Patch...]` version string and compare it against a floor. Used
/// only to gate Quick Connect (10.8.0). A malformed or shorter-than-two-component version answers
/// false, so a server that reports no parseable version is treated as not supporting the feature.
fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    let mut parts = version.split('.');
    let maj = parts.next().and_then(|p| p.trim().parse::<u32>().ok());
    let min = parts.next().and_then(|p| p.trim().parse::<u32>().ok());
    match (maj, min) {
        (Some(m), _) if m > major => true,
        (Some(m), Some(n)) if m == major && n >= minor => true,
        _ => false,
    }
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

    /// The controlled replay world's constructor (#166 replay flip): an ALREADY-authenticated
    /// client built from the typed Initial's identity, with no network round-trip of any kind.
    /// The controlled boot must construct the exact world the recording replays, and a
    /// `/Users/Me` probe here would be an unrecorded request replay can never answer. The
    /// values themselves are synthetic (`Initial::synthetic_home` invents them from the seed);
    /// the recording's data fetches are what proves the world real.
    pub(crate) fn synthetic(origin: Origin, device_id: String, token: String,
        user_id: String, user_name: String) -> JfClient {
        JfClient {
            origin,
            device_id,
            token: RwLock::new(Some(TokenState { token, user_id, user_name })),
            announced: Mutex::new(BTreeSet::new()),
        }
    }

    pub(crate) fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The `MediaBrowser` identity, as the standard `Authorization` header the login wants, and
    /// also the value Jellyfin shows in its own device list. The Client is the DISPLAY name (the
    /// fork's own build says Butaca there — issue #35), while Device/DeviceId/Version keep the
    /// one wire identity the shared identity module reports (`server::identity`), so an install
    /// never describes its device two ways to two servers.
    ///
    /// Jellyfin 12 removed the legacy `X-Emby-Authorization` line this client used to send:
    /// the login POST answers a blanket 400 to it, identity or no identity (verified against
    /// this LAN's 12.0.0 server). The `MediaBrowser` scheme of `Authorization` was already
    /// accepted by every 10.x server, so this is the one shape that works across the split.
    fn identity_header(&self) -> String {
        format!(
            "Authorization: MediaBrowser Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
            crate::server::identity::display_name(),
            crate::server::identity::DEVICE,
            self.device_id,
            crate::server::identity::VERSION,
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
        let _anchor = super::SigninAnchor::enter(&self.origin);
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
        *self.token.write().unwrap_or_else(|e| e.into_inner()) = Some(TokenState {
            token: ok.token.clone(),
            user_id: ok.user_id.clone(),
            user_name: ok.user_name.clone(),
        });
        Ok(ok)
    }

    /// `POST /QuickConnect/Initiate` — start a Quick Connect session. Unauthenticated: the
    /// identity header names the device, and the answer's `Secret`/`Code` are the session
    /// credential, not a user token. `None` on transport failure, a non-2xx (401 = Quick Connect
    /// disabled on the server), or unparseable JSON.
    pub(crate) fn quick_connect_initiate(&self) -> Option<QuickConnectResult> {
        let _anchor = super::SigninAnchor::enter(&self.origin);
        let id = self.identity_header();
        let reply = http::request(
            &self.origin,
            "/QuickConnect/Initiate",
            http::Method::Post,
            &[id.as_str()],
            None,
        )?;
        if !reply.ok() {
            crate::log(&format!("jellyfin: QuickConnect/Initiate answered {}", reply.status));
            return None;
        }
        match serde_json::from_slice::<QuickConnectResult>(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: QuickConnect/Initiate answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// `GET /QuickConnect/Connect?secret=` — the poll: has this Quick Connect request been
    /// authorized on another device yet? The `secret` is what `quick_connect_initiate` returned;
    /// it rides in the query because this route authenticates on the secret alone. `None` on
    /// transport failure, non-2xx (404 = unknown secret), or unparseable JSON.
    pub(crate) fn quick_connect_state(&self, secret: &str) -> Option<QuickConnectResult> {
        let _anchor = super::SigninAnchor::enter(&self.origin);
        let id = self.identity_header();
        let path = format!("/QuickConnect/Connect?secret={secret}");
        let reply = http::request(&self.origin, &path, http::Method::Get, &[id.as_str()], None)?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: QuickConnect/Connect answered {}",
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice::<QuickConnectResult>(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: QuickConnect/Connect answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// `POST /Users/AuthenticateWithQuickConnect` — trade an AUTHORIZED Quick Connect secret for
    /// the access token. Answers the same `AuthenticationResult` shape as
    /// [`authenticate_by_name`], and installs the token exactly as that method does. A 400/401/403
    /// (not authorized, unknown secret, disabled) folds into `Unauthorized` so the flow can fall
    /// back to the password form without inventing a fourth failure word.
    pub(crate) fn authenticate_with_quick_connect(&self, secret: &str) -> Result<AuthOk, AuthError> {
        let _anchor = super::SigninAnchor::enter(&self.origin);
        let body = serde_json::json!({ "Secret": secret });
        let id = self.identity_header();
        let reply = match http::request_post_json(
            &self.origin,
            "/Users/AuthenticateWithQuickConnect",
            &[id.as_str()],
            body.to_string().as_bytes(),
        ) {
            Some(r) => r,
            None => return Err(AuthError::Unreachable),
        };
        if matches!(reply.status, 400 | 401 | 403) {
            return Err(AuthError::Unauthorized);
        }
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: AuthenticateWithQuickConnect answered {}",
                reply.status
            ));
            return Err(AuthError::Unreachable);
        }
        let parsed: AuthResponse = match serde_json::from_slice(&reply.body) {
            Ok(p) => p,
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: AuthenticateWithQuickConnect answered {} bytes that will not parse — {e}",
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
        *self.token.write().unwrap_or_else(|e| e.into_inner()) = Some(TokenState {
            token: ok.token.clone(),
            user_id: ok.user_id.clone(),
            user_name: ok.user_name.clone(),
        });
        Ok(ok)
    }

    /// Whether this server can answer Quick Connect (added in Jellyfin 10.8.0). Reads the public
    /// system info WITHOUT a token — `GET /System/Info/Public` is unauthenticated by design — and
    /// compares the reported version against the floor. A malformed or absent version answers
    /// false, which falls back to the password login rather than gambling on a route that 404s.
    pub(crate) fn quick_connect_supported(&self) -> bool {
        let _anchor = super::SigninAnchor::enter(&self.origin);
        let Some(reply) = http::request(
            &self.origin,
            "/System/Info/Public",
            http::Method::Get,
            &[crate::http::ACCEPT_JSON],
            None,
        ) else {
            return false;
        };
        if !reply.ok() {
            return false;
        }
        let Ok(info) = serde_json::from_slice::<super::dto::PublicInfo>(&reply.body) else {
            return false;
        };
        version_at_least(&info.version, 10, 8)
    }

    /// Adopt a token an earlier run persisted: install it, then ask the server who it belongs to.
    ///
    /// `GET /Users/Me` is the cheapest route that needs nothing but the token, which makes it both
    /// the account lookup and the validity probe: a revoked or expired token answers 401 and the
    /// boot arm can fall back to the password form. The token stays installed only when the probe
    /// answered — a refusal clears it, so no later `views()` fires credentials the server has
    /// already rejected. This is the whole reason a password no longer has to live on disk.
    pub(crate) fn adopt_token(&self, token: &str) -> Result<TokenUser, AuthError> {
        let _anchor = super::SigninAnchor::enter(&self.origin);
        let install = |user_id: &str, user_name: &str| {
            *self.token.write().unwrap_or_else(|e| e.into_inner()) = Some(TokenState {
                token: token.to_string(),
                user_id: user_id.to_string(),
                user_name: user_name.to_string(),
            });
        };
        install("", "");
        let Some(headers) = self.authed_headers() else {
            return Err(AuthError::Unreachable);
        };
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = match http::request(&self.origin, "/Users/Me", http::Method::Get, &refs, None) {
            Some(r) => r,
            None => {
                *self.token.write().unwrap_or_else(|e| e.into_inner()) = None;
                return Err(AuthError::Unreachable);
            }
        };
        if reply.status == 401 || reply.status == 403 {
            *self.token.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(AuthError::Unauthorized);
        }
        if !reply.ok() {
            crate::log(&format!("jellyfin: GET /Users/Me answered {}", reply.status));
            *self.token.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(AuthError::Unreachable);
        }
        let me: AuthUser = match serde_json::from_slice(&reply.body) {
            Ok(me) => me,
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: GET /Users/Me answered {} bytes that will not parse — {e}",
                    reply.body.len()
                ));
                *self.token.write().unwrap_or_else(|e| e.into_inner()) = None;
                return Err(AuthError::Unreachable);
            }
        };
        install(&me.id, &me.name);
        Ok(TokenUser {
            user_id: me.id,
            user_name: me.name,
        })
    }

    /// The signed-in user's display name, once the login landed (None before it). The account
    /// chip reads it: the Jellyfin token names the user, there is no session record to ask.
    pub(crate) fn user_name(&self) -> Option<String> {
        self.token.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|t| t.user_name.clone())
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
            let state = self.token.read().unwrap_or_else(|e| e.into_inner());
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
    /// alike: the failure granularity the earlier stores were written against (`Option` from
    /// `fetch_built`), so the adapters land without new error plumbing. The log line names the
    /// ENDPOINT ONLY: `path` carries no token here (it rides in the header) but does carry the
    /// query a user typed when the endpoint is search.
    pub(crate) fn get_json<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(&self.origin, path, http::Method::Get, &refs, None)?;
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

    /// One authed JSON POST whose answer is parsed, [`get_json`](Self::get_json)'s write twin.
    /// Jellyfin's LOOKUP endpoints take their query in a body (`POST /Items/RemoteSearch/Movie`),
    /// so a path-only GET shape cannot reach them; the failure granularity is the same `Option`.
    pub(crate) fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Option<T> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request_post_json(&self.origin, path, &refs, body.to_string().as_bytes())?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: POST {} answered {}",
                path.split('?').next().unwrap_or(path),
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice::<T>(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: POST {} answered {} bytes that will not parse - {e}",
                    path.split('?').next().unwrap_or(path),
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// **Identify, first half**: the metadata providers' candidates for an item , 
    /// `POST /Items/RemoteSearch/Movie` with a `MovieInfoRemoteSearchQuery` body. Verified against
    /// Jellyfin's own OpenAPI: the typed POST has been the shape since 10.9 and is still the shape
    /// on 12.x, which is the range this client targets. The item's `Id` rides the body so the
    /// providers may reuse the ids it already carries, and `IncludeDisabledProviders` stays off:
    /// a provider the owner switched off is not a candidate.
    pub(crate) fn remote_search_movie(
        &self,
        item_id: &str,
        name: &str,
        year: i32,
    ) -> Option<Vec<super::dto::RemoteSearchResultDto>> {
        let body = serde_json::json!({
            "SearchInfo": {
                "Name": name,
                "Year": (year > 0).then_some(year),
            },
            "ItemId": item_id,
            "IncludeDisabledProviders": false,
        });
        self.post_json("/Items/RemoteSearch/Movie", &body)
    }

    /// **Identify, second half**: apply one candidate , 
    /// `POST /Items/RemoteSearch/Apply/{itemId}`, whose body is the candidate itself. The server
    /// re-reads its `ProviderIds` and refreshes the item, so the candidate travels back exactly as
    /// the search sent it (see `RemoteSearchResultDto`). `replaceAllImages` is left at the server's
    /// own default (`true`): the viewer chose this match, and a half-replaced image set is the
    /// worse answer. The endpoint requires elevation, so a non-admin account is a plain `false`.
    pub(crate) fn apply_remote_search(
        &self,
        item_id: &str,
        hit: &super::dto::RemoteSearchResultDto,
    ) -> bool {
        let Ok(body) = serde_json::to_value(hit) else {
            return false;
        };
        self.post_status(&format!("/Items/RemoteSearch/Apply/{item_id}"), &body)
    }

    /// **Edit images, first half**: the pictures the metadata providers already have for one image
    /// type of one item, `GET /Items/{itemId}/RemoteImages?type=…&startIndex=…&limit=…`. Verified
    /// against Jellyfin's own OpenAPI, which is the shape on 10.9 through 13.x. `includeAllLanguages`
    /// is left at the server's default (false): the picker is a television in one language, and a
    /// poster in a language the household does not read is not a candidate worth decoding.
    pub(crate) fn remote_images(
        &self,
        item_id: &str,
        kind: &str,
        start: i32,
        limit: i32,
    ) -> Option<super::dto::RemoteImageResultDto> {
        self.get_json(&format!(
            "/Items/{item_id}/RemoteImages?type={kind}&startIndex={start}&limit={limit}"
        ))
    }

    /// **Edit images, second half**: `POST /Items/{itemId}/RemoteImages/Download?type=…&imageUrl=…`
    /// — the server fetches the provider's picture ITSELF and makes it the item's image of that
    /// type. The URL is the one the list answered with, percent-encoded here because it carries
    /// its own `?`/`&`. Every parameter rides the query string and the body is empty, which is
    /// what [`request_status`](Self::request_status) is for. The endpoint requires elevation, so
    /// an ordinary viewer's account is a plain `false`.
    pub(crate) fn download_remote_image(&self, item_id: &str, kind: &str, image_url: &str) -> bool {
        self.request_status(
            &format!(
                "/Items/{item_id}/RemoteImages/Download?type={kind}&imageUrl={}",
                crate::catalog::urlenc_str(image_url)
            ),
            http::Method::Post,
        )
    }

    /// **The local intros** (`GET /Items/{itemId}/Intros`), the `IIntroProvider` seam the Local
    /// Intros plugin plugs into. `Fields=MediaSources` rides along because the intro is played as
    /// its own item and `request_play` addresses the media source, not the item.
    ///
    /// `Some(vec![])` and `None` mean different things here: an empty list is "this server has no
    /// intro for this item" (the ordinary answer without the plugin, which installs no provider
    /// and leaves the route answering `Items: []`), `None` is a request that failed.
    pub(crate) fn intros(&self, item_id: &str) -> Option<Vec<super::dto::BaseItemDto>> {
        let user_id = self.user_id()?;
        let result: super::dto::IntrosResult = self.get_json(&format!(
            "/Items/{item_id}/Intros?UserId={user_id}&Fields=MediaSources"
        ))?;
        Some(result.items)
    }

    /// The library list — Jellyfin's counterpart of `/library/sections`.
    pub(crate) fn views(&self) -> Option<ViewsResult> {        let user_id = self.user_id()?;
        self.get_json(&format!("/Users/{user_id}/Views"))
    }    /// The fields every LISTING asks for. MediaSources rides along because the direct-play gate
    /// and the codec badges both read it off the row; Overview feeds the hero's blurb. Kept as
    /// one constant so no two listing queries can drift into requesting different row shapes.
    const LISTING_FIELDS: &'static str = "Overview,MediaSources,OfficialRating,CommunityRating";

    /// Home's Continue Watching: `/Items/Resume` — in-progress movies and episodes, server-sorted
    /// by last-played, which is the deck's own order.
    ///
    /// `IncludeItemTypes` is spelled out because newer servers roll progress up to the series and
    /// season folders and list those as resumable too. A folder on the deck has no play target,
    /// so the row would carry cards that cannot be resumed.
    pub(crate) fn resume(&self, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items/Resume?Limit={limit}&MediaTypes=Video\
             &IncludeItemTypes=Movie,Episode,Video\
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
        self.get_items_loose(&path, "/Items/Latest")
    }

    /// A film's extras: the server's special features (behind the scenes, deleted scenes,
    /// interviews, featurettes) followed by its local trailers, once each. Both routes answer a
    /// bare item array. A route that fails is logged and the other still counts: extras are a
    /// shelf, and one server hiccup must not empty it.
    pub(crate) fn extras(&self, item_id: &str) -> Vec<BaseItemDto> {
        let Some(user_id) = self.user_id() else {
            return Vec::new();
        };
        let mut all: Vec<BaseItemDto> = Vec::new();
        for path in [
            format!(
                "/Items/{item_id}/SpecialFeatures?userId={user_id}&Fields={}",
                Self::LISTING_FIELDS
            ),
            format!(
                "/Items/{item_id}/LocalTrailers?userId={user_id}&Fields={}",
                Self::LISTING_FIELDS
            ),
        ] {
            let Some(page) = self.get_items_loose(&path, "extras") else {
                continue;
            };
            for it in page.items {
                if !all.iter().any(|e| e.id == it.id) {
                    all.push(it);
                }
            }
        }
        all
    }

    /// A GET whose answer may be either shape: the bare array current servers send for
    /// `/Items/Latest`, `/Items/{id}/SpecialFeatures` and `/Items/{id}/LocalTrailers`, or the
    /// `{Items:[…],TotalRecordCount}` envelope every other listing uses. `what` names the route in
    /// the log.
    fn get_items_loose(&self, path: &str, what: &str) -> Option<ItemsResult> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(&self.origin, path, http::Method::Get, &refs, None)?;
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: GET {} answered {}",
                path.split('?').next().unwrap_or(path),
                reply.status
            ));
            return None;
        }
        // Sniff the first non-space byte: '[' is the bare array, '{' the paged envelope.
        let mut i = 0;
        while i < reply.body.len() && reply.body[i].is_ascii_whitespace() {
            i += 1;
        }
        let body = &reply.body[i..];
        if body.first() == Some(&b'[') {
            match serde_json::from_slice::<Vec<BaseItemDto>>(body) {
                Ok(items) => {
                    let total = items.len() as i64;
                    Some(ItemsResult { items, total })
                }
                Err(e) => {
                    crate::log(&format!(
                        "jellyfin: GET {what} (array) will not parse - {e}"
                    ));
                    None
                }
            }
        } else {
            serde_json::from_slice::<ItemsResult>(body)
                .map_err(|e| {
                    crate::log(&format!(
                        "jellyfin: GET {what} (envelope) will not parse - {e}"
                    ));
                })
                .ok()
        }
    }

    /// One page of a library listing with the Library screen's whole query shape — the Jellyfin
    /// counterpart of `server::Client::section_items_query`, and the ONLY listing method so no two
    /// callers can drift into requesting different row shapes. The caller (the browse adapter)
    /// owns the toolbar-menu → Jellyfin-parameter translation, and everything here rides the query
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
        // SortBy/SortOrder ALWAYS go out (unlike the retired wire's "empty until the menus land"):
        // Jellyfin's server default is an implementation detail, while the toolbar's first chip
        // says "Title", so the listing must already be what the chip claims.
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

    /// One page of a library's BoxSets — the Collections type's listing, scoped to the same view
    /// a `browse_page` asks for. A box set has no playable leaves of its own, so the fixed
    /// `SortName` ascending order is the type's one ordering (see
    /// `jellyfin::browse::collection_sorts`) and no watch-state or genre filter is ever sent. The
    /// rows arrive with `Type="BoxSet"` and convert through `convert::boxset_from_dto`.
    pub(crate) fn boxsets(&self, view_id: &str, start: i64, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?ParentId={view_id}&IncludeItemTypes=BoxSet&Recursive=true\
             &SortBy=SortName&SortOrder=Ascending&StartIndex={start}&Limit={limit}\
             &Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        ))
    }

    /// One page of a BoxSet's members — the collection screen's children fetch. `ParentId` is the
    /// box set's id (the row's `rk`), and the members are Movie rows converted by the same
    /// `convert::movie_from_dto` every listing uses. No `IncludeItemTypes` is sent: the box set's
    /// own children ARE its members, and an unexpected kind is still dropped by the converter
    /// rather than misfilled.
    /// One page of the account's favourite items — the strip's Favorites grid. No `ParentId`: the
    /// destination spans every library the viewer can see, and `Filters=IsFavorite` is the
    /// server's own filter, so the page is the same shape a library listing has without being
    /// scoped to one section.
    pub(crate) fn favorites_page(&self, start: i64, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?Recursive=true&IncludeItemTypes=Movie,Series&Filters=IsFavorite\
             &SortBy=SortName&SortOrder=Ascending&StartIndex={start}&Limit={limit}\
             &Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        ))
    }

    pub(crate) fn collection_children(
        &self,
        boxset_id: &str,
        start: i64,
        limit: i64,
    ) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?ParentId={boxset_id}&SortBy=SortName&SortOrder=Ascending\
             &StartIndex={start}&Limit={limit}&Fields={}&EnableImageTypes=Primary,Backdrop",
            Self::LISTING_FIELDS
        ))
    }

    /// Per-letter item counts of one library for the A-Z rail — the Jellyfin counterpart of
    /// the retired wire's `firstCharacter` directory. Jellyfin has no per-letter counts call, but
    /// `NameStartsWith` (verified to filter on `SortName`, the field the listing's
    /// `SortBy=SortName` order rides) with `Limit=0` yields `TotalRecordCount` without
    /// transferring a single item, so 27 lightweight queries replace the one call. The
    /// non-alphabetic bucket is the unfiltered total minus the A-Z sum: digits and punctuation
    /// sort before letters, so it leads the rail, exactly like the retired wire's "#".
    pub(crate) fn letter_counts(
        &self,
        view_id: &str,
        include_types: &str,
        filter: &crate::browse::LettersFilter,
    ) -> Option<Vec<(String, i64)>> {
        let user_id = self.user_id()?;
        // The counts MUST describe exactly the listing the rail rides on — with a filter active
        // (issue #37) the probe carries it, which is what makes the rail stay honest under one:
        // `NameStartsWith` accepts `IsPlayed` and `GenreIds` alongside `ParentId`.
        let mut scope = String::new();
        if filter.unwatched {
            scope.push_str("&IsPlayed=false");
        }
        if let Some(genre) = &filter.genre {
            scope.push_str(&format!("&GenreIds={genre}"));
        }
        let total = self.count_query(&user_id, view_id, include_types, &scope)?;
        let mut counts = Vec::new();
        for b in b'A'..=b'Z' {
            let letter = (b as char).to_string();
            let n = self
                .count_query(
                    &user_id,
                    view_id,
                    include_types,
                    &format!("{scope}&NameStartsWith={letter}"),
                )
                .unwrap_or(0);
            if n > 0 {
                counts.push((letter, n));
            }
        }
        Some(build_letters(total, counts))
    }

    /// One `Limit=0` count query: `TotalRecordCount` rides the envelope, no items transfer.
    fn count_query(
        &self,
        user_id: &str,
        view_id: &str,
        include_types: &str,
        extra: &str,
    ) -> Option<i64> {
        let res: ItemsResult = self.get_json(&format!(
            "/Users/{user_id}/Items?ParentId={view_id}&Recursive=true\
             &IncludeItemTypes={include_types}&Limit=0{extra}"
        ))?;
        Some(res.total)
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
    /// the crate has (`catalog::urlenc_str`, `QueryBuilder`'s choke point) is what encodes them.
    pub(crate) fn search_media(&self, terms: &str, limit: i64) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Users/{user_id}/Items?searchTerm={}&Recursive=true\
             &IncludeItemTypes=Movie,Series,Episode,BoxSet\
             &Fields={}&Limit={limit}&EnableImageTypes=Primary,Backdrop",
            crate::catalog::urlenc_str(terms),
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
            crate::catalog::urlenc_str(terms)
        ))
    }

    /// The server naming ITSELF (`GET /System/Info/Public`) — the Sources list's group header,
    /// the counterpart of the retired `friendlyName`. The endpoint is public by design, and this
    /// wrapper still rides the authed path like every call here, asked only post-login.
    pub(crate) fn server_name(&self) -> Option<String> {
        let info: super::dto::PublicInfo = self.get_json("/System/Info/Public")?;
        (!info.server_name.is_empty()).then_some(info.server_name)
    }

    /// GET raw bytes (the poster/artwork pipeline's fetch — the Jellyfin counterpart of
    /// `server::Client::get_bytes`). The built image path carries its `api_key`, so no header is
    /// needed and none is sent: the path IS the credential, same as the retired shape.
    ///
    /// **An absolute URL is dialled as it stands, and headerless.** A remote image a metadata
    /// provider offered (*Edit images*) is NOT on this server: it is a picture on TheMovieDb's CDN
    /// or whatever host the provider named. Sending this client's credentials there would leak the
    /// household's token to a third party, so the absolute arm sends no headers at all — it is a
    /// plain public GET, which is exactly what the provider's own client does.
    pub(crate) fn get_bytes(&self, built_path: &str) -> Option<Vec<u8>> {
        if let Some((origin, path)) = absolute_http_url(built_path) {
            let reply = http::request(&origin, path, http::Method::Get, &[], None)?;
            return reply.ok().then_some(reply.body);
        }
        let reply = http::request(&self.origin, built_path, http::Method::Get, &[], None)?;
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

    /// The item's trickplay manifest (`GET /Users/{uid}/Items/{id}?Fields=Trickplay`), or `None`
    /// when the server has generated none. The outer key is the tile width as a string ("320");
    /// the seek-bar reader picks a width and fetches the sheet at
    /// `/Videos/{id}/Trickplay/{width}/{index}.jpg`. Data layer only: fetching and decoding the
    /// sheets is deliberately NOT here.
    pub(crate) fn trickplay_info(
        &self,
        item_id: &str,
    ) -> Option<BTreeMap<String, BTreeMap<String, TrickplayInfoDto>>> {
        let user_id = self.user_id()?;
        let dto: BaseItemDto =
            self.get_json(&format!("/Users/{user_id}/Items/{item_id}?Fields=Trickplay"))?;
        if dto.trickplay.is_empty() {
            None
        } else {
            Some(dto.trickplay)
        }
    }

    /// A show's seasons (`GET /Shows/{id}/Seasons`).
    pub(crate) fn seasons(&self, series_id: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Shows/{series_id}/Seasons?UserId={user_id}&Fields=Overview,OfficialRating,ChildCount\
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
    /// page's OnDeck and the player route's Up Next. An empty result is a show never started or
    /// finished, NOT a failure: the caller distinguishes by `Items` emptiness, and only a
    /// transport/HTTP failure is `None`.
    pub(crate) fn next_up(&self, series_id: &str) -> Option<ItemsResult> {
        self.next_up_path(&format!("&SeriesId={series_id}&Limit=1"))
    }

    /// The next unseen episode of EVERY started series (`GET /Shows/NextUp` with no SeriesId) —
    /// one row per series, which is exactly the Home shelf Jellyfin's own clients surface as
    /// "Next Up" (issue #44). Same emptiness-is-not-failure contract as [`next_up`].
    pub(crate) fn next_up_all(&self, limit: i64) -> Option<ItemsResult> {
        self.next_up_path(&format!("&Limit={limit}"))
    }

    fn next_up_path(&self, query: &str) -> Option<ItemsResult> {
        let user_id = self.user_id()?;
        self.get_json(&format!(
            "/Shows/NextUp?UserId={user_id}{query}\
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
            None,
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

    /// The named scores the MDBList Ratings plugin holds for this item
    /// (`GET /Plugins/MdbListRatings/CachedByItemId?itemId={id}`).
    ///
    /// A 404 is folded to EMPTY for the same reason [`Self::media_segments`] folds its own: the
    /// route exists only while the plugin is installed, so "no plugin" and "the plugin holds
    /// nothing for this item" both mean the window shows the two scores the server always has and
    /// invents no third. A 200 with an empty `ratings` array is the plugin's own answer for an
    /// item it has not cached (`hasCache: false`), and needs no separate arm.
    pub(crate) fn mdblist_ratings(&self, item_id: &str) -> Option<super::dto::MdbListCached> {
        let headers = self.authed_headers()?;
        let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
        let reply = http::request(
            &self.origin,
            &format!("/Plugins/MdbListRatings/CachedByItemId?itemId={item_id}"),
            http::Method::Get,
            &refs,
            None,
        )?;
        if reply.status == 404 {
            return Some(super::dto::MdbListCached::default());
        }
        if !reply.ok() {
            crate::log(&format!(
                "jellyfin: GET /Plugins/MdbListRatings/CachedByItemId answered {}",
                reply.status
            ));
            return None;
        }
        match serde_json::from_slice(&reply.body) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!(
                    "jellyfin: the MDBList cache answered {} bytes that will not parse - {e}",
                    reply.body.len()
                ));
                None
            }
        }
    }

    /// Build an image request path — the Jellyfin counterpart of
    /// `server::Client::image_transcode_path`, and like it BOTH the poster store's LRU key and the
    /// bytes it fetches. `src_path` is the stored image path (`/Items/{id}/Images/Primary?tag=…`,
    /// see [`super::convert`]); the server scales, so the size the tile wants rides as
    /// `fillWidth`/`fillHeight`, the direct analogue of the retired `/photo/:/transcode` box.
    ///
    /// `png` is accepted for signature parity and deliberately ignored: Jellyfin's Primary
    /// images are JPEG and its Logo/Art types keep their own transparency — the PNG flag exists
    /// on the retired side for clearLogos, which this backend reads through their own image types.
    pub(crate) fn image_path(&self, src_path: &str, w: i64, h: i64, png: bool) -> Option<String> {
        if png {
            // Not a failure — a Logo/Art request must be built from its own path by the caller.
            crate::log("jellyfin: image_path called with png=1 (clearLogo path); serving JPEG box");
        }
        let token = self.token.read().unwrap_or_else(|e| e.into_inner()).as_ref()?.token.clone();
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
    /// transcode" switch, the byte-exact counterpart of the retired direct-play `part` URL.
    /// `media_source_id` pins the VERSION (Jellyfin's `part`): empty means the server's default
    /// source, which is also the right answer when the detail fetch found exactly one.
    pub(crate) fn direct_stream_url(&self, item_id: &str, media_source_id: &str) -> Option<String> {
        let token = self.token.read().unwrap_or_else(|e| e.into_inner()).as_ref()?.token.clone();
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
        let token = self.token.read().unwrap_or_else(|e| e.into_inner()).as_ref()?.token.clone();
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
    // Jellyfin's progress protocol is three JSON POSTs where the retired wire's was a single POST:
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
    /// jellyfin-web calls when HLS playback ends. Best-effort for the same reason the retired
    /// transcode stop was: the server reaps the encoder on its own once the playlist goes quiet,
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
    // verified against 10.11), where the retired removeFromContinueWatching kept it. The deck row
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

    /// Favorite: `POST/DELETE /Users/{uid}/FavoriteItems/{id}` — bodyless like the played pair,
    /// and the server's answer is the updated UserData the caller does not need (the fan-out
    /// already moved the UI optimistically).
    pub(crate) fn set_favorite(&self, item_id: &str, want: bool) -> bool {
        let Some(user_id) = self.user_id() else {
            return false;
        };
        self.request_status(
            &format!("/Users/{user_id}/FavoriteItems/{item_id}"),
            if want { http::Method::Post } else { http::Method::Delete },
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
    /// the section header for what this costs versus the retired hide-but-keep-resume.
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
        match http::request(&self.origin, path, method, &refs, None) {
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
        Some(self.token.read().unwrap_or_else(|e| e.into_inner()).as_ref()?.user_id.clone())
    }
}

/// The rail's letter table from an unfiltered total and the per-letter A-Z counts (letters
/// absent from `counts` have zero items): "#" first when the non-alphabetic remainder is
/// non-empty (digits sort before letters in `SortName` ascending), then the letters in order.
/// Pure so both the wire path and the tests share ONE derivation.
pub(crate) fn build_letters(total: i64, counts: Vec<(String, i64)>) -> Vec<(String, i64)> {
    let mut out = Vec::with_capacity(counts.len() + 1);
    let sum: i64 = counts.iter().map(|(_, n)| *n).sum();
    if total - sum > 0 {
        out.push(("#".into(), total - sum));
    }
    out.extend(counts);
    out
}

/// `http::Method`'s token is private to the transport; the log line wants the same spelling.
fn method_name(m: http::Method) -> &'static str {
    match m {
        http::Method::Get => "GET",
        http::Method::Post => "POST",
        http::Method::Delete => "DELETE",
    }
}

/// **Is this path already an absolute http(s) URL, and what does it dial?** The poster pipeline's
/// keys are ordinarily server-relative (`/Items/{id}/Images/Primary?tag=…`), and the ONE thing that
/// arrives absolute is a metadata provider's remote image (*Edit images*, issue #369), whose URL
/// points at the provider's own CDN rather than at this server. `origin::split` is the parser that
/// already exists for exactly this: an origin plus everything after the authority.
///
/// `None` for anything else, INCLUDING a bare `//host/path` or a scheme this app does not speak, so
/// a relative image path can never be mistaken for a host to dial.
pub(crate) fn absolute_http_url(url: &str) -> Option<(crate::origin::Origin, &str)> {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let (origin, path) = crate::origin::split(url);
    (!path.is_empty()).then_some((origin, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// One mock Jellyfin server on loopback: accepts exactly `responses.len()` connections
    /// (every request here is `Connection: close`), records each raw request, and answers with
    /// the queued body in order. Loopback is inside the http gate's LAN arm, so the credential
    /// policy being tested is the REAL one, not a bypass.
    use crate::jellyfin::MockServer;


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
        // The dashboard names the fork: Client is the display name, "Butaca" on this build.
        assert!(auth.contains("Authorization: MediaBrowser Client=\"Butaca\""));
        #[cfg(any())]
        assert!(auth.contains("Authorization: MediaBrowser Client=\"Butaca\""));
        // Key order in the body is serde_json's (alphabetical without preserve_order) — assert
        // the two fields, not one literal, so the test does not hinge on serializer internals.
        assert!(auth.contains(r#""Username":"demo""#));
        assert!(auth.contains(r#""Pw":"""#));
        let views_req = &reqs[1];
        assert!(views_req.starts_with("GET /Users/u-1/Views HTTP/1.1"));
        assert!(views_req.contains("Authorization: MediaBrowser Token=\"tok-1\", "));
    }

    /// Continue Watching asks for movies and episodes by type: a newer server also reports the
    /// series and season folders it rolled the progress up to, and neither can be resumed.
    #[test]
    fn resume_asks_only_for_item_types_that_can_be_resumed() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":0}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client
            .authenticate_by_name("demo", "")
            .expect("auth succeeds");
        assert!(client.resume(20).is_some(), "an empty deck is not a failure");

        let reqs = server.finish();
        let resume = &reqs[1];
        assert!(
            resume.starts_with("GET /Users/u-1/Items/Resume?"),
            "{resume}"
        );
        assert!(
            resume.contains("IncludeItemTypes=Movie,Episode,Video"),
            "{resume}"
        );
    }

    /// The extras shelf is two routes, each a bare array, with an item that both routes name kept
    /// once. A route that fails leaves the other's rows in place.
    #[test]
    fn extras_join_the_two_routes_and_keep_each_item_once() {
        let features = r#"[
            {"Id":"sf1","Name":"Making of","Type":"Video","ExtraType":"BehindTheScenes"},
            {"Id":"t1","Name":"Trailer","Type":"Trailer"}
        ]"#;
        let trailers = r#"[
            {"Id":"t1","Name":"Trailer","Type":"Trailer"},
            {"Id":"t2","Name":"Teaser","Type":"Trailer"}
        ]"#;
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, features),
            (200, trailers),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client
            .authenticate_by_name("demo", "")
            .expect("auth succeeds");

        let extras = client.extras("m1");
        let ids: Vec<&str> = extras.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["sf1", "t1", "t2"]);

        let reqs = server.finish();
        assert!(
            reqs[1].starts_with("GET /Items/m1/SpecialFeatures?"),
            "{}",
            reqs[1]
        );
        assert!(
            reqs[2].starts_with("GET /Items/m1/LocalTrailers?"),
            "{}",
            reqs[2]
        );
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

    /// A persisted token is adopted and turned into the account it belongs to: `/Users/Me` is the
    /// probe AND the lookup, and the request carries the token as a parameter of the same
    /// MediaBrowser header every other control route uses.
    #[test]
    fn a_stored_token_is_adopted_and_resolved_through_users_me() {
        let server = MockServer::start(vec![(200, r#"{"Id":"u-9","Name":"demo2"}"#)]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());

        let user = client.adopt_token("tok-9").expect("the probe answered");
        assert_eq!(user.user_id, "u-9");
        assert_eq!(user.user_name, "demo2");
        assert_eq!(client.user_name().as_deref(), Some("demo2"));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].starts_with("GET /Users/Me HTTP/1.1"));
        assert!(reqs[0].contains("Authorization: MediaBrowser Token=\"tok-9\", "));
    }

    /// A revoked token is refused and NOT left installed: the boot arm falls back to the password,
    /// and a client whose token was rejected cannot fire it at a later endpoint.
    #[test]
    fn a_revoked_token_is_unauthorized_and_stays_uninstalled() {
        let server = MockServer::start(vec![(401, "{}")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        assert!(matches!(
            client.adopt_token("tok-old"),
            Err(AuthError::Unauthorized)
        ));
        // No token, no user id: the request is not even attempted.
        assert!(client.views().is_none());
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

    /// The heart: POST favorites on, DELETE off, both bodyless and user-scoped (#47).
    #[test]
    fn favorite_writes_hit_the_favorite_items_endpoint() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, "{}"),
            (200, "{}"),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        assert!(client.set_favorite("mv-1", true));
        assert!(client.set_favorite("mv-1", false));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 3);
        assert!(reqs[1].starts_with("POST /Users/u-1/FavoriteItems/mv-1 HTTP/1.1"));
        assert!(reqs[2].starts_with("DELETE /Users/u-1/FavoriteItems/mv-1 HTTP/1.1"));
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

    /// Identify's two calls: the search posts the item's own title and year as a BODY (Jellyfin's
    /// typed POST, which replaced the older GET form), and Apply posts the chosen candidate back to
    /// the item's own path, the provider ids in it are what the server looks the item up by.
    #[test]
    fn identify_searches_with_a_body_and_applies_the_candidate_verbatim() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"[{"Name":"Alien","ProviderIds":{"Tmdb":"348"},"ProductionYear":1979,
                      "SearchProviderName":"TheMovieDb"}]"#),
            (204, ""),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let hits = client
            .remote_search_movie("mv-1", "Alien", 1979)
            .expect("the candidates parse");
        assert_eq!(hits.len(), 1);
        assert!(client.apply_remote_search("mv-1", &hits[0]));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 3);
        assert!(reqs[1].starts_with("POST /Items/RemoteSearch/Movie HTTP/1.1"));
        assert!(reqs[1].contains("\"ItemId\":\"mv-1\""));
        assert!(reqs[1].contains("\"Name\":\"Alien\""));
        assert!(reqs[1].contains("\"Year\":1979"));
        assert!(reqs[2].starts_with("POST /Items/RemoteSearch/Apply/mv-1 HTTP/1.1"));
        assert!(reqs[2].contains("\"Tmdb\":\"348\""), "the ids Apply looks the item up by");
    }

    /// **Edit images' two calls.** The list asks for ONE image type and a bounded page (the picker
    /// draws every tile at once), and answers the provider's full URL plus its smaller copy. The
    /// download sends the full URL back PERCENT-ENCODED — it carries its own query — and no body,
    /// because every parameter rides the query string.
    #[test]
    fn remote_images_lists_a_page_and_downloads_the_chosen_url() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Images":[{"Url":"https://img.example/original/a.jpg",
                      "ThumbnailUrl":"https://img.example/w300/a.jpg",
                      "Type":"Primary","ProviderName":"TheMovieDb","Width":1000,"Height":1500,
                      "Language":"en"}],"TotalRecordCount":31,"Providers":["TheMovieDb"]}"#),
            (204, ""),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let page = client.remote_images("mv-1", "Primary", 0, 12).expect("the page parses");
        assert_eq!(page.total_record_count, 31);
        assert_eq!(page.images[0].thumbnail_url.as_deref(), Some("https://img.example/w300/a.jpg"));
        assert!(client.download_remote_image("mv-1", "Primary", &page.images[0].url));

        let reqs = server.finish();
        assert_eq!(reqs.len(), 3);
        assert!(reqs[1].starts_with(
            "GET /Items/mv-1/RemoteImages?type=Primary&startIndex=0&limit=12 HTTP/1.1"
        ));
        let dl = &reqs[2];
        assert!(dl.starts_with("POST /Items/mv-1/RemoteImages/Download?type=Primary&imageUrl="));
        assert!(
            dl.contains("imageUrl=https%3A%2F%2Fimg.example%2Foriginal%2Fa.jpg"),
            "the provider URL travels percent-encoded, exactly as the list answered it"
        );
        assert!(!dl.contains("api_key"), "the server's token never rides a provider URL");
    }

    /// **Intros** is the ordinary item envelope (`BaseItemDtoQueryResult`), not a bare array: a
    /// server without the plugin answers `Items: []`, which the caller reads as "no intro".
    #[test]
    fn intros_read_the_item_envelope_and_fold_an_empty_answer() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[{"Id":"intro-1","Name":"Studio logo","Type":"Movie",
                      "MediaSources":[{"Id":"src-1","MediaStreams":[
                        {"Type":"Video","Codec":"h264","Index":0},
                        {"Type":"Audio","Codec":"aac","Index":1}]}]}],
                      "TotalRecordCount":1,"StartIndex":0}"#),
            (200, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#),
            (404, ""),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let one = client.intros("mv-1").expect("the envelope parses");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "intro-1");
        assert_eq!(
            one[0].media_sources.as_ref().unwrap()[0].id, "src-1",
            "the media source is what request_play addresses"
        );
        assert!(client.intros("mv-2").expect("an empty list is an answer").is_empty());
        assert!(client.intros("mv-3").is_none(), "a 404 is not an answer");

        let reqs = server.finish();
        assert!(reqs[1].starts_with("GET /Items/mv-1/Intros?UserId=u-1&Fields=MediaSources HTTP/1.1"));
    }

    /// **An absolute provider URL is dialled as it stands, headerless.** The credential rule is the
    /// point: a remote image lives on somebody else's host, and this client's token must never
    /// travel there. A server-relative path keeps its ordinary shape (that arm is graded by every
    /// other test in this module).
    #[test]
    fn an_absolute_image_url_is_dialled_headerless() {
        let target = MockServer::start(vec![(200, "JPEGBYTES")]);
        let server = MockServer::start(vec![(200, AUTH_OK)]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let url = format!("http://127.0.0.1:{}/t/p/w300/a.jpg", target.port);
        assert_eq!(client.get_bytes(&url).as_deref(), Some(b"JPEGBYTES".as_slice()));

        let reqs = target.finish();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].starts_with("GET /t/p/w300/a.jpg HTTP/1.1"));
        assert!(!reqs[0].contains("Authorization"), "no credential reaches a provider host");
        assert!(!reqs[0].contains("api_key"));
    }

    /// The absolute-URL test is exact about what it accepts: only `http://` and `https://` are
    /// hosts to dial. A relative image path, a protocol-relative one and a scheme this app does not
    /// speak are all "not an absolute URL", so they take the ordinary server-relative arm.
    #[test]
    fn only_a_plain_http_url_counts_as_absolute() {
        assert!(absolute_http_url("https://img.example/a.jpg").is_some());
        assert!(absolute_http_url("HTTP://img.example/a.jpg").is_some());
        assert!(absolute_http_url("/Items/x/Images/Primary?tag=1").is_none());
        assert!(absolute_http_url("//img.example/a.jpg").is_none());
        assert!(absolute_http_url("file:///etc/passwd").is_none());
        assert!(absolute_http_url("https://img.example").is_none(), "no path, nothing to dial");
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

    /// The Collections type's page query: BoxSets scoped to the view, no filter, fixed name order.
    #[test]
    fn boxsets_carries_the_collections_query() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":4}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let res = client.boxsets("view-1", 60, 60).expect("boxsets parse");
        assert_eq!(res.total, 4);

        let reqs = server.finish();
        assert_eq!(reqs.len(), 2);
        let head = reqs[1].lines().next().unwrap();
        assert!(head.starts_with("GET /Users/u-1/Items?"));
        for want in [
            "ParentId=view-1",
            "IncludeItemTypes=BoxSet",
            "Recursive=true",
            "SortBy=SortName",
            "SortOrder=Ascending",
            "StartIndex=60",
            "Limit=60",
        ] {
            assert!(head.contains(want), "missing {want} in {head}");
        }
        assert!(!head.contains("IsPlayed"), "a box set has no watch state: {head}");
        assert!(!head.contains("GenreIds"), "a box set has no genre: {head}");
    }

    /// The collection screen's members fetch scopes `/Items` to the box set's own id.
    #[test]
    fn collection_children_hits_the_boxsets_parent_id() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Items":[],"TotalRecordCount":0}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let res = client.collection_children("box-1", 0, 60).expect("children parse");
        assert_eq!(res.total, 0);

        let reqs = server.finish();
        assert_eq!(reqs.len(), 2);
        let head = reqs[1].lines().next().unwrap();
        assert!(head.starts_with("GET /Users/u-1/Items?"));
        assert!(head.contains("ParentId=box-1"), "{head}");
        assert!(head.contains("SortBy=SortName"), "{head}");
        assert!(head.contains("StartIndex=0"), "{head}");
        assert!(head.contains("Limit=60"), "{head}");
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

    /// The rail's letter table: "#" leads when the non-alphabetic remainder is non-zero, zero
    /// letters are absent, and the input order (A-Z) is preserved.
    #[test]
    fn build_letters_derives_the_hash_bucket_and_keeps_alphabetical_order() {
        let none = build_letters(10, vec![("A".into(), 10)]);
        assert_eq!(none, vec![("A".into(), 10)]); // exact sum: no "#"
        let with_hash = build_letters(12, vec![("A".into(), 10), ("Z".into(), 1)]);
        assert_eq!(
            with_hash,
            vec!["#".to_string(), "A".to_string(), "Z".to_string()]
                .into_iter()
                .zip([1i64, 10, 1])
                .collect::<Vec<_>>()
        );
        let empty = build_letters(0, vec![]);
        assert!(empty.is_empty());
    }

    /// The Jellyfin letter fetch: one unfiltered count plus one `NameStartsWith` count per
    /// letter, every query at `Limit=0` (counts only, no items transferred), and the counts
    /// wired into the rail table.
    /// Issue #37: with a filter active, every probe (the total and each letter) carries it —
    /// `NameStartsWith` accepts `IsPlayed` and `GenreIds` alongside `ParentId`, which is what
    /// makes the A-Z rail able to stay honest under unwatched-only or a genre.
    #[test]
    fn letter_counts_carry_the_active_filter_on_every_probe() {
        let total = r#"{"Items":[],"TotalRecordCount":3}"#;
        let a = r#"{"Items":[],"TotalRecordCount":3}"#;
        let zero = r#"{"Items":[],"TotalRecordCount":0}"#;
        let mut responses = vec![(200, AUTH_OK), (200, total)];
        for l in 'A'..='Z' {
            responses.push((200, if l == 'A' { a } else { zero }));
        }
        let server = MockServer::start(responses);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let letters = client
            .letter_counts(
                "view-1",
                "Series",
                &crate::browse::LettersFilter { unwatched: true, genre: Some("g-7".into()) },
            )
            .expect("letter counts parse");
        assert_eq!(letters, vec![("A".to_string(), 3)]);
        let reqs = server.finish();
        assert_eq!(reqs.len(), 28);
        for (i, req) in reqs.iter().enumerate().skip(1) {
            let q = req.lines().next().unwrap();
            assert!(q.contains("IsPlayed=false"), "probe {i} misses IsPlayed: {q}");
            assert!(q.contains("GenreIds=g-7"), "probe {i} misses GenreIds: {q}");
        }
    }

    #[test]
    fn letter_counts_query_limit_zero_for_the_total_and_every_letter() {
        // total 12: A=10, Z=1, other=1 -> "#" bucket
        let total = r#"{"Items":[],"TotalRecordCount":12}"#;
        let a = r#"{"Items":[],"TotalRecordCount":10}"#;
        let z = r#"{"Items":[],"TotalRecordCount":1}"#;
        let zero = r#"{"Items":[],"TotalRecordCount":0}"#;
        let mut responses = vec![(200, AUTH_OK), (200, total)];
        for l in 'A'..='Z' {
            responses.push((200, if l == 'A' { a } else if l == 'Z' { z } else { zero }));
        }
        let server = MockServer::start(responses);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let letters = client
            .letter_counts("view-1", "Movie", &crate::browse::LettersFilter::default())
            .expect("letter counts parse");
        assert_eq!(
            letters,
            vec!["#".to_string(), "A".to_string(), "Z".to_string()]
                .into_iter()
                .zip([1i64, 10, 1])
                .collect::<Vec<_>>()
        );
        let reqs = server.finish();
        assert_eq!(reqs.len(), 28); // auth + total + 26 letters
        assert!(
            reqs.iter().all(|r| !r.contains("IsPlayed=") && !r.contains("GenreIds=")),
            "an unfiltered probe carries no filter: {reqs:?}"
        );
        let total_q = reqs[1].lines().next().unwrap();
        assert!(total_q.starts_with("GET /Users/u-1/Items?"));
        assert!(total_q.contains("ParentId=view-1"));
        assert!(total_q.contains("IncludeItemTypes=Movie"));
        assert!(total_q.contains("Limit=0"));
        assert!(!total_q.contains("NameStartsWith"));
        let a_q = reqs[2].lines().next().unwrap();
        assert!(a_q.contains("NameStartsWith=A"));
        assert!(a_q.contains("Limit=0"));
        assert!(reqs[27].lines().next().unwrap().contains("NameStartsWith=Z"));
    }

    /// Quick Connect: Initiate is an unauthenticated POST carrying only the device identity, and
    /// the answer's Secret/Code parse out of the result.
    #[test]
    fn quick_connect_initiate_posts_identity_and_parses_secret_and_code() {
        let server = MockServer::start(vec![(
            200,
            r#"{"Secret":"qc-secret-1","Code":"123456","Authenticated":false}"#,
        )]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        let res = client.quick_connect_initiate().expect("initiate parses");
        assert_eq!(res.secret, "qc-secret-1");
        assert_eq!(res.code, "123456");
        assert!(!res.authenticated);

        let reqs = server.finish();
        assert_eq!(reqs.len(), 1);
        let head = reqs[0].lines().next().unwrap();
        assert!(head.starts_with("POST /QuickConnect/Initiate HTTP/1.1"), "{head}");
        assert!(reqs[0].contains("Authorization: MediaBrowser Client="), "identity rides along");
        assert!(!reqs[0].contains("Token="), "unauthenticated: no token is minted yet");
    }

    /// The full Quick Connect dance: Initiate, then poll Connect until Authenticated, then trade
    /// the secret for the token on AuthenticateWithQuickConnect.
    #[test]
    fn quick_connect_polls_state_then_authenticates_with_the_secret() {
        let server = MockServer::start(vec![
            (200, r#"{"Secret":"qc-secret-1","Code":"123456","Authenticated":false}"#),
            (200, r#"{"Secret":"qc-secret-1","Code":"123456","Authenticated":false}"#),
            (200, r#"{"Secret":"qc-secret-1","Code":"123456","Authenticated":true}"#),
            (200, AUTH_OK),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        let initiated = client.quick_connect_initiate().unwrap();
        assert!(!initiated.authenticated);

        let not_yet = client.quick_connect_state(&initiated.secret).unwrap();
        assert!(!not_yet.authenticated, "still waiting");
        let approved = client.quick_connect_state(&initiated.secret).unwrap();
        assert!(approved.authenticated, "authorized on the other device");

        let ok = client.authenticate_with_quick_connect(&initiated.secret).unwrap();
        assert_eq!(ok.user_id, "u-1");
        assert_eq!(ok.token, "tok-1");

        let reqs = server.finish();
        assert_eq!(reqs.len(), 4);
        assert!(reqs[1].starts_with("GET /QuickConnect/Connect?secret=qc-secret-1 HTTP/1.1"));
        assert!(reqs[2].starts_with("GET /QuickConnect/Connect?secret=qc-secret-1 HTTP/1.1"));
        assert!(reqs[3].starts_with("POST /Users/AuthenticateWithQuickConnect HTTP/1.1"));
        assert!(reqs[3].contains(r#""Secret":"qc-secret-1""#));
    }

    /// A refused or not-yet-authorized AuthenticateWithQuickConnect answers Unauthorized, so the
    /// flow can fall back to the password form instead of inventing a fourth failure word.
    #[test]
    fn quick_connect_authenticate_refusal_is_unauthorized() {
        let server = MockServer::start(vec![(400, "{}")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        match client.authenticate_with_quick_connect("qc-secret-1") {
            Err(AuthError::Unauthorized) => {}
            other => panic!("expected Unauthorized, got {other:?}"),
        }
        server.finish();
    }

    /// Quick Connect is gated on the server's public version: 10.8+ advertises it, anything older
    /// or absent does not.
    #[test]
    fn quick_connect_supported_reads_the_public_version() {
        let new = MockServer::start(vec![(200, r#"{"ServerName":"attic","Version":"10.11.11"}"#)]);
        let new_client = JfClient::new(Origin::http("127.0.0.1", new.port as i32), "dev-1".into());
        assert!(new_client.quick_connect_supported());
        new.finish();

        let old = MockServer::start(vec![(200, r#"{"ServerName":"attic","Version":"10.7.7"}"#)]);
        let old_client = JfClient::new(Origin::http("127.0.0.1", old.port as i32), "dev-1".into());
        assert!(!old_client.quick_connect_supported());
        old.finish();

        let absent = MockServer::start(vec![(200, r#"{"ServerName":"attic"}"#)]);
        let absent_client = JfClient::new(Origin::http("127.0.0.1", absent.port as i32), "dev-1".into());
        assert!(!absent_client.quick_connect_supported());
        absent.finish();
    }

    #[test]
    fn version_at_least_parses_dotted_versions_and_fails_closed() {
        assert!(version_at_least("10.8.0", 10, 8));
        assert!(version_at_least("10.11.11", 10, 8));
        assert!(version_at_least("11.0.0", 10, 8));
        assert!(!version_at_least("10.7.7", 10, 8));
        assert!(version_at_least("10.8", 10, 8));
        assert!(!version_at_least("10.8", 10, 9));
        assert!(!version_at_least("", 10, 8));
        assert!(!version_at_least("not-a-version", 10, 8));
    }

    /// Trickplay: the manifest parses from the item detail, an empty one answers None, and the
    /// query asks for the field explicitly.
    #[test]
    fn trickplay_manifest_parses_and_an_empty_one_is_none() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Id":"mv-1","Name":"Movie","Type":"Movie","Trickplay":{"320":{"0":{"Width":320,"Height":180,"TileWidth":10,"TileHeight":10,"ThumbnailCount":100,"Interval":10000,"Bandwidth":123456}}}}"#),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let info = client.trickplay_info("mv-1").expect("manifest parses");
        let sheet = info.get("320").unwrap().get("0").unwrap();
        assert_eq!(sheet.width, 320);
        assert_eq!(sheet.height, 180);
        assert_eq!(sheet.tile_width, 10);
        assert_eq!(sheet.interval, 10000);
        let reqs = server.finish();
        assert!(reqs[1].starts_with("GET /Users/u-1/Items/mv-1?Fields=Trickplay HTTP/1.1"), "{}", reqs[1]);

        let empty = MockServer::start(vec![
            (200, AUTH_OK),
            (200, r#"{"Id":"mv-2","Name":"NoTp","Type":"Movie","Trickplay":{}}"#),
        ]);
        let client2 = JfClient::new(Origin::http("127.0.0.1", empty.port as i32), "dev-1".into());
        client2.authenticate_by_name("demo", "").unwrap();
        assert!(client2.trickplay_info("mv-2").is_none(), "no trickplay generated");
        empty.finish();
    }

    /// #370: a server WITHOUT the MDBList plugin has no `/Plugins/MdbListRatings` route, and the
    /// 404 is the client's proof that the capability is absent. Folded to an empty list here so
    /// the caller cannot tell "no plugin" from "nothing cached" and draw a row for either.
    #[test]
    fn a_server_without_the_plugin_answers_the_cache_route_with_nothing() {
        let server = MockServer::start(vec![(200, AUTH_OK), (404, "")]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let cached = client.mdblist_ratings("mv-1").expect("a 404 is an answer, not a failure");
        assert!(cached.ratings.is_empty());
        let reqs = server.finish();
        assert!(
            reqs[1].starts_with(
                "GET /Plugins/MdbListRatings/CachedByItemId?itemId=mv-1 HTTP/1.1"
            ),
            "{}",
            reqs[1]
        );
    }

    /// The plugin's own numbers are read leniently on purpose: MDBList hands them through
    /// converters that accept `7.4` and `"7.4"` and turn `"N/A"` into null, so a value this
    /// client cannot read must cost that one source, never the payload.
    #[test]
    fn the_plugin_scores_parse_from_numbers_and_strings_and_nonsense_is_absent() {
        let server = MockServer::start(vec![
            (200, AUTH_OK),
            (
                200,
                r#"{"hasCache":true,"ratings":[
                    {"source":"imdb","value":7.2,"score":72},
                    {"source":"trakt","value":"74","score":"74"},
                    {"source":"metacritic","score":"N/A"},
                    {"source":"letterboxd"},
                    {"source":"tmdb","score":null}
                ]}"#,
            ),
        ]);
        let client = JfClient::new(Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();

        let cached = client.mdblist_ratings("mv-1").expect("payload parses");
        assert_eq!(cached.ratings.len(), 5);
        assert_eq!(cached.ratings[0].score, Some(72.0));
        assert_eq!(cached.ratings[1].score, Some(74.0));
        assert_eq!(cached.ratings[2].score, None, "\"N/A\" is no score, not zero");
        assert_eq!(cached.ratings[3].score, None);
        assert_eq!(cached.ratings[4].score, None);
        server.finish();
    }
}
