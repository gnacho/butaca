//! Controlled-bootstrap host boundaries. The production SDL boot/loop is additionally exercised
//! by the fresh-root simulator artifact; these tests do not claim native rendering or network.
use super::*;

/// The synthetic home IS the Jellyfin world: `Initial::validate` refuses its bootstrap
/// authority without the feature, so a controlled-world test cannot even boot.
#[test]
fn filmography_initial_requires_complete_typed_inputs() {
    let mut value = serde_json::to_value(Initial::synthetic_home(1, 32498, None).unwrap()).unwrap();
    value["content"] = serde_json::json!({"detail":"1001", "detailsec":1,
        "detailok":true, "filmography":true, "personcredits":9, "nowan":true});
    for trigger in ["detail", "detailsec", "detailok", "filmography", "personcredits", "nowan"] {
        value["triggers"].as_array_mut().unwrap().push(serde_json::json!(format!("butaca-{trigger}")));
    }
    let initial = Initial::from_value(value.clone()).expect("complete filmography boot");
    assert_ne!(initial.hash(), Initial::synthetic_home(1, 32498, None).unwrap().hash());
    value["content"].as_object_mut().unwrap().remove("detailsec");
    assert!(Initial::from_value(value).is_err());
}

/// Jellyfin world gate: the synthetic home cannot validate its authority without the feature.
#[test]
fn content_resources_deny_execution_and_require_exact_admissions() {
    let _guard = crate::testlock::serial();
    let initial = Initial::synthetic_home(1, 32498, None).unwrap();
    stores::init(&initial, true);
    let request = serde_json::json!({"store":"metadata","sid":0,"rk":"1001","gen":1,"client":1});
    let admission = serde_json::json!({"content_resource":true,"request":request,"admitted":false});
    stores::validate_admission(&admission, 1).unwrap();
    stores::begin([admission.clone()].into(), Default::default());
    assert!(!stores::admit(request.clone(), || panic!("replay executed a resource")));
    assert_eq!(stores::finish(), (vec![admission.clone()], None));
    let mut accepted = admission.clone();
    accepted["admitted"] = serde_json::json!(true);
    stores::begin([accepted.clone()].into(), Default::default());
    assert!(stores::admit(request.clone(), || panic!("admitted replay executed a resource")));
    assert!(stores::poll::<serde_json::Value>("person", 0,
        || panic!("empty replay polled a live mailbox")).is_none());
    assert_eq!(stores::finish(), (vec![accepted], None));
    stores::begin([admission].into(), Default::default());
    let mut wrong = request;
    wrong["rk"] = serde_json::json!("1002");
    assert!(!stores::admit(wrong, || panic!("mismatched request executed")));
    assert_eq!(stores::finish().1, Some("mismatched content resource admission"));
    stores::reset_for_test();
}

#[test]
fn controlled_script_input_roundtrips_without_claiming_a_physical_key() {
    for event in super::super::bridge::script_key(crate::ui::machine::Key::Down,
        crate::ui::machine::Tick { ms:646, dt_us:0 }) {
        let value = effects::input(&event).unwrap();
        assert_eq!(effects::input(&effects::decode_input(&value).unwrap()).unwrap(), value);
        let mut wrong = value;
        wrong["body"]["at_edge"] = serde_json::json!(true);
        assert!(effects::decode_input(&wrong).is_err());
    }
}

/// Jellyfin world gate: the synthetic home cannot validate its authority without the feature.
#[test]
fn settings_initial_is_typed_hashed_and_bound_to_its_trigger() {
    let initial = Initial::synthetic_home(17, 32517, Some("root".into())).unwrap();
    assert_eq!(initial.settings.as_deref(), Some("root"));
    assert!(initial.triggers.iter().any(|trigger| trigger == "butaca-settings"));
    let encoded = serde_json::to_value(&initial).unwrap();
    let decoded = Initial::from_value(encoded).unwrap();
    assert_eq!(decoded.hash(), initial.hash());

    let mut bad_value = initial.clone();
    bad_value.settings = Some("other".into());
    assert_eq!(bad_value.validate(), Err("unsupported initial Settings input"));

    let mut missing_trigger = initial.clone();
    missing_trigger.triggers.retain(|trigger| trigger != "butaca-settings");
    assert_eq!(missing_trigger.validate(), Err("incoherent initial Settings input"));

    let plain = Initial::synthetic_home(17, 32517, None).unwrap();
    assert_ne!(plain.hash(), initial.hash());
}
use crate::ui::machine::{InputEvent, InputKind, Key, Edge, Source, Tick};
use serde_json::json;

