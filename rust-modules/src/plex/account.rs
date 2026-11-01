//! The **plex.tv account** API — the login (PIN/QR), server discovery, and Plex Home managed-user
//! surface. This is a *different service* from the Plex Media Server: the repo's OpenAPI spec
//! (`docs/plex-openapi.json`) is PMS-only and every PMS op assumes you already hold a token, so
//! these endpoints have no entry there. They're modelled here in the same typed style as the rest
//! of the `plex` layer (`hubs.rs`/`library.rs`): typed methods returning serde DTOs, with all the
//! transport, identity headers, and token injection centralised on [`AccountClient`].
//!
//! Transport is [`crate::net`] (libcurl HTTPS) — the TLS analog of `stream::http_get`, since plex.tv
//! needs DNS + TLS that the plain-HTTP PMS socket can't do. Every call is **blocking**, so callers
//! run it on a background thread (the login-poll / discovery / switch threads), never the SDL loop.
//!
//! Tokens (`Resource.access_token`) are secrets: they
//! are never logged here and never printed by callers. What IS logged, on a failure only, is the
//! **status**, the **shape** of the endpoint that returned it, and — for a body that will not
//! deserialize — serde's error category and position. See [`decode`] for why the status is worth a
//! line at all, and [`endpoint_shape`] for what a "shape" is allowed to contain.
//!
//! `discover.rs` is this client's op-file sibling: the plex.tv **metadata provider** speaks the
//! same transport and the same identity headers, so it adds an `impl AccountClient` block rather
//! than a second client — which is why [`AccountClient::get`] is `pub(super)`.
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

// The lenient wire adapters live once, in `models.rs`, next to the note that explains why every
// number and flag needs one — see `de_bool` there for why the plex.tv policy flags fold to `bool`
// while the PMS DTOs keep theirs as `i64`.
use super::models::{de_bool, de_i64, de_str, de_vec};

const PLEX_TV: &str = "https://plex.tv";

/// plex.tv's base URL: [`PLEX_TV`], or — in a dev build only — a LOOPBACK stand-in named by
/// `/tmp/butaca-plextv=http://127.0.0.1:<port>`. The screenshot pipeline's mock
/// (`tests/mock_pms.py --catalog`) answers the sign-in pin and serves its QR there, so no
/// documentation figure ever shows a live code minted by the real service. Anything that is not
/// plain `http://` to `127.0.0.1`/`localhost` is refused and logged: this trigger must never be
/// able to point the account API, and the token it carries, at another host. Read once.
pub(crate) fn plex_tv() -> &'static str {
    static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BASE.get_or_init(|| match crate::dev::read("plextv") {
        Some(v) if loopback_http(&v) => {
            #[cfg(feature = "devtriggers")]
            crate::log("account: plex.tv replaced by a loopback stand-in (/tmp/butaca-plextv)");
            v.trim_end_matches('/').to_string()
        }
        Some(_) => {
            #[cfg(feature = "devtriggers")]
            crate::log("BADTRIGGER plextv: only http://127.0.0.1:<port> or http://localhost:<port> is accepted");
            PLEX_TV.to_string()
        }
        None => PLEX_TV.to_string(),
    })
}

/// `http://127.0.0.1:<port>` or `http://localhost:<port>`, optionally with a trailing `/`.
fn loopback_http(v: &str) -> bool {
    let port = v
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| v.strip_prefix("http://localhost:"))
        .map(|rest| rest.strip_suffix('/').unwrap_or(rest));
    port.is_some_and(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}
const AUDIO_PREFERENCES_SUCCESS_TTL: Duration = Duration::from_secs(5 * 60);
const AUDIO_PREFERENCES_FAILURE_TTL: Duration = Duration::from_secs(45);
const AUDIO_PREFERENCES_FAILURE_TTL_MAX: Duration = Duration::from_secs(10 * 60);

/// The account-language input consumed by the playback route. `None` is a successful, explicit
/// "do not auto-select audio" / unset answer, distinct inside the cache from a failed request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPreferences {
    pub language: Option<String>,
    /// What plex.tv actually answered, kept only so an unset `language` can say WHY on the route
    /// log: auto-select off and no language set otherwise read identically.
    pub auto_select_audio: Option<bool>,
    pub stated_language: Option<String>,
    pub subtitle_language: Option<String>,
    /// Plex autoSelectSubtitle: 0 manually selected, 1 foreign audio, 2 always.
    pub subtitle_mode: i64,
    /// Plex defaultSubtitleForced: 0 prefer non-forced, 1 prefer forced,
    /// 2 only forced, 3 only non-forced.
    pub subtitle_forced: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AudioPreferencesKey { id: i64, uuid: String, generation: u32 }

impl AudioPreferencesKey {
    fn new(user: &super::session::UserRef, generation: u32) -> Self {
        Self { id: user.id, uuid: user.uuid.clone(), generation }
    }
    fn is_current(&self) -> bool {
        let current = super::session::current_snapshot();
        current.generation == self.generation && current.user.as_ref().is_some_and(|user|
            user.id == self.id && user.uuid == self.uuid)
    }
}

#[derive(Clone)]
struct AudioPreferencesEntry {
    outcome: AudioPreferencesOutcome,
    consecutive_failures: u32,
    expires_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioPreferencesOutcome {
    Available(AudioPreferences),
    TimedOut,
    Failed,
}

#[derive(Clone)]
struct AudioPreferencesFlight { key: AudioPreferencesKey, id: u64 }

struct AudioPreferencesState {
    key: Option<AudioPreferencesKey>, entry: Option<AudioPreferencesEntry>,
    flight: Option<u64>, next_flight: u64, revision: u64,
}
struct AudioPreferencesCache { state: Mutex<AudioPreferencesState>, changed: Condvar }
#[derive(Clone, Copy)]
enum FetchPath { Play, Warm }

impl AudioPreferencesCache {
    const fn new() -> Self { Self { state: Mutex::new(AudioPreferencesState {
        key: None, entry: None, flight: None, next_flight: 0, revision: 0,
    }), changed: Condvar::new() } }

