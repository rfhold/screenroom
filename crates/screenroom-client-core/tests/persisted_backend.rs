#![cfg(not(target_arch = "wasm32"))]

use screenroom_client_core::*;
#[path = "support/pagination.rs"]
mod pagination;

#[tokio::test]
#[ignore = "attach to caller-owned real indexed Jellyfin backend with SCREENROOM_API_ENDPOINT and SCREENROOM_USER_ID"]
async fn persisted_backend_native_roundtrip() {
    tokio::time::timeout(pagination::Limits::default().timeout, roundtrip())
        .await
        .expect("persisted backend smoke: overall 120s time budget exceeded");
}

async fn roundtrip() {
    let endpoint = std::env::var("SCREENROOM_API_ENDPOINT")
        .expect("explicit SCREENROOM_API_ENDPOINT required");
    if let Ok(alias) = std::env::var("SCREENROOM_E2E_API_ENDPOINT") {
        assert_eq!(alias, endpoint);
    }
    let user_id =
        std::env::var("SCREENROOM_USER_ID").expect("explicit SCREENROOM_USER_ID required");
    assert!(!user_id.is_empty());
    let client = Client::new(ClientConfig {
        endpoint,
        user_id: user_id.clone(),
        timeout_ms: 5000,
    })
    .unwrap();
    let user = client.clone().get_current_user().await.unwrap();
    assert_eq!(user.id.to_lowercase(), user_id.to_lowercase());
    let page = Page {
        size: 2,
        token: String::new(),
    };
    let info = client.clone().get_catalog_info().await.unwrap();
    assert!(info.notices.len() <= 32 && info.branding.len() <= 8);
    for art in &info.branding {
        assert_artwork_bytes(&client, art).await;
    }
    let sources = pagination::collect("Jellyfin sources", pagination::Limits::default(), |token| {
        let client = client.clone();
        let page = Page {
            token,
            ..page.clone()
        };
        async move {
            let response = client
                .list_jellyfin_sources(page)
                .await
                .map_err(|e| e.to_string())?;
            Ok((response.sources, response.next_page_token))
        }
    })
    .await
    .expect("complete configured Jellyfin source traversal");
    assert!(
        !sources.is_empty(),
        "acceptance prerequisite: configure a real Jellyfin connection before attaching"
    );
    assert_unique_ids(
        sources.iter().map(|source| source.id.as_str()),
        "Jellyfin sources",
    );
    let mut selected = None;
    let mut has_libraries = false;
    let mut has_items = false;
    'sources: for source in sources {
        let libraries = pagination::collect("libraries", pagination::Limits::default(), |token| {
            let client = client.clone();
            let source_id = source.id.clone();
            let page = Page {
                token,
                ..page.clone()
            };
            async move {
                let response = client
                    .list_libraries(source_id, page)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok((response.libraries, response.next_page_token))
            }
        })
        .await
        .expect("complete source-scoped library traversal");
        has_libraries |= !libraries.is_empty();
        assert_unique_ids(
            libraries.iter().map(|library| library.id.as_str()),
            "source libraries",
        );
        for library in libraries {
            let scope = SourceLibraryScope {
                source_id: source.id.clone(),
                library_id: library.id,
            };
            let items =
                pagination::collect("library items", pagination::Limits::default(), |token| {
                    let client = client.clone();
                    let scope = scope.clone();
                    let page = Page {
                        token,
                        ..page.clone()
                    };
                    async move {
                        let response = client
                            .list_library_items(scope, page)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok((response.items, response.next_page_token))
                    }
                })
                .await
                .expect("complete source/library-scoped indexed item traversal");
            has_items |= !items.is_empty();
            assert_unique_ids(items.iter().map(|item| item.id.as_str()), "library items");
            if let Some(item) = items.into_iter().find(|item| item.artwork.is_some()) {
                selected = Some((scope, item.id));
                break 'sources;
            }
        }
    }
    assert!(
        has_libraries,
        "acceptance prerequisite: index at least one real Jellyfin library before attaching"
    );
    assert!(
        has_items,
        "acceptance prerequisite: index at least one real Jellyfin library item before attaching"
    );
    let (scope, item_id) = selected.expect("acceptance prerequisite: publish artwork for at least one indexed Jellyfin library item before attaching; artwork coverage is required, not skipped");
    let home = client.clone().get_home(scope.clone()).await.unwrap();
    assert_home_readable(&home);
    let item = client
        .clone()
        .get_item(item_id.clone(), scope.clone())
        .await
        .unwrap();
    assert_eq!(item.id, item_id);
    assert!(
        !item.title.is_empty(),
        "indexed item must have a searchable title"
    );
    client
        .clone()
        .list_children(item.id.clone(), page.clone(), scope.clone())
        .await
        .unwrap();
    client
        .clone()
        .list_credits(item.id.clone(), page.clone(), scope.clone())
        .await
        .unwrap();
    let art = item
        .artwork
        .as_ref()
        .expect("indexed item detail must retain its published listing artwork");
    assert_artwork_bytes(&client, art).await;
    let artwork_id = art.id.clone();
    let capabilities = client.clone().get_search_capabilities().await.unwrap();
    let methods = requested_search_methods(&capabilities);
    let search = client
        .clone()
        .search(SearchRequest {
            query: item.title,
            source_id: scope.source_id.clone(),
            library_id: scope.library_id.clone(),
            source: "all".into(),
            methods: Some(methods.clone()),
            page: Some(page.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_search_outcome(&methods, &search);
    let artwork = pagination::collect("artwork", pagination::Limits::default(), |token| {
        let client = client.clone();
        let page = Page {
            token,
            ..page.clone()
        };
        let scope = scope.clone();
        async move {
            let response = client
                .list_artwork(scope, page)
                .await
                .map_err(|e| e.to_string())?;
            Ok((response.artwork, response.next_page_token))
        }
    })
    .await
    .expect("complete artwork credits traversal");
    let mut ids = std::collections::HashSet::new();
    for art in artwork {
        assert!(ids.insert(art.id), "duplicate artwork credits");
    }
    assert!(!ids.is_empty());
    assert!(
        ids.contains(&artwork_id),
        "published item artwork must appear in scoped artwork credits"
    );
}

fn assert_unique_ids<'a>(ids: impl IntoIterator<Item = &'a str>, label: &str) {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        assert!(!id.is_empty(), "{label}: empty ID");
        assert!(seen.insert(id), "{label}: duplicate ID");
    }
}

