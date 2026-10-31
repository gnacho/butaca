//! Classification of `/library/sections` rows, including Plex's Other Videos libraries.

use super::*;

const THREE_SECTIONS: &str = r#"{"MediaContainer":{"size":3,"Directory":[
  {"key":"1","type":"movie","title":"Movies","agent":"tv.plex.agents.movie","scanner":"Plex Movie"},
  {"key":"2","type":"show","title":"TV Shows","agent":"tv.plex.agents.series","scanner":"Plex TV Series"},
  {"key":"3","type":"movie","title":"Other Videos","agent":"tv.plex.agents.none","scanner":"Plex Video Files"}]}}"#;

fn project(json: &str) -> Vec<(i64, String, SecKind)> {
    let mc = serde_json::from_str::<crate::plex::Envelope>(json)
        .expect("section answer parses")
        .media_container;
    project_sections(&mc)
}

fn one_movie_type_section(agent: &str, scanner: &str) -> SecKind {
    let json = format!(r#"{{"MediaContainer":{{"Directory":[{{"key":"7","type":"movie","title":"Library","agent":"{agent}","scanner":"{scanner}"}}]}}}}"#);
    project(&json).remove(0).2
}

#[test]
fn a_three_section_answer_projects_movies_shows_and_other_videos() {
    assert_eq!(
        project(THREE_SECTIONS),
        vec![
            (1, "Movies".to_string(), SecKind::Movie),
            (2, "TV Shows".to_string(), SecKind::Show),
            (3, "Other Videos".to_string(), SecKind::Video),
        ],
    );
}

#[test]
fn a_movie_agent_or_a_movie_scanner_keeps_the_section_in_movies() {
    assert_eq!(one_movie_type_section("tv.plex.agents.movie", "Plex Movie"), SecKind::Movie);
    assert_eq!(one_movie_type_section("tv.plex.agents.movie", "Plex Video Files"), SecKind::Movie);
    assert_eq!(one_movie_type_section("tv.plex.agents.none", "Plex Movie"), SecKind::Movie);
}

#[test]
fn an_answer_without_agent_stays_movies() {
    let json = r#"{"MediaContainer":{"Directory":[
      {"key":"1","type":"movie","title":"Movies"},
      {"key":"2","type":"show","title":"TV Shows"},
      {"key":"3","type":"movie","title":"Clips","scanner":"Plex Video Files"}]}}"#;
    assert_eq!(
        project(json).into_iter().map(|(_, _, kind)| kind).collect::<Vec<_>>(),
        [SecKind::Movie, SecKind::Show, SecKind::Movie],
    );
}

#[test]
fn current_and_legacy_other_videos_spellings_are_accepted() {
    assert_eq!(one_movie_type_section("tv.plex.agents.none", "Plex Video Files"), SecKind::Video);
    assert_eq!(
        one_movie_type_section("com.plexapp.agents.none", "Plex Video Files Scanner"),
        SecKind::Video,
    );
    assert_eq!(one_movie_type_section("TV.PLEX.AGENTS.NONE", " plex video files "), SecKind::Video);
}

#[test]
fn legacy_wire_codes_remain_compatible_and_video_has_its_own_code() {
    assert_eq!(SecKind::from_wire("movie"), Some(SecKind::Movie));
    assert_eq!(SecKind::from_wire("show"), Some(SecKind::Show));
    assert_eq!(SecKind::from_wire("video"), Some(SecKind::Video));
    assert_eq!(SecKind::Video.wire(), "video");
    assert_ne!(SecKind::Video.wire(), SecKind::Movie.wire());
    assert_ne!(SecKind::Video.wire(), SecKind::Show.wire());
}
