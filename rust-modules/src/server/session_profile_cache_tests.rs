//! `ProfileCreds` cache: PIN verifier hashing, remember/refresh-by-uuid, unusable entries,
//! and persistence across a save/load round trip.

use super::*;

#[test]
fn a_verifier_accepts_its_pin_and_nothing_else() {
    let v = PinVerifier::new("4821");
    assert!(v.verify("4821"));
    assert!(!v.verify("4822"));
    assert!(!v.verify(""));
    assert_eq!(v.salt.len(), 32, "16 random bytes, hex");
    assert_eq!(v.hash.len(), 64);
    assert_ne!(
        PinVerifier::new("4821").salt,
        v.salt,
        "two verifiers of one PIN never share a salt"
    );
}

#[test]
fn a_malformed_verifier_admits_nobody() {
    let none = PinVerifier::default();
    assert!(
        !none.verify(""),
        "an empty record must not match an empty PIN"
    );
    let mut v = PinVerifier::new("1234");
    v.iters = 0;
    assert!(!v.verify("1234"));
    let mut v = PinVerifier::new("1234");
    v.iters = u32::MAX;
    assert!(
        !v.verify("1234"),
        "an unbounded count is refused before it is run"
    );
    let mut v = PinVerifier::new("1234");
    v.iters = PinVerifier::MAX_ITERS + 1;
    assert!(!v.verify("1234"));
    let mut v = PinVerifier::new("1234");
    v.hash.pop();
    assert!(!v.verify("1234"));
    let mut v = PinVerifier::new("1234");
    v.salt = "zz".into();
    assert!(!v.verify("1234"));
}

/// A file written before the field existed parses with an empty cache, never fails.
#[test]
fn a_legacy_file_has_an_empty_cache() {
    let back: Session =
        serde_json::from_str(r#"{"client_id":"c","account_token":"a"}"#).unwrap();
    assert!(back.profiles.is_empty());
    let back: Session = serde_json::from_str(
        r#"{"client_id":"c","profiles":[{"uuid":"u","user":{"token":"t"},"server":{"address":"10.0.0.1","port":"nope"}}, 7]}"#,
    )
    .unwrap();
    assert!(
        back.profiles.is_empty(),
        "a malformed entry costs the entry, not the session"
    );
}

#[test]
fn pin_verifier_rejects_non_ascii_hex_without_panicking() {
    let verifier = PinVerifier {
        salt: format!("aé{}", "a".repeat(29)),
        hash: "02".repeat(32), iters: 1, ..Default::default()
    };
    assert!(!verifier.verify("1234"));
}

#[test]
fn pin_verifier_rejects_non_16_byte_salt_even_when_hash_matches() {
    let verifier = PinVerifier::with_salt("1234", &[7; 8]);
    assert!(!verifier.verify("1234"), "malformed persisted salt must fail closed");
}
