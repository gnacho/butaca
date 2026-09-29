//! The Jellyfin source's place in the roster sync: it is not in the Plex `live` list (that
//! list is EMPTY in this flavour), so the retire predicate must keep it for exactly as long
//! as its client is installed. A bare `!live.contains(sid)` test retires it on the sync right
//! after adoption, resetting the table it had just joined — the TV symptom was no Movies/Shows
//! pills at all, because no section landing survived the next pump.

use super::*;

fn install_bare_jellyfin_client() {
    // No HTTP wanted: adoption reads only the client's EXISTENCE. Port 1 is never dialed.
    let client = crate::jellyfin::JfClient::new(
        crate::plex::Origin::http("127.0.0.1", 1),
        "dev-1".into(),
    );
    crate::jellyfin::install(client);
}

#[test]
fn an_unadopted_jellyfin_client_opens_the_discovery_gate() {
    let _guard = crate::testlock::serial();
    let mut state = BrowseState::default();
    // Align the session cursor so the gate's generation arm cannot answer for this test.
    state.session_generation = crate::plex::session::visible_generation();
    let adapter = BrowseAdapter::default();
    assert!(!state.discovery_needs_pump(&adapter),
        "empty roster and no client: nothing to do");

    install_bare_jellyfin_client();
    assert!(state.discovery_needs_pump(&adapter),
        "a client the table has not adopted yet is precisely the work the pump exists for");

    crate::jellyfin::uninstall();
}

#[test]
fn the_jellyfin_source_survives_the_sync_after_its_adoption() {
    let _guard = crate::testlock::serial();
    install_bare_jellyfin_client();
    let mut state = BrowseState::default();

    let first = state.sync_roster_owned();
    assert!(first.changed, "the adoption pass changes the table");
    assert!(state.sources.iter().any(|s| s.sid == crate::jellyfin::SERVER_ID));

    let second = state.sync_roster_owned();
    assert!(!second.retire_adapter,
        "the jellyfin source must not retire while its client is installed");
    assert!(state.sources.iter().any(|s| s.sid == crate::jellyfin::SERVER_ID),
        "the source is still adopted after a repeat sync");

    crate::jellyfin::uninstall();
}

#[test]
fn the_jellyfin_source_retires_once_its_client_is_gone() {
    let _guard = crate::testlock::serial();
    install_bare_jellyfin_client();
    let mut state = BrowseState::default();
    let _ = state.sync_roster_owned();
    assert!(state.sources.iter().any(|s| s.sid == crate::jellyfin::SERVER_ID));

    crate::jellyfin::uninstall();
    let sync = state.sync_roster_owned();
    assert!(sync.retire_adapter, "sign-out retires the source through the roster sync");
    assert!(!state.sources.iter().any(|s| s.sid == crate::jellyfin::SERVER_ID));
}