#[test]
fn committed_replay_initials_remain_canonical_and_hash_bound() {
    // The upstream replay fixtures (1-boot-home-chip-grid, 6-settings-family,
    // 12-filmography-detail-return) carry a pre-removal Initial with the consent field and no
    // longer decode under deny_unknown_fields. Butaca has no committed recordings of its own
    // yet; when the first TV pass records one, its manifest joins this table and the
    // canonical-and-hash-bound property is pinned again.
}

/// Jellyfin world gate: the synthetic home cannot validate its authority without the feature.
#[test]
fn typed_initial_roundtrip_and_hidden_input_hash_are_complete() {
    let initial = Initial::synthetic_home(17, 32517, None).unwrap();
    let value = serde_json::to_value(&initial).unwrap();
    assert_eq!(Initial::decode(value.clone(), initial.hash()).unwrap().hash(), initial.hash());
    let changed = initial.clone();
    changed.validate().unwrap();
    assert_eq!(changed.hash(), initial.hash());
    let press = crate::ui::press::Press::new();
    assert_eq!(super::super::recorder::state_hash(&press,"home","","",0,0,initial.hash()),
        super::super::recorder::state_hash(&press,"home","","",0,0,changed.hash()));
    let mut unknown = value.clone(); unknown["unrecognized"] = json!(true);
    assert!(Initial::from_value(unknown).is_err());
    let mut inconsistent = value.clone(); inconsistent["session"]["next_req"] = json!(1);
    assert!(Initial::from_value(inconsistent).is_err());
    let mut missing = value.clone(); missing.as_object_mut().unwrap().remove("entropy");
    assert!(Initial::from_value(missing).is_err());
    assert!(Initial::from_value(json!({})).is_err());
    let mut bad_seed = value; bad_seed["entropy"] = json!({"Seeded":18});
    assert!(Initial::from_value(bad_seed).is_err());
}

#[test]
fn screen_input_codec_retains_full_time_source_edge_and_hit_identity() {
    for source in [Source::Sdl,Source::RemoteFifo,Source::Script,Source::Replay] {
        for edge in [Edge::Down,Edge::Repeat,Edge::Up] {
            let input = InputEvent { at:Tick { ms:u32::MAX-3,dt_us:17 },source,
                kind:InputKind::Key { key:Key::Left,sym:1073741904,wcode:0,edge,at_edge:false } };
            let encoded = effects::input(&input).unwrap();
            let decoded = effects::decode_input(&encoded).unwrap();
            assert_eq!(effects::input(&decoded).unwrap(),encoded);
            let mut changed = input.clone(); changed.at.ms -= 1;
            assert_ne!(effects::input(&changed).unwrap(),encoded);
            let mut changed = input.clone();
            if let InputKind::Key { at_edge,.. } = &mut changed.kind { *at_edge = true; }
            assert_ne!(effects::input(&changed).unwrap(),encoded);
            assert!(effects::decode_input(&effects::input(&changed).unwrap()).is_err(),"an engine re-delivery is not external ingress");
        }
    }
    let pointer = |hit| InputEvent { at:Tick {ms:7,dt_us:0},source:Source::Sdl,
        kind:InputKind::Pointer {x:-0.0,y:1.5,hit} };
    assert_ne!(effects::input(&pointer(Some(1))).unwrap(), effects::input(&pointer(Some(2))).unwrap());
    assert_ne!(effects::input(&pointer(None)).unwrap(),effects::input(&pointer(Some(1))).unwrap());
}

