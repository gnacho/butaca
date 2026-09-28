//! The Jellyfin Home projection's shelf set (issue #44): the Next Up shelf — the next unseen
//! episode of every started series — leads the shelves, ahead of the per-view Recently Added
//! rows, and a failed NextUp must not take the source down with it.

#![cfg(all(test, feature = "jellyfin"))]

use super::*;
use std::io::{Read, Write};

/// One canned response per connection, in order — the same bargain the jellyfin client tests
/// strike with their private MockServer.
struct OneShot {
    port: u16,
    join: Option<std::thread::JoinHandle<()>>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl OneShot {
    fn start(responses: Vec<(i32, String)>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        let join = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().expect("accept");
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = socket.read(&mut chunk).expect("read");
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                seen.lock().unwrap().push(String::from_utf8_lossy(&buf).into_owned());
                let reason = if status == 200 { "OK" } else { "Error" };
                write!(socket, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    .expect("write");
            }
        });
        Self { port, join: Some(join), requests }
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(j) = self.join.take() {
            j.join().expect("server thread");
        }
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

const AUTH_OK: &str = r#"{"User":{"Id":"u-1","Name":"demo"},"AccessToken":"tok-1","ServerId":"srv-1","SessionInfo":null}"#;
const EPISODE: &str = r#"{"Id":"ep9","Name":"The Next One","Type":"Episode","SeriesId":"series9","SeriesName":"The Show","ParentIndexNumber":1,"IndexNumber":9,"ImageTags":{"Primary":"etag"},"BackdropImageTags":[],"UserData":{"Played":false,"PlayCount":0,"PlaybackPositionTicks":0}}"#;
const MOVIE: &str = r#"{"Id":"m1","Name":"A Film","Type":"Movie","ImageTags":{"Primary":"etag"},"BackdropImageTags":[],"UserData":{"Played":false,"PlayCount":0,"PlaybackPositionTicks":0}}"#;

fn installed_client(port: u16) -> &'static crate::jellyfin::JfClient {
    let client = crate::jellyfin::JfClient::new(
        crate::plex::Origin::http("127.0.0.1", port as i32),
        "dev-1".into(),
    );
    client.authenticate_by_name("demo", "").unwrap();
    crate::jellyfin::install(client);
    crate::jellyfin::client().expect("installed above")
}

#[test]
fn next_up_leads_the_shelves_and_a_failed_nextup_kills_nothing() {
    let _guard = crate::testlock::serial();
    // Auth, Resume (empty), NextUp (one episode), Views (one movies view), Latest (one movie).
    let server = OneShot::start(vec![
        (200, AUTH_OK.into()),
        (200, r#"{"Items":[],"TotalRecordCount":0}"#.into()),
        (200, format!(r#"{{"Items":[{EPISODE}],"TotalRecordCount":1}}"#)),
        (200, r#"{"Items":[{"Id":"v1","Name":"Movies","CollectionType":"movies"}]}"#.into()),
        (200, format!(r#"{{"Items":[{MOVIE}],"TotalRecordCount":1}}"#)),
    ]);
    let c = installed_client(server.port);
    let build = fetch_source_jellyfin(c, ServerId::from_raw(0)).expect("Resume answered");
    crate::jellyfin::uninstall();
    let requests = server.finish();

    assert!(build.cw.is_empty());
    assert_eq!(build.shelves.len(), 2, "Next Up plus the view's Latest");
    assert_eq!(build.shelves[0].hub_id, "jf.nextup");
    assert_eq!(build.shelves[0].items[0].rk, "ep9");
    assert!(build.shelves[1].hub_id.starts_with("jf.latest."));
    let nextup_req = requests
        .iter()
        .find(|r| r.contains("/Shows/NextUp?"))
        .expect("a NextUp request happened");
    assert!(!nextup_req.contains("SeriesId="), "the shelf asks for EVERY series: {nextup_req}");
}

#[test]
fn a_failed_nextup_skips_the_shelf_not_the_source() {
    let _guard = crate::testlock::serial();
    // Auth, Resume (empty), NextUp (HTTP 500), Views (one movies view), Latest (one movie).
    let server = OneShot::start(vec![
        (200, AUTH_OK.into()),
        (200, r#"{"Items":[],"TotalRecordCount":0}"#.into()),
        (500, "{}".into()),
        (200, r#"{"Items":[{"Id":"v1","Name":"Movies","CollectionType":"movies"}]}"#.into()),
        (200, format!(r#"{{"Items":[{MOVIE}],"TotalRecordCount":1}}"#)),
    ]);
    let c = installed_client(server.port);
    let build = fetch_source_jellyfin(c, ServerId::from_raw(0)).expect("Resume answered");
    crate::jellyfin::uninstall();
    server.finish();

    assert_eq!(build.shelves.len(), 1, "only the Latest shelf survives");
    assert!(build.shelves[0].hub_id.starts_with("jf.latest."));
}
