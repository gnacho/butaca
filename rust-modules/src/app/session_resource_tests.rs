//! Serial resource-path evidence: real session file and native registry, not fixture mirrors.
//! Account/probe transport and auxiliary telemetry/cache/runtime sweeps remain injected.
#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::auth::owner::{SessionEnvelope, SessionEvent, SessionWork};
    use crate::auth::SessionCmd;
    use crate::plex::session::{self, ProfileCreds, ServerRef, Session, SourceRef, UserRef};

    struct ResourceCleanup<'a>(&'a crate::task::MainThread);
    impl Drop for ResourceCleanup<'_> {
        fn drop(&mut self) {
            crate::plex::reset_servers_for_test();
            // Publish the neutral test resource value, without allocating another profile scope.
            session::ProfilePublisher::new(self.0).publish(None, 0);
        }
    }

    fn stored() -> Session {
        let source = SourceRef {
            machine_id: "resource-server".into(),
            name: "Synthetic".into(),
            address: "127.0.0.1".into(),
            port: 32400,
            origin_url: "http://127.0.0.1:32400".into(),
            token: "synthetic-kid-token".into(),
            owned: true,
            ..Default::default()
        };
        let server = ServerRef {
            machine_id: source.machine_id.clone(),
            name: source.name.clone(),
            address: source.address.clone(),
            port: source.port,
            origin_url: source.origin_url.clone(),
            token: source.token.clone(),
            ..Default::default()
        };
        let kid = UserRef {
            uuid: "kid".into(),
            title: "Kid".into(),
            token: source.token.clone(),
            ..Default::default()
        };
        Session {
            client_id: "synthetic-resource-client".into(),
            account_token: "synthetic-account".into(),
            user: UserRef {
                uuid: "admin".into(),
                title: "Admin".into(),
                token: "synthetic-admin-token".into(),
                ..Default::default()
            },
            server: ServerRef {
                token: "synthetic-admin-token".into(),
                ..server.clone()
            },
            sources: vec![SourceRef {
                token: "synthetic-admin-token".into(),
                ..source.clone()
            }],
            profiles: vec![ProfileCreds {
                uuid: kid.uuid.clone(),
                user: kid,
                server,
                sources: vec![source],
                pin: None,
                extensions: Default::default(),
            }],
            ..Default::default()
        }
    }

    fn stored_admin() -> Session {
        let mut session = stored();
        session.home_users = vec![session::HomeUserRef {
            uuid: session.user.uuid.clone(),
            title: session.user.title.clone(),
            admin: true,
            ..Default::default()
        }];
        session
    }

    fn frame(rig: &mut Bridge, d: &mut Dispatcher<AppHost>, records: Vec<SessionEnvelope>) {
        let results = records
            .into_iter()
            .map(|r| (r.addr, AppMsg::Session(SessionEvent::Result(r))))
            .collect();
        d.frame_with(rig, Tick::default(), Vec::new(), results, &mut NoTap, false);
        rig.settle_session_io_for_test(d);
    }

    fn command(rig: &mut Bridge, d: &mut Dispatcher<AppHost>, cmd: SessionCmd) {
        execute_session_command(d, cmd);
        frame(rig, d, Vec::new());
    }

    mod native_endpoint_regressions {
        use super::*;
        const EPOCH: u64 = u32::MAX as u64 + 120;

        fn live(saved: Session, mt: &crate::task::MainThread) -> Bridge {
            let mut init = crate::auth::SessionInit::captured(saved);
            init.epoch = EPOCH;
            let mut rig = Bridge::for_session_test(init);
            rig.session_adapter = crate::app::adapters::session::SessionAdapter::live_resources_for_test(mt);
            rig
        }

        fn assert_receipt(rig: &Bridge, record: &SessionEnvelope, req: u32) {
            assert_eq!(record.addr, crate::ui::machine::Addr {
                to: MachineId::Session, req: crate::ui::machine::RequestId(req),
            });
            assert_eq!(record.key.epoch, EPOCH);
            assert!(rig.session_adapter.admitted(record));
        }

        #[test]

        fn accepted_activation_preserves_https_and_resolve_pin() {
            let _lock = crate::testlock::serial();
            let mt = unsafe { crate::task::MainThread::assume() };
            let tmp = session::TempSession::new("native-owner-activation");
            let _cleanup = ResourceCleanup(&mt);
            tmp.assert_only_target();
            crate::plex::reset_servers_for_test();
            session::save(&stored_admin());
            let before = std::fs::read(tmp.path()).unwrap();
            let origin = crate::origin::Origin::parse("https://192-0-2-10.h.plex.direct:32400").unwrap();
            let pin = crate::origin::ResolvePin::for_origin(&origin, "192.0.2.10").unwrap();
            let mut rig = live(stored_admin(), &mt);
            let candidate_origin = origin.base();
            rig.session_adapter.inject_fixture_work(1, move |output, input| {
                let SessionWork::ServerRoster { session, .. } = input else { panic!("expected roster work") };
                let expected = crate::auth::SessionIdentity::of(&session);
                // Existing serialized observation seam; unlike the old unsolicited Registry event,
                // this nonterminal belongs to the actual RefreshRoster request and captured identity.
                let activation = serde_json::from_value(serde_json::json!({"Activate": {
                    "epoch": EPOCH, "expected": expected,
                    "candidate": {"machine_id":"tls-test", "token":"synthetic", "name":"Synthetic",
                        "credit":"", "owned":true, "origin":candidate_origin, "address":"192.0.2.10",
                        "location":crate::plex::probe::Location::Local, "ipv6":false}
                }})).unwrap();
                output.progress(crate::auth::AuthProgress::Registry(activation)).unwrap();
                // Synthetic activation-contract proof, not a successful network-roster policy.
                // Return without terminal: the actual launch completion guard emits Dropped.
            });
            let mut d = Dispatcher::<AppHost>::new();
            command(&mut rig, &mut d, SessionCmd::RefreshRoster);
            let mut records = rig.session_adapter.take_results();
            assert_eq!(records.len(), 2);
            for record in &records { assert_receipt(&rig, record, 1); }
            assert!(!records[0].terminal && records[1].terminal);
            assert!(matches!(records[1].outcome, crate::auth::owner::SessionArrival::Dropped));
            assert_eq!(crate::plex::server_ids().count(), 0, "worker has no registry write authority");
            assert_eq!(std::fs::read(tmp.path()).unwrap(), before);
            let terminal = records.pop().unwrap();
            let activation = records.pop().unwrap();
            frame(&mut rig, &mut d, vec![activation.clone()]);
            let ids: Vec<_> = crate::plex::server_ids().collect();
            assert_eq!(ids.len(), 1);
            let client = crate::plex::client_for(ids[0]).unwrap();
            assert_eq!(client.machine_id(), "tls-test");
            assert_eq!(client.origin(), &origin);
            assert_eq!(client.resolve_pin(), Some(&pin));
            assert!(!rig.session_adapter.admitted(&activation));
            assert!(rig.session.snapshot_init().pending.contains_key(&1));
            frame(&mut rig, &mut d, vec![terminal.clone()]);
            assert!(!rig.session_adapter.admitted(&terminal));
            assert!(rig.session.snapshot_init().pending.is_empty());
            assert!(rig.session.snapshot_init().pending_commit.is_none());
            assert_eq!(std::fs::read(tmp.path()).unwrap(), before,
                "candidate activation changes registry, not persisted credentials");
            tmp.assert_only_target();
        }

        #[test]

        fn endpoint_result_from_a_replaced_client_incarnation_cannot_overwrite_its_route() {
            let _lock = crate::testlock::serial();
            let mt = unsafe { crate::task::MainThread::assume() };
            let tmp = session::TempSession::new("native-owner-endpoint-incarnation");
            let _cleanup = ResourceCleanup(&mt);
            tmp.assert_only_target();
            crate::plex::reset_servers_for_test();
            let saved = stored();
            session::save(&saved);
            let original_disk = std::fs::read(tmp.path()).unwrap();
            let id = crate::plex::register_for_test("resource-server", "127.0.0.1", 32400,
                "synthetic-admin-token", "synthetic-resource-client");
            let old_client = crate::plex::client_for(id).unwrap();
            let old_instance = old_client.instance_gen();
            let old_token = old_client.token_gen();
            let mut rig = live(saved.clone(), &mt);
            rig.session_adapter.inject_fixture_work(1, move |output, input| {
                let SessionWork::Endpoint { session, expected, lifecycle, machine_id } = input
                    else { panic!("expected actual endpoint capture") };
                assert_eq!(lifecycle.sid, id.raw());
                assert_eq!(lifecycle.instance_gen, old_instance);
                assert_eq!(lifecycle.token_gen, old_token);
                let stale = SourceRef { address: "127.0.0.9".into(),
                    origin_url: "http://127.0.0.9:32400".into(),
                    token: "account-token-not-authoritative".into(), ..session.sources[0].clone() };
                let probe = crate::auth::settled_probe_for_test(&machine_id,
                    crate::plex::probe::Outcome::Reachable,
                    Some(crate::plex::probe::Location::Local), Some("127.0.0.9".into()));
                output.complete(crate::auth::endpoint_work_fact(EPOCH, expected, lifecycle,
                    machine_id, Some(stale), Some(probe))).unwrap();
            });
            let mut d = Dispatcher::<AppHost>::new();
            command(&mut rig, &mut d, SessionCmd::RequestEndpoint { sid: id });
            let records = rig.session_adapter.take_results();
            assert_eq!(records.len(), 1);
            assert_receipt(&rig, &records[0], 1);
            assert!(records[0].terminal);
            assert!(std::ptr::eq(old_client, crate::plex::client_for(id).unwrap()));
            assert_eq!(std::fs::read(tmp.path()).unwrap(), original_disk);
            let mut newer = saved;
            newer.server.address = "127.0.0.2".into();
            newer.server.origin_url = "http://127.0.0.2:32400".into();
            newer.server.token = "synthetic-new-tok".into();
            newer.user.token = newer.server.token.clone();
            newer.sources[0].address = newer.server.address.clone();
            newer.sources[0].origin_url = newer.server.origin_url.clone();
            newer.sources[0].token = newer.server.token.clone();
            assert!(rig.session.snapshot_init().pending[&1].expected.matches(&newer),
                "identity still matches: this test must exercise native incarnation rejection");
            session::save(&newer);
            let disk = std::fs::read(tmp.path()).unwrap();
            let replacement = crate::plex::register_for_test("resource-server", "127.0.0.2", 32400,
                "synthetic-new-tok", "synthetic-resource-client");
            assert_eq!(replacement, id);
            let new_client = crate::plex::client_for(id).unwrap();
            assert!(!std::ptr::eq(old_client, new_client));
            assert_ne!(new_client.instance_gen(), old_instance);
            assert_ne!(new_client.token_gen(), old_token);
            let replacement_generation = new_client.token_gen();
            // URL construction reads the actual Client token without making any request.
            let replacement_url = new_client.direct_play_url("/synthetic", "test").to_url();
            assert!(replacement_url.contains("X-Plex-Token=synthetic-new-tok"));
            let terminal = records[0].clone();
            frame(&mut rig, &mut d, records);
            assert!(std::ptr::eq(new_client, crate::plex::client_for(id).unwrap()));
            assert_eq!(new_client.host(), "127.0.0.2");
            assert_eq!(new_client.token_gen(), replacement_generation);
            assert_eq!(new_client.direct_play_url("/synthetic", "test").to_url(), replacement_url);
            assert_eq!(std::fs::read(tmp.path()).unwrap(), disk, "stale result cannot rewrite actual file");
            assert_eq!(session::peek().sources[0].address, "127.0.0.2");
            assert_eq!(session::peek().server.address, "127.0.0.2");
            assert!(rig.session.snapshot_init().pending.is_empty());
            assert!(rig.session.snapshot_init().pending_commit.is_none());
            assert!(!rig.session_adapter.admitted(&terminal));
            // Real readmission, then guard terminal/main application: no manual flight release.
            let req = rig.session.snapshot_init().next_req + 1;
            rig.session_adapter.inject_fixture_work(req, |_, input| {
                assert!(matches!(input, SessionWork::Endpoint { .. }));
            });
            command(&mut rig, &mut d, SessionCmd::RequestEndpoint { sid: id });
            assert!(rig.session.snapshot_init().pending.contains_key(&req));
            let next = rig.session_adapter.take_results();
            assert_eq!(next.len(), 1);
            assert_receipt(&rig, &next[0], req);
            frame(&mut rig, &mut d, next);
            assert!(rig.session.snapshot_init().pending.is_empty());
            assert_eq!(std::fs::read(tmp.path()).unwrap(), disk);
            tmp.assert_only_target();
        }
    }
}
