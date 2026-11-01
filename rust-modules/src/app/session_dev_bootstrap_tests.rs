//! Production boot capture + Bridge authority tests, with synthetic inputs only.
use super::*;

#[test]
fn dev_boot_capture_cannot_keep_saved_account_as_worker_authority() {
    use crate::plex::session::{Session, ServerRef, UserRef};
    let saved = Session { client_id: "synthetic-client".into(), account_token: "synthetic-account-a".into(),
        user: UserRef { uuid: "synthetic-a".into(), token: "synthetic-profile-a".into(), ..Default::default() },
        ..Default::default() };
    let dev = ServerRef { address: "127.0.0.2".into(), port: 32400,
        origin_url: "http://127.0.0.2:32400".into(), token: "synthetic-pms-b".into(), ..Default::default() };
    // This is the same capture function actual boot uses before creating its sole Bridge.
    let init = super::super::boot::captured_session_for_boot(saved, Some(dev), Vec::new());
    let rig = Bridge::for_session_test(init);
    let state = rig.session.snapshot_init();
    assert!(state.persisted.account_token.is_empty(), "dev B must not inherit saved A's worker authority");
    assert!(state.persisted.user.token.is_empty());
    assert!(state.committed_credentials.account_token.is_empty(), "BACK must not recover A");
}
