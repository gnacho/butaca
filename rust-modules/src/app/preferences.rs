//! Live executor for Settings preference effects. The bridge's controlled-IO guard must admit
//! the effect before this entrypoint: thread admission and persistence all live here, never in
//! a screen constructor or step.
use crate::screens::registry::PreferenceCmd;

pub(super) fn execute(command: PreferenceCmd) {
    match command {
        PreferenceCmd::Quality { quality, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_default_quality(quality));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::Language { language, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::server::session::set_language(language));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::DirectPlay { mode, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_direct_play_mode(mode));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::LipSync { offset_ms, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_lip_sync_offset_ms(offset_ms));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::SubtitleScale { step, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_subtitle_scale_step(step));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::SubtitlePosition { step, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_subtitle_position_step(step));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::SubtitleBackground { step, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_subtitle_background_step(step));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::UpNext { policy, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_up_next_policy(policy));
                crate::ui::idle::invalidate();
            });
        }
        PreferenceCmd::UpNextWindow { seconds, reply } => {
            let _ = crate::storage_worker::submit_retained(move || {
                let _ = reply.send(crate::route::set_up_next_window_s(seconds));
                crate::ui::idle::invalidate();
            });
        }
    }
}
