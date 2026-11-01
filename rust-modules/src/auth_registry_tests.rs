//! Server registry and roster reconciliation tests: registration order, profile-switch
//! re-keying, endpoint recovery, revoked/surviving grants, and primary drift repair.

use super::*;
use crate::origin::CredentialPolicy;

/// Minimal stored-source fixture (recovered from the removed probe-engine test support).
fn source(machine_id: &str, owned: bool, token: &str) -> SourceRef {
    SourceRef {
        machine_id: machine_id.into(),
        name: machine_id.into(),
        shared_by: if owned { String::new() } else { "friend".into() },
        owned,
        address: "10.0.0.1".into(),
        port: 32400,
        token: token.into(),
        ..Default::default()
    }
}

fn install_stored_source(source: &SourceRef, policy: CredentialPolicy) -> ServerId {
    let origin = source.origin().expect("stored source has a well-formed origin");
    let id = crate::plex::register_pinned_with_client_id_and_policy(
        &source.machine_id,
        &origin,
        &source.token,
        source.resolve_pin().as_ref(),
        "stored-install-test",
        crate::plex::ConnectionFacts::new(
            source.tier,
            crate::plex::IpVersion::of_host(&source.address),
        ),
        policy,
    );
    crate::plex::describe_server(id, &source.name, &source.shared_by, grant_of(source));
    id
}

/// Shipping policy must be exercised through the same registry effects the boot owner emits,
/// not through the explicit-policy fixture above. One legacy plaintext source remains visible as
/// recovery metadata, but neither the primary activation nor roster install may make it current.
#[cfg(not(feature = "devtriggers"))]

/// Endpoint recovery reuses the retained slot. Re-pointing it to verified HTTPS makes the build
/// policy admit its credential, restores CURRENT, and lets the ordinary endpoint effect publish
/// the successful refresh verdict.
#[cfg(not(feature = "devtriggers"))]

#[test]
fn stored_credential_policy_https_only_does_not_activate_plaintext_with_its_credential() {
    let _g = crate::testlock::serial();
    crate::plex::reset_servers_for_test();
    let mut previously_live = source("stored-http", true, "previous-token");
    previously_live.origin_url = "https://stored.example.test:32400".into();
    install_stored_source(&previously_live, CredentialPolicy::HttpsOnly);
    assert!(crate::plex::client_opt().is_some());

    let mut stored = source("stored-http", true, "stored-token");
    stored.origin_url = "http://192.0.2.10:32400".into();

    let id = install_stored_source(&stored, CredentialPolicy::HttpsOnly);

    assert!(crate::plex::client_opt().is_none(), "plaintext must not become the current credentialed client");
    assert_eq!(crate::plex::server_probe_result(id), Some(Outcome::InsecureOnly));
    crate::plex::reset_servers_for_test();
}

#[test]
fn stored_credential_policy_covers_legacy_address_and_port_that_synthesizes_http() {
    let _g = crate::testlock::serial();
    crate::plex::reset_servers_for_test();
    let legacy = source("legacy-http", true, "legacy-token");
    assert!(legacy.origin_url.is_empty());
    assert!(!legacy.origin().expect("legacy fallback").is_tls());

    let id = install_stored_source(&legacy, CredentialPolicy::HttpsOnly);

    assert!(crate::plex::client_opt().is_none(), "the synthesized HTTP origin is governed by the same gate");
    assert_eq!(crate::plex::server_probe_result(id), Some(Outcome::InsecureOnly));
    crate::plex::reset_servers_for_test();
}

#[test]
fn stored_credential_policy_allow_plaintext_keeps_dev_installation_usable() {
    let _g = crate::testlock::serial();
    let stored = source("dev-http", true, "developer-token");

    crate::plex::reset_servers_for_test();
    install_stored_source(&stored, CredentialPolicy::HttpsOnly);
    assert!(crate::plex::client_opt().is_none(), "the store policy rejects the same source");

    crate::plex::reset_servers_for_test();
    let id = install_stored_source(&stored, CredentialPolicy::AllowPlaintext);
    assert_eq!(crate::plex::current_server(), id);
    assert!(crate::plex::client_opt().is_some(), "developer builds keep plaintext support");
    crate::plex::reset_servers_for_test();
}

#[test]
fn stored_credential_policy_rejection_retains_insecure_recovery_metadata() {
    let _g = crate::testlock::serial();
    crate::plex::reset_servers_for_test();
    let stored = source("recover-http", false, "stored-token");

    let id = install_stored_source(&stored, CredentialPolicy::HttpsOnly);

    assert_eq!(crate::plex::server_ids().collect::<Vec<_>>(), vec![id]);
    assert_eq!(crate::plex::server_facts(id).map(|f| f.name.as_str()), Some("recover-http"));
    assert_eq!(crate::plex::server_probe_result(id), Some(Outcome::InsecureOnly));
    crate::plex::reset_servers_for_test();
}

/// #95 step 8 / A2: the boot primary install (`install_captured_registry`, what
/// `RegistryPlan::DevInstall` and the boot gate's `install_pms_owned` both call) derives the
/// IP family from the ADVERTISED ADDRESS, not `origin.host()` — a `plex.direct` origin's host
/// is a certificate NAME `IpVersion::of_host` cannot parse as a literal, which is exactly why
/// R3(a) found this reading unknown on every real boot before the fix. The stored tier is
/// applied in the same write.
#[test]
fn a_boot_primary_install_of_a_plex_direct_origin_derives_ip_from_the_stored_address() {
    let _g = crate::testlock::serial();
    crate::plex::reset_servers_for_test();
    let origin = Origin::parse("https://192-168-1-50.h4sh.plex.direct:32400").unwrap();
    install_captured_registry(&origin, "192.168.1.50", "tok",
        Some(probe::Location::Local), None, &[], Some("cid"));
    let id = crate::plex::current_server();
    let client = crate::plex::client_for(id).expect("the primary is registered and current");
    assert_eq!(client.link(), Some(probe::Location::Local), "the stored tier");
    assert_eq!(
        client.ip_version(),
        Some(crate::plex::IpVersion::V4),
        "derived from the address, not the plex.direct hostname `origin.host()` carries"
    );
    crate::plex::reset_servers_for_test();
}

/// Our own server registers first and is the primary, whatever order plex.tv listed the account
/// in — because the registry makes the first registration `current` when nothing is yet, so the
/// ordering is what stops a boot coming up pointed at a friend's server and building Home from
/// their library.
#[test]
fn our_own_server_leads_the_roster_however_plex_tv_ordered_it() {
    let roster = vec![
        source("share-1", false, "t1"),
        source("ours", true, "t2"),
        source("share-2", false, "t3"),
    ];
    assert_eq!(
        registration_order(&roster),
        vec![1, 0, 2],
        "ours first, then plex.tv's own order"
    );
    // an entry with no credential (or no address) cannot be dialled, so it is not registered —
    // registering it would put a `Client` in the table that 401s everything asked of it
    let mut half = roster.clone();
    half[0].token.clear();
    half[2].address.clear();
    assert_eq!(registration_order(&half), vec![1]);
}