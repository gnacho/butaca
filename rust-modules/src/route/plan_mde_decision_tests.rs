//! Streamability tests (the survivors of the MDE-decision suite: the part-id parser and the
//! `/decision` refusal grader went with the retired wire layer).

use super::*;
#[allow(unused_imports)]
use super::test_support::*;


/// The direct-play gate: MKV and MP4/M4V parts are fed to the demuxer untouched — everything
/// else takes the remux branch. mp4 moved sides on 2026-08-11 (issue #22): the mkv-only gate
/// dated from an unseekable AVIO, and on a server that cannot transcode it turned every mp4
/// into a failure.
#[test]
fn mkv_and_mp4_parts_are_direct_playable() {
    assert!(part_is_streamable("/library/parts/1/2/movie.mkv"));
    assert!(
        part_is_streamable("/library/parts/1/2/movie.mkv?x=1"),
        "the query must not defeat it"
    );
    assert!(part_is_streamable("/library/parts/1/2/movie.mp4"));
    assert!(part_is_streamable("/library/parts/1/2/movie.m4v"));
    assert!(
        !part_is_streamable("/library/parts/1/2/movie.mov"),
        "mov still remuxes"
    );
    assert!(!part_is_streamable(""));
    assert!(
        !part_is_streamable("/library/parts/1/2/mkv.avi"),
        "the extension, not a substring"
    );
    assert!(
        !part_is_streamable("/library/parts/1/2/mp4.avi"),
        "the extension, not a substring"
    );
}
