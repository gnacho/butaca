//! Jellyfin Play On receiver: authenticated session capabilities, WebSocket commands,
//! and main-thread playback intents.

mod adapter;
pub(crate) use adapter::{PlayOnAdapter, RemoteCommand};
mod prepare;
pub(crate) use prepare::PreparedPlay;
pub(crate) mod protocol;
mod websocket;

#[cfg(test)]
mod tests {
    #[test]
    fn play_on_transcode_maps_explicit_subtitle_off_to_minus_one() {
        let tracks = |sub_sid| crate::route::StartTracks {
            audio_sid: 0,
            sub_sid,
        };
        assert_eq!(crate::route::playback_info_subtitle_index(tracks(-1)), None);
        assert_eq!(crate::route::playback_info_subtitle_index(tracks(0)), Some(-1));
        assert_eq!(crate::route::playback_info_subtitle_index(tracks(5)), Some(5));
    }

    #[test]
    fn play_on_external_or_unknown_subtitles_force_transcode() {
        let embedded = crate::metadata::Stream {
            id: 5,
            index: 5,
            codec: "subrip".into(),
            external: false,
            ..Default::default()
        };
        let external = crate::metadata::Stream {
            external: true,
            ..embedded.clone()
        };
        let pick = |sub_sid| crate::route::StartTracks {
            audio_sid: 0,
            sub_sid,
        };
        assert!(!crate::route::requested_subtitle_requires_transcode(
            std::slice::from_ref(&embedded),
            pick(5),
        ));
        assert!(crate::route::requested_subtitle_requires_transcode(
            std::slice::from_ref(&external),
            pick(5),
        ));
        assert!(crate::route::requested_subtitle_requires_transcode(
            std::slice::from_ref(&embedded),
            pick(99),
        ));
        assert!(!crate::route::requested_subtitle_requires_transcode(
            std::slice::from_ref(&embedded),
            pick(0),
        ));
    }

    #[test]
    fn play_on_quality_uses_the_selected_source_bitrate() {
        let mut selected = crate::metadata::PlayingItem::with_subs(Vec::new());
        selected.bitrate = 20_000;
        assert_eq!(
            crate::route::effective_source_kbps(3_000, Some(&selected), true),
            20_000
        );
        assert_eq!(
            crate::route::effective_source_kbps(3_000, Some(&selected), false),
            3_000
        );
        assert_eq!(crate::route::effective_source_kbps(3_000, None, true), 3_000);
    }

    #[test]
    fn play_on_playstate_waits_for_a_pending_remote_player() {
        assert!(crate::app::play_on::should_defer_playstate(false, true, false));
        assert!(crate::app::play_on::should_defer_playstate(false, false, true));
        assert!(!crate::app::play_on::should_defer_playstate(false, false, false));
        assert!(!crate::app::play_on::should_defer_playstate(true, false, false));
    }

    #[test]
    fn play_on_poll_prioritizes_pending_io_over_hup() {
        assert!(crate::net::websocket::has_ready_event(
            libc::POLLIN | libc::POLLHUP,
            libc::POLLIN,
        ));
        assert!(!crate::net::websocket::has_ready_event(
            libc::POLLHUP,
            libc::POLLIN,
        ));
    }

