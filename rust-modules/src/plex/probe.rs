//! Address and probe vocabulary that outlived the plex.tv discovery engine: [`Location`]
//! tiers an address (local, remote, relay), [`Outcome`] grades one identity-probe answer, and
//! [`configured_tier`] answers "local or not" for a boot that has no plex.tv connection list to
//! read a flag from. Pure policy, no I/O. The candidate ranking engine itself (`Candidate`,
//! `candidates`, `plan`) went out with the plex.tv account surface; [`PlaintextEligibility`]
//! stays as `grant`'s closed reason vocabulary.
pub use crate::origin::dial_port;
use serde::{Deserialize, Serialize};

/// Where an address sits relative to us. The ranking axis every Plex client agrees on, ordered
/// best-first by declaration so the derived `Ord` *is* the preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Location {
    /// Same LAN — no internet needed, and the only tier that survives the WAN going away.
    Local,
    /// Reached over the internet, directly to the owner's address.
    Remote,
    /// Plex's relay tunnel: ~2 Mbit/s, server-side transcode to fit. Last resort.
    Relay,
}


/// How a probe of one candidate ended. Spelled out here because the distinction the caller must not
/// collapse is structural, not incidental: only [`Unreachable`](Self::Unreachable) and
/// [`WrongServer`](Self::WrongServer) mean "try the next address".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Answered, and the `machineIdentifier` matched [`ProbePlan::machine_id`].
    Reachable,
    /// Answered, but as a different machine. Discard the candidate; do not retry it.
    WrongServer,
    /// 401 — an authorization/access-policy answer, not silence. The live coordinator still lets
    /// parallel origins and the relay fallback settle; this becomes the final reason only if none
    /// of them verifies the requested machine.
    Unauthorized,
    /// No answer: refused, timed out, or unresolvable. The only outcome the next candidate can fix.
    Unreachable,
    /// The whole-server aggregate for [`crate::auth::Reach::InsecureOnly`] (issue #95, plan §4):
    /// verified, provably the right server, but only over a transport this build may not put a
    /// credential on without a consented grant (`super::grant`). A single per-candidate probe never classifies to this — [`classify`] has no
    /// arm that produces it — it exists for the coordinator's SETTLED, whole-server verdict, which
    /// is a different question from "what did this one response say". Kept apart from
    /// [`Self::Unreachable`] because the remedy and the words are both different: the server
    /// answered, so telling the user it did not sends them to look at a router for nothing.
    InsecureOnly,
}


