//! Server discovery and probe-racing tests: retry policy, candidate racing, relay fallback,
//! identity verification, address dialability, and the resolve-roster end-to-end scenarios.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;
use std::sync::Mutex;

/// Completion order is responsiveness, never preference. A lower-scoring remote candidate
/// finishing last cannot replace the local winner that already activated.
#[test]
fn a_worse_candidate_finishing_last_never_downgrades_the_winner() {
    let plan = race_plan();
    let dial: ProbeDial = status_dial(|origin, _, _| {
        if origin.host().starts_with("203-") {
            std::thread::sleep(Duration::from_millis(30));
        }
        (200, identity_json("race-machine"))
    });
    let mut activated = Vec::new();
    let reach = probe_server_racing(
        &plan,
        dial,
        &threaded_spawn,
        test_policy(),
        &mut |_, _, origin| activated.push(origin.base()),
    );

    let Reach::At(candidate, _) = reach else {
        panic!("the local candidate must win")
    };
    assert_eq!(candidate.location, probe::Location::Local);
    assert_eq!(
        activated.len(),
        1,
        "the worse last result must not cause a re-point"
    );
    assert!(activated[0].contains("192-0-2-10"));
}

#[test]
fn a_better_candidate_finishing_last_causes_exactly_one_final_repoint() {
    let mut plan = race_plan();
    plan.candidates.swap(0, 1); // remote launches first; local remains the better score
    let dial: ProbeDial = status_dial(|origin, _, _| {
        if origin.host().starts_with("192-") {
            std::thread::sleep(Duration::from_millis(30));
        }
        (200, identity_json("race-machine"))
    });
    let mut activated = Vec::new();
    let reach = probe_server_racing(
        &plan,
        dial,
        &threaded_spawn,
        test_policy(),
        &mut |_, c, _| activated.push(c.location),
    );
    assert!(matches!(reach, Reach::At(ref c, _) if c.location == probe::Location::Local));
    assert_eq!(
        activated,
        [probe::Location::Remote, probe::Location::Local],
        "first usable, then one final best-score re-point"
    );
}

/// Pending means a worker really exists. Refusing one launch cannot leave the coordinator
/// awaiting a message that can never be sent.
#[test]
fn one_refused_spawn_still_settles_on_the_worker_that_exists() {
    let plan = race_plan();
    let dial: ProbeDial = status_dial(|_, _, _| (200, identity_json("race-machine")));
    let spawn = |index: usize, job: ProbeJob| {
        if index == 0 {
            false
        } else {
            std::thread::spawn(job);
            true
        }
    };
    let mut activated = Vec::new();
    let reach = probe_server_racing(&plan, dial, &spawn, test_policy(), &mut |_, c, _| {
        activated.push(c.location)
    });
    assert!(matches!(reach, Reach::At(..)));
    assert_eq!(activated, vec![probe::Location::Remote]);
}

#[test]
fn all_refused_spawns_terminate_as_failure() {
    let plan = race_plan();
    let dial: ProbeDial = status_dial(|_, _, _| panic!("a refused job must never run"));
    let mut activations = 0;
    let reach =
        probe_server_racing(&plan, dial, &|_, _| false, test_policy(), &mut |_, _, _| {
            activations += 1
        });
    assert!(matches!(reach, Reach::No));
    assert_eq!(activations, 0);
}

