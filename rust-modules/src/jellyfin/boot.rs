//! boot — the Jellyfin flavor's boot gate arm: config file → authenticated client → installed.
//!
//! There is no plex.tv on this backend, so "signing in" is reading a small JSON config the
//! install carries, authenticating against the server it names, and installing the resulting
//! client for the stores to find. The config is searched in this order, first hit wins:
//!
//! 1. `/tmp/butaca-jellyfin` — the dev trigger, so the on-device harness (and a quick
//!    `make run` iteration) can point a build at a server without touching persistent state.
//!    Like every `/tmp` file on the TV it does not survive a reboot.
//! 2. `/media/developer/<app id>-jellyfin.json` — the persistent one, beside the session file
//!    (`paths::session_candidates`); this is where an installed PoC keeps its server.
//!
//! ```json
//! { "url": "http://192.168.1.20:8096", "user": "gleb", "token": "…" }
//! ```
//!
//! `url` must be an **IP literal** for plaintext servers: the credential gate's LAN relaxation
//! reads addresses, not names (`http::is_lan_host`), so `jellyfin.local` over http is refused —
//! say it as `https://…` or as an address.
//!
//! **The file keeps a token, never the password.** An access token is revocable on the server and
//! says nothing about the password the user reuses elsewhere, so it is the one credential worth
//! persisting: the boot arm adopts it, probes `/Users/Me` to learn whether it still stands, and
//! asks for the password only when the server has stopped honouring it. The sign-in form still
//! writes the field a password build wrote (`password`) and still READS it — an install that
//! upgrades signs in with it once, then the file is rewritten with the token and the plaintext is
//! gone. Legacy files are never rewritten with a password.
//!
//! The device id is minted once and persisted beside the config, so the server's
//! authorized-devices list does not grow a new "LG webOS TV" on every boot.

use super::client::{AuthError, JfClient};
use serde::{Deserialize, Serialize};

/// One configured server: the URL it answers at, the account that signed in, the ACCESS TOKEN that
/// stands in for the password, and (once minted) the per-server device id.
///
/// `password` is the legacy field: read for a file a password-era install wrote, never serialized
/// again. It is `None` on every entry this code saves, so a successful boot rewrites the file
/// without the plaintext (see [`remember_token`]).
#[derive(Deserialize, Serialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct ServerEntry {
    pub(crate) url: String,
    pub(crate) user: String,
    /// The password, from a file an older build wrote. Used once to mint a token, then dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) password: Option<String>,
    /// The access token minted by the last successful sign-in — what this file exists to keep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token: Option<String>,
    /// Optional stable device id; when absent one is minted and persisted (see module doc).
    #[serde(default)]
    pub(crate) device_id: Option<String>,
}

/// The flat single-server shape this install wrote before the multi-server list. Still read, never
/// written: a legacy file parses as a one-entry list with that entry active.
#[derive(Deserialize)]
struct BootConfig {
    url: String,
    user: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    token: Option<String>,
    /// Optional stable device id; when absent one is minted and persisted (see module doc).
    device_id: Option<String>,
}

/// The multi-server shape: every configured server and which one is active (an index into
/// `servers`).
#[derive(Deserialize, Serialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct ServerList {
    pub(crate) servers: Vec<ServerEntry>,
    #[serde(default)]
    pub(crate) active: usize,
}

/// The persisted file is either the list shape or, from an older install, the flat one. `untagged`
/// tries the list first (it requires a `servers` key) and falls back to the flat shape.
#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigFile {
    List(ServerList),
    Flat(BootConfig),
}

/// The boot gate's question, as one function: is there a Jellyfin server to go to, and did it
/// let us in? `true` means the client is INSTALLED and the app should boot to Home. `false`
/// covers every other outcome — no config (the common case on a Plex build, and the reason this
/// fn is cheap: the first read misses and nothing else runs), a config that will not parse, an
/// unreachable server, and wrong credentials — each logged on its own terms.
///
/// Runs BEFORE the main loop on the boot thread: a blocking round-trip here delays the first
/// frame by one LAN request, which is the honest price of knowing whether Home can fill.
pub(crate) fn try_boot() -> bool {
    let Some(cfg) = active_server() else {
        return false;
    };
    authenticate_server(&cfg)
}

