//! WebSocket wire format for the Jellyfin session socket.

use crate::jellyfin::client::JfClient;
use crate::net::websocket::RawConnection;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::time::{Duration, Instant};

const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub(super) fn websocket_accept(key: &str) -> String {
    let mut input = String::with_capacity(key.len() + WEBSOCKET_GUID.len());
    input.push_str(key);
    input.push_str(WEBSOCKET_GUID);
    base64(&sha1(input.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HandshakeError;

pub(super) fn validate_handshake(bytes: &[u8], key: &str) -> Result<(), HandshakeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| HandshakeError)?;
    let mut lines = text.split("\r\n");
    let status = lines.next().ok_or(HandshakeError)?;
    let mut status_parts = status.split_whitespace();
    if status_parts.next() != Some("HTTP/1.1") || status_parts.next() != Some("101") {
        return Err(HandshakeError);
    }
    let headers: Vec<(&str, &str)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name, value.trim()))
        .collect();
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| *value)
    };
    if !header("Upgrade").is_some_and(|value| value.eq_ignore_ascii_case("websocket")) {
        return Err(HandshakeError);
    }
    if !header("Connection").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
    }) {
        return Err(HandshakeError);
    }
    if header("Sec-WebSocket-Accept") != Some(websocket_accept(key).as_str()) {
        return Err(HandshakeError);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) enum WsError {
    Io(io::Error),
    Handshake,
    Frame(FrameError),
    CredentialRefused,
}

impl From<io::Error> for WsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FrameError> for WsError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

pub(super) struct WebSocket {
    connection: RawConnection,
    decoder: Decoder,
    pending: VecDeque<Frame>,
}

impl WebSocket {
    pub(super) fn connect(client: &JfClient) -> Result<Self, WsError> {
        let authorization = client
            .play_on_authorization()
            .ok_or(WsError::CredentialRefused)?;
        let headers = [authorization.as_str()];
        if !crate::http::credential_transport_allowed(client.origin(), "/socket", &headers) {
            return Err(WsError::CredentialRefused);
        }
        let mut connection = RawConnection::connect(client.origin())?;
        let mut nonce = [0_u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let key = base64(&nonce);
        let request = format!(
            "GET /socket HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n{}\r\n\r\n",
            client.origin().authority(),
            key,
            authorization
        );
        connection.send_all(request.as_bytes(), Duration::from_secs(4))?;
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut response = Vec::new();
        let header_end = loop {
            if let Some(end) = find_subslice(&response, b"\r\n\r\n").map(|start| start + 4) {
                break end;
            }
            if response.len() >= 16 * 1024 {
                return Err(WsError::Handshake);
            }
            let timeout = remaining(deadline)?;
            let mut chunk = [0_u8; 2048];
            let read = connection.read_some(&mut chunk, timeout)?;
            if read == 0 {
                return Err(WsError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "handshake EOF",
                )));
            }
            response.extend_from_slice(&chunk[..read]);
        };
        validate_handshake(&response[..header_end], &key).map_err(|_| WsError::Handshake)?;
        let mut decoder = Decoder::default();
        let pending = decoder.push(&response[header_end..])?.into();
        Ok(Self {
            connection,
            decoder,
            pending,
        })
    }

    pub(super) fn fd(&self) -> std::os::raw::c_int {
        self.connection.fd()
    }

    pub(super) fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        self.send_frame(0x01, text.as_bytes())
    }

    pub(super) fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut mask = [0_u8; 4];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut mask)?;
        let frame = encode_client_frame(opcode, payload, mask);
        self.connection
            .send_all(&frame, Duration::from_secs(4))?;
        Ok(())
    }

    pub(super) fn next_frame(&mut self, timeout: Duration) -> Result<Frame, WsError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Ok(frame);
            }
            let mut chunk = [0_u8; 4096];
            let read = self.connection.read_some(&mut chunk, remaining(deadline)?)?;
            if read == 0 {
                return Err(WsError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "WebSocket EOF",
                )));
            }
            self.pending.extend(self.decoder.push(&chunk[..read])?);
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "WebSocket deadline"))
}

