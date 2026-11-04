// Keep mock workers inside an explicit test module for the production dependency gate.
#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn tv_library_type_changes_query_and_preserves_each_sections_choice() {
        let _guard = crate::testlock::serial();
        let mut state = BrowseState::default();
        seed_two_source_table_for_owner_test(&mut state);
        let adapter = Arc::new(BrowseAdapter::default());
        state.set_cur(1);
        let target = crate::stores::browse::SectionAddress {
            epoch: state.table_epoch(), sid: state.section_sid(1).unwrap(), section: 2,
        };
        let old = state.listing_snapshot();
        let current = &mut state.states[1];
        current.total = 1;
        current.sorts = Arc::new(vec![SortEntry {
            key: "addedAt".into(), title: "Date Added".into(), default_desc: true,
        }]);
        current.sort_desc = true;
        current.genre = Some(Arc::new(GenreEntry { id: "7".into(), title: "Drama".into() }));
        current.genres_done = true;
        current.letters = Arc::new(vec![("A".into(), 1), ("B".into(), 1)]);
        current.letters_done = true;
        current.cursor = Some(Arc::new(Cursor { at: CursorAt::SlotIndex(0), scroll: 12.0 }));
        current.unwatched = true;

        assert!(state.addressed_with_adapter(&adapter, target, crate::stores::browse::LibraryWork::Commit {
            select: false, choice: false,
            query: Some(crate::stores::browse::QueryEdit::LibraryType(LibraryType::Episodes)),
        }));
        let listing = state.listing_snapshot();
        assert_eq!(listing.view().library_type(), LibraryType::Episodes);
        assert_eq!(old.view().library_type(), LibraryType::Primary, "retained publication stays immutable");
        assert_ne!(listing.view().id().unwrap().query, old.view().id().unwrap().query);
        assert_eq!(listing.view().total(), -1);
        assert!(listing.view().item(0).is_none());
        assert!(listing.view().sorts().is_empty());
        assert!(!listing.view().sort_desc());
        assert!(listing.view().genre().is_none());
        assert!(listing.view().letters().is_empty());
        assert!(listing.view().cursor().is_none());
        assert!(listing.view().unwatched());
        assert!(!state.states[1].genres_done && !state.states[1].letters_done);
        assert!(!state.set_genre_by_id(Some("7")));

        state.set_cur(3);
        assert_eq!(state.listing_snapshot().view().library_type(), LibraryType::Primary);
        assert!(state.set_library_type(LibraryType::Seasons));
        state.set_unwatched(true);
        state.set_cur(1);
        assert_eq!(state.listing_snapshot().view().library_type(), LibraryType::Episodes);
        let gen = state.query_gen();
        assert!(state.set_library_type(LibraryType::Episodes));
        assert_eq!(state.query_gen(), gen, "reselecting the current type keeps the loaded listing");

        state.set_cur(0);
        let gen = state.query_gen();
        assert!(!state.set_library_type(LibraryType::Episodes));
        assert_eq!(state.query_gen(), gen);
    }

    /// **Design A: a movie library lists its collections from the TYPE menu.** The listing asks
    /// for `type=18` on the server wire, which also scoped its letters), sends neither
    /// filter — a collection has no watch state or genre — and shows neither as applied. Unwatched
    /// is the section's own switch and comes back with its films; a genre belongs to one metadata
    /// type's menu, so a type change drops it, as it always has between TV types.
    #[test]
    fn a_movie_library_lists_its_collections_without_filters() {
        let _guard = crate::testlock::serial();
        assert_eq!(LibraryType::offered(SecKind::Movie), [LibraryType::Primary, LibraryType::Collections]);
        assert_eq!(LibraryType::offered(SecKind::Show),
            [LibraryType::Primary, LibraryType::Seasons, LibraryType::Episodes, LibraryType::Collections]);

        let mut state = BrowseState::default();
        seed_two_source_table_for_owner_test(&mut state);
        state.set_cur(0);
        assert_eq!(state.sections[0].kind, SecKind::Movie);
        let gen = state.query_gen();
        assert!(!state.set_library_type(LibraryType::Seasons), "a movie library has no seasons");
        assert_eq!(state.query_gen(), gen);
        state.set_unwatched(true);
        state.states[0].genre = Some(Arc::new(GenreEntry { id: "7".into(), title: "Drama".into() }));
        assert!(state.set_library_type(LibraryType::Collections));
        assert_ne!(state.query_gen(), gen);
        let listing = state.listing_snapshot();
        assert_eq!(listing.view().library_type(), LibraryType::Collections);
        assert!(!listing.view().unwatched() && listing.view().genre().is_none(),
            "neither filter reads as applied to a collections listing");
        assert!(!state.set_genre_by_id(Some("7")), "genres are the primary listing's");

        assert!(state.set_library_type(LibraryType::Primary));
    }

    #[test]
    fn an_other_videos_library_has_the_movie_listing_shape() {
        assert_eq!(LibraryType::offered(SecKind::Video), LibraryType::offered(SecKind::Movie));
    }

    #[test]
    fn an_other_videos_library_has_its_own_labels() {
        let _en = crate::i18n::language_on_this_thread_for_test(crate::i18n::Preference::En);
        assert_eq!(LibraryType::Primary.title(SecKind::Video), "Other Videos");
        assert_eq!(LibraryType::Primary.empty_readout(SecKind::Video, "Home Clips"), "No videos in Home Clips");
        assert_eq!(count_line(12, SecKind::Video), "12 videos");
        assert_eq!(count_line(-1, SecKind::Video), "Videos");
    }
}