/// The tier of an origin that arrived WITHOUT a plex.tv connection list to read a flag off — the
/// `butaca-token` boot, whose host and port are compiled into the C shim. Before this existed
/// that path passed `None`, which is "nothing has said" and is the one answer that is certainly
/// wrong: it left [`Client::link`](super::client::Client::link) unknown, so `abr::bootstrap` never
/// ran and **Auto could not choose Original on a gigabit LAN** — on the exact boot path the device
/// harness and `tools/tv-session.sh` use, i.e. every automated run.
///
/// It is the same rule [`tier`] already applies, not a new one: `Connection.local` is itself an
/// RFC1918 address-shape test (see this module's note and `plex/CLAUDE.md`), so answering it here
/// from the address is answering it the way plex.tv would have. `Relay` is unreachable by
/// construction — a relay is a plex.tv-brokered tunnel, and nobody hand-configures one.
///
/// **Address shape is weaker evidence than a probe, and deliberately so.** A private literal
/// reached through a VPN is not on this LAN, and this will call it `Local` anyway. That is
/// affordable exactly here: a wrong `Local` starts Original, and `OriginalModeController` watches
/// it — measuring 750 ms windows and leaving on the starvation horizon once a window has actually
/// observed the reserve draining. A wrong `Remote` costs one bounded startup probe. Neither can
/// strand a playback, which is why this guesses rather than blocking the boot on a measurement.
///
/// **This paragraph was FALSE from the day it was written until 2026-08-27, in both halves.**
/// `route::auto_original_watch` required `Location::Remote`, so on a `Local` link that controller
/// was never constructed at all and a LAN which did not carry the source ran the film at 8-25 %
/// of real time forever, with no `abr:` line in the log
/// (`docs/measurements/local-original-blind.md`). And the horizon it names fired on the FIRST
/// window, where the reserve is the prime remnant and `conservative_kbps` is pinned to half the
/// measurement by the uncertainty floor — which cost a real playback its 4K Dolby Vision
/// (`docs/measurements/orig-first-window-fallback.md`). Both are fixed; the wording above is now
/// the behaviour rather than the intention.
pub fn configured_tier(host: &str) -> Location {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(v4) = h.parse::<std::net::Ipv4Addr>() {
        return if v4.is_private() || v4.is_loopback() || v4.is_link_local() {
            Location::Local
        } else {
            Location::Remote
        };
    }
    if let Ok(v6) = h.parse::<std::net::Ipv6Addr>() {
        let [first, ..] = v6.segments();
        // Hand-rolled rather than `is_unique_local`/`is_unicast_link_local`, which are still
        // unstable: ULA is `fc00::/7`, link-local is `fe80::/10`.
        let ula = first & 0xfe00 == 0xfc00;
        let link_local = first & 0xffc0 == 0xfe80;
        return if v6.is_loopback() || ula || link_local {
            Location::Local
        } else {
            Location::Remote
        };
    }
    // A name, not a literal. `.local` is mDNS (RFC 6762), which is link-scoped by definition of the
    // protocol; `localhost` is reserved to the loopback (RFC 6761). Anything else is a name this
    // function cannot resolve without a socket, and `Remote` is the answer that pays for a probe
    // instead of assuming.
    let lower = h.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".localhost") {
        Location::Local
    } else {
        Location::Remote
    }
}

/// **May the person be OFFERED a plaintext connection to this server** — the closed answer of
/// the same-network plaintext rule, and its reason when the answer is no. The order of
/// the variants after `Eligible` is the order the rule checks them in, so the reason reported is
/// the first condition that failed. Every value is a telemetry code (`plaintext_eligibility`).
///
/// Eligibility is necessary, never sufficient: a credential still goes to a plaintext origin only
/// under a live `grant::PlaintextGrant`, which is minted from an eligible verdict AND the
/// person's recorded consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PlaintextEligibility {
    /// Every condition holds: the person may be asked.
    Eligible,
    /// The server requires secure connections (`httpsRequired`). Its owner's decision; never
    /// overridden.
    HttpsRequired,
    /// The plaintext connection is not one plex.tv marked `local` — remote or relay. Never offered:
    /// that is the owner's credential crossing the internet unencrypted.
    NotLocal,
    /// plex.tv did not see this television behind the server's own public address
    /// (`publicAddressMatches == false`), so "the same network" is unproven.
    NotSameNetwork,
    /// The host the credential would travel to is not a numeric private literal (RFC 1918,
    /// 169.254/16, IPv6 ULA or link-local) — a hostname, a public or a loopback address.
    NotPrivateAddress,
    /// The tokenless `/identity` did not verify the resource's `machineIdentifier`.
    IdentityUnverified,
    /// Some HTTPS route to the server (the relay included) was advertised but never settled.
    HttpsUnsettled,
    /// Some HTTPS route ANSWERED — verified, refused (401, or another 4xx such as 403), or failed
    /// with any other status (a 5xx, anything non-2xx). The server is up over HTTPS from here, and
    /// no answer there — least of all a refusal — is permission to downgrade.
    HttpsAnswered,
}

impl PlaintextEligibility {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::HttpsRequired => "https_required",
            Self::NotLocal => "not_local",
            Self::NotSameNetwork => "not_same_network",
            Self::NotPrivateAddress => "not_private_address",
            Self::IdentityUnverified => "identity_unverified",
            Self::HttpsUnsettled => "https_unsettled",
            Self::HttpsAnswered => "https_answered",
        }
    }
}
