//! A one-response-per-connection HTTP mock for the Jellyfin tests. Shared by the client
//! suite and the pms fetch projection tests. It drains the whole request — head AND the
//! Content-Length'd body — before responding: a body left unread RSTs the socket under the
//! client's response read on some kernels, which is the flake class the pms fetch tests hit
//! with a headers-only mock.

#![cfg(test)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub(crate) struct MockServer {
    pub(crate) port: u16,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    pub(crate) fn start<S: Into<String> + Send + 'static>(responses: Vec<(i32, S)>) -> MockServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests2 = requests.clone();
        let join = std::thread::spawn(move || {
            for (status, body) in responses {
                let body = body.into();
                let (mut socket, _) = listener.accept().expect("accept");
                // Read head AND the Content-Length'd body — the whole point of the fixture
                // is to see what the transport put on the wire, and leaving the body unread
                // would RST the socket under the client's response read on some kernels.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut content_length = None::<usize>;
                let mut head_end = None::<usize>;
                loop {
                    let n = socket.read(&mut chunk).expect("read");
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if head_end.is_none() {
                        if let Some(pos) = find(&buf, b"\r\n\r\n") {
                            head_end = Some(pos + 4);
                            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                            content_length = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse().ok());
                        }
                    }
                    if let (Some(he), Some(cl)) = (head_end, content_length) {
                        if buf.len() >= he + cl {
                            break;
                        }
                    } else if head_end.is_some() && content_length.is_none() {
                        break;
                    }
                }
                requests2
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).into_owned());
                let reason = if status == 200 { "OK" } else { "Error" };
                write!(socket, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    .expect("write");
            }
        });
        MockServer {
            port,
            requests,
            join: Some(join),
        }
    }

    pub(crate) fn finish(self) -> Vec<String> {
        let mut this = self;
        if let Some(j) = this.join.take() {
            j.join().expect("server thread");
        }
        let recorded = std::mem::take(&mut *this.requests.lock().unwrap());
        recorded
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
