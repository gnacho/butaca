//! Build one coherent playback request from a remote PlayNow and its Jellyfin item DTO.

use super::protocol::PlayRequest;
use crate::catalog::CatalogMovie;
use crate::metadata::{Marker, PlayingItem};
use crate::route::StartTracks;

pub(crate) struct PreparedPlay {
    pub(crate) movie: CatalogMovie,
    pub(crate) playing: PlayingItem,
    pub(crate) start_position_ns: u64,
    pub(crate) tracks: StartTracks,
}

pub(super) fn fetch(client: &crate::jellyfin::client::JfClient, request: &PlayRequest) -> Option<PreparedPlay> {
    let item = client.item_detail(&request.item_id)?;
    let markers = super::super::detail::media_markers(client, &request.item_id);
    from_dto(&item, request, markers)
}

pub(super) fn from_dto(
    item: &super::super::dto::BaseItemDto,
    request: &PlayRequest,
    markers: Vec<Marker>,
) -> Option<PreparedPlay> {
    if item.id != request.item_id {
        return None;
    }
    let source_id = request.media_source_id.as_deref().or_else(|| {
        item.media_sources
            .as_ref()?
            .first()
            .map(|source| source.id.as_str())
    });
    let movie = super::super::convert::movie_from_dto_source(
        item,
        crate::jellyfin::SERVER_ID,
        0,
        source_id,
    )?;
    if !matches!(item.kind.as_str(), "Movie" | "Episode") {
        return None;
    }
    let playing = super::super::detail::playing_item_from_dto_source(
        item,
        crate::jellyfin::SERVER_ID,
        &request.item_id,
        markers,
        source_id,
    )?;
    let audio_sid = match request.audio_stream_index {
        None => 0,
        Some(index) if index > 0 => i64::from(index),
        Some(_) => return None,
    };
    let sub_sid = match request.subtitle_stream_index {
        None => -1,
        Some(-1) => 0,
        Some(index) if index >= 0 => i64::from(index),
        Some(_) => return None,
    };
    Some(PreparedPlay {
        movie,
        playing,
        start_position_ns: request.start_position_ns,
        tracks: StartTracks { audio_sid, sub_sid },
    })
}
