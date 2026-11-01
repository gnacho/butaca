//! Shared fixtures and helpers for the `auth` test modules split out below.

use super::*;
pub(super) use crate::origin::Scheme;

pub(super) fn resource(json: &str) -> Resource {
    serde_json::from_str(json).expect("fixture parses")
}

/// A share with FOUR advertised addresses, which between them cover every case the probe loop
/// has to get right: the owner's LAN address (policy keeps only its TLS URI), a hostname the
/// transport resolves, and two public IPv4s so "the first one answered as somebody else" has a
/// second one to fall through to. Shaped on the live capture of 2026-08-11
/// (`docs/shared-servers.md` §2); the addresses are stand-ins, the arrangement is not.
pub(super) fn identity_json(mid: &str) -> Vec<u8> {
    format!(
        r#"{{"MediaContainer":{{"size":0,"machineIdentifier":"{mid}","version":"1.43.3"}}}}"#
    )
    .into_bytes()
}

/// A recording dial. Returns whatever the script says for an address, and remembers the order
/// it was asked — which is how "it stopped" and "it never tried that one" become assertions.
pub(super) fn race_plan() -> ProbePlan {
    let candidate = |url: &str, address: &str, location: probe::Location| {
        let scheme = if url.starts_with("https://") {
            Scheme::Https
        } else {
            Scheme::Http
        };
        Candidate {
            url: url.into(),
            scheme,
            location,
            address: address.into(),
            port: 32400,
            ipv6: false,
            credential_eligible: scheme == Scheme::Https,
        }
    };
    ProbePlan {
        machine_id: "race-machine".into(),
        token: "race-token".into(),
        owned: true,
        name: "race-server".into(),
        source_title: None,
        candidates: vec![
            candidate(
                "https://192-0-2-10.h.plex.direct:32400",
                "192.0.2.10",
                probe::Location::Local,
            ),
            candidate(
                "https://203-0-113-9.h.plex.direct:32400",
                "203.0.113.9",
                probe::Location::Remote,
            ),
        ],
        policy: CredentialPolicy::HttpsOnly,
    }
}

/// A [`ProbeDial`] scripted in the legacy `(status, body)` shape — status `0` is "nothing
/// answered", with no transport evidence (`ProbeReply::from`). For the racing fixtures that are
/// about completion order and acceptance, not about how a failure is named.
pub(super) fn status_dial(
    f: impl Fn(&Origin, Option<&crate::origin::ResolvePin>, Duration) -> (i32, Vec<u8>) + Send + Sync + 'static,
) -> ProbeDial {
    Arc::new(move |origin, pin, budget| ProbeReply::from(f(origin, pin, budget)))
}

pub(super) fn test_policy() -> ProbeDeadlines {
    ProbeDeadlines {
        local: Duration::from_secs(1),
        remote: Duration::from_secs(1),
    }
}

pub(super) fn threaded_spawn(_: usize, job: ProbeJob) -> bool {
    std::thread::spawn(job);
    true
}

/// The account this feature exists for, as `/api/v2/resources` really returns it: OUR server
/// (owned, LAN + public + relay) and the SHARE (not owned, the owner's 172.20 LAN, an internal
/// hostname, and one public IPv4). Shaped on the live capture of 2026-08-11
/// (`docs/shared-servers.md` §2) — the addresses are stand-ins, the arrangement is not, and the
/// share is listed FIRST because plex.tv's order is not ours to rely on.
pub(super) fn source(machine_id: &str, owned: bool, token: &str) -> SourceRef {
    SourceRef {
        machine_id: machine_id.into(),
        name: machine_id.into(),
        shared_by: if owned {
            String::new()
        } else {
            "friend".into()
        },
        owned,
        address: "10.0.0.1".into(),
        port: 32400,
        token: token.into(),
        ..Default::default()
    }
}

/// The primary in the **LEGACY shape** — see [`source`] above.
pub(super) fn primary(machine_id: &str, address: &str, port: i64, token: &str) -> ServerRef {
    ServerRef {
        name: "Mac mini".into(),
        machine_id: machine_id.into(),
        address: address.into(),
        port,
        token: token.into(),
        ..Default::default()
    }
}

