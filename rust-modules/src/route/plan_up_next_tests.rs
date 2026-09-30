//! The Jellyfin lane's Up Next descriptor (issues #41/#42): one projection of the show's
//! `GET /Shows/NextUp` answer into the `UpNext` shape the HUD control and the auto-advance
//! countdown already read.

#![cfg(feature = "jellyfin")]

use super::*;

fn dto(json: &str) -> crate::jellyfin::BaseItemDto {
    serde_json::from_str(json).expect("fixture parses")
}

#[test]
fn a_next_episode_becomes_a_playable_up_next() {
    let it = dto(r#"{
        "Id": "ep2", "Name": "The Second One", "Type": "Episode",
        "SeriesId": "series9", "SeriesName": "The Show",
        "ParentIndexNumber": 2, "IndexNumber": 3,
        "RunTimeTicks": 36000000000,
        "ImageTags": {"Primary": "etag"}, "BackdropImageTags": [],
        "UserData": {"Played": false, "PlayCount": 0, "PlaybackPositionTicks": 0},
        "MediaSources": [{
            "Id": "ms-2",
            "MediaStreams": [
                {"Type": "Video", "Codec": "h264"},
                {"Type": "Audio", "Codec": "aac"}
            ]
        }]
    }"#);
    let u = up_next_from_dto(&it).expect("an episode must become an UpNext");
    assert_eq!(u.rk, "ep2");
    assert_eq!(u.part, "ms-2");
    assert_eq!(u.vcodec, "h264");
    assert_eq!(u.acodec, "aac");
    assert_eq!(u.show_title, "The Show");
    assert_eq!(u.ep_title, "The Second One");
    assert_eq!(u.season, 2);
    assert_eq!(u.index, 3);
    assert_eq!(u.dur_ms, 3_600_000, "3600 s at 36e9 ticks, carried in milliseconds");
    assert_eq!(u.resume_ms, 0);
}

#[test]
fn a_non_episode_answer_means_no_up_next() {
    // NextUp answers an episode in practice; the gate stays anyway — "up next" is a show idea,
    // the same rule the Plex lane's up_next_of applies to its queue rows.
    let it = dto(r#"{"Id": "m1", "Name": "A Film", "Type": "Movie",
        "ImageTags": {}, "BackdropImageTags": []}"#);
    assert!(up_next_from_dto(&it).is_none());
}
