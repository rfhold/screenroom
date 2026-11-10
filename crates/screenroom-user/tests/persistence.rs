use screenroom_backend::{
    Uuid,
    sqlx::{self, postgres::PgPoolOptions},
};
use screenroom_catalog_adapter::{
    Faults, JELLYFIN_SOURCE, MOCK_USER_ALICE, MOCK_USER_BOB, MockAdapters,
};
use screenroom_proto::{
    common::v1::SourceLibraryScope, user::internal::v1::GetContinueWatchingRequest,
};
use screenroom_user::{UserService, repository::Repository};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires migrated disposable DB: USER_TEST_DATABASE_URL runtime role and USER_TEST_ADMIN_DATABASE_URL owner"]
async fn persistence_links_users_and_scoped_progress() {
    let url = std::env::var("USER_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .connect(&std::env::var("USER_TEST_ADMIN_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    let repository = Repository::new(pool.clone());
    repository.ready().await.unwrap();
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let absent = Uuid::new_v4();
    for (id, name) in [(alice, "Alice"), (bob, "Bob"), (absent, "Unmapped")] {
        repository.init_user(id, name).await.unwrap();
        let before: screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc> =
            sqlx::query_scalar("SELECT updated_at FROM identity.users WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        repository.init_user(id, name).await.unwrap();
        let after: screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc> =
            sqlx::query_scalar("SELECT updated_at FROM identity.users WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            before, after,
            "unchanged bootstrap must not update timestamps"
        );
    }
    repository
        .set_link(alice, JELLYFIN_SOURCE, MOCK_USER_ALICE)
        .await
        .unwrap();
    repository
        .set_link(alice, JELLYFIN_SOURCE, MOCK_USER_ALICE)
        .await
        .unwrap();
    repository
        .set_link(bob, JELLYFIN_SOURCE, MOCK_USER_BOB)
        .await
        .unwrap();
    assert!(
        sqlx::query("SELECT id FROM catalog.sources LIMIT 1")
            .execute(&pool)
            .await
            .is_err(),
        "runtime role must not access catalog"
    );
    pool.close().await;
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    let repository = Repository::new(pool.clone());
    assert_eq!(repository.user(alice).await.unwrap().display_name, "Alice");
    assert_eq!(
        repository
            .link(alice, JELLYFIN_SOURCE)
            .await
            .unwrap()
            .as_deref(),
        Some(MOCK_USER_ALICE)
    );
    let service = UserService {
        repository: repository.clone(),
        adapters: Arc::new(
            MockAdapters::load(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/catalog"),
                Default::default(),
            )
            .unwrap(),
        ),
    };
    let request = |id: Uuid, library: &str| GetContinueWatchingRequest {
        user_id: id.to_string(),
        scopes: vec![SourceLibraryScope {
            source_id: JELLYFIN_SOURCE.into(),
            library_id: library.into(),
        }],
        page_size: 1,
        page_token: String::new(),
    };
    let page = service.progress(request(alice, "")).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(!page.next_page_token.is_empty());
    let mut next = request(alice, "");
    next.page_token = page.next_page_token.clone();
    let second = service.progress(next.clone()).await.unwrap();
    let mut changed_scope = next.clone();
    changed_scope.scopes[0].library_id = "movies".into();
    assert_eq!(
        service.progress(changed_scope).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    let mut changed_size = next.clone();
    changed_size.page_size = 2;
    assert_eq!(
        service.progress(changed_size).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
    assert_ne!(
        page.entries[0].external_item_id,
        second.entries[0].external_item_id
    );
    next.user_id = bob.to_string();
    assert!(service.progress(next).await.is_err());
    let alice_tv = service.progress(request(alice, "tv")).await.unwrap();
    assert_eq!(
        alice_tv.entries[0].external_item_id,
        "beverly-hillbillies-s1-e1"
    );
    assert!(
        service
            .progress(request(bob, "tv"))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        service
            .progress(request(absent, ""))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    let mut bob_request = request(bob, "movies");
    bob_request.page_size = 200;
    let bob_page = service.progress(bob_request).await.unwrap();
    assert!(
        bob_page
            .entries
            .iter()
            .any(|e| e.external_item_id == "his-girl-friday")
    );
    assert_eq!(
        bob_page
            .entries
            .iter()
            .find(|e| e.external_item_id == "big-buck-bunny")
            .unwrap()
            .position_seconds,
        60
    );
    for faults in [
        Faults {
            unavailable_page: Some(0),
            ..Default::default()
        },
        Faults {
            rate_limit_page: Some(0),
            ..Default::default()
        },
        Faults {
            partial_page: Some(0),
            ..Default::default()
        },
        Faults {
            loop_cursor_page: Some(0),
            ..Default::default()
        },
    ] {
        let faulty = UserService {
            repository: repository.clone(),
            adapters: Arc::new(
                MockAdapters::load(
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/catalog"),
                    faults,
                )
                .unwrap(),
            ),
        };
        assert_eq!(
            faulty
                .progress(request(alice, ""))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unavailable
        );
    }
    let mut locked = admin.begin().await.unwrap();
    sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
        .bind(alice)
        .execute(&mut *locked)
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let set = {
        let repository = repository.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            repository
                .set_link(alice, JELLYFIN_SOURCE, MOCK_USER_ALICE)
                .await
        })
    };
    let clear = {
        let repository = repository.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            repository.clear_link(alice, JELLYFIN_SOURCE).await
        })
    };
    barrier.wait().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !set.is_finished() && !clear.is_finished(),
        "both operations must respect the user row lock"
    );
    locked.commit().await.unwrap();
    set.await.unwrap().unwrap();
    clear.await.unwrap().unwrap();
    assert!(
        repository
            .link(alice, JELLYFIN_SOURCE)
            .await
            .unwrap()
            .is_none_or(|value| value == MOCK_USER_ALICE)
    );
    repository.clear_link(alice, JELLYFIN_SOURCE).await.unwrap();
    repository.clear_link(alice, JELLYFIN_SOURCE).await.unwrap();
    assert!(
        service
            .progress(request(alice, ""))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    sqlx::query("UPDATE identity.users SET state='disabled' WHERE id=$1")
        .bind(bob)
        .execute(&admin)
        .await
        .unwrap();
    assert_eq!(
        service.progress(request(bob, "")).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    assert_eq!(
        repository.user(Uuid::new_v4()).await.err().unwrap().code(),
        tonic::Code::NotFound
    );
    for id in [alice, bob, absent] {
        sqlx::query("DELETE FROM identity.jellyfin_user_links WHERE user_id=$1")
            .bind(id)
            .execute(&admin)
            .await
            .unwrap();
        sqlx::query("DELETE FROM identity.users WHERE id=$1")
            .bind(id)
            .execute(&admin)
            .await
            .unwrap();
    }
}