/// Sign in against one configured server: adopt the token the file carries (or, for a
/// password-era file, mint one with the password first), and install the client on success. This
/// is the one place the boot gate and the Settings "switch server" action share — a switch is a
/// boot against a different entry of the same list.
pub(crate) fn authenticate_server(entry: &ServerEntry) -> bool {
    let Some(origin) = crate::origin::Origin::parse(&entry.url) else {
        crate::log("jellyfin: config ignored — url is not a parseable http(s) origin");
        return false;
    };
    let device_id = entry.device_id.clone().unwrap_or_else(device_id_persistent);
    let client = JfClient::new(origin, device_id);
    if let Some(token) = entry.token.as_deref() {
        match client.adopt_token(token) {
            Ok(user) => {
                crate::log(&format!(
                    "jellyfin: signed in as {} — stored token",
                    user.user_name
                ));
                super::install(client);
                return true;
            }
            Err(AuthError::Unauthorized) => {
                crate::log("jellyfin: stored token REFUSED (401) — falling back to the password");
            }
            Err(AuthError::Unreachable) => {
                crate::log("jellyfin: server unreachable at boot — staying at the gate");
                return false;
            }
        }
    }
    let Some(password) = entry.password.as_deref() else {
        crate::log("jellyfin: no usable token and no password — staying at the gate");
        return false;
    };
    match client.authenticate_by_name(&entry.user, password) {
        Ok(ok) => {
            crate::log(&format!(
                "jellyfin: signed in as {} — server {}",
                ok.user_name, ok.server_id
            ));
            super::install(client);
            remember_token(entry, &ok.token);
            true
        }
        Err(AuthError::Unauthorized) => {
            crate::log("jellyfin: login REFUSED (401) — check user/password in the config");
            false
        }
        Err(AuthError::Unreachable) => {
            crate::log("jellyfin: server unreachable at boot — staying at the gate");
            false
        }
    }
}

/// Replace a persisted entry's password with the token just minted for it — the step that takes
/// the plaintext off the television. Only the PERSISTENT file is touched: a `/tmp` dev trigger is
/// the harness's business and must not invent a config beside the session. A miss (no persistent
/// list, or no entry for this url and user) is silent: the boot that called this has already
/// signed in, and the only cost is asking again next time.
fn remember_token(entry: &ServerEntry, token: &str) {
    let Some(mut list) = load_servers_persistent() else {
        return;
    };
    let Some(slot) = list
        .servers
        .iter_mut()
        .find(|s| s.url == entry.url && s.user == entry.user)
    else {
        return;
    };
    if slot.token.as_deref() == Some(token) && slot.password.is_none() {
        return;
    }
    slot.token = Some(token.to_string());
    slot.password = None;
    save_servers(&list.servers, list.active);
}

/// Read the configured server list, in the same order the boot gate searches: the dev trigger
/// first, then the persistent file beside the session.
pub(crate) fn load_servers() -> Option<ServerList> {
    // 1. the dev trigger
    if let Some(raw) = crate::dev::read("jellyfin") {
        // Not the literal path: the tmppath gate keeps `/tmp/butaca-` spellings inside dev.rs,
        // and `dev::read` already owns that path. The label only feeds the log line.
        return parse_config(&raw, "dev trigger (jellyfin)");
    }
    // 2. the persistent file beside the session
    load_servers_persistent()
}

/// The persistent file only (the `/tmp` trigger is the harness's business, not the form's nor the
/// picker's).
fn load_servers_persistent() -> Option<ServerList> {
    let path = persistent_config_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    parse_config(&raw, &path.display().to_string())
}