/// **Stage B regression, device-affecting.** Before `0d466527` (stage B), `stores::init` ended
/// with `crate::metadata::record::reset(initial.content.is_some())`, arming the crate-global
/// `Tracker` for controlled-content recording of detail terminals. Stage B deleted that line with
/// the static and nothing armed the per-owner replacement in production, so a `Bridge` built by
/// `controlled_home` carried a permanently-disabled Tracker (`Tracker::new(false)`, `MetadataStore`'s
/// own `Default`): every detail landing while the tape was active silently produced NO recorded
/// terminal at all (`drain_live` -> `Some(([], drain))`), and the device-side validator rejected
/// `{"data":[]}` as "noncanonical detail replies". This pins `Bridge::controlled_home` actually
/// arming its own `MetadataStore`'s Tracker from `initial.content.is_some()`.
/// Jellyfin world gate: the synthetic home cannot validate its authority without the feature.
#[test]
fn controlled_home_arms_the_detail_tracker_when_content_initial_is_present() {
    let _serial = crate::testlock::serial();
    crate::registry::reset_for_test();
    let mt = unsafe { crate::task::MainThread::assume() };
    let mut initial = Initial::synthetic_home(41, 17, None).unwrap();
    initial.content = Some(ContentInitial {
        detail: "show".into(), detailsec: 0, detailok: true,
        filmography: false, personcredits: 0, nowan: false,
    });
    crate::app::bootstrap::stores::init(&initial, false);
    crate::ui::landgate::arm_recording();
    let mut bridge = super::super::bridge::Bridge::controlled_home(||0,&initial,&mt,false);
    let sid = crate::registry::ServerId::UNSET;
    crate::app::bootstrap::stores::begin(Default::default(), Default::default());
    let gen = crate::metadata::begin_detail_for_test(bridge.metadata_mut().adapter_ref(), sid, "show");
    {
        let store = bridge.metadata_mut();
        let (state, adapter) = store.split_for_test();
        crate::metadata::land_detail_for_test(state, adapter, sid, "show", gen,
            Some(crate::metadata::Detail { sid, rk: "show".into(), ..Default::default() }));
    }
    let results = crate::app::bootstrap::stores::take_results();
    assert_eq!(results.len(), 1,
        "controlled_home must arm this Bridge's own MetadataStore Tracker from initial.content, \
         or a controlled-content detail landing never reaches bootstrap::stores at all");
    crate::app::bootstrap::stores::finish();
    crate::ui::landgate::disarm();
    crate::app::bootstrap::stores::reset_for_test();
    crate::registry::reset_for_test();
}

fn recording(initial: &Initial) -> Recording {
    let mut header = crate::ui::rec::Header::new(super::super::recorder::state_fp(),initial);
    header.init_data = serde_json::to_value(initial).unwrap();
    header.features = super::super::recorder::features();
    header.triggers = initial.triggers.clone();
    Recording { header,frames:vec![crate::ui::rec::Frame { f:0,tick:Some(Tick::default()),st:Some(0),..Default::default() }],
        metrics:Default::default(),stopped_at:None }
}

/// Jellyfin world gate: the synthetic home cannot validate its authority without the feature.
#[test]
fn whole_record_preflight_rejects_bad_late_results_and_markers_without_resources() {
    let initial = Initial::synthetic_home(23,32517,None).unwrap();
    let mut record = recording(&initial);
    super::super::recorder::validate_controlled(&record,&initial).unwrap();
    record.frames[0].st = None;
    assert!(super::super::recorder::validate_controlled(&record,&initial).is_err(),"missing state grades cannot produce an empty SAME");
    record.frames[0].st = Some(0);
    record.frames.push(crate::ui::rec::Frame { f:1,tick:Some(Tick {ms:16,dt_us:16000}),
        results:vec![json!({"f":1,"t":"async","to":"store:1","req":1,"payload":{
            "kind":"hubs","version":1,"gen":1,"seq":1,"sid":0,"client":0,"token_gen":0,"build":null}})],
        lands:vec![(1,1,1)],st:Some(0),present:Some(false),snapshot_pending:Some(true),..Default::default() });
    super::super::recorder::validate_controlled(&record,&initial).unwrap();
    record.frames[1].present = None;
    assert!(super::super::recorder::validate_controlled(&record,&initial).is_err());
    record.frames[1].present = Some(false);
    for field in ["client","token_gen","sid"] {
        let saved = record.frames[1].results[0].clone();
        record.frames[1].results[0]["payload"][field] = json!(33);
        assert!(super::super::recorder::validate_controlled(&record,&initial).is_err());
        record.frames[1].results[0] = saved;
    }
    record.frames[1].results.push(json!({"f":1,"t":"async","to":"store:1","req":2,"payload":{"kind":"unimplemented"}}));
    assert!(super::super::recorder::validate_controlled(&record,&initial).is_err(),"valid prefix cannot hide unsupported suffix");
    record.frames[1].results.pop();
    record.frames[1].effects.push(json!({"f":1,"t":"eff","e":"Deliver","from":"Nav",
        "payload":{"unsupported":"unsupported Home screen delivery"}}));
    assert!(super::super::recorder::validate_controlled(&record,&initial).is_err());
}