    #[test]
    fn play_on_worker_posts_capabilities_and_stops_with_the_client() {
        use std::io::{Read, Write};
        use std::time::Duration;
        let _serial = crate::testlock::serial();
        assert!(crate::net::global_init());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = crate::origin::Origin::http("127.0.0.1", i32::from(port));
        crate::jellyfin::install(crate::jellyfin::client::JfClient::synthetic(
            origin,
            "device".to_owned(),
            "synthetic-token".to_owned(),
            "user".to_owned(),
            "Guest".to_owned(),
        ));
        let (posted_tx, posted_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            fn read_head(socket: &mut std::net::TcpStream) -> Vec<u8> {
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 2048];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = socket.read(&mut chunk).unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&chunk[..read]);
                }
                bytes
            }
            let (mut websocket, _) = crate::testnet::accept(&listener).unwrap();
            websocket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let request = read_head(&mut websocket);
            let request_text = String::from_utf8(request).unwrap();
            let key = request_text
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .trim();
            let accept = super::websocket::websocket_accept(key);
            write!(
                websocket,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .unwrap();

            let (mut capabilities, _) = crate::testnet::accept(&listener).unwrap();
            capabilities.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = read_head(&mut capabilities);
            let header_end = request.windows(4).position(|window| window == b"\r\n\r\n").unwrap() + 4;
            let head = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("Content-Length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 2048];
                let read = capabilities.read(&mut chunk).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
            }
            assert!(head.starts_with("POST /Sessions/Capabilities/Full HTTP/1.1\r\n"));
            assert!(head.contains("Authorization: MediaBrowser Token=\"synthetic-token\""));
            let body: serde_json::Value = serde_json::from_slice(
                &request[header_end..header_end + content_length],
            )
            .unwrap();
            assert_eq!(body["PlayableMediaTypes"], serde_json::json!(["Video"]));
            capabilities
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            posted_tx.send(()).unwrap();

            let mut byte = [0_u8; 1];
            let read = websocket.read(&mut byte).unwrap();
            assert_eq!(read, 0, "retiring the client closes the session socket");
        });
        let client = crate::jellyfin::client().unwrap();
        let mut adapter = super::PlayOnAdapter::default();
        adapter.sync(Some(client), true);
        posted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        adapter.sync(None, true);
        adapter.shutdown();
        crate::jellyfin::uninstall();
        server.join().unwrap();
    }

    #[test]
    fn play_on_playstate_uses_existing_player_requests() {
        use crate::jellyfin::play_on::protocol::Playstate;
        use crate::screens::registry::PlayerReq;
        assert_eq!(
            crate::app::play_on::playstate_request(Playstate::Pause),
            Some(PlayerReq::Transport(Some(false)))
        );
        assert_eq!(
            crate::app::play_on::playstate_request(Playstate::Unpause),
            Some(PlayerReq::Transport(Some(true)))
        );
        assert_eq!(
            crate::app::play_on::playstate_request(Playstate::PlayPause),
            Some(PlayerReq::Transport(None))
        );
        assert_eq!(
            crate::app::play_on::playstate_request(Playstate::Stop),
            Some(PlayerReq::Exit)
        );
        assert_eq!(
            crate::app::play_on::playstate_request(Playstate::Seek(42)),
            Some(PlayerReq::CommitSeek(42))
        );
    }

    #[test]
    fn play_on_reconnect_backoff_is_bounded_and_resets() {
        let mut backoff = super::adapter::Backoff::default();
        let delays: Vec<u64> = (0..7).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30]);
        backoff.reset();
        assert_eq!(backoff.next_delay().as_secs(), 1);
    }

    #[test]
    fn play_on_preparation_uses_one_source_for_codecs_and_tracks() {
        let item: crate::jellyfin::dto::BaseItemDto = serde_json::from_str(r#"{
            "Id":"movie",
            "Name":"Remote film",
            "Type":"Movie",
            "RunTimeTicks":36000000000,
            "MediaSources":[
                {
                    "Id":"default-source",
                    "Container":"mp4",
                    "MediaStreams":[
                        {"Index":0,"Codec":"h264","Type":"Video","Width":1920,"Height":1080},
                        {"Index":1,"Codec":"aac","Type":"Audio"}
                    ]
                },
                {
                    "Id":"alternate-source",
                    "Container":"mkv",
                    "Bitrate":20000000,
                    "MediaStreams":[
                        {"Index":0,"Codec":"hevc","Type":"Video","Width":3840,"Height":2160},
                        {"Index":7,"Codec":"eac3","Type":"Audio","Language":"eng","Title":"Director"},
                        {"Index":9,"Codec":"subrip","Type":"Subtitle","Language":"spa"}
                    ]
                }
            ]
        }"#)
        .unwrap();
        let request = super::protocol::PlayRequest {
            item_id: "movie".to_owned(),
            media_source_id: Some("alternate-source".to_owned()),
            start_position_ns: 4_000_000_000,
            audio_stream_index: Some(7),
            subtitle_stream_index: Some(-1),
        };
        let prepared = super::prepare::from_dto(&item, &request, Vec::new()).unwrap();
        assert_eq!(prepared.movie.part, "alternate-source");
        assert_eq!((prepared.movie.vcodec.as_str(), prepared.movie.acodec.as_str()), ("hevc", "eac3"));
        assert_eq!(prepared.playing.container, "mkv");
        assert_eq!((prepared.playing.width, prepared.playing.height), (3840, 2160));
        assert_eq!(prepared.playing.audio.iter().map(|stream| stream.id).collect::<Vec<_>>(), vec![7]);
        assert_eq!(prepared.playing.subs.iter().map(|stream| stream.id).collect::<Vec<_>>(), vec![9]);
        assert_eq!(prepared.start_position_ns, 4_000_000_000);
        assert_eq!(prepared.tracks.audio_sid, 7);
        assert_eq!(prepared.tracks.sub_sid, 0);
    }

    #[test]
    fn play_on_websocket_authenticates_and_decodes_a_server_command() {
        use std::io::{Read, Write};
        use std::time::Duration;
        let _serial = crate::testlock::serial();
        assert!(crate::net::global_init());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = crate::origin::Origin::http("127.0.0.1", i32::from(port));
        let client = crate::jellyfin::client::JfClient::synthetic(
            origin.clone(),
            "device".to_owned(),
            "synthetic-token".to_owned(),
            "user".to_owned(),
            "Guest".to_owned(),
        );
        crate::jellyfin::install(client);
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = crate::testnet::accept(&listener).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            let text = String::from_utf8(request).unwrap();
            let key = text
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .trim();
            let accept = super::websocket::websocket_accept(key);
            request_tx.send(text).unwrap();
            write!(
                socket,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .unwrap();
            let command = br#"{"MessageType":"Playstate","Data":{"Command":"Pause"}}"#;
            socket.write_all(&[0x81, command.len() as u8]).unwrap();
            socket.write_all(command).unwrap();
        });
        let client = crate::jellyfin::client().unwrap();
        let mut socket = super::websocket::WebSocket::connect(client).unwrap();
        let request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(request.starts_with("GET /socket HTTP/1.1\r\n"));
        assert!(request.contains("Authorization: MediaBrowser Token=\"synthetic-token\""));
        assert!(!request.lines().next().unwrap().contains("synthetic-token"));
        let frame = socket.next_frame(Duration::from_secs(2)).unwrap();
        let super::websocket::Frame::Text(json) = frame else {
            panic!("expected text frame");
        };
        assert_eq!(
            super::protocol::parse(&json).unwrap(),
            Some(super::protocol::Action::Playstate(
                super::protocol::Playstate::Pause
            ))
        );
        drop(socket);
        crate::jellyfin::uninstall();
        server.join().unwrap();
    }

    #[test]
    fn play_on_raw_connection_sends_and_receives_over_libcurl() {
        use std::io::{Read, Write};
        use std::time::Duration;
        let _serial = crate::testlock::serial();
        assert!(crate::net::global_init());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (closed_tx, closed_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = crate::testnet::accept(&listener).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = [0_u8; 5];
            socket.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"hello");
            socket.write_all(b"pong").unwrap();
            socket.shutdown(std::net::Shutdown::Write).unwrap();
            drop(socket);
            closed_tx.send(()).unwrap();
        });
        let origin = crate::origin::Origin::http("127.0.0.1", i32::from(port));
        let mut connection = crate::net::websocket::RawConnection::connect(&origin).unwrap();
        connection.send_all(b"hello", Duration::from_secs(2)).unwrap();
        closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut response = [0_u8; 4];
        let read = connection
            .read_some(&mut response, Duration::from_secs(2))
            .unwrap();
        assert_eq!(&response[..read], b"pong");
        drop(connection);
        server.join().unwrap();
    }

    #[test]
    fn play_on_capabilities_advertise_only_the_mvp() {
        let body: serde_json::Value =
            serde_json::from_slice(super::protocol::capabilities_body()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "PlayableMediaTypes": ["Video"],
                "SupportsMediaControl": true,
                "SupportsPersistentIdentifier": true,
                "SupportedCommands": ["Play", "Playstate"]
            })
        );
    }

    #[test]
    fn play_on_handshake_requires_the_rfc_accept() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let response = b"HTTP/1.1 101 Switching Protocols\r\n\
                         Upgrade: websocket\r\n\
                         Connection: Upgrade\r\n\
                         Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        super::websocket::validate_handshake(response, key).unwrap();
        let invalid = b"HTTP/1.1 101 Switching Protocols\r\n\
                        Upgrade: websocket\r\n\
                        Connection: Upgrade\r\n\
                        Sec-WebSocket-Accept: wrong\r\n\r\n";
        assert!(super::websocket::validate_handshake(invalid, key).is_err());
    }

    #[test]
    fn play_on_duplicate_message_ids_are_delivered_once() {
        let message = br#"{
            "MessageType":"Playstate",
            "MessageId":"pause-1",
            "Data":{"Command":"Pause"}
        }"#;
        let parsed = super::protocol::parse_detailed(message).unwrap();
        let mut deduper = super::protocol::Deduper::default();
        assert!(deduper.accept(&parsed));
        assert!(!deduper.accept(&parsed));
        let without_id = super::protocol::parse_detailed(
            br#"{"MessageType":"Playstate","Data":{"Command":"Pause"}}"#,
        )
        .unwrap();
        assert!(deduper.accept(&without_id));
        assert!(deduper.accept(&without_id));
    }

    #[test]
    fn play_on_force_keepalive_carries_the_server_interval() {
        let message = br#"{
            "MessageType":"ForceKeepAlive",
            "Data":30
        }"#;
        assert_eq!(
            super::protocol::parse(message).unwrap(),
            Some(super::protocol::Action::ForceKeepAlive(30))
        );
    }

    #[test]
    fn play_on_playstate_maps_transport_and_seek_commands() {
        use super::protocol::{parse, Action, Playstate};
        for (command, expected) in [
            ("Pause", Playstate::Pause),
            ("Unpause", Playstate::Unpause),
            ("PlayPause", Playstate::PlayPause),
            ("Stop", Playstate::Stop),
        ] {
            let message = format!(
                r#"{{"MessageType":"Playstate","Data":{{"Command":"{command}"}}}}"#
            );
            assert_eq!(
                parse(message.as_bytes()).unwrap(),
                Some(Action::Playstate(expected))
            );
        }
        let seek = br#"{
            "MessageType":"Playstate",
            "Data":{"Command":"Seek","SeekPositionTicks":90000000}
        }"#;
        assert_eq!(
            parse(seek).unwrap(),
            Some(Action::Playstate(Playstate::Seek(9_000_000_000)))
        );
    }

    #[test]
    fn play_on_playnow_selects_only_the_requested_item_and_options() {
        let message = br#"{
            "MessageType":"Play",
            "MessageId":"message-1",
            "Data":{
                "PlayCommand":"PlayNow",
                "ItemIds":["first-item","selected-item"],
                "StartIndex":1,
                "StartPositionTicks":12345678900,
                "MediaSourceId":"alternate-source",
                "AudioStreamIndex":2,
                "SubtitleStreamIndex":-1
            }
        }"#;
        assert_eq!(
            super::protocol::parse(message).unwrap(),
            Some(super::protocol::Action::Play(super::protocol::PlayRequest {
                item_id: "selected-item".to_owned(),
                media_source_id: Some("alternate-source".to_owned()),
                start_position_ns: 1_234_567_890_000,
                audio_stream_index: Some(2),
                subtitle_stream_index: Some(-1),
            }))
        );
    }

    #[test]
    fn play_on_websocket_accept_matches_the_rfc_vector() {
        assert_eq!(
            super::websocket::websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn play_on_client_text_frames_are_final_and_masked() {
        assert_eq!(
            super::websocket::encode_client_frame(0x1, b"Hi", [1, 2, 3, 4]),
            vec![0x81, 0x82, 1, 2, 3, 4, b'H' ^ 1, b'i' ^ 2]
        );
    }

    #[test]
    fn play_on_decoder_handles_partial_and_coalesced_reads() {
        use super::websocket::{Decoder, Frame};
        let mut decoder = Decoder::default();
        assert!(decoder.push(&[0x81]).unwrap().is_empty());
        assert_eq!(
            decoder
                .push(&[0x02, b'H', b'i', 0x81, 0x03, b'b', b'y', b'e'])
                .unwrap(),
            vec![Frame::Text(b"Hi".to_vec()), Frame::Text(b"bye".to_vec())]
        );
    }

    #[test]
    fn play_on_decoder_reassembles_text_around_a_ping() {
        use super::websocket::{Decoder, Frame};
        let mut decoder = Decoder::default();
        let frames = decoder
            .push(&[
                0x01, 0x02, b'H', b'i', 0x89, 0x01, b'?', 0x80, 0x03, b'b', b'y', b'e',
            ])
            .unwrap();
        assert_eq!(
            frames,
            vec![Frame::Ping(b"?".to_vec()), Frame::Text(b"Hibye".to_vec())]
        );
    }

    #[test]
    fn play_on_decoder_reads_a_sixteen_bit_length() {
        use super::websocket::{Decoder, Frame};
        let payload = vec![b'x'; 300];
        let mut wire = vec![0x81, 126, 0x01, 0x2c];
        wire.extend_from_slice(&payload);
        assert_eq!(
            Decoder::default().push(&wire).unwrap(),
            vec![Frame::Text(payload)]
        );
    }

    #[test]
    fn play_on_decoder_reads_a_bounded_sixty_four_bit_length() {
        use super::websocket::{Decoder, Frame};
        let payload = vec![b'x'; 70_000];
        let mut wire = vec![0x81, 127];
        wire.extend_from_slice(&70_000_u64.to_be_bytes());
        wire.extend_from_slice(&payload);
        assert_eq!(
            Decoder::default().push(&wire).unwrap(),
            vec![Frame::Text(payload)]
        );
    }

    #[test]
    fn play_on_decoder_rejects_an_oversized_message_from_its_header() {
        use super::websocket::{Decoder, FrameError};
        let mut wire = vec![0x81, 127];
        wire.extend_from_slice(&262_145_u64.to_be_bytes());
        assert_eq!(
            Decoder::default().push(&wire),
            Err(FrameError::MessageTooLarge)
        );
    }

    #[test]
    fn play_on_decoder_caps_the_reassembled_message() {
        use super::websocket::{Decoder, FrameError};
        let mut decoder = Decoder::default();
        let mut first = vec![0x01, 127];
        first.extend_from_slice(&200_000_u64.to_be_bytes());
        first.extend_from_slice(&vec![b'x'; 200_000]);
        assert!(decoder.push(&first).unwrap().is_empty());
        let mut last = vec![0x80, 127];
        last.extend_from_slice(&100_000_u64.to_be_bytes());
        last.extend_from_slice(&vec![b'x'; 100_000]);
        assert_eq!(decoder.push(&last), Err(FrameError::MessageTooLarge));
    }
}
