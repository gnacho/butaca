//! Playback-session protocol op (impl Client): the `/:/timeline` progress report — the one
//! write that keeps a session visible in /status/sessions with its viewOffset, watched state
//! and the Direct Play vs Transcode badge (correlated by the active encoder's coupled
//! `X-Plex-Session-Identifier == session=`).
use super::client::{Client, QueryBuilder};
use super::params::TimelineReport;

impl Client {
    /// POST /:/timeline (the spec verb; params ride the query) — the server updates viewOffset
    /// (the resume point) + watched state. `true` when it took the report.
    ///
    /// The outcome used to stop here (`post_void`), which made this the one write in the playback
    /// protocol that could fail in complete silence: a 401 on a revoked token, a refused connect
    /// and a 500 were indistinguishable from a committed resume point, and the caller logged the
    /// same success line for all four. The reporting is [`crate::route::scrobble_stop`]'s; this
    /// only has to stop throwing the answer away.
    pub fn timeline(&self, r: &TimelineReport) -> bool {
        let q = QueryBuilder::new("/:/timeline")
            .str("ratingKey", r.rating_key)
            .str("key", &format!("/library/metadata/{}", r.rating_key))
            .str("identifier", "com.plexapp.plugins.library")
            .str("state", r.state.as_str())
            .int("time", r.time_ms)
            .int("duration", r.duration_ms)
            .str("X-Plex-Session-Identifier", r.session);
        let q = self
            .playback_identity(q)
            .opt_str("playQueueID", r.play_queue_id)
            .opt_str("playQueueItemID", r.play_queue_item_id)
            .opt_int("audioStreamID", r.audio_stream_id)
            .opt_int("subtitleStreamID", r.subtitle_stream_id);
        self.post_ok(&q.build())
    }

}
