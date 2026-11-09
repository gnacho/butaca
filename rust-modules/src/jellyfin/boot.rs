//! boot — the Jellyfin flavor's boot gate arm: config file → authenticated client → installed.
//!
//! There is no account service on this backend, so "signing in" is reading a small JSON config the
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
/// covers every other outcome: no config (the common case on a fresh install, and the reason this
/// fn is cheap: the first read misses and nothing else runs), a config that will not parse, an
/// unreachable server, and wrong credentials, each logged on its own terms.
///
/// **The round trip is BOUNDED and runs on a worker.** It used to run inline on the boot thread,
/// before the event loop exists: one LAN request was "the honest price of knowing whether Home can
/// fill", but nothing bounded it. `net::API` bounds libcurl's connect (8 s) and whole transfer
/// (25 s) and NOT the name lookup - with `NOSIGNAL` and a synchronous resolver a lookup can outlive
/// both - so a television whose server name no longer resolves could sit in `getaddrinfo` with no
/// deadline at all. That is a process with no event loop: the webOS splash (`splashBackground`)
/// stays up because no frame is ever presented, and the close request SAM sends is never read
/// either, which is exactly issue #400's "stuck on loading screen, impossible even to force-stop".
///
/// So the probe answers Home only if it can answer inside [`BOOT_PROBE_BUDGET`]; otherwise the app
/// comes up at the sign-in gate, where the address and user are prefilled and every key works. The
/// client is installed HERE, on the caller's thread, from what the worker hands back - a late
/// answer is dropped with the channel instead of installing a server the boot already abandoned.
pub(crate) fn try_boot() -> bool {
    let Some(cfg) = active_server() else {
        return false;
    };
    match within(BOOT_PROBE_BUDGET, "jf-boot-probe", move || authenticate(&cfg)) {
        Some(client) => {
            super::install(client);
            true
        }
        None => {
            crate::log("jellyfin: boot probe did not answer in time - starting at the sign-in gate");
            false
        }
    }
}

/// How long the boot gate waits for its one LAN round trip. Long enough for a slow-but-alive
/// server on a home LAN (a healthy one answers in tens of milliseconds), short enough that a dead
/// address costs a glance at the splash rather than a screen nobody can leave.
const BOOT_PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// Run one blocking round trip on a worker and wait at most `budget` for its answer.
///
/// `None` means the work did not answer in time (or the OS refused the thread), and the caller
/// must proceed as though it had failed. The worker is NOT joined: a probe that is still out when
/// the budget expires is abandoned, and its answer dies with the channel.
fn within<T: Send + 'static>(
    budget: std::time::Duration,
    what: &str,
    work: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = crate::task::spawn_small(what, move || {
        if let Some(value) = work() {
            let _ = tx.send(value);
        }
    });
    if !spawned {
        return None;
    }
    rx.recv_timeout(budget).ok()
}

/// Sign in against one configured server: adopt the token the file carries (or, for a
/// password-era file, mint one with the password first), and install the client on success. This
/// is the one place the boot gate and the Settings "switch server" action share — a switch is a
/// boot against a different entry of the same list.
pub(crate) fn authenticate_server(entry: &ServerEntry) -> bool {
    match authenticate(entry) {
        Some(client) => {
            super::install(client);
            true
        }
        None => false,
    }
}

