//! #319: the subtitle appearance is written by Settings and has to come back at boot. The three
//! steps are process globals whose only boot-time reader is `restore_subtitle_appearance`, so the
//! thing worth grading is the wiring: a file with the picks, loaded, and read back through the
//! same accessors the picker and the composer use.

use super::boot::restore_subtitle_appearance;

fn restore(scale: u8, position: u8, background: u8) {
    crate::route::restore_subtitle_scale_step(scale);
    crate::route::restore_subtitle_position_step(position);
    crate::route::restore_subtitle_background_step(background);
}

#[test]
fn a_subtitle_appearance_in_the_file_is_what_the_picker_and_the_composer_read_after_boot() {
    let _serial = crate::testlock::serial();
    let dir = crate::server::session::TempSession::new("boot-subtitle-appearance");
    std::fs::write(
        dir.path(),
        r#"{"client_id":"cid-test","subtitle_scale":3,"subtitle_position":2,"subtitle_background":1}"#,
    )
    .expect("the fixture writes");
    // The factory look first, so a pass cannot come from a leftover global.
    restore(0, 0, 0);

    let loaded = crate::server::session::load();
    assert_eq!(loaded.subtitle_scale(), 3, "the file carried the pick");
    restore_subtitle_appearance(&loaded);

    assert_eq!(crate::route::subtitle_scale_step(), 3, "the picker reads the file's pick");
    assert_eq!(crate::route::subtitle_position_step(), 2);
    assert_eq!(crate::route::subtitle_background_step(), 1);
    assert_eq!(
        crate::route::subtitle_style_overrides(),
        "FontSize=30\nMarginV=45\nBorderStyle=3\nBackColour=&H80000000",
        "and so does the composer"
    );
    restore(0, 0, 0);
}
