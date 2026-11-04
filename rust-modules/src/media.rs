//! serde response DTOs — only the fields the app consumes. The wire shapes below are gated
//! `#[cfg(test)]`: every production constructor went with the Plex wire layer (#166), and what
//! remains is the test vocabulary that parses recorded shapes plus the canon-only `Resource` /
//! `Connection` rows and the lenient adapters they share with the live code.
use serde::Deserialize;




/// A SHOW's language settings — its Advanced dialog in Plex Web. `None` / `-1` mean "Account
/// default", inherited from the active Plex profile when its preferences are available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowLangPrefs {
    /// `audioLanguage`, e.g. `"hu-HU"`
    pub audio: Option<String>,
    /// `subtitleLanguage`, e.g. `"hu-HU"`
    pub subtitle: Option<String>,
    /// `subtitleMode`: -1 account default · 0 manually selected · 1 shown with foreign audio ·
    /// 2 always enabled
    pub subtitle_mode: i32,
}

impl Default for ShowLangPrefs {
    fn default() -> Self { Self { audio: None, subtitle: None, subtitle_mode: -1 } }
}

impl ShowLangPrefs {
}



/// Lenient i64: a JSON integer, a float (truncated), a numeric string, or null → 0. The old
/// code read every number with `.as_i64().unwrap_or(0)`, degrading a bad value to 0 for THAT
/// field only. serde's strict i64 would instead fail the WHOLE container parse on a
/// string-encoded int (the server does this: e.g. `size:"40"`, `streamType:"2"`), dropping every
/// item in the response. This keeps one field's-worth of degradation (and actually recovers a
/// stringy int rather than zeroing it). Applied to every i64 in a parsed DTO.
pub(crate) fn de_i64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntFloatStrBool {
        I(i64),
        F(f64),
        S(String),
        // The server sends the flag fields (default/selected/forced/…) as JSON booleans on some
        // endpoints (e.g. Stream inside /hubs) and as "1"/"0" strings on others. A missing bool
        // arm here fails the untagged enum → the WHOLE MediaContainer parse fails (blast radius),
        // so accept it: true→1, false→0.
        B(bool),
    }
    Ok(match Option::<IntFloatStrBool>::deserialize(d)? {
        Some(IntFloatStrBool::I(n)) => n,
        Some(IntFloatStrBool::F(f)) => f as i64,
        Some(IntFloatStrBool::S(s)) => s.trim().parse().unwrap_or(0),
        Some(IntFloatStrBool::B(b)) => b as i64,
        None => 0,
    })
}

/// Lenient `Vec<T>`: a JSON array, or null → empty. Same trap as [`de_str`], one container up:
/// `#[serde(default)]` fills an ABSENT field and does nothing for one that is present and `null`,
/// and a strict `Vec` meeting a `null` fails the whole array it sits in. `connections` is the field
/// that matters — a server with a null connection list must cost that server, never the roster.
pub(super) fn de_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

/// Lenient bool: a JSON bool, the `1`/`0` (or `"1"`/`"0"`) forms Plex also uses, or null → false.
///
/// The server DTOs above keep their 0/1 flags as `i64` and test `!= 0`, because those fields are
/// read once, near the wire. This adapter exists for the plex.tv **connection-policy** flags
/// (`account.rs`'s `httpsRequired`/`publicAddressMatches`/`local`/`relay`/`IPv6`), which are read
/// in boolean position by `probe.rs` on every candidate: folding the encodings here once beats
/// spelling `!= 0` at every branch of a policy that has to stay readable to be trusted. Leniency
/// itself is not optional — plex.tv sends these as real JSON bools today, and a strict `bool`
/// that meets a `"1"` fails the WHOLE resources parse, which is a silent sign-in failure.
pub(super) fn de_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolIntStr {
        B(bool),
        I(i64),
        S(String),
    }
    Ok(match Option::<BoolIntStr>::deserialize(d)? {
        Some(BoolIntStr::B(b)) => b,
        Some(BoolIntStr::I(n)) => n != 0,
        Some(BoolIntStr::S(s)) => matches!(s.trim(), "1" | "true" | "True"),
        None => false,
    })
}


/// Lenient String: a JSON string, a number stringified, or null → "". `videoResolution` is a
/// string on this server ("1080"/"4k", verified live) but reads like a number, and a strict
/// `String` that meets one is a WHOLE-`MediaContainer` parse failure — the same blast radius
/// `de_i64` exists to avoid, just pointing the other way. Use it for any stringly-typed number.
///
/// `pub(super)` because `account.rs` needs the null half of it: plex.tv sends an explicit `null` for
/// an absent string, and there a whole-container failure is the account's whole server list.
pub(super) fn de_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StrIntFloatBool {
        S(String),
        I(i64),
        F(f64),
        // no arm for a shape = the whole container fails on it, which is the one outcome this
        // adapter exists to prevent — so carry the bool too (`de_i64` learned that the hard way)
        B(bool),
    }
    Ok(match Option::<StrIntFloatBool>::deserialize(d)? {
        Some(StrIntFloatBool::S(s)) => s,
        Some(StrIntFloatBool::I(n)) => n.to_string(),
        Some(StrIntFloatBool::F(f)) => f.to_string(),
        Some(StrIntFloatBool::B(b)) => b.to_string(),
        None => String::new(),
    })
}

