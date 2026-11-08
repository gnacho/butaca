//! **Edit images**, choosing one of the pictures the server's metadata providers already hold for
//! an item and telling the server to use it (issue #369).
//!
//! Two server calls and one model:
//!
//! * `GET /Items/{itemId}/RemoteImages?type=Primary&…` answers the pictures TheMovieDb (or whichever
//!   provider is enabled) has for the item. They are NOT on this server: each one is an absolute
//!   URL on the provider's own host, which is why the picker's thumbnails go through
//!   `JfClient::get_bytes`' absolute arm and why nothing here downloads anything itself.
//! * `POST /Items/{itemId}/RemoteImages/Download?type=Primary&imageUrl=…` tells the SERVER to fetch
//!   that URL and make it the item's picture. **The endpoint requires elevation**, so an ordinary
//!   viewer's account gets a refusal, which this module reports as a plain `false` rather than
//!   dressing up.
//!
//! **This is not an upload.** The viewer picks from what the server already knows about the item,
//! which is the only shape a television can offer: a webOS set has no file picker this app can
//! reach, so a local file would have no source for its bytes (the argument
//! the item menu makes for *Edit subtitles* still holding).
//!
//! **One image type, `Primary`.** The API takes a `type` on both calls and the picker is built for
//! the poster, which is the picture a person means by "change the cover". Offering Backdrop, Logo
//! and the rest is a second axis of the surface (a type switch above a grid whose tiles are then
//! 2:3 or 16:9 or square), and nothing in this client reads those types off a television window
//! yet.
//!
//! This module is the WORKER's half: it holds no state. The pictures, the phase and the item they
//! describe live in `crate::metadata`'s images store, which is what a screen reads.

use crate::registry::ServerId;

/// The image type both calls name. `Primary` is the poster (and the thumbnail a card wears).
pub(crate) const PRIMARY: &str = "Primary";

/// How many pictures one page asks for. A bound, not a preference: the picker draws them all at
/// once in a fixed grid, and every tile is a decode of somebody else's JPEG. The server sorts the
/// page by its own community rating, so the twelve it answers are the twelve it thinks best.
pub(crate) const PAGE: i32 = 12;

/// One picture, as a tile of the grid reads it, plus the URL the Download call sends back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RemoteImage {
    /// The provider's full-size picture. It is sent back VERBATIM: the server fetches exactly this
    /// URL, and rebuilding it here would be this client asking for a picture the viewer never saw.
    pub(crate) url: String,
    /// The provider's own smaller copy of the same picture, for the tile (`RemoteImageInfo`'s
    /// `ThumbnailUrl`, which the server fills for TheMovieDb). Falls back to [`Self::url`] when the
    /// provider publishes none: a full-size decode is slower, not wrong.
    pub(crate) thumb: String,
    /// WHICH provider offered it ("TheMovieDb", "The Open Movie Database").
    pub(crate) provider: String,
    /// The picture's pixel size as the provider reports it, for the tile's caption. Zero when the
    /// provider sent none, and the caption then drops the size rather than inventing one.
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// The picture's language tag, empty when the provider sends none (the ordinary case for a
    /// poster, which is language-free artwork).
    pub(crate) language: String,
}

impl RemoteImage {
    fn from_dto(raw: super::dto::RemoteImageInfoDto) -> Option<Self> {
        // A row with no URL is neither drawable nor applicable, so it is not a candidate: the
        // server would be asked to download nothing.
        if raw.url.is_empty() {
            return None;
        }
        let thumb = raw
            .thumbnail_url
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| raw.url.clone());
        Some(Self {
            url: raw.url,
            thumb,
            provider: raw.provider_name,
            width: raw.width.max(0),
            height: raw.height.max(0),
            language: raw.language.unwrap_or_default(),
        })
    }

    /// The caption under the tile: the provider, and the size when the provider reported one.
    /// A separate line rather than one string, so the caller can draw them in two weights.
    pub(crate) fn size_label(&self) -> String {
        match (self.width, self.height) {
            (0, _) | (_, 0) => String::new(),
            (w, h) => format!("{w}x{h}"),
        }
    }
}

/// One page of pictures, and how many the server holds in total (which the picker says when it is
/// more than the page).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RemoteImages {
    pub(crate) images: Vec<RemoteImage>,
    pub(crate) total: i32,
}

/// The pictures for one item's `Primary` image. `None` is a transport/auth/parse failure: the list
/// and "the server did not answer" are different answers and the store keeps them apart.
pub(crate) fn list(sid: ServerId, item_id: &str) -> Option<RemoteImages> {
    let client = super::client()?;
    let page = client.remote_images(item_id, PRIMARY, 0, PAGE)?;
    let images: Vec<RemoteImage> = page
        .images
        .into_iter()
        .filter_map(RemoteImage::from_dto)
        .collect();
    crate::log(&format!(
        "images: rk={item_id} sid={sid:?} - {} picture(s) of {}",
        images.len(),
        page.total_record_count
    ));
    Some(RemoteImages {
        total: page.total_record_count.max(images.len() as i32),
        images,
    })
}

/// Make one picture the item's `Primary` image. `false` covers every refusal the endpoint can
/// give: transport, a non-admin account (403), an item the server no longer has (404), and a
/// provider URL the server will not fetch.
pub(crate) fn apply(sid: ServerId, item_id: &str, img: &RemoteImage) -> bool {
    let Some(client) = super::client() else {
        return false;
    };
    let ok = client.download_remote_image(item_id, PRIMARY, &img.url);
    crate::log(&format!(
        "images: rk={item_id} sid={sid:?} provider={} download ok={ok}",
        img.provider
    ));
    ok
}

/// TEST ONLY: one picture with the fields a tile reads, so a store test can post the answer a
/// worker would have produced without a server. Compiled out of the shipped binary.
#[cfg(test)]
pub(crate) fn image_for_test(url: &str, provider: &str, w: i32, h: i32) -> RemoteImage {
    RemoteImage {
        url: url.to_string(),
        thumb: url.to_string(),
        provider: provider.to_string(),
        width: w,
        height: h,
        language: String::new(),
    }
}