/// Relay is a second phase, not one more concurrent candidate. It is launched only after the
/// non-relay set has settled without a winner.
#[test]
fn relay_is_dialled_only_after_every_nonrelay_candidate_settles() {
    let mut plan = race_plan();
    plan.candidates.truncate(1);
    plan.candidates.push(Candidate {
        url: "https://relay.example.test:443".into(),
        scheme: Scheme::Https,
        location: probe::Location::Relay,
        address: "relay.example.test".into(),
        port: 443,
        ipv6: false,
        credential_eligible: true,
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_dial = Arc::clone(&seen);
    let dial: ProbeDial = status_dial(move |origin, _, _| {
        seen_by_dial.lock().unwrap().push(origin.host().to_string());
        if origin.host() == "relay.example.test" {
            (200, identity_json("race-machine"))
        } else {
            (0, Vec::new())
        }
    });
    let reach = probe_server_racing(
        &plan,
        dial,
        &threaded_spawn,
        test_policy(),
        &mut |_, _, _| {},
    );
    assert!(matches!(reach, Reach::At(ref c, _) if c.location == probe::Location::Relay));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["192-0-2-10.h.plex.direct", "relay.example.test"]
    );
}

#[test]
fn relay_only_server_gets_a_fresh_probe_budget_after_direct_timeouts_and_is_admitted() {
    let mut resource = resource(
        r#"{"name":"ours","clientIdentifier":"race-machine","provides":"server","owned":true,
            "accessToken":"profile-token","connections":[
              {"protocol":"https","address":"192.0.2.10","port":32400,
               "uri":"https://192-0-2-10.h.plex.direct:32400","local":true,"relay":false},
              {"protocol":"https","address":"relay.example.test","port":443,
               "uri":"https://relay.example.test:443","local":false,"relay":true}]}"#,
    );
    resource.public_address_matches = true;
    let policy = ProbeDeadlines {
        local: Duration::from_millis(10),
        remote: Duration::from_millis(50),
    };
    let relay_budgets = Arc::new(Mutex::new(Vec::new()));
    let relay_budgets_at_dial = Arc::clone(&relay_budgets);
    let dial: ProbeDial = status_dial(move |origin, _, budget| {
        if origin.host() == "relay.example.test" {
            relay_budgets_at_dial.lock().unwrap().push(budget);
            if budget == policy.remote {
                return (200, identity_json("race-machine"));
            }
            return (0, Vec::new());
        }
        std::thread::sleep(policy.local + Duration::from_millis(5));
        (0, Vec::new())
    });
    let plan = probe::plan(&resource, CredentialPolicy::HttpsOnly);
    let mut probe_one = move |_: &ProbePlan, _: &[String]| {
        probe_server_racing(&plan, Arc::clone(&dial), &threaded_spawn, policy,
            &mut |_, _, _| {})
    };
    let mut admissions = 0;
    let resolved = resolve_roster_using_admission(
        &[resource], &[], CredentialPolicy::HttpsOnly, &PlaintextAsk::undecided(), &mut probe_one,
        &mut |_| {
            admissions += 1;
            crate::plex::EndpointAdmission::Usable
        },
        &mut |_, _, _| {}, &mut || {}, &mut |_, _, _, _| {},
    );

    assert!(matches!(resolved.outcome, Resolved::Reached(ref found)
        if found.len() == 1 && found[0].tier == Some(probe::Location::Relay)));
    assert_eq!(resolved.admitted_machine_id.as_deref(), Some("race-machine"));
    assert_eq!(admissions, 1, "the relay winner must reach authenticated admission");
    assert_eq!(relay_budgets.lock().unwrap().as_slice(), [policy.remote],
        "the relay identity phase owns a fresh remote probe budget");
}

#[test]
fn a_reachable_relay_beats_a_direct_proxy_401() {
    let mut plan = race_plan();
    plan.candidates.truncate(1);
    plan.candidates.push(Candidate {
        url: "https://relay.example.test:443".into(),
        scheme: Scheme::Https,
        location: probe::Location::Relay,
        address: "relay.example.test".into(),
        port: 443,
        ipv6: false,
        credential_eligible: true,
    });
    let dial: ProbeDial = status_dial(|origin, _, _| {
        if origin.host() == "relay.example.test" {
            (200, identity_json("race-machine"))
        } else {
            (401, Vec::new())
        }
    });
    let reach = probe_server_racing(
        &plan,
        dial,
        &threaded_spawn,
        test_policy(),
        &mut |_, _, _| {},
    );
    assert!(matches!(reach, Reach::At(ref c, _) if c.location == probe::Location::Relay));
}