fn assert_home_readable(home: &[HomeRow]) {
    assert!(
        home.iter().all(|row| row.read_error.is_none()),
        "indexed home rows must be readable"
    );
}

fn requested_search_methods(capabilities: &SearchCapabilities) -> SearchMethods {
    let mut methods = default_settings().methods.expect("default search methods");
    let supported = |method: &str| capabilities.methods.iter().any(|value| value == method);
    methods.text &= supported("text");
    methods.fuzzy &= supported("fuzzy");
    methods.semantic &= supported("semantic");
    assert!(
        methods.text || methods.fuzzy || methods.semantic,
        "search capabilities must expose a requested search method"
    );
    methods
}

fn assert_search_outcome(requested: &SearchMethods, search: &SearchPage) {
    let effective = search
        .effective_methods
        .as_ref()
        .expect("effective search methods");
    assert!(
        !effective.semantic || requested.semantic,
        "unrequested semantic method"
    );
    assert!(!effective.text || requested.text, "unrequested text method");
    assert!(
        !effective.fuzzy || requested.fuzzy,
        "unrequested fuzzy method"
    );
    if effective.semantic {
        assert!(
            search.degradation_reason.is_empty(),
            "semantic success must not report degradation"
        );
    } else {
        assert!(
            effective.text || effective.fuzzy,
            "lexical result must use text or fuzzy retrieval"
        );
        if requested.semantic {
            assert!(
                matches!(
                    search.degradation_reason.as_str(),
                    "embedding_unavailable"
                        | "embedding_incompatible"
                        | "embedding_coverage_incomplete"
                ),
                "semantic fallback must report a documented degradation reason"
            );
        } else {
            assert!(
                search.degradation_reason.is_empty(),
                "unrequested semantic retrieval must not report degradation"
            );
        }
    }
}

async fn assert_artwork_bytes(client: &std::sync::Arc<Client>, art: &Artwork) {
    assert!(
        !art.id.is_empty() && !art.url.is_empty(),
        "present artwork must have an ID and fetch URL"
    );
    assert!(
        !client
            .clone()
            .load_artwork(art.url.clone())
            .await
            .expect("fetch published artwork bytes")
            .is_empty()
    );
}

