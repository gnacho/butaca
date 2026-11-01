//! Audio and subtitle track selection tests: the server-selection/file-default
//! ladder, direct-play-eligible fallbacks, and subtitle stream id resolution.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

#[test]
fn an_empty_track_list_falls_back_to_the_codec_default() {
    assert_eq!(
        pick_dp_audio(&[], "ac3").map(|(i, c, _)| (i, c)),
        Some((-1, "ac3".into()))
    );
    assert!(
        pick_dp_audio(&[], "truehd").is_none(),
        "a non-direct-playable default must transcode"
    );
}


/// **With no Plex language preference, the file's default track wins — English has no say.**
/// Issue #202: a French MKV whose default-flagged track is French, beside an English one, opened
/// in English because of a built-in English rung. Both orderings, so the pick is the flag and
/// not the list position.
#[test]
fn the_files_default_track_wins_without_a_language_preference() {
    let tracks = [trk(1, "ac3", "fre", true), trk(2, "ac3", "eng", false)];
    assert_eq!(pick_dp_audio(&tracks, "ac3"), Some((0, "ac3".into(), 1)));
    let tracks = [trk(1, "ac3", "eng", false), trk(2, "eac3", "fre", true)];
    assert_eq!(pick_dp_audio(&tracks, "eac3"), Some((1, "eac3".into(), 2)));
}


/// The re-encode half of #202: a video transcode of the same French file must not swap the
/// French default for an English track either — it keeps what direct play would have chosen.



/// PMS account auto-picks and manual picks are indistinguishable, so both outrank the show.





/// A PMS selection the pipeline cannot decode (French DTS) is answered by a
/// direct-playable track in THAT language, not by the file's default in another one.
#[test]
fn a_non_playable_pick_is_answered_by_a_sibling_in_its_language() {
    let tracks = [
        trk(1, "ac3", "eng", true),
        server_selected(trk(2, "dca", "fre", false)),
        trk(3, "ac3", "fre", false),
    ];
    assert_eq!(pick_dp_audio(&tracks, "ac3"), Some((2, "ac3".into(), 3)));
}


#[test]
fn the_flagged_default_wins_over_an_earlier_track() {
    let tracks = [trk(1, "ac3", "deu", false), trk(2, "ac3", "fra", true)];
    assert_eq!(pick_dp_audio(&tracks, "ac3"), Some((1, "ac3".into(), 2)));
}


#[test]
fn smart_dp_takes_a_playable_sibling_over_a_non_playable_default() {
    // A 4K HEVC item: TrueHD default + an AC3 sibling — direct-play beats the server's
    // video-downscaling transcode.
    let tracks = [trk(1, "truehd", "eng", true), trk(2, "ac3", "eng", false)];
    assert_eq!(pick_dp_audio(&tracks, "truehd"), Some((1, "ac3".into(), 2)));
}


#[test]
fn no_direct_playable_track_means_transcode() {
    let tracks = [trk(1, "truehd", "eng", true), trk(2, "dts", "eng", false)];
    assert!(pick_dp_audio(&tracks, "truehd").is_none());
}


#[test]
fn the_servers_selected_track_outranks_the_files_default() {
    // A user picks the second Russian dub on their phone. The file's default is still the
    // FIRST direct-playable track, which the ladder would otherwise hand back on every play.
    let tracks = [
        trk(2693, "ac3", "rus", true),
        server_selected(trk(2694, "ac3", "rus", false)),
        trk(2695, "ac3", "eng", false),
    ];
    assert_eq!(pick_dp_audio(&tracks, "ac3"), Some((1, "ac3".into(), 2694)));
}




#[test]
fn a_selected_track_that_cannot_direct_play_falls_through_to_the_ladder() {
    // A live shape off the server: it holds the English DTS track (a PMS selection — it is
    // not the file default), which this pipeline cannot decode. Honouring it would force a
    // whole-video transcode for one audio track, so the ladder runs on instead.
    let tracks = [
        trk(2663, "ac3", "rus", true),
        server_selected(trk(2669, "dca", "eng", false)),
        trk(2673, "ac3", "eng", false),
    ];
    assert_eq!(pick_dp_audio(&tracks, "dca"), Some((2, "ac3".into(), 2673)));
}


/// The 720p re-encode must name the selected DTS, not the Russian AC3 sibling smart-DP
/// would copy. Remux still names that sibling — a copy of DTS would not play.


