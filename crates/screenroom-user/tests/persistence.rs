use screenroom_backend::{Uuid, sqlx};
use screenroom_proto::{
    common::v1::SourceLibraryScope, user::internal::v1::GetContinueWatchingRequest,
};
use screenroom_user::{UserService, repository::Repository};
mod support;

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn persistence_links_isolate_users_and_missing_provider_is_unavailable() {
    let fixture = support::Fixture::new().await;
    let repository = Repository::new(fixture.user.clone());
    repository.ready().await.unwrap();
    let (one, two, unmapped) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for id in [one, two, unmapped] {
        repository.init_user(id, "Local viewer").await.unwrap();
        let before: screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc> =
            sqlx::query_scalar("SELECT updated_at FROM identity.users WHERE id=$1")
                .bind(id)
                .fetch_one(&fixture.user)
                .await
                .unwrap();
        repository.init_user(id, "Local viewer").await.unwrap();
        let after = sqlx::query_scalar::<
            _,
            screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc>,
        >("SELECT updated_at FROM identity.users WHERE id=$1")
        .bind(id)
        .fetch_one(&fixture.user)
        .await
        .unwrap();
        assert_eq!(before, after);
    }
    support::set_link(&fixture.user, one, "owned-source", "profile-one")
        .await
        .unwrap();
    support::set_link(&fixture.user, two, "owned-source", "profile-two")
        .await
        .unwrap();
    assert_eq!(
        repository
            .link(one, "owned-source")
            .await
            .unwrap()
            .as_deref(),
        Some("profile-one")
    );
    assert_eq!(
        repository
            .link(two, "owned-source")
            .await
            .unwrap()
            .as_deref(),
        Some("profile-two")
    );
    assert!(
        sqlx::query("SELECT id FROM catalog.source_items")
            .execute(&fixture.user)
            .await
            .is_err()
    );
    let service = UserService {
        repository: repository.clone(),
        connections: screenroom_backend::connections::ConnectionRepository::new(
            fixture.user.clone(),
            None,
        ),
    };
    let request = |id: Uuid| GetContinueWatchingRequest {
        user_id: id.to_string(),
        scopes: vec![SourceLibraryScope {
            source_id: "owned-source".into(),
            library_id: String::new(),
        }],
        page_size: 1,
        page_token: String::new(),
    };
    assert_eq!(
        service.progress(request(one)).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    assert!(
        service
            .progress(request(unmapped))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    let mut lock = fixture.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
        .bind(one)
        .execute(&mut *lock)
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let set = {
        let pool = fixture.user.clone();
        let b = barrier.clone();
        tokio::spawn(async move {
            b.wait().await;
            support::set_link(&pool, one, "owned-source", "profile-one").await
        })
    };
    let clear = {
        let pool = fixture.user.clone();
        let b = barrier.clone();
        tokio::spawn(async move {
            b.wait().await;
            support::clear_link(&pool, one, "owned-source").await
        })
    };
    barrier.wait().await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!set.is_finished() && !clear.is_finished());
    lock.commit().await.unwrap();
    set.await.unwrap().unwrap();
    clear.await.unwrap().unwrap();
    assert!(
        repository
            .link(one, "owned-source")
            .await
            .unwrap()
            .is_none_or(|v| v == "profile-one")
    );
    support::clear_link(&fixture.user, one, "owned-source")
        .await
        .unwrap();
    support::clear_link(&fixture.user, one, "owned-source")
        .await
        .unwrap();
    assert!(
        service
            .progress(request(one))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    sqlx::query("UPDATE identity.users SET state='disabled' WHERE id=$1")
        .bind(two)
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(
        service.progress(request(two)).await.unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    assert_eq!(
        repository.user(Uuid::new_v4()).await.err().unwrap().code(),
        tonic::Code::NotFound
    );
    fixture.cleanup().await;
}