pub(super) fn encode_client_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    assert!(payload.len() <= 125, "short WebSocket frame only");
    let mut frame = Vec::with_capacity(payload.len() + 6);
    frame.push(0x80 | (opcode & 0x0f));
    frame.push(0x80 | payload.len() as u8);
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, byte)| byte ^ mask[i % 4]));
    frame
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Frame {
    Text(Vec<u8>),
    Ping(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FrameError {
    Protocol(&'static str),
    MessageTooLarge,
}

const MAX_MESSAGE_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct Decoder {
    input: Vec<u8>,
    fragmented_text: Option<Vec<u8>>,
}

impl Decoder {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, FrameError> {
        self.input.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut consumed = 0;
        loop {
            let Some(header) = self.input.get(consumed..consumed + 2) else {
                break;
            };
            let first = header[0];
            let fin = first & 0x80 != 0;
            let opcode = first & 0x0f;
            if first & 0x70 != 0 {
                return Err(FrameError::Protocol("RSV bits are unsupported"));
            }
            if opcode & 0x08 != 0 && !fin {
                return Err(FrameError::Protocol("fragmented control frame"));
            }
            if header[1] & 0x80 != 0 {
                return Err(FrameError::Protocol("server frames must not be masked"));
            }
            let length_code = header[1] & 0x7f;
            if opcode & 0x08 != 0 && length_code > 125 {
                return Err(FrameError::Protocol("oversized control frame"));
            }
            let mut header_len = 2;
            let len = match length_code {
                126 => {
                    let Some(extended) = self.input.get(consumed + 2..consumed + 4) else {
                        break;
                    };
                    header_len = 4;
                    u16::from_be_bytes([extended[0], extended[1]]) as usize
                }
                127 => {
                    let Some(extended) = self.input.get(consumed + 2..consumed + 10) else {
                        break;
                    };
                    header_len = 10;
                    let raw = u64::from_be_bytes([
                        extended[0],
                        extended[1],
                        extended[2],
                        extended[3],
                        extended[4],
                        extended[5],
                        extended[6],
                        extended[7],
                    ]);
                    usize::try_from(raw)
                        .map_err(|_| FrameError::Protocol("length does not fit usize"))?
                }
                len => len as usize,
            };
            if opcode & 0x08 == 0 && len > MAX_MESSAGE_BYTES {
                return Err(FrameError::MessageTooLarge);
            }
            if self.input.len() < consumed + header_len + len {
                break;
            }
            let start = consumed + header_len;
            let payload = &self.input[start..start + len];
            match opcode {
                0x0 => {
                    let Some(text) = self.fragmented_text.as_mut() else {
                        return Err(FrameError::Protocol("continuation without text"));
                    };
                    if text.len().saturating_add(payload.len()) > MAX_MESSAGE_BYTES {
                        return Err(FrameError::MessageTooLarge);
                    }
                    text.extend_from_slice(payload);
                    if fin {
                        frames.push(Frame::Text(self.fragmented_text.take().unwrap_or_default()));
                    }
                }
                0x1 => {
                    if self.fragmented_text.is_some() {
                        return Err(FrameError::Protocol("text interrupted a fragmented message"));
                    }
                    if fin {
                        frames.push(Frame::Text(payload.to_vec()));
                    } else {
                        self.fragmented_text = Some(payload.to_vec());
                    }
                }
                0x9 => frames.push(Frame::Ping(payload.to_vec())),
                _ => return Err(FrameError::Protocol("unsupported opcode")),
            }
            consumed = start + len;
        }
        self.input.drain(..consumed);
        Ok(frames)
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(ALPHABET[(a >> 2) as usize] as char);
        out.push(ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(c & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn sha1(bytes: &[u8]) -> [u8; 20] {
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((bytes.len() + 72) & !63);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut h = [
        0x6745_2301_u32,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for chunk in padded.chunks_exact(64) {
        let mut w = [0_u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *state = state.wrapping_add(value);
        }
    }
    let mut out = [0_u8; 20];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}
