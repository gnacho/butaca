//! Main-thread execution of validated Jellyfin Play On commands.

use super::App;
use crate::jellyfin::play_on::protocol::Playstate;
use crate::jellyfin::play_on::{PreparedPlay, RemoteCommand};
use crate::screens::registry::PlayerReq;

pub(crate) fn playstate_request(state: Playstate) -> Option<PlayerReq> {
    match state {
        Playstate::Pause => Some(PlayerReq::Transport(Some(false))),
        Playstate::Unpause => Some(PlayerReq::Transport(Some(true))),
        Playstate::PlayPause => Some(PlayerReq::Transport(None)),
        Playstate::Stop => Some(PlayerReq::Exit),
        Playstate::Seek(position) => i64::try_from(position).ok().map(PlayerReq::CommitSeek),
    }
}

pub(crate) fn should_defer_playstate(
    player_mounted: bool,
    resolve_pending: bool,
    remote_play_pending: bool,
) -> bool {
    !player_mounted && (resolve_pending || remote_play_pending)
}

pub(crate) fn pump(app: &mut App, now: u32) {
    app.adapters.play_on.sync(
        crate::jellyfin::client(),
        app.boot_initial.is_none(),
    );
    let commands = app.adapters.play_on.drain();
    if commands.is_empty() {
        return;
    }
    for command in commands {
        match command {
            RemoteCommand::Playstate(state) => execute_playstate(app, now, state),
            RemoteCommand::Play(prepared) => execute_play(app, prepared),
        }
    }
    crate::ui::idle::invalidate();
}

fn execute_playstate(app: &mut App, now: u32, state: Playstate) {
    let player_mounted = super::bridge::player(&app.pages).is_some();
    if !player_mounted {
        let defer = should_defer_playstate(
            false,
            crate::route::play_pending(),
            app.adapters.play_on.has_pending_play(),
        );
        if defer {
            app.adapters
                .play_on
                .defer(RemoteCommand::Playstate(state));
        }
        return;
    }
    if !matches!(state, Playstate::Stop)
        && crate::ui::player_hud::transport_hidden(&app.player.session)
    {
        return;
    }
    let Some(mut request) = playstate_request(state) else {
        return;
    };
    if let Playstate::Seek(position) = state {
        let duration = crate::player::duration_ns();
        if duration <= 0 {
            return;
        }
        let Ok(position) = i64::try_from(position) else {
            return;
        };
        request = PlayerReq::CommitSeek(crate::ui::player_hud::scrub_clamp_target(
            position,
            duration,
        ));
    }
    super::playback::player_requests(
        &mut app.player.repair,
        &mut app.player.session,
        &mut app.adapters.player,
        vec![request],
        now,
        &mut app.refresh_hubs_at,
        &mut app.pages,
        &mut app.bridge,
        &mut app.ok_armed,
        &mut app.input.press,
        &mut app.repause_at,
    );
}

fn execute_play(app: &mut App, prepared: PreparedPlay) {
    let mounted = super::bridge::player(&app.pages).is_some();
    let origin = if mounted {
        crate::route::cancel_play(&mut app.player.session);
        super::playback::close_player_overlays(&mut app.pages);
        crate::player::stop_bufferfeed(&mut app.player.session, &mut app.adapters.player);
        super::playback::Origin::Unchanged
    } else {
        if !super::content::clear_engine_for_play(
            &mut app.player.session,
            &mut app.adapters.player,
            false,
        ) {
            app.adapters.play_on.defer(RemoteCommand::Play(prepared));
            return;
        }
        super::playback::Origin::Here
    };
    let PreparedPlay {
        movie,
        playing,
        start_position_ns,
        tracks,
    } = prepared;
    let rk = movie.rk.clone();
    let started = crate::route::request_play_prepared(
        &mut app.player.session,
        app.bridge.metadata_mut(),
        movie.sid,
        &movie.rk,
        &movie.part,
        &movie.vcodec,
        &movie.acodec,
        &movie.title,
        "",
        tracks,
        playing,
    );
    if !started {
        crate::log("play on: PlayNow refused by the playback route");
        return;
    }
    app.bridge
        .metadata_mut()
        .run(crate::stores::metadata::MetadataCmd::RetirePlaying);
    app.bridge.metadata_mut().run(
        crate::stores::metadata::MetadataCmd::RequestDetail {
            sid: movie.sid,
            rk,
        },
    );
    super::playback::start_playback(
        &mut app.player.session,
        &mut app.adapters.player,
        i64::try_from(start_position_ns).unwrap_or(i64::MAX),
        origin,
        crate::ui::player_hud::LINGER_MS,
        None,
        &mut app.pages,
        &mut app.bridge,
    );
}
