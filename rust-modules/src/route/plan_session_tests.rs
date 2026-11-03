//! Prime-decision and playback-session classification tests: clip queue rows, control-
//! snapshot expiry, and prime refusal causes.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;



#[test]
fn a_route_change_wins_over_an_expired_control_snapshot() {
    assert!(matches!(
        classify_prime_decision(false, crate::plex::JsonDeadlineOutcome::Deadline),
        Err(PrimeRefusal::Session),
    ));
}


#[test]
fn prime_refusals_follow_the_issued_cause_not_the_clock_at_return() {
    let response = |status, body: &[u8]| crate::plex::JsonDeadlineOutcome::Response {
        reply: crate::http::Reply {
            status,
            body: body.to_vec(),
        },
        parsed: None,
    };
    assert!(matches!(
        classify_prime_decision(true, response(500, b"nope")),
        Err(PrimeRefusal::Control),
    ));
    assert!(matches!(
        classify_prime_decision(true, response(200, b"not-json")),
        Err(PrimeRefusal::Control),
    ));
    assert!(matches!(
        classify_prime_decision(true, crate::plex::JsonDeadlineOutcome::Transport),
        Err(PrimeRefusal::Control),
    ));
    assert!(matches!(
        classify_prime_decision(true, crate::plex::JsonDeadlineOutcome::Deadline),
        Err(PrimeRefusal::Deadline),
    ));
    assert!(matches!(
        classify_prime_decision(false, response(500, b"nope")),
        Err(PrimeRefusal::Session),
    ));
    assert!(matches!(
        classify_prime_decision(false, crate::plex::JsonDeadlineOutcome::Transport),
        Err(PrimeRefusal::Session),
    ));
}