    fn publish(&self, key: AudioPreferencesKey) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.key.as_ref() != Some(&key) {
            state.key = Some(key); state.entry = None; state.flight = None;
            self.changed.notify_all();
        }
    }
    #[cfg_attr(test, allow(dead_code))]
    fn clear_profile(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.key.is_some() || state.entry.is_some() || state.flight.is_some() {
            state.key = None; state.entry = None; state.flight = None;
            self.changed.notify_all();
        }
    }
    fn reserve(&self, key: AudioPreferencesKey, now: Instant) -> Option<AudioPreferencesFlight> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.key.as_ref() != Some(&key) {
            state.key = Some(key.clone()); state.entry = None; state.flight = None;
            self.changed.notify_all();
        }
        if state.entry.as_ref().is_some_and(|entry| now < entry.expires_at)
            || state.flight.is_some() { return None; }
        state.next_flight = state.next_flight.wrapping_add(1);
        let id = state.next_flight;
        state.flight = Some(id);
        Some(AudioPreferencesFlight { key, id })
    }
    fn cancel(&self, flight: &AudioPreferencesFlight) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.key.as_ref() == Some(&flight.key) && state.flight == Some(flight.id) {
            state.flight = None; self.changed.notify_all();
        }
    }
    fn complete<C>(&self, flight: AudioPreferencesFlight, outcome: AudioPreferencesOutcome,
        path: FetchPath, completed_at: Instant, still_current: C) -> bool
    where C: FnOnce() -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.key.as_ref() != Some(&flight.key) || state.flight != Some(flight.id)
            || !still_current() { return false; }
        state.flight = None;
        if matches!(outcome, AudioPreferencesOutcome::Available(_)) {
            state.revision = state.revision.wrapping_add(1);
        }
        match (&outcome, path) {
            (AudioPreferencesOutcome::TimedOut, FetchPath::Play) => {}
            (AudioPreferencesOutcome::Available(_), _) => state.entry = Some(AudioPreferencesEntry {
                outcome: outcome.clone(), consecutive_failures: 0,
                expires_at: completed_at + AUDIO_PREFERENCES_SUCCESS_TTL,
            }),
            (AudioPreferencesOutcome::Failed | AudioPreferencesOutcome::TimedOut, _) => {
                let failures = state.entry.as_ref()
                    .filter(|entry| matches!(&entry.outcome, AudioPreferencesOutcome::Failed))
                    .map_or(1, |entry| entry.consecutive_failures.saturating_add(1));
                let shift = failures.saturating_sub(1).min(31);
                let ttl = AUDIO_PREFERENCES_FAILURE_TTL.saturating_mul(1_u32 << shift)
                    .min(AUDIO_PREFERENCES_FAILURE_TTL_MAX);
                state.entry = Some(AudioPreferencesEntry { outcome: AudioPreferencesOutcome::Failed,
                    consecutive_failures: failures, expires_at: completed_at + ttl });
            }
        }
        self.changed.notify_all();
        true
    }
}
static AUDIO_PREFERENCES_CACHE: AudioPreferencesCache = AudioPreferencesCache::new();

/// Identity + optional account token for plex.tv calls. `client_id` is the stable per-device
/// `X-Plex-Client-Identifier` (persisted across launches — plex.tv keys the authorized-device list
/// and the pin↔device binding on it). `token` is the account token, absent during the login handshake
/// and set once a pin resolves.
pub struct AccountClient {
    client_id: String,
    token: Option<String>,
}

impl AccountClient {
    pub fn new(client_id: &str, token: Option<&str>) -> AccountClient {
        AccountClient {
            client_id: client_id.to_owned(),
            token: token.map(|t| t.to_owned()),
        }
    }

    /// The `X-Plex-*` identity headers every plex.tv request carries (+ the token when present).
    ///
    /// **This surface is the account's AUTHORIZED-DEVICE LIST**, and that is what decides which
    /// fields belong here rather than on the PMS query-parameter copy
    /// (`Client::playback_identity`). A user reads this list to find one television among several
    /// and revoke it, so every field that helps them TELL DEVICES APART belongs here: what the app
    /// is, what it runs on, which firmware, who made the panel, and the per-install identifier the
    /// entry is keyed on. Fields that exist so a SERVER can decide what to send — a codec profile,
    /// a screen size — do not: plex.tv sends no media.
    ///
    /// Every value comes from [`identity`](super::identity) rather than a literal here. The two
    /// lists had drifted on five of seven fields, and one of them ("Plex for webOS") read as an
    /// official Plex client in a stranger's account.
    ///
    /// Three of these were added when the control plane learned to reach a server over the public
    /// internet, because a reviewer signing in from somewhere else is exactly the person reading
    /// this list:
    ///
    /// * **`X-Plex-Platform-Version`** — the real firmware, off the set (`identity::platform_version`).
    ///   PMS has been told this since issue #22, and plex.tv had not been, so an account's list
    ///   said "webOS" with no version while `/status/sessions` said "webOS 6.5.2".
    /// * **`X-Plex-Device-Vendor`** — `LG`. Not a choice this app makes; see [`identity::VENDOR`].
    /// * **`X-Plex-Provides`** — `player`. plex.tv reports this field back per device in
    ///   `/api/v2/resources` (it is what `Resource::is_server` reads on the way in), so a client
    ///   that never sends it is asking to be classified by absence.
    ///
    /// Two headers the official webOS client sends are **deliberately absent from both surfaces**,
    /// and the reasons are in `Client::playback_identity`: `X-Plex-Device-Screen-Resolution` and
    /// `X-Plex-Features`. `X-Plex-Language` carries the UI language resolved at boot by
    /// `identity::language`, including the English fallback.
    fn headers(&self) -> Vec<String> {
        use super::identity as id;
        let mut h = vec![
            "Accept: application/json".to_string(),
            format!("X-Plex-Product: {}", id::PRODUCT),
            format!("X-Plex-Version: {}", id::VERSION),
            format!("X-Plex-Platform: {}", id::PLATFORM),
            format!("X-Plex-Platform-Version: {}", id::platform_version()),
            format!("X-Plex-Device: {}", id::DEVICE),
            format!("X-Plex-Device-Name: {}", id::device_name()),
            format!("X-Plex-Device-Vendor: {}", id::VENDOR),
            format!("X-Plex-Model: {}", id::MODEL),
            format!("X-Plex-Provides: {}", id::PROVIDES),
            format!("X-Plex-Client-Identifier: {}", self.client_id),
        ];
        if let Some(language) = id::language() {
            h.push(format!("X-Plex-Language: {language}"));
        }
        if let Some(t) = &self.token {
            h.push(format!("X-Plex-Token: {t}"));
        }
        h
    }

    fn get_evidence_with<T: DeserializeOwned>(&self, url: &str, timeouts: crate::net::Timeouts)
        -> Result<T, CallEvidence> {
        let response = crate::net::request_evidence(url, &self.headers(), "GET", None,
            timeouts, false, None, None);
        decode_evidence("GET", url, response)
    }


    /// GET /api/v2/user — the active Plex Home profile's account-level audio preference.
    ///
    /// The caller supplies the explicit plex.tv credential beside the SAME captured
    /// [`UserRef`](super::session::UserRef). Constructing the narrow client here prevents an
    /// arbitrary account client (including one holding a PMS token) from selecting the authority.
    /// Optional or malformed preference fields merely disable this rung; transport, parse and
    /// identity failures use the shorter retry cache.
    pub fn audio_preferences(client_id: &str, credential: &str,
        expected: &super::session::UserRef, generation: u32)
        -> AudioPreferencesOutcome
    {
        if client_id.is_empty() || credential.is_empty() { return AudioPreferencesOutcome::Failed; }
        let client = Self::new(client_id, Some(credential));
        let key = AudioPreferencesKey::new(expected, generation);
        audio_preferences_cached_at(&AUDIO_PREFERENCES_CACHE, key.clone(),
            Duration::from_millis(1500), Instant::now,
            |remaining| client.fetch_audio_preferences(expected, play_timeouts(remaining)),
            || key.is_current())
    }

    fn fetch_audio_preferences(&self, expected: &super::session::UserRef,
        timeouts: crate::net::Timeouts) -> AudioPreferencesOutcome
    {
        let url = format!("{}/api/v2/user", plex_tv());
        let response = crate::net::request_evidence(&url, &self.headers(), "GET", None,
            timeouts, false, None, None);
        match response {
            Err(failure) if failure.cause == crate::net::RequestError::TimedOut =>
                AudioPreferencesOutcome::TimedOut,
            Err(_) => AudioPreferencesOutcome::Failed,
            Ok(response) => decode::<AccountUser>("GET", &url, response)
                .and_then(|dto| dto.audio_preferences_for(expected))
                .map_or(AudioPreferencesOutcome::Failed, AudioPreferencesOutcome::Available),
        }
    }