/// A selected flag that only echoes the container default is not a 720p pick: with a show
/// preference set, treating it as one would name the Russian default over the preferred dub.


/// Live three-track shape: selected English DTS plus an English AC3 sibling. Remux copies
/// the AC3; re-encode names the DTS.


/// Unselected English DTS beside a Russian AC3 default, no preference: remux copies the
/// default, and a re-encode keeps it too — nothing asked for English (#202).


/// First-English-any-codec would PUT TrueHD here. The sibling is already English, so 720p
/// keeps that AC3 copy instead of re-encoding lossless.


/// No AC3 sibling: smart-DP has nothing to copy. A real selected DTS must still be named,
/// not omitted (PUT 0 encodes the TrueHD default).


/// The whole ladder, rung by rung, with the selected flag switched on and off — the order is
/// the contract, and every row here is a shape the live server actually serves.
#[test]
fn the_audio_ladder_walks_its_rungs_in_order() {
    let cases: [(
        &str,
        Vec<crate::metadata::Stream>,
        &str,
        Option<(i32, String, i64)>,
    ); 7] = [
        (
            "rung 1: a PMS selection wins even against English",
            vec![
                trk(1, "eac3", "rus", true),
                server_selected(trk(2, "eac3", "deu", false)),
                trk(3, "eac3", "eng", false),
            ],
            "eac3",
            Some((1, "eac3".into(), 2)),
        ),
        (
            "rung 1 needs a non-default PMS selection: the default echo is not one",
            vec![
                server_selected(trk(1, "eac3", "rus", true)),
                trk(2, "eac3", "eng", false),
            ],
            "eac3",
            Some((0, "eac3".into(), 1)),
        ),
        (
            "rung 1 is skipped when the pick can't direct-play, not obeyed by transcoding",
            vec![
                trk(1, "ac3", "rus", true),
                server_selected(trk(2, "dca", "eng", false)),
                trk(3, "ac3", "eng", false),
            ],
            "ac3",
            Some((2, "ac3".into(), 3)), // rung 2: a DP track in the pick's language
        ),
        (
            "no selection at all → the file's default, never a built-in English (#202)",
            vec![trk(1, "ac3", "rus", true), trk(2, "ac3", "eng", false)],
            "ac3",
            Some((0, "ac3".into(), 1)),
        ),
        (
            "rung 3: the file's flagged default, wherever it sits",
            vec![trk(1, "ac3", "deu", false), trk(2, "ac3", "fra", true)],
            "ac3",
            Some((1, "ac3".into(), 2)),
        ),
        (
            "rung 4: a selected non-DP track with only a foreign DP sibling — smart-DP",
            vec![
                server_selected(trk(1, "truehd", "eng", false)),
                trk(2, "ac3", "fra", false),
            ],
            "truehd",
            Some((1, "ac3".into(), 2)),
        ),
        (
            "nothing direct-playable, selected or not → transcode",
            vec![
                server_selected(trk(1, "truehd", "eng", false)),
                trk(2, "dts", "rus", true),
            ],
            "truehd",
            None,
        ),
    ];
    for (what, tracks, acodec, want) in cases {
        assert_eq!(pick_dp_audio(&tracks, acodec), want, "{what}");
    }
}


#[test]
fn the_selected_subtitle_resolves_to_the_renderers_embedded_ordinal() {
    // Document order is NOT container order and a sidecar sits in the middle of the list:
    // the renderer counts only embedded streams, sorted on PMS `Stream.index` — the same
    // identifier space the track menu commits (metadata::sub_render_ordinal).
    let subs = [
        sub(10, 7, "fra", true),  // sidecar — not in the container, not counted
        sub(11, 3, "rus", false), // embedded, container-first
        server_selected(sub(12, 4, "eng", false)),
    ];
    assert_eq!(pick_dp_subtitle(&subs), Some((12, 1)));
}


#[test]
fn an_external_selected_subtitle_is_left_off() {
    // A sidecar has no demux ordinal; forcing a transcode to obey a stored
    // flag is not a trade the user asked for, so the direct-play path leaves subs off.
    let subs = [
        server_selected(sub(10, 3, "eng", true)),
        sub(11, 4, "rus", false),
    ];
    assert_eq!(pick_dp_subtitle(&subs), None);
}


