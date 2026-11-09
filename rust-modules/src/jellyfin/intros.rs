//! **The local intro**, the clip the Local Intros plugin plays before a film (issue #370).
//!
//! `GET /Items/{itemId}/Intros` is the `IIntroProvider` seam the plugin plugs into. It is a CORE
//! route: a server without the plugin installs no provider and answers `Items: []`, which is the
//! ordinary answer and the reason nothing on screen is drawn for it. Verified against Jellyfin's
//! own OpenAPI (`BaseItemDtoQueryResult`, stable and 13.x alike) rather than assumed from the
//! plugin's README.
//!
//! **The intro is a real item, and this app plays it as one.** The official client splices intros
//! into the film's own playback queue; this app resolves ONE item and has no queue to splice into
//! (`route::request_play` starts exactly what it is given), so the honest shape is the one
//! the pre-playback window took: the intro is played as its own item, and when it ends or the
//! viewer backs out the ordinary player exit returns to the page it was started from. Nothing
//! claims the film follows it.
//!
//! Every failure folds to "no intros": an item the server no longer has (404), a transport error,
//! and a plugin that answers with a shape this client cannot read are all the same answer for a
//! window whose subject is the FILM. There is no line to show for it: the row is simply not drawn.
//!
//! This module is the WORKER's half: it holds no state. The intros live in `crate::metadata`'s
//! store beside the item they belong to.

use crate::registry::ServerId;

/// One intro, with everything `route::request_play` needs to start it: the item, its media source,
/// the codecs the direct-play gate reads, and the title the HUD wears while it runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Intro {
    pub(crate) rk: String,
    pub(crate) title: String,
    /// The `MediaSource` id — what `request_play` addresses (`MediaSourceId=`). Falls back to the
    /// item id, which Jellyfin also addresses (`convert::movie_from_dto`'s rule).
    pub(crate) part: String,
    pub(crate) vcodec: String,
    pub(crate) acodec: String,
}

/// The intros for one item, in the server's order (the plugin decides what plays first). Empty is
/// the answer on every server without the plugin, and the caller draws nothing for it.
pub(crate) fn list(sid: ServerId, item_id: &str) -> Vec<Intro> {
    let Some(client) = super::client() else {
        return Vec::new();
    };
    let Some(items) = client.intros(item_id) else {
        // A 404 (an item the server no longer has) and a failed request both land here, and both
        // mean the same thing to the window: there is no intro to offer.
        crate::log(&format!(
            "intros: rk={item_id} sid={sid:?} - the server answered nothing, offering none"
        ));
        return Vec::new();
    };
    let out: Vec<Intro> = items
        .into_iter()
        .filter(|it| !it.id.is_empty())
        .map(|it| {
            let (vcodec, acodec) = super::convert::codecs(&it);
            Intro {
                part: it
                    .media_sources
                    .as_ref()
                    .and_then(|s| s.first())
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| it.id.clone()),
                rk: it.id,
                title: it.name,
                vcodec,
                acodec,
            }
        })
        .collect();
    if !out.is_empty() {
        crate::log(&format!(
            "intros: rk={item_id} sid={sid:?} - {} intro(s)",
            out.len()
        ));
    }
    out
}

/// TEST ONLY: one intro with the fields the play request reads. Compiled out of the shipped binary.
#[cfg(test)]
pub(crate) fn intro_for_test(rk: &str, title: &str) -> Intro {
    Intro {
        rk: rk.to_string(),
        title: title.to_string(),
        part: format!("part-{rk}"),
        vcodec: "h264".to_string(),
        acodec: "aac".to_string(),
    }
}
