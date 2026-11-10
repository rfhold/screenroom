#![cfg(not(target_arch = "wasm32"))]

use screenroom_client_core::*;
#[path = "support/pagination.rs"]
mod pagination;

#[tokio::test]
#[ignore = "attach to caller-owned seeded backend with SCREENROOM_API_ENDPOINT and SCREENROOM_USER_ID"]
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
    let scope = SourceLibraryScope::default();
    let page = Page {
        size: 2,
        token: String::new(),
    };
    client.clone().get_home(scope.clone()).await.unwrap();
    let info = client.clone().get_catalog_info().await.unwrap();
    assert!(info.notices.len() <= 32 && info.branding.len() <= 8);
    let logo = info
        .branding
        .iter()
        .find(|a| a.kind == "logo")
        .expect("seeded branding logo");
    assert!(
        !client
            .clone()
            .load_artwork(logo.url.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let libraries = pagination::collect("libraries", pagination::Limits::default(), |token| {
        let client = client.clone();
        let page = Page {
            token,
            ..page.clone()
        };
        async move {
            let response = client
                .list_libraries(String::new(), page)
                .await
                .map_err(|e| e.to_string())?;
            Ok((response.libraries, response.next_page_token))
        }
    })
    .await
    .expect("complete library traversal");
    assert!(!libraries.is_empty());
    let items = client
        .clone()
        .list_library_items(scope.clone(), page.clone())
        .await
        .unwrap();
    let item = client
        .clone()
        .get_item(
            items.items.first().expect("seeded catalog item").id.clone(),
            scope.clone(),
        )
        .await
        .unwrap();
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
    if let Some(art) = item.artwork {
        assert!(
            !client
                .clone()
                .load_artwork(art.url)
                .await
                .unwrap()
                .is_empty()
        );
    }
    client.clone().get_search_capabilities().await.unwrap();
    let search = client
        .clone()
        .search(SearchRequest {
            query: item.title,
            source: "all".into(),
            methods: default_settings().methods,
            page: Some(page.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let effective = search.effective_methods.expect("effective search methods");
    if !search.degradation_reason.is_empty() {
        assert!(!effective.semantic);
    }
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
}
