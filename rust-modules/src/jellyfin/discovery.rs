//! Finding a Jellyfin server on the LAN, so the address field can fill itself.
//!
//! Jellyfin answers a UDP broadcast of `Who is JellyfinServer?` on port 7359 with one JSON object
//! per server: `{"Address":"http://192.168.1.20:8096","Id":"...","Name":"Home",...}`. The question
//! is sent a few times because a broadcast can be dropped, and an answer is kept once per `Id`.
//!
//! The screen never waits on this: [`start`] runs the question on a worker and parks what answered
//! in [`take`]. A failure is the ordinary case (the server may be remote, the network may filter
//! broadcasts), so it is silence, not an error line.

use serde::Deserialize;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The port Jellyfin listens on for the LAN question.
const PORT: u16 = 7359;
const QUESTION: &[u8] = b"Who is JellyfinServer?";
/// How long the question is asked before the answer is taken as complete.
const WINDOW: Duration = Duration::from_millis(1500);
/// How often the question is repeated inside the window.
const ASK_EVERY: Duration = Duration::from_millis(700);
/// One read's wait. Short, so the loop can re-ask inside the window.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// One server that answered.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Found {
    pub address: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// One answer as a server, or `None` for anything else: the question echoed back, another service
/// on the port, or an object without the two fields that identify a server.
pub(crate) fn parse(datagram: &[u8]) -> Option<Found> {
    let f: Found = serde_json::from_slice(datagram).ok()?;
    (!f.address.is_empty() && !f.id.is_empty()).then_some(f)
}

/// Ask `target` for servers until `wait` runs out, first answer first, one per `Id`.
fn collect(sock: &UdpSocket, target: SocketAddr, wait: Duration) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let started = Instant::now();
    let mut next_ask = started;
    let mut buf = [0u8; 2048];
    while started.elapsed() < wait {
        if Instant::now() >= next_ask {
            let _ = sock.send_to(QUESTION, target);
            next_ask += ASK_EVERY;
        }
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(f) = parse(&buf[..n]) {
                    if !found.iter().any(|g| g.id == f.id) {
                        found.push(f);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    found
}

/// Broadcast the question for [`WINDOW`]: what answered, or nothing.
pub(crate) fn discover() -> Vec<Found> {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else {
        return Vec::new();
    };
    if sock.set_broadcast(true).is_err() {
        return Vec::new();
    }
    let _ = sock.set_read_timeout(Some(READ_TIMEOUT));
    collect(&sock, SocketAddr::from(([255, 255, 255, 255], PORT)), WINDOW)
}

/// Starts at most one question for the life of the process.
static ASKED: AtomicBool = AtomicBool::new(false);
static FOUND: Mutex<Option<Vec<Found>>> = Mutex::new(None);

/// Ask the LAN once. Callers may call this on every screen entry: the second call is a no-op.
pub(crate) fn start() {
    if ASKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = crate::task::spawn_small("jellyfin-discovery", || {
        let found = discover();
        if !found.is_empty() {
            crate::log(&format!(
                "jellyfin: discovery found {} server(s): {}",
                found.len(),
                found
                    .iter()
                    .map(|f| f.address.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Ok(mut slot) = FOUND.lock() {
            *slot = Some(found);
        }
        crate::ui::idle::invalidate();
    });
    // A refused thread releases the latch: the next entry may try again instead of the screen
    // waiting on an answer that will never come.
    if !spawned {
        ASKED.store(false, Ordering::SeqCst);
    }
}

/// The answer, once. `None` while the question is in flight or after it was taken.
pub(crate) fn take() -> Option<Vec<Found>> {
    FOUND.lock().ok().and_then(|mut slot| slot.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_becomes_the_server_it_names() {
        let f = parse(br#"{"Address":"http://192.168.1.20:8096","Id":"abc","Name":"Casa"}"#)
            .expect("a server answer parses");
        assert_eq!(f.address, "http://192.168.1.20:8096");
        assert_eq!(f.id, "abc");
        assert_eq!(f.name, "Casa");
    }

    #[test]
    fn the_question_echoed_back_is_not_a_server() {
        assert_eq!(parse(QUESTION), None);
    }

    #[test]
    fn an_object_without_a_server_identity_is_not_a_server() {
        assert_eq!(parse(br#"{"Address":"http://192.168.1.20:8096"}"#), None);
        assert_eq!(parse(br#"{"Id":"abc","Name":"Casa"}"#), None);
        assert_eq!(parse(br#"{"Address":"","Id":"abc"}"#), None);
    }

    /// Two servers that answer are both reported, and an answer repeated by the same `Id` is not
    /// reported twice: the field offers one address, and a duplicate would read as a second server.
    #[test]
    fn every_server_that_answers_is_reported_once() {
        let responder = UdpSocket::bind(("127.0.0.1", 0)).expect("responder");
        responder
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("responder timeout");
        let target = responder.local_addr().expect("responder address");
        let answers: [&[u8]; 3] = [
            br#"{"Address":"http://127.0.0.1:8096","Id":"a","Name":"Casa"}"#,
            br#"{"Address":"http://127.0.0.1:8096","Id":"a","Name":"Casa"}"#,
            br#"{"Address":"http://127.0.0.1:8097","Id":"b","Name":"Oficina"}"#,
        ];
        let thread = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            for _ in 0..4 {
                let Ok((_, from)) = responder.recv_from(&mut buf) else {
                    break;
                };
                for answer in answers {
                    let _ = responder.send_to(answer, from);
                }
            }
        });

        let sock = UdpSocket::bind(("127.0.0.1", 0)).expect("asker");
        sock.set_read_timeout(Some(Duration::from_millis(50)))
            .expect("asker timeout");
        let found = collect(&sock, target, Duration::from_millis(300));
        let _ = thread.join();

        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "{found:?}");
    }
}