    // ---- server discovery ----

    pub(crate) fn resources_with(&self, timeouts: crate::net::Timeouts)
        -> Result<Vec<Resource>, CallEvidence> {
        self.get_evidence_with(&format!(
            "{}/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1", plex_tv()
        ), timeouts)
    }
}





/// What an account request observed when it produced nothing usable: `Ok(status)` for a complete
/// response that was refused or would not decode, `Err` for `net`'s own failure. Closed evidence
/// only — the onboarding report reduces it to a link class plus one number
/// (`telemetry::incident::classify`); no body, URL or header is kept.
pub type CallEvidence = Result<u16, crate::net::RequestFailure>;

/// Is this a status by which plex.tv refused the IDENTITY a request carried (the token
/// `headers()` attached), as opposed to failing to serve it? One definition, read by the log line
/// below and by every caller that must tell "this account may not do this" from "no answer" — a
/// managed Plex Home profile's token gets 401 from `/api/v2/home/users` (#132), and reporting that
/// as a connection problem sends the person looking for a network fault that does not exist.
pub fn refuses_identity(status: u16) -> bool {
    matches!(status, 401 | 403)
}

/// Whether repeating an account call may turn this observation into a usable answer.
/// Certificate/CA verification failures are stable until the TV is fixed; TLS connect failure
/// (curl 35) is a transient handshake/transport observation and remains retryable.
pub(crate) fn transient(evidence: &CallEvidence) -> bool {
    let status = match evidence { Ok(status) => Some(*status), Err(failure) => failure.status };
    if let Some(status) = status {
        return matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
    }
    match evidence {
        Err(failure) => matches!(failure.curl_rc, Some(rc) if !matches!(rc, 60 | 77 | 90)),
        Ok(_) => false,
    }
}

/// Shared capped geometric delay. `misses == 0` is the base cadence; callers which count the
/// first failure as one pass `misses - 1` to preserve their established schedule.
pub(crate) fn backoff(misses: u32, base: Duration, ceiling: Duration) -> Duration {
    let factor = 1u32.checked_shl(misses.min(30)).unwrap_or(u32::MAX);
    base.checked_mul(factor).unwrap_or(ceiling).min(ceiling)
}

/// One event-log phrase for a failed account call's evidence: the HTTP status when a response
/// arrived, the cause and curl code when none did. Closed evidence only — no URL, body or header.
pub fn describe_evidence(evidence: &CallEvidence) -> String {
    match evidence {
        Ok(status) if (200..300).contains(status) => format!("HTTP {status}, body unreadable"),
        Ok(status) if refuses_identity(*status) => format!("HTTP {status}, identity refused"),
        Ok(status) => format!("HTTP {status}"),
        Err(failure) => match (failure.status, failure.curl_rc) {
            (Some(status), _) => format!("HTTP {status}, transfer incomplete"),
            (None, _) if failure.cause == crate::net::RequestError::TimedOut => "no answer (timed out)".into(),
            (None, Some(rc)) => format!("no answer (curl rc={rc})"),
            (None, None) => "no answer".into(),
        },
    }
}

/// [`decode`], keeping the evidence of a call that yields nothing.
fn decode_evidence<T: DeserializeOwned>(verb: &str, url: &str,
    response: Result<crate::net::Resp, crate::net::RequestFailure>) -> Result<T, CallEvidence> {
    match response {
        Err(failure) => Err(Err(failure)),
        Ok(resp) => {
            let status = resp.status;
            decode(verb, url, resp).ok_or(Ok(status))
        }
    }
}


/// Status + body → the typed DTO, with a LOG LINE for each of the two ways that fails.
///
/// `net::request_tls_evidence` names its **transport** failures well (`net: curl rc=60 — peer
/// certificate has expired — the device clock may be wrong (…)`, worded by
/// `net::tls_verify_why`) and returns `Some(Resp)` for every
/// request that *completed*, whatever the server said in it. So the two failures that reach here
/// arrive carrying no description of themselves: a status this client declines, and a 2xx body
/// that will not deserialize. Both still leave by the same `None` — the callers' contract does not
/// change — but the evidence-preserving account methods keep the status after this function
/// returns: auth can therefore distinguish an identity refusal, a service answer and transport
/// silence, retry only the transient classes, and describe the failed plex.tv edge without
/// pretending it contacted a Plex Media Server. The status is what separates those outcomes, and
/// this function is where it exists.
///
/// The sign-in flow used to grade `r.ok()` itself. Switch refusal and poll Gone
/// classify received status before decoding, including incomplete responses. Every typed body
/// decoded here (including `discover.rs`) belongs to a complete transfer; failures carry no prefix.
fn decode<T: DeserializeOwned>(verb: &str, url: &str, resp: crate::net::Resp) -> Option<T> {
    if !resp.ok() {
        log_status_failure(verb, url, resp.status);
        return None;
    }
    match serde_json::from_slice::<T>(&resp.body) {
        Ok(v) => Some(v),
        // serde's CATEGORY and position, deliberately NOT its message: `Error`'s `Display` quotes
        // the value it choked on (`invalid type: string "…"`), and the values on these endpoints
        // include `Pin::auth_token` and `SwitchedUser::auth_token`. The category is the diagnostic
        // half anyway — `Data` is one field's shape drifting, `Syntax` a body that is not JSON at
        // all, `Eof` a truncated one — and the byte count separates "empty" from "a page of
        // something else". The test below pins the reason, so this is not simplified back to `{e}`.
        Err(e) => {
            crate::log(&format!(
                "account: {verb} {} -> HTTP {} but the body did not parse: {:?} at line {} col {} ({} bytes)",
                endpoint_shape(url),
                resp.status,
                e.classify(),
                e.line(),
                e.column(),
                resp.body.len()
            ));
            None
        }
    }
}

fn log_status_failure(verb: &str, url: &str, status: u16) {
    // 401/403 earns a word of its own because the app has nowhere else to say it: the request
    // arrived, and what plex.tv refused is the IDENTITY it carried — the token `headers()`
    // attached. Downstream that becomes a verdict about a server or a network (see this
    // function's doc for the exact copy), so the distinction has to be drawn in the line that
    // still knows it.
    let hint = if refuses_identity(status) {
        " — plex.tv refused this identity (token no longer valid?)"
    } else {
        ""
    };
    // Tagged for the CLIENT (`account:`) and not for the host, because the host is already in
    // the shape and the two services share this door — `discover.provider.plex.tv` lines would
    // otherwise read as coming from plex.tv proper.
    crate::log(&format!(
        "account: {verb} {} -> HTTP {}{hint}",
        endpoint_shape(url),
        status
    ));
}

/// The **shape** of one of this client's URLs, for a log line: host + path, every id-shaped segment
/// folded to `{id}`, and the query string dropped whole.
///
/// None of these URLs may be logged verbatim. The query carries [`AccountClient::switch_user`]'s
/// `?pin=NNNN` — the managed user's PIN — so it goes entirely rather than field by
/// field, which would need re-auditing every time a parameter is added. The path carries
/// the pin id, about which `auth.rs` already says, where it declines to log it: "the id is a handle
/// that redeems a credential, and the code is what authorizes it — and this file is the one we ask
/// users to send us when something goes wrong."
///
/// The host is kept, because it is the half that says WHICH service answered — `plex.tv` (the
/// account API) or `discover.provider.plex.tv` (the metadata provider, `discover.rs`) — and both
/// are `const`s in this crate rather than anything a user or a server chose.
fn endpoint_shape(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split_once("://").map(|(_, rest)| rest).unwrap_or(path);
    let mut out = String::with_capacity(path.len());
    for (i, seg) in path.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        // `i == 0` is the host, kept whole; everything after it is a path segment.
        if i > 0 && id_shaped(seg) {
            out.push_str("{id}");
        } else {
            out.push_str(seg);
        }
    }
    out
}

