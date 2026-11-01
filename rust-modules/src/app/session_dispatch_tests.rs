//! Session/browse dispatch mechanics: chrome tab generation ownership, per-bridge browse
//! isolation, and the auth owner's queued commit-reply/carry machinery.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;
use super::test_support::{frame, frame_with_results};

#[test]
fn browse_tab_generation_is_owned_and_chrome_never_replays_a_stale_shape() {
    let _guard = crate::testlock::serial();
    let session = crate::plex::session::TempSession::new("bridge-browse-tabs");
    session.watching("u-bridge-browse-tabs");
    crate::plex::reset_servers_for_test();
    let own = crate::plex::register_for_test(
        "bridge-tabs-own", "127.0.0.1", 9, "synthetic", "fixture");
    let shared = crate::plex::register_for_test(
        "bridge-tabs-shared", "127.0.0.1", 10, "synthetic", "fixture");
    let mut rig = Bridge::for_test(|| 0);
    rig.stores.browse.borrow_mut().seed_registered_table_for_test([own, shared]);
    let mut pages = Dispatcher::<AppHost>::new();
    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    let before = rig.chrome.labels();
    assert!(before.labels.iter().any(|label| label == "TV Shows"));
    let before_gen = before.generation;

    rig.stores.browse.borrow_mut().run(
        crate::stores::browse::BrowseCmd::ApplyPins(vec![(1, false)]));
    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    let changed = rig.chrome.labels();
    assert!(changed.generation > before_gen);
    assert!(!changed.labels.iter().any(|label| label == "TV Shows"));
    let changed_gen = changed.generation;

    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    assert_eq!(rig.chrome.labels().generation, changed_gen,
        "capturing again must not republish the owner's old tab generation");
    assert!(!rig.chrome.labels().labels.iter().any(|label| label == "TV Shows"));
    crate::plex::reset_servers_for_test();
}

// Dev-only: this fixture drives a plaintext loopback PMS with a real, token-bearing registered
// server, which a store build's `CredentialPolicy::HttpsOnly` refuses before the library GET
// reaches the wire (see `http::credential_transport_allowed`) — the connection this test waits
// on then never arrives. The store-build case is covered instead by `auth.rs`'s
// `e2e_real_curl_*` HTTPS harness.
#[cfg(feature = "devtriggers")]
#[test]
fn production_bridges_do_not_share_browse_state_or_landings() {
    use std::io::{Read, Write};
    let _guard = crate::testlock::serial();
    let session = crate::plex::session::TempSession::new("bridge-browse-owners");
    session.watching("u-bridge-browse-owners");
    crate::plex::reset_servers_for_test();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
    let port = listener.local_addr().unwrap().port();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("page request");
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let n = socket.read(&mut chunk).expect("read request");
            assert!(n > 0, "request closed before headers");
            request.extend_from_slice(&chunk[..n]);
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains("GET /library/sections/1/all?"));
        assert!(request.contains("X-Plex-Container-Start=0"));
        assert!(request.contains("X-Plex-Container-Size=60"));
        accepted_tx.send(()).unwrap();
        release_rx.recv().expect("release held response");
        let body = br#"{"MediaContainer":{"totalSize":1,"Metadata":[{"ratingKey":"1","type":"movie","title":"first-owner-only"}]}}"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len());
        socket.write_all(head.as_bytes()).unwrap();
        socket.write_all(body).unwrap();
    });
    let own = crate::plex::register_for_test(
        "bridge-browse-own", "127.0.0.1", 9, "synthetic", "fixture");
    let shared = crate::plex::register_for_test(
        "bridge-browse-shared", "127.0.0.1", port as i32, "synthetic", "fixture");
    let mut first = Bridge::for_test(|| 0);
    let mut second = Bridge::for_test(|| 0);
    first.stores.browse.borrow_mut().seed_registered_table_for_test([own, shared]);
    first.stores.browse.borrow_mut().prepare_page_for_test(shared);
    let mut pages = Dispatcher::<AppHost>::new();
    frame(&mut pages, &mut first, AppArg::Library, tick(0), vec![]);
    let _ = first.stores.browse.borrow_mut().pump();
    accepted_rx.recv_timeout(std::time::Duration::from_secs(2))
        .expect("production page request reached loopback");
    let _ = first.stores.take_notices();
    let before = first.stores.gen(StoreId::Browse);
    second.capture_views(&mut pages);
    assert_eq!(second.directory.view().current(), None,
        "a command and its later worker landings belong only to the addressed Bridge store");
    release_tx.send(()).unwrap();
    server.join().unwrap();
    assert!(!second.stores.browse.borrow().has_page_result_for_test(),
        "the origin worker must not write the other Bridge's adapter");
    let mut page_landed = false;
    for _ in 0..100_000 {
        if first.stores.browse.borrow_mut().pump().changed {
            page_landed = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(page_landed, "page result did not land");
    assert_eq!(first.stores.gen(StoreId::Browse), before + 1,
        "one parsed page landing advances exactly one owned generation");
    assert_eq!(first.stores.browse.borrow_mut().listing_snapshot().view().item(0)
        .map(|item| item.title.as_str()), Some("first-owner-only"));
    assert!(second.stores.browse.borrow_mut().listing_snapshot().view().item(0).is_none());

    struct BrowseNotices(usize);
    impl crate::ui::dispatch::Tap<AppHost> for BrowseNotices {
        fn effect(&mut self, _: u64, stamped: &crate::ui::machine::Stamped<AppHost>) {
            if matches!(&stamped.fx, Fx::Deliver(_, Delivery::Screen(
                ScreenEvent::StoreChanged(ord, _))) if *ord == StoreId::Browse.ord()) {
                self.0 += 1;
            }
        }
    }
    let mut notices = BrowseNotices(0);
    frame_with_results(&mut pages, &mut first, AppArg::Library, tick(1), vec![],
        || Vec::new(), &mut notices);
    assert_eq!(notices.0, 1,
        "capture publication and notice drain must coalesce one page into one StoreChanged");
    assert!(first.stores.take_notices().is_empty());
    crate::plex::reset_servers_for_test();
}