#[test]
fn mde_subtitle_stream_id_names_advertised_codecs_and_zeroes_the_rest() {
    assert_eq!(mde_subtitle_stream_id(&[]), 0);
    assert_eq!(
        mde_subtitle_stream_id(&[server_selected(sub(10, 3, "eng", true))]),
        0,
        "sidecar → 0"
    );
    let mut pgs = server_selected(sub(12, 4, "eng", false));
    pgs.codec = "pgs".into();
    assert_eq!(mde_subtitle_stream_id(&[pgs]), 12);
    let mut mov = server_selected(sub(14, 4, "eng", false));
    mov.codec = "mov_text".into();
    assert_eq!(mde_subtitle_stream_id(&[mov]), 14);
    let mut dvd = server_selected(sub(15, 4, "eng", false));
    dvd.codec = "dvd_subtitle".into();
    assert_eq!(mde_subtitle_stream_id(&[dvd]), 15);
    let mut obscure = server_selected(sub(13, 4, "eng", false));
    obscure.codec = "vplayer".into();
    assert_eq!(
        mde_subtitle_stream_id(&[obscure]),
        0,
        "unadvertised but still client-rendered → 0 so MDE does not transcode"
    );
}


#[test]
fn no_selected_subtitle_means_subtitles_stay_off() {
    assert_eq!(pick_dp_subtitle(&[]), None);
    let subs = [sub(10, 3, "eng", false), sub(11, 4, "rus", false)];
    assert_eq!(
        pick_dp_subtitle(&subs),
        None,
        "the file's own tracks are not an instruction"
    );
}


#[test]
fn a_selection_with_no_stream_id_is_left_off_rather_than_half_applied() {
    // id and ordinal travel together: the id is what the menu checkmark and the timeline
    // report key on, so an id-less stream would render subtitles while the menu said Off.
    let subs = [server_selected(sub(0, 3, "eng", false))];
    assert_eq!(pick_dp_subtitle(&subs), None);
}


/// **A show's preferred audio language beats the file's flag.** The reported case (#160): a
/// series set to Hungarian whose episodes carry the Hungarian dub as the FILE default beside an
/// English track opened in English, through a built-in English rung since removed (#202).

#[test]
fn preferred_language_tags_match_both_three_letter_spellings() {
    assert!(lang_matches("hu-HU", "hun"));
    assert!(lang_matches("de-DE", "ger") && lang_matches("de-DE", "deu"));
    assert!(lang_matches("pt-BR", "por"));
    assert!(lang_matches("en-GB", "eng"));
    assert!(!lang_matches("hu-HU", "eng"));
    assert!(!lang_matches("", "hun"));
    assert!(!lang_matches("hu-HU", ""));
}

/// **A show's subtitle settings turn a subtitle on at the start of a direct play.**

#[test]
fn show_settings_are_read_out_of_a_setting_list() {
    use crate::plex::{Setting, ShowLangPrefs};
    let s = |id: &str, v: &str| Setting { id: id.into(), value: v.into() };
    assert_eq!(ShowLangPrefs::from_settings(&[s("episodeSort", "-1")]), None);
    assert_eq!(
        ShowLangPrefs::from_settings(&[
            s("audioLanguage", "hu-HU"),
            s("subtitleLanguage", ""),
            s("subtitleMode", "1"),
        ]),
        Some(ShowLangPrefs { audio: Some("hu-HU".into()), subtitle: None, subtitle_mode: 1 })
    );
    assert_eq!(
        ShowLangPrefs::from_settings(&[s("audioLanguage", "")]),
        Some(ShowLangPrefs { audio: None, subtitle: None, subtitle_mode: -1 })
    );
}



/// **One precedence for every path.** A PMS selection the pipeline cannot decode (English DTS)
/// still outranks the show's language: direct play carries it as the English AC3
/// sibling, a re-encode names the DTS itself. The direct-play pick used to hand this case to the
/// show's Hungarian while the re-encode named the DTS — two answers to one question.

/// A pick direct play cannot carry hands over to the NEXT entry in the ranking, not straight to
/// the file's default: French DTS picked, no playable French track, show set to Hungarian →
/// direct play and remux take the Hungarian AC3; a re-encode still encodes the French pick.

