//! search — the Search screen's wire side: one `/Items?searchTerm=` page plus a people lookup,
//! projected into the screen's fixed shelves.
//!
//! Plex answers a search as a hub list (`/hubs/search`: media in `Metadata[]`, people and
//! collections as tags in `Directory[]`) and `search.rs`'s `project` shelves it. Jellyfin answers
//! as ONE flat item page, so the shelfing happens HERE instead. Movies, shows and episodes
//! convert through the same [`super::convert::movie_from_dto`] as every other listing — watched
//! flags, resume points and codec badges come free — and people and collections become
//! `search::TagHit`s, the same struct the Plex arm hands the screen, so `merge`, the round-robin
//! and every tile path draw this backend unchanged.
//!
//! Two deliberate PoC edges, both documented where they land:
//!
//! * A `TagHit`'s `key` is empty. On Plex that field is the listing path the press opens; the
//!   Jellyfin person/collection page is its own arm (`person.rs`), and until it exists a press
//!   on one of these rows is the one dead end this shelf can produce.
//! * A person's portable id is absent by definition — `tag_key` is a plex.tv concept. The
//!   server-local GUID rides in `id`, which `search::same_tag` only ever compares within one
//!   server, and this backend has exactly one.

use super::client::JfClient;
use super::dto::BaseItemDto;
use super::SERVER_ID;
use crate::search::{Item, Kind, Projection, TagHit};

/// One source's whole answer to the settled query, in [`crate::search::KINDS`] order — the exact
/// mailbox shape the Plex worker fills. `None` is the FAILURE sentinel (`search.rs`'s rule: a
/// failed fetch and an empty answer are different states, and only the first retries).
///
/// The people fetch failing degrades its shelf to empty rather than failing the query — see
/// [`JfClient::search_people`].
pub(crate) fn search(c: &JfClient, terms: &str, limit: i64) -> Option<Projection> {
    let res = c.search_media(terms, limit)?;
    let mut out: Projection = Default::default();
    for it in &res.items {
        match it.kind.as_str() {
            "Movie" | "Series" | "Episode" => {
                if let Some(m) = super::convert::movie_from_dto(it, SERVER_ID, 0) {
                    let k = match it.kind.as_str() {
                        "Movie" => shelf(Kind::Movie),
                        "Series" => shelf(Kind::Show),
                        _ => shelf(Kind::Episode),
                    };
                    out[k].push(Item::Media(m));
                }
            }
            // A collection opens a listing, not a detail page — the `TagHit` shape, with its
            // extent (ChildCount) as the caption, exactly what `items_word` reads on Plex rows.
            "BoxSet" => out[shelf(Kind::Collection)].push(Item::Tag(tag_hit(it, true))),
            _ => {} // a kind this screen has no shelf for is dropped, as `project` drops them
        }
    }
    if let Some(people) = c.search_people(terms, limit) {
        let k = shelf(Kind::Person);
        for p in &people.items {
            out[k].push(Item::Tag(tag_hit(p, false)));
        }
    }
    Some(out)
}

/// A person or BoxSet as a search hit. `id` is the Jellyfin GUID — server-local but stable, and
/// `search::same_tag`'s id fallback compares it only within one server, which is all this backend
/// ever has. The thumb is the UNSIZED image path (`convert`'s rule: the poster store adds the
/// box), empty where no Primary exists — the tile then draws the skeleton face `TagHit`'s doc
/// specifies, rather than spending the poster store on a 404.
fn tag_hit(it: &BaseItemDto, collection: bool) -> TagHit {
    TagHit {
        sid: SERVER_ID,
        name: it.name.clone(),
        tag_key: String::new(), // a plex.tv global person guid — this backend has no such thing
        id: it.id.clone(),
        thumb: match it.image_tags.get("Primary") {
            Some(t) => format!("/Items/{}/Images/Primary?tag={t}", it.id),
            None => String::new(),
        },
        key: String::new(), // the Plex listing path — see the module doc for the dead-end note
        count: if collection {
            it.child_count.unwrap_or(0)
        } else {
            0
        },
    }
}

/// The shelf a kind draws on — `KINDS`'s index space, which is what a per-source projection is
/// keyed by.
fn shelf(kind: Kind) -> usize {
    crate::search::KINDS
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::KINDS;

    #[test]
    fn shelves_line_up_with_kinds() {
        // the projection is keyed by KINDS position — a drift here shelves films under TV Shows
        assert_eq!(shelf(Kind::Movie), KINDS.iter().position(|k| *k == Kind::Movie).unwrap());
        assert_eq!(shelf(Kind::Show), KINDS.iter().position(|k| *k == Kind::Show).unwrap());
        assert_eq!(shelf(Kind::Episode), KINDS.iter().position(|k| *k == Kind::Episode).unwrap());
        assert_eq!(shelf(Kind::Person), KINDS.iter().position(|k| *k == Kind::Person).unwrap());
        assert_eq!(
            shelf(Kind::Collection),
            KINDS.iter().position(|k| *k == Kind::Collection).unwrap()
        );
    }
}