#[test]
fn traversal_ids_accept_unique_values() {
    assert_unique_ids(["first", "second"], "local traversal");
}

#[test]
#[should_panic(expected = "local traversal: duplicate ID")]
fn traversal_ids_reject_duplicates() {
    assert_unique_ids(["same", "same"], "local traversal");
}

#[test]
#[should_panic(expected = "local traversal: empty ID")]
fn traversal_ids_reject_empty_values() {
    assert_unique_ids([""], "local traversal");
}

#[test]
fn home_rows_without_read_errors_pass() {
    assert_home_readable(&[HomeRow::default()]);
}

#[test]
#[should_panic(expected = "indexed home rows must be readable")]
fn failed_home_row_is_rejected() {
    assert_home_readable(&[HomeRow {
        read_error: Some(HomeRowReadError {
            code: "unavailable".into(),
            message: String::new(),
        }),
        ..HomeRow::default()
    }]);
}

#[test]
fn search_accepts_semantic_success_and_documented_lexical_fallbacks() {
    let requested = requested_search_methods(&SearchCapabilities {
        methods: vec!["text".into(), "fuzzy".into(), "semantic".into()],
        ..SearchCapabilities::default()
    });
    assert_eq!(requested, default_settings().methods.unwrap());
    assert!(requested.semantic);
    assert_search_outcome(
        &requested,
        &SearchPage {
            effective_methods: Some(requested.clone()),
            ..SearchPage::default()
        },
    );
    for reason in [
        "embedding_unavailable",
        "embedding_incompatible",
        "embedding_coverage_incomplete",
    ] {
        assert_search_outcome(
            &requested,
            &SearchPage {
                effective_methods: Some(SearchMethods {
                    semantic: false,
                    ..requested.clone()
                }),
                degradation_reason: reason.into(),
                ..SearchPage::default()
            },
        );
    }
}

#[test]
fn explicitly_absent_semantic_capability_allows_undegraded_lexical_search() {
    let requested = requested_search_methods(&SearchCapabilities {
        methods: vec!["text".into(), "fuzzy".into()],
        ..SearchCapabilities::default()
    });
    assert!(!requested.semantic && requested.text && requested.fuzzy);
    assert_search_outcome(
        &requested,
        &SearchPage {
            effective_methods: Some(requested.clone()),
            ..SearchPage::default()
        },
    );
}

#[test]
fn inconsistent_search_outcomes_are_rejected() {
    let requested = default_settings().methods.unwrap();
    let lexical = SearchMethods {
        semantic: false,
        ..requested.clone()
    };
    let invalid = [
        SearchPage {
            effective_methods: Some(lexical.clone()),
            ..SearchPage::default()
        },
        SearchPage {
            effective_methods: Some(requested.clone()),
            degradation_reason: "embedding_unavailable".into(),
            ..SearchPage::default()
        },
        SearchPage {
            effective_methods: Some(lexical),
            degradation_reason: "undocumented".into(),
            ..SearchPage::default()
        },
        SearchPage {
            effective_methods: Some(SearchMethods {
                title_only: true,
                ..SearchMethods::default()
            }),
            degradation_reason: "embedding_unavailable".into(),
            ..SearchPage::default()
        },
        SearchPage::default(),
    ];
    for search in invalid {
        assert!(std::panic::catch_unwind(|| assert_search_outcome(&requested, &search)).is_err());
    }
    let lexical_only = SearchMethods {
        semantic: false,
        ..requested.clone()
    };
    let unexpected_semantic = SearchPage {
        effective_methods: Some(requested),
        ..SearchPage::default()
    };
    assert!(
        std::panic::catch_unwind(|| assert_search_outcome(&lexical_only, &unexpected_semantic))
            .is_err()
    );
    let unrequested_degradation = SearchPage {
        effective_methods: Some(lexical_only.clone()),
        degradation_reason: "embedding_unavailable".into(),
        ..SearchPage::default()
    };
    assert!(
        std::panic::catch_unwind(|| assert_search_outcome(&lexical_only, &unrequested_degradation))
            .is_err()
    );
}