/// Parse either the list shape or the flat legacy shape into a [`ServerList`], validating that
/// every entry names a url and a user.
fn parse_config(raw: &str, where_from: &str) -> Option<ServerList> {
    let list = match serde_json::from_str::<ConfigFile>(raw) {
        Ok(ConfigFile::List(list)) => list,
        Ok(ConfigFile::Flat(c)) => ServerList {
            servers: vec![ServerEntry {
                url: c.url,
                user: c.user,
                password: c.password,
                token: c.token,
                device_id: c.device_id,
            }],
            active: 0,
        },
        Err(e) => {
            crate::log(&format!("jellyfin: {where_from} ignored — not valid JSON: {e}"));
            return None;
        }
    };
    if list
        .servers
        .iter()
        .all(|c| !c.url.is_empty() && !c.user.is_empty())
    {
        Some(list)
    } else {
        crate::log(&format!(
            "jellyfin: {where_from} ignored — url and user must be non-empty"
        ));
        None
    }
}

/// The active server, if the configured list names one.
pub(crate) fn active_server() -> Option<ServerEntry> {
    let list = load_servers()?;
    list.servers.get(list.active).cloned()
}

/// Write the server list the boot arm reads. Entries carry tokens, not passwords: the file is a
/// token store, and [`ServerEntry::password`] is `None` on everything this writes. 0600 like every
/// secret this install keeps; a failed write is reported, not retried — the session still runs, the
/// next boot simply asks again.
pub(crate) fn save_servers(servers: &[ServerEntry], active: usize) -> bool {
    let body = serde_json::json!({
        "servers": servers,
        "active": active,
    });
    let path = persistent_config_path();
    let ok = std::fs::write(&path, body.to_string()).is_ok();
    if ok {
        // Best-effort 0600 — the umask usually beat us to it; a refusal is untidy, not fatal.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        crate::log("jellyfin: config saved — the next boot signs in by itself");
    } else {
        crate::log("jellyfin: config NOT saved — the next boot will ask again");
    }
    ok
}

/// Write the config the boot arm reads, after the sign-in form's first successful auth — a fresh
/// single server REPLACES the whole list and becomes active. What it keeps is the token the server
/// just minted: the password the user typed is not written anywhere.
pub(crate) fn save_config(url: &str, user: &str, token: &str, device_id: &str) -> bool {
    save_servers(
        &[ServerEntry {
            url: url.to_string(),
            user: user.to_string(),
            password: None,
            token: Some(token.to_string()),
            device_id: Some(device_id.to_string()),
        }],
        0,
    )
}

/// Append a newly signed-in server to the list and make it active (the Settings "Conexión rápida"
/// half of [`save_config`], which replaces instead).
pub(crate) fn append_server(url: &str, user: &str, token: &str, device_id: &str) -> bool {
    let mut list = load_servers().unwrap_or(ServerList {
        servers: Vec::new(),
        active: 0,
    });
    append_to(
        &mut list,
        ServerEntry {
            url: url.to_string(),
            user: user.to_string(),
            password: None,
            token: Some(token.to_string()),
            device_id: Some(device_id.to_string()),
        },
    );
    save_servers(&list.servers, list.active)
}

/// Remove the active server from the list and return what remains (the first survivor becomes the
/// new active pointer). The caller decides whether to [`save_servers`] the remainder or
/// [`erase_config`] an empty list.
pub(crate) fn remove_active() -> Option<ServerList> {
    let mut list = load_servers()?;
    remove_active_from(&mut list);
    Some(list)
}

/// The pure append step: push an entry and point `active` at it.
fn append_to(list: &mut ServerList, entry: ServerEntry) {
    list.servers.push(entry);
    list.active = list.servers.len() - 1;
}

/// The pure disconnect step: retire the active entry, the first survivor becomes the pointer.
fn remove_active_from(list: &mut ServerList) {
    if list.active < list.servers.len() {
        list.servers.remove(list.active);
    }
    list.active = 0;
}

