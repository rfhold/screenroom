use crate::catalog::{Item, Kind};
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Library(pub String);
impl Library {
    pub fn id(&self) -> String {
        self.0.clone()
    }
    pub fn label(&self) -> String {
        crate::catalog::libraries()
            .iter()
            .find(|l| l.id == self.0)
            .map(|l| l.title.clone())
            .unwrap_or_default()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Browse,
    Search,
}
pub struct State {
    pub screen: Screen,
    pub library: Library,
    pub path: Vec<String>,
    pub detail: Option<String>,
    pub expanded: bool,
    pub subtitle: usize,
    pub query: String,
    pub search_pending: bool,
    pub query_debouncing: bool,
    pub search_revision: u64,
    pub types: [bool; 3],
    pub methods: [bool; 3],
    pub title_only: bool,
    pub settings: bool,
    pub source: String,
    pub search_origin: bool,
    pub result_ids: Vec<String>,
    pub search_error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            screen: Screen::Home,
            library: Library::default(),
            path: vec![],
            detail: None,
            expanded: false,
            subtitle: 0,
            query: String::new(),
            search_pending: false,
            query_debouncing: false,
            search_revision: 0,
            types: [true, true, false],
            methods: [true; 3],
            title_only: false,
            settings: false,
            source: String::new(),
            search_origin: false,
            result_ids: vec![],
            search_error: None,
        }
    }
}
impl State {
    pub fn apply_capabilities(
        &mut self,
        capabilities: &screenroom_client_core::SearchCapabilities,
    ) {
        for (index, kind) in ["movie", "series", "episode"].iter().enumerate() {
            self.types[index] &= capabilities
                .types
                .iter()
                .any(|value| value.eq_ignore_ascii_case(kind));
        }
        if !self.types.iter().any(|value| *value) {
            if let Some(index) = ["movie", "series", "episode"].iter().position(|kind| {
                capabilities
                    .types
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(kind))
            }) {
                self.types[index] = true;
            }
        }
        for (index, method) in ["text", "fuzzy", "semantic"].iter().enumerate() {
            self.methods[index] &= capabilities.methods.iter().any(|value| value == method);
        }
        if !self.methods.iter().any(|value| *value) {
            if let Some(index) = ["text", "fuzzy", "semantic"]
                .iter()
                .position(|method| capabilities.methods.iter().any(|value| value == method))
            {
                self.methods[index] = true;
            }
        }
        self.title_only &= capabilities
            .methods
            .iter()
            .any(|value| value == "title_only");
        if !capabilities.sources.contains(&self.source) {
            self.source = capabilities.sources.first().cloned().unwrap_or_default();
        }
    }
    pub fn fresh_search(&mut self, library: Library) {
        self.navigate(Screen::Search, library);
        self.query.clear();
        self.methods = [true; 3];
        self.title_only = false;
        self.settings = false;
        self.source.clear();
        self.result_ids.clear();
    }
    pub fn edit_query(&mut self, query: String) -> u64 {
        self.search_revision += 1;
        self.search_pending = !query.trim().is_empty();
        self.query_debouncing = self.search_pending;
        self.query = query;
        self.result_ids.clear();
        self.search_error = None;
        self.search_revision
    }
    pub fn settle_query(&mut self, revision: u64) -> bool {
        if revision != self.search_revision || !self.query_debouncing {
            return false;
        }
        self.query_debouncing = false;
        true
    }
    pub fn navigate(&mut self, screen: Screen, library: Library) {
        self.search_revision += 1;
        self.search_pending = false;
        self.query_debouncing = false;
        self.screen = screen;
        self.library = library;
        self.path.clear();
        self.detail = None;
        self.search_origin = false;
    }
    pub fn open(&mut self, id: &str, items: &[Item]) {
        let Some(item) = items.iter().find(|i| i.id == id) else {
            return;
        };
        match item.kind {
            Kind::Movie | Kind::Episode => self.detail = Some(id.into()),
            Kind::Series | Kind::Season => {
                self.search_origin |= self.screen == Screen::Search;
                self.screen = Screen::Browse;
                self.path = item
                    .parent
                    .iter()
                    .cloned()
                    .chain(std::iter::once(id.into()))
                    .collect();
                self.detail = None;
            }
        }
    }
    pub fn toggle_type(&mut self, index: usize) {
        toggle_required(&mut self.types, index);
    }
    pub fn toggle_method(&mut self, index: usize) {
        if index != 2 || !self.title_only {
            toggle_required(&mut self.methods, index);
        }
    }
    pub fn toggle_title_only(&mut self) {
        self.title_only = !self.title_only;
        if self.title_only {
            self.methods[2] = false;
            if !self.methods[0] && !self.methods[1] {
                self.methods[0] = true;
            }
        }
    }
    pub fn results(&self, items: &[Item]) -> Vec<usize> {
        if self.search_pending {
            return vec![];
        }
        self.result_ids
            .iter()
            .filter_map(|id| items.iter().position(|i| &i.id == id))
            .collect()
    }
}
fn toggle_required(values: &mut [bool; 3], index: usize) {
    if !values[index] || values.iter().filter(|v| **v).count() > 1 {
        values[index] = !values[index];
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_identity_and_order_are_server_owned() {
        use crate::catalog::{self, Catalog};
        use screenroom_client_core::MediaLibrary;
        let library = Library("strange-id".into());
        let libraries = vec![
            MediaLibrary {
                id: "other".into(),
                title: "First".into(),
            },
            MediaLibrary {
                id: "strange-id".into(),
                title: "Documentaries".into(),
            },
        ];
        let members = std::collections::HashMap::from([(
            "strange-id".into(),
            vec!["series-as-returned".into(), "movie-as-returned".into()],
        )]);
        catalog::install(Catalog {
            libraries: libraries.clone(),
            members: members.clone(),
            ..Default::default()
        });
        assert_eq!(library.label(), "Documentaries");
        assert_eq!(catalog::members(&library.id()), members["strange-id"]);
        let mut reordered = libraries;
        reordered.reverse();
        catalog::install(Catalog {
            libraries: reordered,
            ..Default::default()
        });
        assert_eq!(library.id(), "strange-id");
        assert_eq!(library.label(), "Documentaries");
        assert_eq!(catalog::libraries()[0].id, "strange-id");
    }
    #[test]
    fn capability_subsets_and_source_reordering_preserve_supported_choices() {
        use screenroom_client_core::SearchCapabilities;
        let mut state = State::default();
        let mut caps = SearchCapabilities {
            types: vec!["Episode".into()],
            methods: vec!["fuzzy".into()],
            sources: vec!["archive".into(), "regional".into()],
            maximum_query_length: 23,
            semantic_description: "Server explanation".into(),
        };
        state.apply_capabilities(&caps);
        assert_eq!(state.types, [false, false, true]);
        assert_eq!(state.methods, [false, true, false]);
        assert_eq!(state.source, "archive");
        caps.sources.reverse();
        state.apply_capabilities(&caps);
        assert_eq!(state.source, "archive");
        assert_eq!(caps.maximum_query_length, 23);
        assert_eq!(caps.semantic_description, "Server explanation");
    }
    #[test]
    fn stale_search_revisions_are_rejected() {
        let mut s = State::default();
        let old = s.edit_query("one".into());
        let current = s.edit_query("two".into());
        assert!(!s.settle_query(old));
        assert!(s.settle_query(current));
        s.navigate(Screen::Home, Library::default());
        assert!(!s.settle_query(current));
    }
    #[test]
    fn required_options_and_title_guard() {
        let mut s = State::default();
        s.toggle_type(0);
        s.toggle_type(1);
        assert_eq!(s.types, [false, true, false]);
        s.methods = [false, false, true];
        s.toggle_title_only();
        assert_eq!(s.methods, [true, false, false]);
        s.toggle_title_only();
        assert!(!s.methods[2]);
    }
    #[test]
    fn navigation_uses_ids_after_record_reordering() {
        use crate::catalog::map_item;
        use screenroom_client_core::Item;
        let season = map_item(
            Item {
                id: "season-x".into(),
                parent_id: Some("series-x".into()),
                kind: "Season".into(),
                ..Item::default()
            },
            vec![],
        );
        let episode = map_item(
            Item {
                id: "episode-x".into(),
                parent_id: Some("season-x".into()),
                kind: "Episode".into(),
                ..Item::default()
            },
            vec![],
        );
        let mut records = vec![episode, season];
        let mut state = State::default();
        state.open("season-x", &records);
        assert_eq!(state.path, ["series-x", "season-x"]);
        records.reverse();
        state.open("episode-x", &records);
        assert_eq!(state.detail.as_deref(), Some("episode-x"));
    }
    #[test]
    fn query_debounce_and_network_pending_are_distinct() {
        let mut state = State::default();
        let revision = state.edit_query("Bunny".into());
        state.toggle_type(2);
        assert!(state.query_debouncing);
        assert!(state.settle_query(revision));
        assert!(state.search_pending);
        assert!(!state.query_debouncing);
        assert!(!state.settle_query(revision));
    }
    #[test]
    fn unknown_votes_preserve_the_numeric_rating() {
        use crate::catalog::rating_labels;
        use serde_json::json;
        let (score, info) = rating_labels(
            &json!({"rating":6.1,"ratingProvider":"TMDB","ratingScale":10,"votes":null,"retrievedAt":"2026-11-07"}),
        );
        assert_eq!(score, "TMDB 6.1/10");
        assert_eq!(info, "Vote count unavailable · Snapshot 2026-11-07");
    }
    #[test]
    fn zero_votes_are_explicitly_unrated() {
        use crate::catalog::rating_labels;
        use serde_json::json;
        assert_eq!(
            rating_labels(&json!({"rating":0,"ratingProvider":"TMDB","ratingScale":10,"votes":0}))
                .0,
            "TMDB · Unrated"
        );
        assert!(
            rating_labels(&json!({"rating":0,"votes":0}))
                .1
                .starts_with("0 votes · Snapshot ")
        );
    }
    #[test]
    fn absent_rating_does_not_invent_a_score() {
        use crate::catalog::rating_labels;
        use serde_json::json;
        for record in [json!({}), json!({"rating":null,"votes":null})] {
            assert_eq!(
                rating_labels(&record),
                ("Rating —".into(), "Rating snapshot not imported.".into())
            );
        }
    }
}
