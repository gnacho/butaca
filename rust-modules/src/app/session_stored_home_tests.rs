//! Stored-home observation contracts through the real owner and actual disk/registry resources.
//! Synthetic admitted observations here do not claim account discovery/network policy coverage.
use super::*;
use crate::auth::owner::{SessionEnvelope, SessionEvent, SessionWork};
use crate::plex::session::{self, Session, SourceRef, ServerRef, UserRef};

struct Cleanup<'a>(&'a crate::task::MainThread);
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        crate::plex::reset_servers_for_test();
        session::ProfilePublisher::new(self.0).publish(None, 0);
    }
}

fn source(address: &str, token: &str) -> SourceRef {
    SourceRef { machine_id: "stored-machine".into(), name: "Synthetic".into(), address: address.into(),
        port: 32400, origin_url: format!("http://{address}:32400"), token: token.into(), owned: true,
        tier: Some(crate::plex::probe::Location::Local), ..Default::default() }
}

fn saved() -> Session {
    let source = source("127.0.0.1", "profile-token-a");
    Session { client_id: "synthetic-client".into(), account_token: "synthetic-account".into(),
        user: UserRef { uuid: "synthetic-profile".into(), token: source.token.clone(), ..Default::default() },
        server: ServerRef { machine_id: source.machine_id.clone(), address: source.address.clone(),
            port: source.port, origin_url: source.origin_url.clone(), token: source.token.clone(), ..Default::default() },
        sources: vec![source], ..Default::default() }
}

fn frame(rig: &mut Bridge, d: &mut Dispatcher<AppHost>, records: Vec<SessionEnvelope>) {
    let results = records.into_iter().map(|r| (r.addr, AppMsg::Session(SessionEvent::Result(r)))).collect();
    d.frame_with(rig, Tick::default(), Vec::new(), results, &mut NoTap, false);
        rig.settle_session_io_for_test(d);
}

fn command(rig: &mut Bridge, d: &mut Dispatcher<AppHost>, cmd: crate::auth::SessionCmd) {
    execute_session_command(d, cmd);
    frame(rig, d, Vec::new());
}

fn inject_roster(rig: &mut Bridge, expected: crate::auth::SessionIdentity) {
    let mut witnessed = false;
    for remaining in 1..20 {
        let disk = saved();
        let mut rig = Bridge::for_session_test(crate::auth::SessionInit::captured(disk.clone()));
        rig.session_adapter.fixture_resources().disk = disk.clone();
        let mut d = Dispatcher::<AppHost>::new();
        let mut wrong = disk.clone();
        wrong.account_token = "different-account".into();
        frame(&mut rig, &mut d, records.clone());
        let state = rig.session.snapshot_init();
        if state.pending_commit.is_none() || state.inbox.len() != 1 { continue; }
        witnessed = true;
        assert!(state.pending.contains_key(&req), "busy is not rejection or completion");
        assert!(records.iter().all(|r| rig.session_adapter.admitted(r)));
        let writes_before = rig.session_adapter.fixture_resources().registry_writes.len();
        frame(&mut rig, &mut d, Vec::new());
        assert!(!rig.session.snapshot_init().pending.contains_key(&req));
        assert!(rig.session.snapshot_init().pending_commit.is_none());
        assert!(rig.session.snapshot_init().inbox.is_empty());
        assert!(records.iter().all(|r| !rig.session_adapter.admitted(r)));
        let resources = rig.session_adapter.fixture_resources();
        assert!(resources.registry_writes.len() >= writes_before);
        assert_eq!(resources.disk.sources[0].address, "127.0.0.1");
        assert!(resources.registry_writes.iter().all(|plan|
            matches!(plan, crate::auth::owner::RegistryPlan::Probe(_))));
        assert!(rig.take_session_ready().is_none());
        break;
    }
    assert!(witnessed, "must carry a rejected terminal behind a real pending resource commit");
}