/// Drop the persisted config — the sign-out path's half of [`save_config`], so a television that
/// signs out asks for a server again instead of silently returning to the last one.
pub(crate) fn erase_config() {
    if std::fs::remove_file(persistent_config_path()).is_ok() {
        crate::log("jellyfin: config erased");
    }
}

/// The persisted config as the sign-in form's PREFILL — `(url, user, password)`. A boot that
/// reached the form despite a config on disk means the token no longer stands (or the server was
/// gone); the password half is empty for every file this build wrote, and the form hands it back
/// as an empty field rather than as a secret it does not have. Read from the persistent file only:
/// the `/tmp` trigger is the harness's business, not the form's.
pub(crate) fn stored_config() -> Option<(String, String, String)> {
    let list = load_servers_persistent()?;
    let cfg = list.servers.get(list.active)?;
    Some((
        cfg.url.clone(),
        cfg.user.clone(),
        cfg.password.clone().unwrap_or_default(),
    ))
}

/// `/media/developer/<app id>-jellyfin.json` — one of the locations
/// [`crate::paths::session_candidates`] already established as persistent and writable for this
/// install, so the config and the session age exactly alike. A steerable (hostsim) build answers
/// inside its instance root instead, for that function's own reason: concurrent simulators must
/// not share one sign-in, and `/media/developer` is not writable off-device anyway.
pub(crate) fn persistent_config_path() -> std::path::PathBuf {
    if crate::paths::ENV_STEERABLE {
        return crate::paths::in_runtime_dir("jellyfin.json");
    }
    std::path::PathBuf::from(format!(
        "/media/developer/{}-jellyfin.json",
        crate::paths::app_id()
    ))
}

/// Read the persisted device id, or mint and persist one. A failed write degrades to a per-boot
/// random id — the server grows a device entry per boot, which is untidy, not broken.
pub(crate) fn device_id_persistent() -> String {
    // Same steering as `persistent_config_path`: off-device (the simulator) the id lives in the
    // runtime dir, because /media/developer exists only on a television.
    let path = if crate::paths::ENV_STEERABLE {
        crate::paths::in_runtime_dir("jellyfin-device-id")
    } else {
        std::path::PathBuf::from(format!(
            "/media/developer/{}-jellyfin-device-id",
            crate::paths::app_id()
        ))
    };
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return existing;
        }
    }
    let minted = mint_device_id();
    if std::fs::write(&path, &minted).is_err() {
        crate::log("jellyfin: device id not persistable here — a fresh one each boot");
    }
    minted
}