/// Is this path segment an identifier rather than a route name? Two shapes reach these endpoints: a
/// decimal id (`/api/v2/pins/12345`) and a hex guid or uuid (`/api/v2/home/users/{uuid}/switch`,
/// `/library/people/5d77682aeb5d26001f1de4b0`).
///
/// The rule is written to be safe in the direction that matters: an unrecognised id is worse than a
/// folded route name. It still keeps every literal segment the two files that build these URLs use
/// — `api`, `v2`, `pins`, `resources`, `home`, `users`, `switch` here and `library`, `people` in
/// `discover.rs` — because each is either too short to be a guid or contains a letter past `f`.
/// The test below enumerates them, so adding an endpoint whose name would fold is a failing test
/// rather than a log line that no longer says which call failed.
fn id_shaped(seg: &str) -> bool {
    if seg.is_empty() {
        return false;
    }
    seg.bytes().all(|b| b.is_ascii_digit())
        || (seg.len() >= 8 && seg.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

fn play_timeouts(remaining: Duration) -> crate::net::Timeouts {
    let millis = remaining.as_millis().max(1).min(i32::MAX as u128) as _;
    crate::net::Timeouts { total_ms: millis, ..crate::net::API }
}

fn audio_preferences_cached_at<F, N, C>(cache: &AudioPreferencesCache,
    key: AudioPreferencesKey, budget: Duration, now: N, fetch: F,
    still_current: C) -> AudioPreferencesOutcome
where F: FnOnce(Duration) -> AudioPreferencesOutcome, N: Fn() -> Instant, C: Fn() -> bool {
    let deadline = now() + budget;
    let flight = {
        let mut state = cache.state.lock().unwrap_or_else(|e| e.into_inner());
        // An old playback worker must not evict the active profile's saved preferences or
        // settings request merely by entering after the profile publication changed.
        if !still_current() { return AudioPreferencesOutcome::Failed; }
        if state.key.as_ref() != Some(&key) {
            state.key = Some(key.clone()); state.entry = None; state.flight = None;
            cache.changed.notify_all();
        }
        loop {
            if state.key.as_ref() != Some(&key) || !still_current() {
                return AudioPreferencesOutcome::Failed;
            }
            let at = now();
            if let Some(entry) = &state.entry {
                if at < entry.expires_at { return entry.outcome.clone(); }
            }
            if state.flight.is_some() {
                if at >= deadline { return AudioPreferencesOutcome::TimedOut; }
                let (next, _) = cache.changed.wait_timeout(state, deadline - at)
                    .unwrap_or_else(|e| e.into_inner());
                state = next;
                continue;
            }
            state.next_flight = state.next_flight.wrapping_add(1);
            let id = state.next_flight;
            state.flight = Some(id);
            break AudioPreferencesFlight { key: key.clone(), id };
        }
    };
    let at = now();
    let outcome = if at >= deadline { AudioPreferencesOutcome::TimedOut }
        else { fetch(deadline - at) };
    let completed_at = now();
    let outcome = if completed_at >= deadline { AudioPreferencesOutcome::TimedOut } else { outcome };
    let installed = cache.complete(flight, outcome.clone(), FetchPath::Play, completed_at, still_current);
    if installed { outcome } else { AudioPreferencesOutcome::Failed }
}

fn warm_audio_preferences_with<S, F, C>(cache: &'static AudioPreferencesCache,
    key: AudioPreferencesKey, spawn: S, fetch: F, still_current: C)
where S: FnOnce(Box<dyn FnOnce() + Send>) -> bool,
    F: FnOnce() -> AudioPreferencesOutcome + Send + 'static,
    C: FnOnce() -> bool + Send + 'static {
    let Some(flight) = cache.reserve(key, Instant::now()) else { return; };
    let cancel = flight.clone();
    let spawned = spawn(Box::new(move || {
        let outcome = fetch();
        cache.complete(flight, outcome, FetchPath::Warm, Instant::now(), still_current);
    }));
    if !spawned { cache.cancel(&cancel); }
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn warm_audio_preferences(client_id: String, credential: String,
    user: super::session::UserRef, generation: u32) {
    if client_id.is_empty() || credential.is_empty() { return; }
    let key = AudioPreferencesKey::new(&user, generation);
    let current = key.clone();
    warm_audio_preferences_with(&AUDIO_PREFERENCES_CACHE, key,
        |job| crate::task::spawn_small("account-audio", job),
        move || AccountClient::new(&client_id, Some(&credential))
            .fetch_audio_preferences(&user, crate::net::API),
        move || current.is_current());
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn publish_audio_preferences_profile(user: Option<&super::session::UserRef>,
    generation: u32) {
    match user {
        Some(user) => AUDIO_PREFERENCES_CACHE.publish(AudioPreferencesKey::new(user, generation)),
        None => AUDIO_PREFERENCES_CACHE.clear_profile(),
    }
}

fn de_soft_i64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}
fn de_soft_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match value {
        serde_json::Value::Bool(v) => Some(v),
        serde_json::Value::Number(n) => n.as_i64().map(|v| v != 0),
        serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true), "false" | "0" => Some(false), _ => None,
        },
        _ => None,
    })
}
fn de_soft_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.as_str().map(str::to_owned))
}
fn de_soft_profile<'de, D: serde::Deserializer<'de>>(d: D)
    -> Result<Option<AccountAudioProfile>, D::Error>
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// Narrow `/api/v2/user` DTO. Every field is soft because this optional fetch must never break
/// playback when plex.tv adds, removes or malforms a preference field.
#[derive(Deserialize, Default)]
#[serde(default)]
struct AccountUser {
    #[serde(deserialize_with = "de_soft_i64")]
    id: Option<i64>,
    #[serde(deserialize_with = "de_soft_string")]
    uuid: Option<String>,
    #[serde(deserialize_with = "de_soft_profile")]
    profile: Option<AccountAudioProfile>,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AccountAudioProfile {
    #[serde(deserialize_with = "de_soft_bool")]
    auto_select_audio: Option<bool>,
    #[serde(deserialize_with = "de_soft_string")]
    default_audio_language: Option<String>,
    #[serde(deserialize_with = "de_soft_string")]
    default_subtitle_language: Option<String>,
    #[serde(deserialize_with = "de_soft_i64", alias = "subtitleMode")]
    auto_select_subtitle: Option<i64>,
    #[serde(deserialize_with = "de_soft_i64", alias = "subtitleForced")]
    default_subtitle_forced: Option<i64>,
}
impl AccountAudioProfile {
    fn preferences(self) -> AudioPreferences {
        let clean_language = |value: Option<String>| value.map(|language| language.trim().to_owned())
            .filter(|language| !language.is_empty() && language != "-1");
        let stated_language = clean_language(self.default_audio_language);
        let language = (self.auto_select_audio == Some(true))
            .then(|| stated_language.clone()).flatten();
        AudioPreferences {
            language, auto_select_audio: self.auto_select_audio, stated_language,
            subtitle_language: clean_language(self.default_subtitle_language),
            subtitle_mode: self.auto_select_subtitle.unwrap_or(0),
            subtitle_forced: self.default_subtitle_forced.unwrap_or(0),
        }
    }
}
impl AccountUser {
    fn audio_preferences_for(self, expected: &super::session::UserRef)
        -> Option<AudioPreferences>
    {
        let expected_knows_id = expected.id != 0;
        let expected_knows_uuid = !expected.uuid.is_empty();
        let id_matches = expected_knows_id && self.id == Some(expected.id);
        let uuid_matches = expected_knows_uuid
            && self.uuid.as_deref() == Some(expected.uuid.as_str());
        let id_disagrees = expected_knows_id && self.id.is_some_and(|id| id != expected.id);
        let uuid_disagrees = expected_knows_uuid
            && self.uuid.as_deref().is_some_and(|uuid| !uuid.is_empty() && uuid != expected.uuid);
        if id_disagrees || uuid_disagrees
            || ((expected_knows_id || expected_knows_uuid) && !id_matches && !uuid_matches)
        {
            return None;
        }
        Some(self.profile.unwrap_or_default().preferences())
    }
}

// ---- serde DTOs (only the fields the app consumes; all optional to tolerate shape drift) ----

/// One account server/resource (`/api/v2/resources`) — owned **and** shared alike; `owned` is the
/// only thing that tells them apart, and it is a preference here, never a wall.
///
/// Everything past `connections` is a **connection-policy input** rather than something drawn:
/// `https_required` and `public_address_matches` decide which candidate URLs may exist at all
/// (`probe.rs`), and `source_title` is the owner's plex.tv handle — the one string the UI ever says
/// about a shared server ("Shared by friend"), the machine name staying in the sources list.
///
/// **No field here is strict, and that is the whole point.** plex.tv sends an explicit `null` for an
/// absent value (`sourceTitle` is null on every owned server), and serde's `default` covers a field
/// that is ABSENT, not one that is present and `null` — a distinction that costs nothing until it
/// costs everything: one strict `String` meeting one `null` fails the WHOLE resources array, so a
/// single odd row takes every other server with it and sign-in ends at "no server found".
///
/// So `sourceTitle` is a real [`Option`] (absent and empty mean different things — it is the handle
/// or there is no handle), every other string goes through `de_str`, `connections` through `de_vec`,
/// the flags through `de_bool` and the ids through `de_i64`. Adding a field means picking one of
/// those, never a bare `String`. Same trap as [`HomeUser`] below, which documents the day it bit.
#[derive(Deserialize, Default)]
pub struct Resource {
    #[serde(default, deserialize_with = "de_str")]
    pub name: String,
    #[serde(rename = "clientIdentifier", default, deserialize_with = "de_str")]
    pub client_identifier: String,
    #[serde(default, deserialize_with = "de_str")]
    pub provides: String, // may be a comma list ("server,player")
    #[serde(default, deserialize_with = "de_bool")]
    pub owned: bool,
    #[serde(rename = "accessToken", default, deserialize_with = "de_str")]
    pub access_token: String,
    /// The owner's plex.tv username, present ONLY on a shared resource (null when `owned`). This is
    /// what "shared" looks like on the wire, and the label Plex's own TV client shows.
    #[serde(rename = "sourceTitle", default)]
    pub source_title: Option<String>,
    /// plex.tv id of the account that owns the server; 0 on our own. Identity, not a label — the
    /// handle to show a user is `source_title`.
    #[serde(rename = "ownerId", default, deserialize_with = "de_i64")]
    pub owner_id: i64,
    /// **Undocumented, and read as a FALLBACK only.** The name says "this grant reaches me through
    /// my Plex Home (a household, not a friend share)", and that is the reading
    /// [`super::servers::is_household`] uses — but python-plexapi documents this field as
    /// *"home (bool): Unknown"*, and nobody in this project has observed it `true`. One rival
    /// reading IS refuted: the dev account is a Plex Home ADMIN and both its grants (its own server
    /// and a friend's share) report `false`, so it is not "the owner has a Plex Home".
    ///
    /// So it decides nothing while `ownerId` can be compared against the Home roster, which is the
    /// measured signal; see `docs/shared-servers.md` §13 for the evidence and the precedence.
    #[serde(default, deserialize_with = "de_bool")]
    pub home: bool,
    /// plex.tv's own liveness hint from its last check-in. A hint, never a verdict: only a probe
    /// that answered with the right `machineIdentifier` proves a server reachable from this TV.
    #[serde(default, deserialize_with = "de_bool")]
    pub presence: bool,
    /// **Our** public IP equals the one this server checked in from, i.e. we really are behind the
    /// same NAT — the only field that means "you are on that LAN". `Connection.local` does NOT
    /// (see [`Connection::local`]); this is the field that makes a non-owned `local` usable.
    #[serde(rename = "publicAddressMatches", default, deserialize_with = "de_bool")]
    pub public_address_matches: bool,
    /// The owner set *Require secure connections*, so plain HTTP is refused. It suppresses every
    /// synthesized `http://` candidate — see `probe::candidates`.
    #[serde(rename = "httpsRequired", default, deserialize_with = "de_bool")]
    pub https_required: bool,
    #[serde(default, deserialize_with = "de_vec")]
    pub connections: Vec<Connection>,
}
impl Resource {
    pub fn is_server(&self) -> bool {
        self.provides.split(',').any(|p| p.trim() == "server")
    }