#[cfg(test)]
/// A route key the server handed us, safe to interpolate back into a request path: ids
/// (`ratingKey` and friends) and `/library/...` paths whose bytes are
/// `[A-Za-z0-9./_-]`. Anything else — a `?` that would truncate the path into a query, a `&`
/// that would inject a parameter, spaces, percent-escapes, control or non-ASCII bytes, or an
/// over-long blob — comes back EMPTY, which every consumer already treats as "no route" rather
/// than a request. The value only ever rides back to the server that sent it, so this is
/// request-shape hygiene at the parse boundary, not a trust boundary.
pub(super) fn de_route_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let raw = de_str(d)?;
    Ok(if is_route_key(&raw) { raw } else { String::new() })
}

#[cfg(test)]
fn is_route_key(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    let plain = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-');
    // Directory keys carry their own query (`/library/sections/1/all?actor=921`): the server
    // authored it, the app forwards it verbatim. ONE `?` separates it; its charset stays on the
    // list, so a bare `?` (truncation), spaces, `%`, control and non-ASCII bytes still refuse.
    let path = match s.split_once('?') {
        Some((p, q)) => {
            if q.is_empty()
                || !q
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b'&' | b'/' | b'.' | b'_' | b'-'))
            {
                return false;
            }
            p
        }
        None => s,
    };
    !path.is_empty() && path.bytes().all(plain)
}

#[cfg(test)]
mod route_key_allowlist_tests {
    use super::*;

    #[derive(Deserialize)]
    struct Probe {
        #[serde(default, deserialize_with = "de_route_key")]
        key: String,
    }

    fn key_of(raw: &str) -> String {
        serde_json::from_str::<Probe>(raw).expect("parse").key
    }

    #[test]
    fn route_keys_are_allow_listed_at_the_deserialization_boundary() {
        assert_eq!(key_of(r#"{"key":"12345"}"#), "12345");
        assert_eq!(
            key_of(r#"{"key":"/library/parts/1/1/file.mkv"}"#),
            "/library/parts/1/1/file.mkv"
        );
        assert_eq!(key_of(r#"{"key":"?next=/evil"}"#), "", "a ? would truncate the path into a query");
        assert_eq!(
            key_of(r#"{"key":"/library/sections/1/all?actor=921"}"#),
            "/library/sections/1/all?actor=921",
            "a directory key's query IS the key, and its charset is on the list",
        );
        assert_eq!(key_of(r#"{"key":"/x?"}"#), "", "an empty query is truncation");
        assert_eq!(key_of(r#"{"key":"1&x=2"}"#), "", "a & would inject a parameter");
        assert_eq!(key_of(r#"{"key":"a b"}"#), "", "a space would split the request line");
        assert_eq!(key_of(r#"{"key":"café"}"#), "", "non-ASCII is not on the list");
        assert_eq!(key_of(r#"{"key":"%2F..%2F"}"#), "", "percent-escapes are not on the list");
        let long = "a".repeat(201);
        assert_eq!(key_of(&format!(r#"{{"key":"{long}"}}"#)), "", "over-long is refused");
    }
}

/// Canon-only wire shapes since the plex.tv account surface was removed: the session-protocol
/// observation digest is frozen (SessionInit::write layout), so the recorded observation
/// vocabulary and the `/api/v2/resources` rows it named survive here with no producer. The
/// lenient adapters are `server::models`' — the shapes must keep tolerating exactly what the
/// account API used to send, because old recordings are replayed against this code.
#[derive(serde::Deserialize, Default)]
pub(crate) struct Resource {
    #[serde(default, deserialize_with = "de_str")]
    pub name: String,
    #[serde(rename = "clientIdentifier", default, deserialize_with = "de_str")]
    pub client_identifier: String,
    #[serde(default, deserialize_with = "de_str")]
    pub provides: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub owned: bool,
    #[serde(rename = "accessToken", default, deserialize_with = "de_str")]
    pub access_token: String,
    #[serde(rename = "sourceTitle", default)]
    pub source_title: Option<String>,
    #[serde(rename = "ownerId", default, deserialize_with = "de_i64")]
    pub owner_id: i64,
    #[serde(default, deserialize_with = "de_bool")]
    pub home: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub presence: bool,
    #[serde(rename = "publicAddressMatches", default, deserialize_with = "de_bool")]
    pub public_address_matches: bool,
    #[serde(rename = "httpsRequired", default, deserialize_with = "de_bool")]
    pub https_required: bool,
    #[serde(default, deserialize_with = "de_vec")]
    pub connections: Vec<Connection>,
}

/// Canon-only: one recorded address of a recorded [`Resource`].
#[derive(serde::Deserialize, Default)]
pub(crate) struct Connection {
    #[serde(default, deserialize_with = "de_str")]
    pub protocol: String,
    #[serde(default, deserialize_with = "de_str")]
    pub address: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub port: i64,
    #[serde(default, deserialize_with = "de_str")]
    pub uri: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub local: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub relay: bool,
    #[serde(rename = "IPv6", default, deserialize_with = "de_bool")]
    pub ipv6: bool,
}