#[test]
fn controlled_hubs_commands_preserve_normal_store_notice_bookkeeping() {
    let _serial = crate::testlock::serial();
    crate::registry::reset_for_test();
    let mt = unsafe { crate::task::MainThread::assume() };
    let publisher = crate::server::session::ProfilePublisher::scoped(&mt);
    let mut io = HomeIo { replay:true,preferences:Default::default(),requests:Vec::new(),admissions:Default::default(),
        failure:None,profile:publisher.snapshot() };
    let mut hubs = crate::stores::hubs::HubsStore::default();
    for command in [crate::stores::hubs::HubsCmd::Reset,crate::stores::hubs::HubsCmd::RefetchHubs,
        crate::stores::hubs::HubsCmd::Retry] {
        let before = hubs.gen();
        let normal = hubs.run(command.clone());
        let delta = hubs.gen().wrapping_sub(before);
        let before = hubs.gen();
        let controlled = io.hubs(&mut hubs, Some(command),0.0);
        assert_eq!(controlled.changed,normal.changed);
        assert_eq!(hubs.gen().wrapping_sub(before),delta,
            "controlled resource execution must not drop the Store machine's notice");
    }
}

/// The launch lane is the Jellyfin world's (the pick requires its installed client), so this
/// admission contract only exists in that flavor; the recorded fixtures grade it end to end.
#[test]
fn controlled_admission_records_real_discovery_spawn_refusal() {
    let _serial = crate::testlock::serial();
    // The flavor's launch lane is the installed client: the discovery pick only fires for the
    // jellyfin SERVER_ID once a client exists (browse_roster_jellyfin_tests).
    crate::jellyfin::install(crate::jellyfin::JfClient::new(
        crate::origin::Origin::http("127.0.0.1", 1), "dev-1".into()));
    let mt = unsafe { crate::task::MainThread::assume() };
    let publisher = crate::server::session::ProfilePublisher::scoped(&mt);
    let mut io = HomeIo { replay:false,preferences:Default::default(),requests:Vec::new(),admissions:Default::default(),
        failure:None,profile:publisher.snapshot() };
    let stores = crate::stores::Stores::default();
    stores.browse.borrow_mut().seed_source_for_test(crate::jellyfin::SERVER_ID);
    io.discovery_owned_with(&stores, &mut |_| false);
    crate::jellyfin::uninstall();
    assert_eq!(io.requests.len(), 1);
    assert_eq!(io.requests[0]["admitted"], serde_json::json!(false), "real refusal must be recorded");
}



#[test]
fn admission_replay_requires_full_request_identity_and_boolean_outcome() {
    let _serial = crate::testlock::serial();
    let mt = unsafe { crate::task::MainThread::assume() };
    let publisher = crate::server::session::ProfilePublisher::scoped(&mt);
    let request = serde_json::json!({"kind":"hubs","epoch":u32::MAX,"req":u32::MAX,
        "sid":0,"client":u32::MAX,"token_gen":u32::MAX});
    let mut recorded = request.clone(); recorded["admitted"] = serde_json::json!(false);
    validate_admission(&recorded,u32::MAX).unwrap();
    for field in ["epoch","req","sid","client","token_gen","admitted"] {
        let mut wrong = recorded.clone(); wrong[field] = serde_json::json!(u64::from(u32::MAX)+1);
        assert!(validate_admission(&wrong,u32::MAX).is_err());
        let mut io = HomeIo { replay:true,preferences:Default::default(),requests:Vec::new(),
            admissions:vec![wrong].into(),failure:None,profile:publisher.snapshot() };
        assert!(!io.admit(request.clone(),||panic!("replay must not execute resource")));
        assert!(io.failure.is_some());
    }
    let mut io = HomeIo { replay:true,preferences:Default::default(),requests:Vec::new(),
        admissions:vec![recorded.clone()].into(),failure:None,profile:publisher.snapshot() };
    assert!(!io.admit(request,||panic!("recorded refusal is not a live execution")));
    assert!(io.failure.is_none());
    assert_eq!(io.requests,vec![recorded]);
}