/// A picked stream and its sibling may spell one language two ways (`fre` / `fra`); the
/// sibling still counts as the pick's language.
#[test]
fn a_picks_sibling_matches_across_iso_639_spellings() {
    let tracks = [
        trk(1, "ac3", "eng", true),
        server_selected(trk(2, "dca", "fre", false)),
        trk(3, "ac3", "fra", false),
    ];
    assert_eq!(pick_dp_audio(&tracks, "ac3"), Some((2, "ac3".into(), 3)));
    assert!(lang_matches("fre", "fra") && lang_matches("ger", "deu") && lang_matches("no", "nob"));
    assert!(!lang_matches("fre", "eng") && !lang_matches("", "") && !lang_matches("xx", "yy"));
}

/// `""` and `"-1"` are Plex's "Account default": unset, so the file's default wins.





// Issue #88: a series reopens with the tracks it was last watched with. The stored id belongs to
// the FILE (container stream ids are per-file), so the language tag is what travels; the id is the
// fallback for a re-encode whose tags moved. Everything below is pure: the table itself is
// `plex::session::SeriesTracks`.

use crate::plex::session::SeriesTracks;

fn remembered(series: &str, audio: &str, audio_index: i64) -> SeriesTracks {
    SeriesTracks {
        series: series.into(),
        audio_lang: audio.into(),
        audio_index,
        ..Default::default()
    }
}

fn remembered_subs(sub_lang: &str, sub_index: i64) -> SeriesTracks {
    SeriesTracks { sub_lang: sub_lang.into(), sub_index, ..remembered("series9", "", 0) }
}

#[test]
fn a_series_reopens_the_audio_it_was_watched_with() {
    // The file's default is English; last time this show was watched in Spanish.
    let tracks = [trk(101, "aac", "eng", true), trk(102, "aac", "spa", false)];
    assert_eq!(
        remembered_audio(&tracks, &remembered("series9", "spa", 102)),
        Some((1, "aac".into(), 102)),
        "the remembered tag outranks the file's default flag"
    );
}

#[test]
fn the_remembered_tag_matches_every_spelling_of_the_language() {
    // Stored as an ISO tag, reported by the container as a three-letter code.
    let tracks = [trk(101, "aac", "eng", true), trk(102, "aac", "spa", false)];
    assert_eq!(
        remembered_audio(&tracks, &remembered("series9", "es-ES", 999)).map(|(_, _, id)| id),
        Some(102),
        "es-ES and spa are the same language, and the stale id must not win"
    );
}

#[test]
fn a_pick_falls_back_to_the_stream_id_when_the_language_tag_changed() {
    // Same re-encode, but the container now calls the Spanish track "fra" by mistake.
    let tracks = [trk(101, "aac", "eng", true), trk(102, "aac", "fra", false)];
    assert_eq!(
        remembered_audio(&tracks, &remembered("series9", "spa", 102)).map(|(_, _, id)| id),
        Some(102),
        "an id that still exists is better than nothing"
    );
}

#[test]
fn an_empty_tag_only_matches_on_the_id() {
    let tracks = [trk(101, "aac", "eng", true), trk(102, "aac", "fra", false)];
    assert_eq!(remembered_audio(&tracks, &remembered("series9", "", 102)).map(|(_, _, id)| id), Some(102));
    assert_eq!(remembered_audio(&tracks, &remembered("series9", "", 999)), None);
}

#[test]
fn a_remembered_audio_that_cannot_direct_play_is_not_offered() {
    // TrueHD does not direct play: the plan must not carry it just because it was chosen once.
    let tracks = [trk(101, "truehd", "spa", false), trk(102, "aac", "eng", true)];
    assert_eq!(remembered_audio(&tracks, &remembered("series9", "spa", 101)), None);
}

#[test]
fn the_remembered_subtitle_wins_over_the_language_rules() {
    let subs = [sub(11, 3, "spa", false), sub(12, 4, "eng", false)];
    assert_eq!(
        remembered_subtitle(&subs, &remembered_subs("eng", 12)),
        Some((12, 1)),
        "the renderer counts embedded streams, not document order"
    );
}

#[test]
fn a_sidecar_cannot_be_remembered_as_a_subtitle() {
    // A sidecar has no demux ordinal, so there is nothing the client renderer could turn on.
    let subs = [sub(10, 7, "spa", true)];
    assert_eq!(remembered_subtitle(&subs, &remembered_subs("spa", 10)), None);
}

#[test]
fn a_remembered_subtitle_the_menu_cannot_name_is_ignored() {
    let subs = [sub(0, 3, "spa", false)];
    assert_eq!(
        remembered_subtitle(&subs, &remembered_subs("spa", 0)),
        None,
        "an id the menu cannot name is not a subtitle worth turning on"
    );
}