/// The network half of [`authenticate_server`], with no install: it either signs in and hands the
/// client back, or explains itself and answers `None`. Split out because the two callers that run
/// it OFF the main thread (the boot probe above and the server picker's switch) have to decide
/// what to do with the client on the frame thread, and because a unit test can drive this half
/// without the global.
fn authenticate(entry: &ServerEntry) -> Option<JfClient> {
    let Some(origin) = crate::origin::Origin::parse(&entry.url) else {
        crate::log("jellyfin: config ignored — url is not a parseable http(s) origin");
        return None;
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
                return Some(client);
            }
            Err(AuthError::Unauthorized) => {
                crate::log("jellyfin: stored token REFUSED (401) — falling back to the password");
            }
            Err(AuthError::Unreachable) => {
                crate::log("jellyfin: server unreachable at boot — staying at the gate");
                return None;
            }
        }
    }
    let Some(password) = entry.password.as_deref() else {
        crate::log("jellyfin: no usable token and no password — staying at the gate");
        return None;
    };
    match client.authenticate_by_name(&entry.user, password) {
        Ok(ok) => {
            crate::log(&format!(
                "jellyfin: signed in as {} — server {}",
                ok.user_name, ok.server_id
            ));
            remember_token(entry, &ok.token);
            Some(client)
        }
        Err(AuthError::Unauthorized) => {
            crate::log("jellyfin: login REFUSED (401) — check user/password in the config");
            None
        }
        Err(AuthError::Unreachable) => {
            crate::log("jellyfin: server unreachable at boot — staying at the gate");
            None
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
    // Created 0600 rather than chmodded after: this file holds an access token, and the window
    // between a 0644 create and the chmod is a window where another app can read it (#346).
    let ok = {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut f| f.write_all(body.to_string().as_bytes()))
            .is_ok()
    };
    if ok {
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
///
/// A list that will not load counts as a list with nothing left in it, never as "nothing to do".
/// The session can be live with no readable file behind it: a Developer Mode install whose jail
/// refuses writes to `/media/developer` signs in, logs "config NOT saved", and runs. When this
/// answered `None` there, Sign out returned early, left the client installed, and did nothing on
/// screen.
pub(crate) fn remove_active() -> ServerList {
    remaining_after_disconnect(load_servers())
}

/// The pure half of [`remove_active`]: what a disconnect leaves of whatever list was loaded.
fn remaining_after_disconnect(loaded: Option<ServerList>) -> ServerList {
    let Some(mut list) = loaded else {
        return ServerList { servers: Vec::new(), active: 0 };
    };
    remove_active_from(&mut list);
    list
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
    // One answer per process, chosen once. Read and write must never disagree about the file:
    // a read that looked somewhere the write did not is a sign-in that silently does not persist,
    // which is the very failure this ladder exists to end (#346).
    static CHOSEN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    CHOSEN
        .get_or_init(|| {
            let candidates = config_candidates();
            let path = pick_config_path(&candidates, |p| p.exists(), |p| dir_writable(p));
            if path != candidates[0] {
                crate::log(&format!(
                    "jellyfin: config lives at {} — the first candidate refuses writes on this jail",
                    path.display()
                ));
            }
            path
        })
        .clone()
}

/// **Where the config may live, in the order this install prefers.** The same ladder
/// [`crate::paths::session_candidates`] already established, with the config's own filename: the
/// two files age alike and a jail that refuses one refuses the other. `/tmp` is last for that
/// function's reason and no better one — a save that lands there is not durable.
fn config_candidates() -> Vec<std::path::PathBuf> {
    let id = crate::paths::app_id();
    let mut v = vec![
        std::path::PathBuf::from(format!("/media/developer/{id}-jellyfin.json")),
        std::path::PathBuf::from(format!("/media/internal/.{id}-jellyfin.json")),
        crate::paths::in_app_dir(&format!("{id}-jellyfin.json")),
    ];
    // `/tmp` last, for `session_candidates`'s reason: it is the one thing this jail class
    // guarantees writable, and a save that lands there is NOT durable.
    v.push(crate::paths::in_runtime_dir("jellyfin.json"));
    v
}

/// **The choice, as a pure function** — first candidate that already holds a file, else the first
/// one that accepts a write, else the first one (so a hopeless install keeps the shape it always
/// had and the failed write is reported, not hidden).
///
/// Existing-before-writable is the rule that makes a rescue stick: an install rescued onto
/// `/media/internal` because `/media/developer` refused the write must keep reading THAT file on
/// the next boot even if the jail later loosens, or the rescue silently costs a sign-in every time
/// the permission flips back.
fn pick_config_path(
    candidates: &[std::path::PathBuf],
    exists: impl Fn(&std::path::Path) -> bool,
    writable: impl Fn(&std::path::Path) -> bool,
) -> std::path::PathBuf {
    if let Some(found) = candidates.iter().find(|p| exists(p)) {
        return found.clone();
    }
    if let Some(ok) = candidates.iter().find(|p| writable(p)) {
        return ok.clone();
    }
    candidates[0].clone()
}

/// Can this process create a file in the candidate's directory? Probed, not assumed: the jail
/// profiles disagree about which of these accept writes, and a mode bit is not a guarantee.
fn dir_writable(path: &std::path::Path) -> bool {
    let Some(dir) = path.parent() else { return false };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".butaca-probe-{}", std::process::id()));
    let made = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
        .is_ok();
    let _ = std::fs::remove_file(&probe);
    made
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

/// 16 bytes of `/dev/urandom` as hex, the same mint the retired session applied to its client id.
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
        return format!("butaca-jf-{n:x}");
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The bound, not the network.** A boot probe that cannot answer inside its budget must
    /// answer `None` - the app then comes up at the sign-in gate instead of waiting on a round
    /// trip with no deadline (the shape of #400: the splash stays because no frame is presented
    /// and the process never reaches its event loop).
    #[test]
    fn a_probe_that_outlives_its_budget_answers_none() {
        let started = std::time::Instant::now();
        let answer = within(
            std::time::Duration::from_millis(60),
            "test-slow-probe",
            || {
                std::thread::sleep(std::time::Duration::from_millis(400));
                Some(7u32)
            },
        );
        assert_eq!(answer, None, "a late answer is dropped, not waited for");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(400),
            "and the caller was released at the budget, not at the worker's pace"
        );
    }

    /// A probe that answers inside the budget is handed over whole.
    #[test]
    fn a_probe_that_answers_in_time_is_returned() {
        assert_eq!(
            within(std::time::Duration::from_secs(5), "test-fast-probe", || Some(7u32)),
            Some(7)
        );
    }

    /// A probe that fails answers `None` at once rather than waiting out the budget.
    #[test]
    fn a_failed_probe_answers_none_without_waiting() {
        let started = std::time::Instant::now();
        assert_eq!(
            within(std::time::Duration::from_secs(5), "test-failed-probe", || None::<u32>),
            None
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

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

    /// Signed in with no readable config (a Developer Mode jail that refused the write) is still
    /// signed in, and signing out of it must land on the sign-in gate, not be a no-op.
    #[test]
    fn disconnect_without_a_loadable_list_leaves_nothing_configured() {
        let remaining = remaining_after_disconnect(None);
        assert!(
            remaining.servers.is_empty(),
            "no list on disk means no server survives the sign-out"
        );
        assert_eq!(remaining.active, 0);
        let mut one = ServerList { servers: vec![entry("http://10.0.0.2:8096", "a")], active: 0 };
        one = remaining_after_disconnect(Some(one));
        assert!(one.servers.is_empty(), "the only server signed out leaves none");
    }

    /// **#346: the config follows the same ladder the session already uses.** On a jail that
    /// refuses `/media/developer`, the old hardcoded path dropped the write and every launch was a
    /// fresh sign-in. These four cases are the whole rule.
    #[test]
    fn the_config_lands_on_the_first_place_that_can_hold_it() {
        use std::path::PathBuf;
        let c = |n: &str| PathBuf::from(n);
        let cands = [c("/media/developer/x.json"), c("/media/internal/x.json"), c("/tmp/x.json")];

        // Ordinary install: the first candidate takes the write.
        assert_eq!(
            pick_config_path(&cands, |_| false, |p| p == &cands[0]),
            cands[0]
        );

        // The reported jail: developer refuses, internal accepts. The config must land there
        // instead of being dropped.
        assert_eq!(
            pick_config_path(&cands, |_| false, |p| p == &cands[1]),
            cands[1]
        );

        // A rescue must STICK: the file is already on the fallback, so it keeps being read there
        // even if the jail later loosens and the first candidate becomes writable again. Reading
        // the first writable instead would silently cost a sign-in every time the bit flipped.
        assert_eq!(
            pick_config_path(&cands, |p| p == &cands[1], |_| true),
            cands[1]
        );

        // Nothing accepts a write: keep the shape the install always had, so the failure is the
        // same reported failure and not a surprise relocation.
        assert_eq!(pick_config_path(&cands, |_| false, |_| false), cands[0]);
    }

    /// The ladder is the session's, in the session's order: `/media/developer` first (today's
    /// ordinary install), then the internal volume, then the app dir, and `/tmp` last because a
    /// save that lands there does not survive a reboot.
    #[test]
    fn the_config_candidates_are_the_sessions_ladder() {
        let c = config_candidates();
        assert!(c.len() >= 3, "the ladder has to have somewhere to fall back to");
        assert!(c[0].to_string_lossy().starts_with("/media/developer/"));
        assert!(c[1].to_string_lossy().starts_with("/media/internal/"));
        assert!(
            c.last().is_some_and(|p| p.starts_with(crate::paths::runtime_dir())),
            "the runtime dir is the last resort, never the first"
        );
        assert!(
            c.iter().all(|p| p.to_string_lossy().ends_with("-jellyfin.json")
                || p.ends_with("jellyfin.json")),
            "every candidate names the same file"
        );
    }
}