    /// This row reduced to **whose server it is** — the input to the one "Shared by …" rule,
    /// [`servers::owner_credit`](super::servers::owner_credit). It is the only place a wire row
    /// becomes a [`Grant`](super::servers::Grant), so no caller can decide ownership from a subset
    /// of these fields — and each of the two obvious subsets is a shipped bug: `owned` alone reads
    /// a Plex Home managed user's own household server as somebody else's, and a non-empty
    /// `sourceTitle` alone reads it as a friend's share.
    pub fn grant(&self) -> super::servers::Grant<'_> {
        super::servers::Grant {
            owned: self.owned,
            home: self.home,
            owner_id: self.owner_id,
            source_title: self.source_title.as_deref().unwrap_or_default(),
        }
    }
}

/// One reachable address for a [`Resource`]. `address`:`port` is the raw host:port (plain HTTP is
/// all the PMS socket can speak); `uri` is the full scheme URL — an https `*.plex.direct` name whose
/// dashed-IP label resolves to `address`, held verbatim because the hash label is the certificate's,
/// not the machine id, and https to the bare IP fails validation by design.
#[derive(Deserialize, Default)]
pub struct Connection {
    #[serde(default, deserialize_with = "de_str")]
    pub protocol: String,
    #[serde(default, deserialize_with = "de_str")]
    pub address: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub port: i64,
    #[serde(default, deserialize_with = "de_str")]
    pub uri: String,
    /// **"This address is RFC1918", not "you are on that LAN."** Measured 2026-08-11: a shared
    /// server advertises `local:true` on the OWNER's `172.20.x.x`, which from here times out after
    /// 8 s — or, worse, reaches a different machine of ours at that address.
    /// `Resource::public_address_matches` is the field that means what this one looks like it means.
    #[serde(default, deserialize_with = "de_bool")]
    pub local: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub relay: bool,
    /// Capital on the wire. Ranked last rather than dropped (`probe.rs`).
    #[serde(rename = "IPv6", default, deserialize_with = "de_bool")]
    pub ipv6: bool,
}

