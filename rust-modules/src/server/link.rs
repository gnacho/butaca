//! The relay policy — what the LINK to a server allows a plan to ask for.
//!
//! Split out of the deleted `plex/transcoder.rs`: the policy itself is the only part the live
//! tree still reads (`route::playback_preview` and the routing tests). The wire half it used to
//! govern (`transcode_query`'s two flavors) is gone with the Plex transcoder.

use super::probe::Location;
use crate::playback::LinkPolicy;

/// **The relay policy**, and the only place it is spelled out. `None` = nobody has said how this
/// server is reached (see [`LinkPolicy::UNRESTRICTED`]).
///
/// Plex's relay is a tunnel the server holds open to a Plex relay host: a second https connection,
/// conventionally port 8443, **capped at 2 Mbit/s**, with the server transcoding down to fit
/// whatever the client asks for. [`probe`](super::probe) ranks it last, so it only ever wins when
/// there is nothing else — a share behind CGNAT, an owner with no port forward. When it does win,
/// the two fast paths this client is built around become the wrong answer, and both have to go:
///
/// * **No direct play.** Direct play streams the file's own bytes, and a library's own bytes are
///   tens of Mbit/s (the share measured on 2026-08-11: 31 Mbit/s)
///   down a 2 Mbit/s pipe. The server cannot rescue it, because nothing is being transcoded and
///   so there is nothing for it to fit; the cap arrives as a stall two minutes into the film,
///   with the AU queue draining and no error on any surface. Forcing the transcode branch turns
///   that into a `/decision` the server answers with something that fits.
/// * **No container REMUX either** — the half that is easy to miss, because a remux *feels* like
///   a concession already. It is not: a remux copies the codecs and deliberately sends **no
///   cap** (the deleted `transcode_query` omitted `videoResolution`/`maxVideoBitrate` on that
///   branch, because a cap is exactly what would force the re-encode it is trying to avoid). So
///   it ships the same bytes at the same rate one layer down, and stalls identically. Denying it
///   leaves the re-encode, which is the only flavor whose whole point is that the server picks
///   the rate.
///
/// **Not a parameter, deliberately — THIS policy sends no number at all.** A cap is meaningless on
/// direct play (no encoder is running to obey it) and on the re-encode branch the server already
/// receives an upper bound it is free to come in far under. The relay's real ceiling is known to
/// the server, which is on the other end of the tunnel and applies it; this policy's whole job is
/// to stop asking for the two flavors that leave it no way to.
///
/// A **user-chosen** ceiling (`route::Quality`) is a different input through the identical
/// mechanism: `route::quality_policy` returns this same [`LinkPolicy`], `route::flavors_allowed`
/// composes the two so the stricter wins per flavor. Nothing about the relay tier changed — it
/// still denies both flavors unconditionally and still names no rate.
///
/// **No relay connection has been observed on a device by this codebase.** Before discovery racing,
/// the old chooser discarded every relay. Relay is now a second-phase candidate, but the 2 Mbit/s
/// figure and port-8443 convention remain Plex documentation rather than our measurement, and this
/// policy is **unverified against a real relay**. What is
/// asserted is the shape, not the number: whatever the ceiling turns out to be, only the server can
/// apply it, and only if an encoder runs.
pub fn link_policy(link: Option<Location>) -> LinkPolicy {
    match link {
        Some(Location::Relay) => LinkPolicy {
            direct_play: false,
            remux: false,
        },
        // Exhaustive on purpose: a new tier must come here and say what it allows, rather than
        // inheriting "unrestricted" from a wildcard because nobody thought about it.
        Some(Location::Local) | Some(Location::Remote) | None => LinkPolicy::UNRESTRICTED,
    }
}

#[cfg(test)]
mod tests {
    use super::{link_policy, Location};
    use crate::playback::LinkPolicy;

    /// Relay denies BOTH flavors that put the file's own bytes on the wire. Direct play is the
    /// obvious one; the remux is the one a "force a transcode" instinct leaves behind, and it is
    /// no better — it copies the codecs and its query carries no cap by design, so it is the same
    /// 31 Mbit/s down the same 2 Mbit/s tunnel. What survives is the re-encode, where the server
    /// is the one choosing the rate.
    #[test]
    fn a_relay_link_forbids_both_flavors_that_ship_the_file_at_its_own_rate() {
        let p = link_policy(Some(Location::Relay));
        assert!(!p.direct_play, "a relay cannot carry the source file");
        assert!(
            !p.remux,
            "a remux is the same bytes at the same rate, one layer down"
        );
    }

    /// Every other tier — and a link nobody has described — must change nothing at all. This is
    /// the assertion that keeps the policy free for the single-server LAN install: a wrong answer
    /// here would be a wrong answer for everybody, on a code path they all take.
    #[test]
    fn every_other_tier_and_an_unknown_link_leave_playback_alone() {
        for l in [Some(Location::Local), Some(Location::Remote), None] {
            assert_eq!(
                link_policy(l),
                LinkPolicy::UNRESTRICTED,
                "link {l:?} must restrict nothing"
            );
        }
    }
}