/// 16 bytes of `/dev/urandom` as hex — the same mint the Plex session applies to its client id.
fn mint_device_id() -> String {
    let mut b = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .is_ok();
    if !filled {
        // urandom does not fail on any Linux this runs on; if it somehow did, fall back to
        // time-derived bytes rather than a fixed string (a fixed id would collide installs).
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("plx-jf-{n:x}");
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_must_name_a_url_and_a_user() {
        assert!(parse_config(r#"{"url":"http://10.0.0.2:8096","user":"a","password":""}"#, "t")
            .is_some());
        assert!(parse_config(r#"{"url":"","user":"a","password":""}"#, "t").is_none());
        assert!(parse_config(r#"{"url":"http://10.0.0.2","user":"","password":""}"#, "t").is_none());
        assert!(parse_config("not json", "t").is_none());
    }

    fn entry(url: &str, user: &str) -> ServerEntry {
        ServerEntry {
            url: url.to_string(),
            user: user.to_string(),
            password: None,
            token: Some("tok".to_string()),
            device_id: Some("dev".to_string()),
        }
    }

    /// A saved entry carries the token and NOT the password: the wire shape is the proof, since
    /// this is the string that lands in the file.
    #[test]
    fn a_saved_entry_serializes_the_token_and_no_password() {
        let wire = serde_json::to_string(&entry("http://10.0.0.2:8096", "a")).unwrap();
        assert!(wire.contains("\"token\":\"tok\""), "wire shape: {wire}");
        assert!(
            !wire.contains("password"),
            "the password must not reach the file: {wire}"
        );
    }

    /// An install that upgrades from a password-era file still signs in: the field is read, and it
    /// is the only thing that can be read — such a file has no token.
    #[test]
    fn a_password_era_entry_still_reads_its_password() {
        let list = parse_config(
            r#"{"servers":[{"url":"http://10.0.0.2:8096","user":"a","password":"pw"}],"active":0}"#,
            "t",
        )
        .expect("a password-era list is a valid config");
        assert_eq!(list.servers[0].password.as_deref(), Some("pw"));
        assert_eq!(list.servers[0].token, None);
    }

    /// An entry with neither credential parses (the picker may hold one mid-edit) and the boot arm
    /// answers "no usable token and no password" instead of dereferencing an absent secret.
    #[test]
    fn an_entry_may_carry_no_credential_at_all() {
        let list = parse_config(
            r#"{"servers":[{"url":"http://10.0.0.2:8096","user":"a"}],"active":0}"#,
            "t",
        )
        .expect("url and user are the only required fields");
        assert_eq!(list.servers[0].password, None);
        assert_eq!(list.servers[0].token, None);
    }

    /// A flat legacy file reads as a one-entry list with that entry active.
    #[test]
    fn a_flat_legacy_file_reads_as_a_one_entry_list() {
        let list = parse_config(
            r#"{"url":"http://10.0.0.2:8096","user":"gleb","password":"x","device_id":"d"}"#,
            "t",
        )
        .expect("a flat file is a valid single-server config");
        assert_eq!(list.servers.len(), 1);
        assert_eq!(list.active, 0);
        assert_eq!(list.servers[0].url, "http://10.0.0.2:8096");
        assert_eq!(list.servers[0].user, "gleb");
    }

    /// A list file picks the server its `active` index names.
    #[test]
    fn a_list_file_picks_the_active_server() {
        let list = parse_config(
            r#"{"servers":[
                {"url":"http://10.0.0.2:8096","user":"a","password":"x"},
                {"url":"http://10.0.0.3:8096","user":"b","password":"y"}
            ],"active":1}"#,
            "t",
        )
        .expect("a list is a valid config");
        assert_eq!(list.servers.len(), 2);
        assert_eq!(list.active, 1);
        assert_eq!(list.servers[list.active].user, "b");
    }

    /// The whole list round-trips through the wire shape: serialize then parse.
    #[test]
    fn the_server_list_round_trips_through_the_wire_shape() {
        let list = ServerList {
            servers: vec![entry("http://10.0.0.2:8096", "a"), entry("http://10.0.0.3:8096", "b")],
            active: 1,
        };
        let wire = serde_json::to_string(&list).unwrap();
        let back = parse_config(&wire, "t").expect("a serialized list parses back");
        assert_eq!(back, list);
    }

    /// `append_server` adds a server to the list and makes it active.
    #[test]
    fn append_activates_the_new_server() {
        let mut list = ServerList {
            servers: vec![entry("http://10.0.0.2:8096", "a")],
            active: 0,
        };
        append_to(&mut list, entry("http://10.0.0.3:8096", "b"));
        assert_eq!(list.servers.len(), 2);
        assert_eq!(list.active, 1, "the appended server becomes active");
        assert_eq!(list.servers[list.active].user, "b");
    }

    /// Disconnecting the active server leaves the others configured, the first survivor active.
    #[test]
    fn disconnect_of_active_leaves_the_others() {
        let mut list = ServerList {
            servers: vec![
                entry("http://10.0.0.2:8096", "a"),
                entry("http://10.0.0.3:8096", "b"),
                entry("http://10.0.0.4:8096", "c"),
            ],
            active: 1,
        };
        remove_active_from(&mut list);
        assert_eq!(list.servers.len(), 2, "the active server is gone, the others remain");
        assert_eq!(list.servers[0].user, "a");
        assert_eq!(list.servers[1].user, "c");
        assert_eq!(list.active, 0, "the first survivor becomes the active pointer");
    }
}