#[cfg(test)]
mod tests {
    use super::{endpoint_shape, AccountClient, Resource};
    use std::time::Duration;

    fn audio_user(id: i64, uuid: &str) -> super::super::session::UserRef {
        super::super::session::UserRef { id, uuid: uuid.into(), token: "profile-token".into(),
            ..Default::default() }
    }
    fn parsed_audio(body: &str, expected: &super::super::session::UserRef)
        -> Option<super::AudioPreferences>
    {
        serde_json::from_str::<super::AccountUser>(body).ok()?.audio_preferences_for(expected)
    }

    #[test]
    fn account_audio_preferences_parse_the_measured_user_shape() {
        let user = audio_user(7, "profile-uuid");
        let body = r#"{"id":7,"uuid":"profile-uuid","home":true,"homeAdmin":false,
            "restricted":true,"profile":{"autoSelectAudio":true,
            "defaultAudioLanguage":"fr","defaultAudioLanguages":null}}"#;
        assert_eq!(parsed_audio(body, &user), Some(french_prefs()));
    }

    #[test]
    fn account_subtitle_preferences_reach_the_playback_cache() {
        let user = audio_user(7, "profile-uuid");
        let body = r#"{"id":7,"profile":{"autoSelectAudio":true,
            "defaultAudioLanguage":"en-GB","defaultSubtitleLanguage":"fr-CA",
            "autoSelectSubtitle":"2","defaultSubtitleForced":3}}"#;
        let prefs = parsed_audio(body, &user).unwrap();
        assert_eq!(prefs.subtitle_language.as_deref(), Some("fr-CA"));
        assert_eq!(prefs.subtitle_mode, 2);
        assert_eq!(prefs.subtitle_forced, 3);
    }

    #[test]
    fn account_audio_preferences_require_auto_select_audio() {
        let user = audio_user(7, "profile-uuid");
        for (body, auto_select_audio) in [
            (r#"{"id":7,"profile":{"autoSelectAudio":false,"defaultAudioLanguage":"fr"}}"#,
                Some(false)),
            (r#"{"id":7,"profile":{"defaultAudioLanguage":"fr"}}"#, None),
        ] {
            assert_eq!(parsed_audio(body, &user), Some(super::AudioPreferences {
                language: None, auto_select_audio, stated_language: Some("fr".into()),
                ..Default::default()
            }));
        }
    }

    #[test]
    fn account_audio_preferences_treat_null_or_empty_language_as_unset() {
        let user = audio_user(7, "profile-uuid");
        for language in ["null", "\"\"", "\"   \""] {
            let body = format!(r#"{{"uuid":"profile-uuid","profile":{{"autoSelectAudio":true,"defaultAudioLanguage":{language}}}}}"#);
            assert_eq!(parsed_audio(&body, &user), Some(super::AudioPreferences {
                auto_select_audio: Some(true), ..Default::default()
            }));
        }
    }

    #[test]
    fn account_audio_preferences_accept_a_response_when_the_expected_owner_has_no_identity_fields() {
        let owner = audio_user(0, "");
        let body = r#"{"id":7,"uuid":"owner-uuid","profile":{"autoSelectAudio":true,
            "defaultAudioLanguage":"fr"}}"#;
        assert_eq!(parsed_audio(body, &owner), Some(french_prefs()));
    }

    #[test]
    fn account_audio_preferences_reject_a_mismatching_known_id() {
        let user = audio_user(7, "");
        let body = r#"{"id":8,"profile":{"autoSelectAudio":true,"defaultAudioLanguage":"fr"}}"#;
        assert_eq!(parsed_audio(body, &user), None);
    }

    #[test]
    fn account_audio_preferences_reject_a_mismatching_known_uuid() {
        let user = audio_user(0, "profile-uuid");
        let body = r#"{"uuid":"other","profile":{"autoSelectAudio":true,"defaultAudioLanguage":"fr"}}"#;
        assert_eq!(parsed_audio(body, &user), None);
    }

    fn audio_key(id: i64, generation: u32) -> super::AudioPreferencesKey {
        super::AudioPreferencesKey { id, uuid: format!("user-{id}"), generation }
    }
    fn french_prefs() -> super::AudioPreferences {
        super::AudioPreferences {
            language: Some("fr".into()), auto_select_audio: Some(true),
            stated_language: Some("fr".into()), ..Default::default()
        }
    }
    fn french() -> super::AudioPreferencesOutcome {
        super::AudioPreferencesOutcome::Available(french_prefs())
    }

    #[test]
    fn play_timeout_does_not_install_backoff() {
        let cache = super::AudioPreferencesCache::new();
        let calls = std::cell::Cell::new(0);
        let fetch = |_| { calls.set(calls.get() + 1); if calls.get() == 1 {
            super::AudioPreferencesOutcome::TimedOut } else { french() } };
        let key = audio_key(1, 1);
        assert_eq!(super::audio_preferences_cached_at(&cache, key.clone(), Duration::from_secs(1),
            std::time::Instant::now, fetch, || true), super::AudioPreferencesOutcome::TimedOut);
        assert_eq!(super::audio_preferences_cached_at(&cache, key, Duration::from_secs(1),
            std::time::Instant::now, fetch, || true), french());
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn completed_failure_still_backs_off() {
        let cache = super::AudioPreferencesCache::new();
        let now = std::cell::Cell::new(std::time::Instant::now());
        let calls = std::cell::Cell::new(0);
        let fetch = |_| { calls.set(calls.get() + 1); super::AudioPreferencesOutcome::Failed };
        let key = audio_key(1, 1);
        assert_eq!(super::audio_preferences_cached_at(&cache, key.clone(), Duration::from_secs(1),
            || now.get(), fetch, || true), super::AudioPreferencesOutcome::Failed);
        now.set(now.get() + Duration::from_secs(44));
        assert_eq!(super::audio_preferences_cached_at(&cache, key, Duration::from_secs(1),
            || now.get(), fetch, || true), super::AudioPreferencesOutcome::Failed);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn waiter_joins_an_inflight_fetch_without_starting_another() {
        let cache = Box::leak(Box::new(super::AudioPreferencesCache::new()));
        let key = audio_key(1, 1);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        let worker_handle = handle.clone();
        super::warm_audio_preferences_with(cache, key.clone(), move |job| {
            *worker_handle.lock().unwrap() = Some(std::thread::spawn(job)); true
        }, move || { worker_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            release_rx.recv().unwrap(); french() }, || true);
        let waiter_calls = calls.clone();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5)); release_tx.send(()).unwrap();
        });
        assert_eq!(super::audio_preferences_cached_at(cache, key, Duration::from_millis(100),
            std::time::Instant::now, move |_| {
                waiter_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst); french()
            }, || true), french());
        releaser.join().unwrap();
        handle.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn waiter_expiry_leaves_the_flight_running_and_it_later_populates_cache() {
        let cache = Box::leak(Box::new(super::AudioPreferencesCache::new()));
        let key = audio_key(1, 1);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        let worker_handle = handle.clone();
        super::warm_audio_preferences_with(cache, key.clone(), move |job| {
            *worker_handle.lock().unwrap() = Some(std::thread::spawn(job)); true
        }, move || { release_rx.recv().unwrap(); french() }, || true);
        assert_eq!(super::audio_preferences_cached_at(cache, key.clone(), Duration::from_millis(3),
            std::time::Instant::now, |_| panic!("waiter started a second fetch"), || true),
            super::AudioPreferencesOutcome::TimedOut);
        release_tx.send(()).unwrap();
        handle.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(super::audio_preferences_cached_at(cache, key, Duration::from_millis(3),
            std::time::Instant::now, |_| panic!("warm result was not cached"), || true), french());
    }

    #[test]
    fn stale_key_or_flight_completion_cannot_overwrite_a_newer_key() {
        let cache = super::AudioPreferencesCache::new();
        let old = audio_key(1, 1); let new = audio_key(2, 2);
        let old_flight = cache.reserve(old, std::time::Instant::now()).unwrap();
        cache.publish(new.clone());
        let new_flight = cache.reserve(new.clone(), std::time::Instant::now()).unwrap();
        assert!(!cache.complete(old_flight, french(), super::FetchPath::Warm,
            std::time::Instant::now(), || false));
        let german = super::AudioPreferencesOutcome::Available(super::AudioPreferences {
            language: Some("de".into()), ..Default::default() });
        assert!(cache.complete(new_flight, german.clone(), super::FetchPath::Warm,
            std::time::Instant::now(), || true));
        assert_eq!(super::audio_preferences_cached_at(&cache, new, Duration::from_millis(3),
            std::time::Instant::now, |_| panic!("new result was not cached"), || true), german);
    }

    #[test]
    fn account_regression_stale_fetch_start_cannot_evict_current_profile() {
        let cache = super::AudioPreferencesCache::new();
        let old = audio_key(1, 1); let current = audio_key(2, 2);
        let flight = cache.reserve(current.clone(), std::time::Instant::now()).unwrap();
        let result = super::audio_preferences_cached_at(&cache, old, Duration::from_millis(10),
            std::time::Instant::now, |_| panic!("stale identity reached the network"), || false);
        assert_eq!(result, super::AudioPreferencesOutcome::Failed);
        {
            let state = cache.state.lock().unwrap();
            assert_eq!(state.key.as_ref(), Some(&current));
            assert_eq!(state.flight, Some(flight.id));
        }
        assert!(cache.complete(flight, french(), super::FetchPath::Warm,
            std::time::Instant::now(), || true));
    }

    #[test]
    fn spawn_failure_clears_the_flight() {
        let cache = Box::leak(Box::new(super::AudioPreferencesCache::new()));
        let key = audio_key(1, 1);
        super::warm_audio_preferences_with(cache, key.clone(), |_| false,
            || panic!("refused spawn ran a worker"), || true);
        assert_eq!(super::audio_preferences_cached_at(cache, key, Duration::from_millis(20),
            std::time::Instant::now, |_| french(), || true), french());
    }

    #[test]
    fn account_headers_use_the_same_honest_language_source_as_pms() {
        let headers = AccountClient::new("cid", None).headers();
        let sent = headers
            .iter()
            .find_map(|h| h.strip_prefix("X-Plex-Language: "));
        assert_eq!(sent, super::super::identity::language());
    }

    /// The log's shape rule, on the exact URLs this file and `discover.rs` build. Both halves are
    /// asserted: the route survives (or the line stops saying which call failed) and every
    /// identifier does not — the pin id redeems the account token, the `?pin=` is a managed user's
    /// PIN, and the person guid is an id no log needs.
    #[test]
    fn an_endpoint_shape_keeps_the_route_and_drops_every_identifier() {
        // create_pin / resources / home_users: the query is dropped, the route is untouched.
        assert_eq!(
            endpoint_shape("https://plex.tv/api/v2/pins?strong=false"),
            "plex.tv/api/v2/pins"
        );
        assert_eq!(
            endpoint_shape(
                "https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1"
            ),
            "plex.tv/api/v2/resources"
        );
        assert_eq!(
            endpoint_shape("https://plex.tv/api/v2/home/users"),
            "plex.tv/api/v2/home/users"
        );

        // poll_pin: a decimal id folds — `auth.rs` refuses to log this number for a reason.
        let shape = endpoint_shape("https://plex.tv/api/v2/pins/1234567");
        assert_eq!(shape, "plex.tv/api/v2/pins/{id}");
        assert!(!shape.contains("1234567"));

        // switch_user: a uuid mid-path folds while the trailing route name survives, and the PIN
        // in the query is gone with the rest of it.
        let shape = endpoint_shape("https://plex.tv/api/v2/home/users/2b3c4d5e-6f70-4a81-9b2c-3d4e5f607182/switch?pin=4321");
        assert_eq!(shape, "plex.tv/api/v2/home/users/{id}/switch");
        assert!(!shape.contains("4321") && !shape.contains("2b3c"));

        // discover.rs: the OTHER host is kept — it is which service answered — and the tagKey guid
        // folds like any other id.
        assert_eq!(
            endpoint_shape(
                "https://discover.provider.plex.tv/library/people/5d77682aeb5d26001f1de4b0"
            ),
            "discover.provider.plex.tv/library/people/{id}"
        );
    }

    /// Why the parse-failure line logs serde's CATEGORY and position instead of the message: the
    /// The real `/api/v2/resources` shape, both kinds of server in one array: ours (`owned`, with
    /// `sourceTitle`/`ownerId` sent as explicit **nulls**) and a share (`owned:false`, a handle in
    /// `sourceTitle`, `httpsRequired:false`). Shaped on the live response measured 2026-08-11
    /// (docs/shared-servers.md §2); addresses and identifiers are stand-ins, the SHAPE is not.
    ///
    /// The null is the whole point. serde's `default` does not cover an explicit null, so a strict
    /// `String` on `sourceTitle` fails the entire array — not one label, but every server, i.e.
    /// sign-in reporting "no server found" on an account that has two.
    #[test]
    fn owned_and_shared_resources_round_trip_with_explicit_nulls() {
        let json = br#"[
          {"name":"Gleb's Mac mini","clientIdentifier":"aaaa1111","provides":"server",
           "owned":true,"home":true,"presence":true,"publicAddressMatches":true,
           "httpsRequired":false,"sourceTitle":null,"ownerId":null,"accessToken":"tok-own",
           "connections":[
             {"protocol":"https","address":"192.168.0.10","port":32400,
              "uri":"https://192-168-0-10.hash1.plex.direct:32400","local":true,"relay":false,"IPv6":false},
             {"protocol":"https","address":"2001:db8::1","port":32400,
              "uri":"https://2001-db8--1.hash1.plex.direct:32400","local":true,"relay":false,"IPv6":true}]},
          {"name":"nas-home","clientIdentifier":"bbbb2222","provides":"server",
           "owned":false,"home":false,"presence":true,"publicAddressMatches":false,
           "httpsRequired":false,"sourceTitle":"friend","ownerId":987654,"accessToken":"tok-share",
           "connections":[
             {"protocol":"https","address":"10.9.9.7","port":32400,
              "uri":"https://172-20-4-7.hash2.plex.direct:32400","local":true,"relay":false,"IPv6":false},
             {"protocol":"https","address":"203.0.113.9","port":31234,
              "uri":"https://203-0-113-9.hash2.plex.direct:31234","local":false,"relay":false,"IPv6":false}]}
        ]"#;
        let rs: Vec<Resource> =
            serde_json::from_slice(json).expect("explicit nulls must not fail the array");
        assert_eq!(
            rs.len(),
            2,
            "both servers survive — the null did not take the container with it"
        );

        let own = &rs[0];
        assert!(own.owned && own.is_server());
        assert_eq!(
            own.source_title, None,
            "an owned server has no owner to name"
        );
        assert_eq!(own.owner_id, 0);
        assert!(own.home && own.presence && own.public_address_matches && !own.https_required);
        assert!(own.connections[1].ipv6, "IPv6 is capital on the wire");

        let share = &rs[1];
        assert!(!share.owned && share.is_server());
        // the handle is the ONE string the browsing UI says about a share; the machine name
        // (`name`) stays in the sources list.
        assert_eq!(share.source_title.as_deref(), Some("friend"));
        assert_eq!(share.owner_id, 987_654);
        assert!(
            !share.public_address_matches,
            "their 172.20 LAN is not ours — the load-bearing flag"
        );
        assert_eq!(
            share.access_token, "tok-share",
            "per-(user,server) grant, never the account token"
        );
        assert_eq!(share.connections.len(), 2);
    }

    /// Shape drift must cost one field, never the container: the 0/1 encodings Plex also uses for
    /// its flags, a string-encoded port, an absent flag, and an unknown field all land. A resource
    /// that fails to parse here would take every OTHER server in the array down with it.
    #[test]
    fn a_malformed_or_oddly_encoded_field_does_not_fail_the_container() {
        let json = br#"[
          {"name":"quirky","clientIdentifier":"cccc3333","provides":"server,player",
           "owned":1,"httpsRequired":"1","publicAddressMatches":0,"sourceTitle":null,
           "unknownFutureField":{"nested":true},
           "connections":[{"address":"10.0.0.4","port":"32400","uri":"","local":"1","IPv6":null}]},
          {"name":"sparse","clientIdentifier":"dddd4444","provides":"server"}
        ]"#;
        let rs: Vec<Resource> = serde_json::from_slice(json).expect("lenient parse");
        assert_eq!(rs.len(), 2);
        assert!(rs[0].owned, "1 is true");
        assert!(rs[0].https_required, "\"1\" is true");
        assert!(!rs[0].public_address_matches, "0 is false");
        assert_eq!(
            rs[0].connections[0].port, 32400,
            "a string-encoded port is still a port"
        );
        assert!(rs[0].connections[0].local);
        assert!(
            !rs[0].connections[0].ipv6,
            "an explicit null flag is false, not a parse failure"
        );
        // everything absent on the second resource degrades to the zero value, and it still counts
        // as a server — which is what keeps ONE odd row from emptying the roster.
        assert!(rs[1].is_server() && !rs[1].owned && rs[1].connections.is_empty());
        assert_eq!(rs[1].source_title, None);
    }

    /// An explicit `null` on EVERY string and on the connection list itself. This is the shape the
    /// struct's doc promises to survive, and until `de_str`/`de_vec` were applied to the non-`Option`
    /// fields it did not: `name`, `provides`, `clientIdentifier`, `accessToken`, `connections` and a
    /// connection's own `address`/`uri`/`protocol` were bare, and any one of them meeting a null
    /// failed the whole array. The blast radius is the point — the SECOND resource here is a
    /// perfectly good server, and the test is really asserting that it still arrives.
    #[test]
    fn an_explicit_null_on_any_string_costs_that_field_and_never_the_roster() {
        let json = br#"[
          {"name":null,"clientIdentifier":null,"provides":null,"accessToken":null,
           "sourceTitle":null,"ownerId":null,"connections":null},
          {"name":"survivor","clientIdentifier":"eeee5555","provides":"server","owned":true,
           "connections":[{"protocol":null,"address":null,"uri":null,"port":32400}]}
        ]"#;
        let rs: Vec<Resource> =
            serde_json::from_slice(json).expect("a null must not fail the array");
        assert_eq!(rs.len(), 2, "the good server must survive the bad row");

        // the all-null row degrades field by field, and stops being a server rather than exploding
        assert!(rs[0].name.is_empty() && rs[0].access_token.is_empty());
        assert!(!rs[0].is_server(), "a null `provides` names no capability");
        assert!(
            rs[0].connections.is_empty(),
            "a null connection list is an empty one"
        );

        // and the row that matters is untouched
        assert!(rs[1].is_server() && rs[1].owned);
        assert_eq!(rs[1].name, "survivor");
        assert_eq!(rs[1].connections.len(), 1);
        assert!(
            rs[1].connections[0].address.is_empty(),
            "null address degrades to empty"
        );
        assert_eq!(rs[1].connections[0].port, 32400);
    }

    /// **The `/api/v2/resources` a Plex Home MANAGED user gets**, and the [`Resource::grant`] that
    /// reads it — the shape behind the reported "Shared by <the account holder>" on the user's own
    /// household server.
    ///
    /// A profile switch re-fetches this endpoint with the SWITCHED user's token, so plex.tv answers
    /// about that user: the household's own server is not theirs (`owned:false`), it names the
    /// admin in `sourceTitle`, and `ownerId` is the admin's plex.tv account id — the same id
    /// `/api/v2/home/users` lists for the admin row and `/api/v2/user` reports for the account
    /// (measured 2026-09-03 on the dev account). Fed to the rule with the household enumerated,
    /// none of that is a credit.
    #[test]
    fn a_managed_users_view_of_the_household_server_grants_no_credit() {
        const ADMIN: i64 = 111_111;
        const MANAGED: i64 = 222_222;
        let json = br#"[
          {"name":"Mac mini","clientIdentifier":"aaaa1111","provides":"server",
           "owned":false,"home":true,"sourceTitle":"admin","ownerId":111111,
           "accessToken":"tok-managed-own","connections":[]},
          {"name":"nas-home","clientIdentifier":"bbbb2222","provides":"server",
           "owned":false,"home":false,"sourceTitle":"friend","ownerId":987654,
           "accessToken":"tok-managed-share","connections":[]}
        ]"#;
        let rs: Vec<Resource> = serde_json::from_slice(json).expect("lenient parse");

        let household = [ADMIN, MANAGED];
        let (house, share) = (&rs[0], &rs[1]);

        // plex.tv's raw answer, which is what makes this look exactly like a share
        assert!(!house.owned && house.source_title.as_deref() == Some("admin"));

        assert_eq!(
            super::super::servers::owner_credit(house.grant(), &household),
            "",
            "the house's own server credits nobody, whichever profile is watching"
        );
        assert_eq!(
            super::super::servers::owner_credit(share.grant(), &household),
            "friend",
            "and a person outside the house still is credited"
        );

        // **The `home:true` in that fixture is not what carries this test.** That field is the one
        // value in the row nobody here has measured (`docs/shared-servers.md` §13), so the case has
        // to hold on `ownerId` alone — which is the signal `/api/v2/home/users` proves is in the
        // same id space.
        let mut without_home = house.grant();
        without_home.home = false;
        assert_eq!(
            super::super::servers::owner_credit(without_home, &household),
            ""
        );

        // `grant` is the ONE reduction, so the null-vs-empty distinction dies here rather than at
        // six call sites: an owned server's absent `sourceTitle` arrives as the empty handle the
        // rule takes, and its absent `ownerId` as the `0` that matches no household member.
        let own: Resource = serde_json::from_slice(
            br#"{"clientIdentifier":"cccc3333","provides":"server","owned":true,
                 "sourceTitle":null,"ownerId":null}"#,
        )
        .expect("lenient parse");
        let g = own.grant();
        assert!(g.owned && g.source_title.is_empty() && g.owner_id == 0);
        assert_eq!(super::super::servers::owner_credit(g, &household), "");
    }
}
