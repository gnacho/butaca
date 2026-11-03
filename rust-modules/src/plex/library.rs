//! Library operations (impl Client): the guid lookup and item metadata reads that still have
//! live callers, plus the part-level direct-play target. (The part's stream-selection PUT went
//! with the Plex transcoder slice: its one caller, route's `put_selection`, is gone.)
use super::client::{Client, QueryBuilder, StreamUrl};
use crate::media::{MediaContainer, Metadata};

fn sidecar_key_allowed(key: &str) -> bool {
    let Some(tail) = key.strip_prefix("/library/streams/") else { return false; };
    let (id, ext) = tail.split_once('.').unwrap_or((tail, ""));
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
        && ext.len() <= 10 && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Client {
    /// GET /library/all?guid=… — **does THIS server hold this film, and under which key?**
    ///
    /// The one query in the app that crosses libraries rather than naming one, which is why the
    /// rows it returns carry `librarySectionTitle`: the caller does not know in advance which
    /// library will answer, and "Also available" names the library, not the machine.
    ///
    /// `None` is a transport/parse failure; `Some` with an empty `metadata` is the server
    /// answering *"I do not have it"*, and the two must not be collapsed — a share that is merely
    /// unreachable would otherwise read as one that does not hold the film, which is a row silently
    /// missing from the panel rather than a source visibly not answering.
    ///
    /// Verified live against both of this household's servers, 2026-08-14: `size=0` for a film only
    /// ours holds, `size=1` for one both hold — returning the SHARE's own `ratingKey` and its own
    /// localized title for the same guid.
    /// **`type` is what makes the answer carry `Media[]`**, and without it the row has no quality to
    /// show. Measured against this server 2026-08-14, same guid three ways:
    ///
    /// ```text
    /// ?guid=…               size 1, no Media
    /// ?guid=…&includeMedia=1 size 1, no Media   (not the knob it looks like)
    /// ?guid=…&type=1        size 1, Media[0] = 4k 3840x2160
    /// ```
    ///
    /// The type comes off the guid itself (`plex://movie/…`), which is the only place it is known
    /// without another round trip — and a guid whose kind we do not recognise sends no `type` at
    /// all rather than guessing 1, because a wrong type answers `size 0` and would read as "that
    /// server does not have it".
    #[allow(dead_code)] // frozen until the wire layer goes
    pub fn find_by_guid(&self, guid: &str) -> Option<MediaContainer> {
        if guid.is_empty() {
            return None;
        }
        let mut q = QueryBuilder::new("/library/all".to_string()).str("guid", guid);
        if let Some(t) = guid_type(guid) {
            q = q.int("type", t);
        }
        self.get_json(&q.build())
    }


    /// GET /library/metadata/{rating_key} → the single item (`.metadata[0]`), or None.
    /// `includeChapters=1` / `includeMarkers=1` — PMS omits BOTH the `Chapter[]` and `Marker[]`
    /// arrays from the default response. Markers drive the in-player Skip Intro / Skip Credits
    /// prompt, so they ride the detail fetch rather than costing a second round trip.
    ///
    /// `includeOnDeck=1` adds, for a SHOW, the one episode the server considers next to watch —
    /// `OnDeck.Metadata`, a whole episode record (thumb, summary, `Media`, `viewOffset`). It is the
    /// only show-level answer to "what's next": the client otherwise holds ONE season's episodes at a
    /// time, so anything it computed itself would change with the selected season tab. Verified live
    /// 2026-07-30 on rk 437 → S2E2 at 635510/3130720 while season 1 was the loaded tab.
    pub fn metadata(&self, rating_key: &str) -> Option<Metadata> {
        let path = QueryBuilder::new(format!("/library/metadata/{rating_key}"))
            .int("includeChapters", 1)
            .int("includeMarkers", 1)
            .int("includeOnDeck", 1)
            .build();
        self.get_json(&path)?.metadata.into_iter().next()
    }


    /// Fetch a SIDECAR subtitle (`Stream.key`, i.e. `/library/streams/{id}`) for the client
    /// renderer. The endpoint takes `encoding` and `format` (the upstream Plex API reference). ASS/SSA
    /// requests only UTF-8 re-encoding, preserving styles, drawings and overlapping events.
    /// Other text formats request UTF-8 SubRip so formats such as SAMI keep their conversion
    /// path. Conversion is the server's and has been seen refusing a FORMAT before (`.vtt` → 501):
    /// without `format` PMS re-encodes only, and the bare key is the file as it lies on disk.
    /// `player::sidecar` retains styled scripts and parses plain captions from the response.
    #[allow(dead_code)] // frozen until the wire layer goes
    pub fn sidecar_subtitle(&self, key: &str, codec: &str) -> Option<Vec<u8>> {
        if !sidecar_key_allowed(key) {
            return None; // a key is server data: only ever the path this method is for
        }
        let sep = if key.contains('?') { '&' } else { '?' };
        let mut paths = Vec::with_capacity(3);
        if !codec.eq_ignore_ascii_case("ass") && !codec.eq_ignore_ascii_case("ssa") {
            paths.push(format!("{key}{sep}encoding=utf-8&format=srt"));
        }
        paths.push(format!("{key}{sep}encoding=utf-8"));
        paths.push(key.to_string());
        paths.iter().find_map(|path| self.get_sidecar_bytes(path).filter(|b| !b.is_empty()))
    }

    /// The direct-play stream target: the raw part `key` GET, carrying the per-playback
    /// session id + identity so PMS keys the /status/sessions entry by session (not a
    /// token= fallback), keeping the timeline correlation consistent.
    ///
    /// `part_key` may already contain a query. Library parts do not; IVA extras do
    /// (`/services/iva/assets?…`). [`QueryBuilder`] joins onto that query instead of
    /// writing a second `?`.
    pub fn direct_play_url(&self, part_key: &str, session: &str) -> StreamUrl {
        let q = QueryBuilder::new(part_key).str("X-Plex-Session-Identifier", session);
        let path = self.playback_identity(q).build();
        StreamUrl {
            origin: self.origin.clone(),
            path: self.with_token(&path),
        }
    }
}

/// PMS's numeric `type` for a `plex://<kind>/<id>` guid — the metadata provider's own kind, which is
/// the one thing about an item a guid states outright.
///
/// `None` for a kind this app has no number for, and the caller then omits the parameter: a WRONG
/// type answers `size 0`, which is indistinguishable from "that server does not hold this item" and
/// would quietly drop a real copy out of "Also available".
fn guid_type(guid: &str) -> Option<i64> {
    let kind = guid.strip_prefix("plex://")?.split('/').next()?;
    match kind {
        "movie" => Some(1),
        "show" => Some(2),
        "season" => Some(3),
        "episode" => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "devtriggers")]
    #[test]
    fn sidecar_download_preserves_ass_and_keeps_other_format_conversion() {
        use std::io::{Read, Write};
        use std::time::{Duration, Instant};
        let script = b"[Script Info]\nScriptType: v4.00+\n[Events]\n\
            Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\pos(300,100)}SIGN\n";
        let converted = b"1\n00:00:01,000 --> 00:00:03,000\nSIGN\n";
        // SSA also exercises a PMS that refuses re-encoding: the original file is the fallback,
        // never a style-destroying SubRip request. SAMI still needs its existing conversion.
        for (codec, failed_requests, styled) in [("ass", 0, true), ("SSA", 1, true), ("sami", 0, false)] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                let mut requests = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(3);
                while requests.len() <= failed_requests && Instant::now() < deadline {
                    let Ok((mut socket, _)) = crate::testnet::accept(&listener) else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut request = [0; 8192];
                    let n = socket.read(&mut request).unwrap();
                    let request = String::from_utf8_lossy(&request[..n]).into_owned();
                    let body: &[u8] = if request.contains("format=srt") { converted } else { script };
                    let status = if requests.len() < failed_requests { "501 Not Implemented" } else { "200 OK" };
                    write!(socket, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                    socket.write_all(body).unwrap();
                    requests.push(request);
                }
                requests
            });
            let client = Client::new(
                crate::plex::ServerId::UNSET, "fixture",
                crate::plex::Origin::http("127.0.0.1", port as i32), "", "cid",
            );
            // The source has no extension: metadata's codec must determine download policy.
            let body = client.sidecar_subtitle("/library/streams/42", codec);
            let requests = server.join().unwrap();
            let expected: &[u8] = if styled { script } else { converted };
            assert_eq!(body.as_deref(), Some(expected), "{codec}: {requests:?}");
            assert_eq!(requests.len(), failed_requests + 1);
            assert!(requests[0].contains("encoding=utf-8"));
            if styled {
                assert!(requests.iter().all(|request| !request.contains("format=srt")));
            }
            if failed_requests > 0 {
                assert!(!requests.last().unwrap().contains("encoding="), "retry the original file");
            }
        }
    }

    #[test]
    fn sidecar_download_refuses_path_traversal() {
        for key in ["/library/streams/../../identity", "/library/streams/%2e%2e/identity",
                    "/library/streams/1?path=/identity", "/library/streams/1#fragment"] {
            assert!(!sidecar_key_allowed(key), "{key}");
        }
        assert!(sidecar_key_allowed("/library/streams/123"));
        assert!(sidecar_key_allowed("/library/streams/123.srt"));
    }

    /// The numbers are PMS's, and the mapping is the only thing standing between "Also available"
    /// showing a quality badge and showing none — `type` is what makes `/library/all?guid=…` return
    /// `Media[]` at all (see `find_by_guid`).
    #[test]
    fn a_guid_states_its_own_kind_and_an_unknown_kind_sends_no_type() {
        assert_eq!(guid_type("plex://movie/6856893830a4aaafd5c4291d"), Some(1));
        assert_eq!(guid_type("plex://show/5d9c081b170e05001f303f9e"), Some(2));
        assert_eq!(guid_type("plex://season/abc"), Some(3));
        assert_eq!(guid_type("plex://episode/abc"), Some(4));
        // an agent guid from before the plex:// scheme, and a kind with no level here: no type
        // rather than a guess
        assert_eq!(
            guid_type("com.plexapp.agents.imdb://tt0083658?lang=en"),
            None
        );
        assert_eq!(guid_type("plex://artist/abc"), None);
        assert_eq!(guid_type(""), None);
    }
}
