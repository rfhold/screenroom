use super::*;

#[test]
fn search_origin_detail_does_not_restart_search_and_cancel_artwork() {
    let mut state = State {
        screen: Screen::Search,
        query: "Man of Steel".into(),
        ..Default::default()
    };
    assert!(should_resume_search(&state));

    // Resuming search cancels the detail's artwork load, needed for its new size.
    let item = catalog::map_item(
        screenroom_client_core::Item {
            id: "selected-item".into(),
            kind: "Movie".into(),
            ..Default::default()
        },
        vec![],
    );
    state.open(&item.id, std::slice::from_ref(&item));
    assert_eq!(state.detail.as_deref(), Some("selected-item"));
    assert!(!should_resume_search(&state));
    assert_eq!(state.query, "Man of Steel");

    state.detail = None;
    assert!(should_resume_search(&state));
}

#[test]
fn post_load_search_resume_requires_the_search_page_and_a_query() {
    let mut state = State::default();
    assert!(!should_resume_search(&state));
    state.query = "retained query".into();
    for screen in [Screen::Home, Screen::Browse] {
        state.screen = screen;
        assert!(!should_resume_search(&state));
    }
    state.screen = Screen::Search;
    assert!(should_resume_search(&state));
    state.query.clear();
    assert!(!should_resume_search(&state));
}
