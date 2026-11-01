//! Production boot capture + Bridge authority tests, with synthetic inputs only.
use super::*;

fn frame(rig: &mut Bridge, d: &mut Dispatcher<AppHost>) {
    d.frame_with(rig, Tick::default(), Vec::new(), Vec::new(), &mut NoTap, false);
    rig.settle_session_io_for_test(d);
}

fn dev_fixture() -> Bridge {
    let saved = crate::plex::session::Session { client_id: "synthetic-device".into(),
        account_token: "synthetic-saved-a".into(), ..Default::default() };
    let dev = crate::plex::session::ServerRef { address: "127.0.0.2".into(), port: 32400,
        token: "synthetic-dev-b".into(), ..Default::default() };
    let mut rig = Bridge::for_session_test(super::super::boot::captured_session_for_boot(saved.clone(), Some(dev), Vec::new()));
    // Explicit resource baseline is the actual saved A, not sanitized in-memory auth input.
    rig.session_adapter.fixture_resources().disk = saved;
    rig
}

#[test]
fn carried_dev_ready_is_not_handed_off_after_erase() {
    let mut witnessed = false;
    for remaining in 1..16 {
        let mut rig = dev_fixture();
        let mut d = Dispatcher::<AppHost>::new();
        for _ in 0..crate::ui::dispatch::MAX_STEPS_PRE + crate::ui::dispatch::MAX_STEPS_POST - remaining {
            execute_session_command(&mut d, crate::auth::SessionCmd::NoteDeleteLeftovers(0));
        }
        execute_session_command(&mut d, crate::auth::SessionCmd::ActivateDevBootstrap);
        frame(&mut rig, &mut d);
        if rig.auth_read().0.phase != crate::auth::Phase::Ready || rig.session_ready.is_some() { continue; }
        witnessed = true;
        execute_session_command(&mut d, crate::auth::SessionCmd::EraseLocal);
        frame(&mut rig, &mut d);
        assert_eq!(rig.auth_read().0.phase, crate::auth::Phase::Deleted);
        assert!(rig.take_session_ready().is_none());
        assert!(matches!(rig.session_adapter.fixture_resources().registry_writes.last(),
            Some(crate::auth::owner::RegistryPlan::Revoke)));
        break;
    }
    assert!(witnessed, "actual Ready effect must be carried after activation ACK");
}

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