#[test]
fn discovery_retries_the_same_server_via_relay_after_direct_admission_times_out() {
    let resource = resource(
        r#"{"name":"ours","clientIdentifier":"machine","provides":"server","owned":true,
            "accessToken":"profile-token","connections":[
              {"protocol":"https","address":"192.0.2.10","port":32400,
               "uri":"https://192-0-2-10.h.plex.direct:32400","local":true,"relay":false},
              {"protocol":"https","address":"relay.example.test","port":443,
               "uri":"https://relay.example.test:443","local":false,"relay":true}]}"#,
    );
    let plan = probe::plan(&resource, CredentialPolicy::HttpsOnly);
    let direct = plan.candidates.iter().find(|c| c.location != probe::Location::Relay).unwrap().clone();
    let relay = plan.candidates.iter().find(|c| c.location == probe::Location::Relay).unwrap().clone();
    let mut probes = 0;
    let mut probe_one = |_: &ProbePlan, rejected: &[String]| {
        probes += 1;
        if probes == 2 {
            assert_eq!(rejected, ["https://192-0-2-10.h.plex.direct:32400"]);
        }
        let candidate = if probes == 1 { direct.clone() } else { relay.clone() };
        Reach::At(candidate.clone(), candidate.origin().unwrap())
    };
    let mut admissions = vec![crate::plex::EndpointAdmission::Timeout,
        crate::plex::EndpointAdmission::Usable].into_iter();
    let resolved = resolve_roster_using_admission(
        &[resource], &[], CredentialPolicy::HttpsOnly, &PlaintextAsk::undecided(), &mut probe_one,
        &mut |_| admissions.next().unwrap(), &mut |_, _, _| {},
        &mut || {}, &mut |_, _, _, _| {},
    );
    assert_eq!(resolved.admitted_machine_id.as_deref(), Some("machine"));
    let Resolved::Reached(found) = resolved.outcome else { panic!("relay admission must retain the server") };
    assert_eq!(probes, 2);
    assert_eq!(found[0].origin_url, "https://relay.example.test:443");
}

#[test]
fn a_direct_401_remains_the_reason_when_relay_is_silent() {
    let mut plan = race_plan();
    plan.candidates.truncate(1);
    plan.candidates.push(Candidate {
        url: "https://relay.example.test:443".into(),
        scheme: Scheme::Https,
        location: probe::Location::Relay,
        address: "relay.example.test".into(),
        port: 443,
        ipv6: false,
        credential_eligible: true,
    });
    let dial: ProbeDial = status_dial(|origin, _, _| {
        if origin.host() == "relay.example.test" {
            (0, Vec::new())
        } else {
            (401, Vec::new())
        }
    });
    let reach = probe_server_racing(
        &plan,
        dial,
        &threaded_spawn,
        test_policy(),
        &mut |_, _, _| {},
    );
    assert!(matches!(reach, Reach::Refused));
}

/// A result's completion timestamp, not a delayed coordinator observation, decides whether it
/// met the deadline. The injected spawn holds the coordinator after the job has already sent.
#[test]
fn an_on_time_result_queued_before_the_deadline_survives_coordinator_delay() {
    let mut plan = race_plan();
    plan.candidates.truncate(1);
    let dial: ProbeDial = status_dial(|_, _, _| (200, identity_json("race-machine")));
    let spawn = |_: usize, job: ProbeJob| {
        job();
        std::thread::sleep(Duration::from_millis(20));
        true
    };
    let policy = ProbeDeadlines {
        local: Duration::from_millis(5),
        remote: Duration::from_millis(5),
    };
    let reach = probe_server_racing(&plan, dial, &spawn, policy, &mut |_, _, _| {});
    assert!(matches!(reach, Reach::At(..)));
}
