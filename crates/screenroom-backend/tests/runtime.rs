use screenroom_backend::media::*;
use screenroom_backend::{
    Uuid, catalog::*, db::MIGRATOR, embedding::input_hash, ingest::*, search::*, sqlx,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool,
    migrate::{Migration, MigrationType, Migrator},
    postgres::PgPoolOptions,
};
use std::borrow::Cow;
mod support;
use support::{CatalogRepository, IngestRepository, PolicyWrite};
#[path = "support/image.rs"]
mod neutral_image;

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_item_context_scope_and_source_ownership() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let (name, pool) = database(&admin, &url).await;
    MIGRATOR.run(&pool).await.unwrap();
    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let ingest = IngestRepository::new(worker_pool.clone());
    let source = catalog
        .configure_source("context", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    let library = catalog
        .configure_library(source.id, "context-library", "context-library", "Context")
        .await
        .unwrap();
    let policy = operator
        .create_policy(
            source.id,
            &policy(json!({"library_ids":["context-library"]})),
        )
        .await
        .unwrap();
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("context", 60).await.unwrap().unwrap();
    let mut season = item("context-season", "season", "context-library", "season");
    season.parent_public_id = Some("context-series".into());
    season.metadata["index"] = json!(2);
    let mut episode = item("context-episode", "episode", "context-library", "episode");
    episode.parent_public_id = Some("context-season".into());
    episode.metadata["index"] = json!(5);
    ingest
        .commit_fixture_page(
            &claim,
            &page(
                vec![
                    item("context-series", "series", "context-library", "series"),
                    season,
                    episode,
                ],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    let base = Scope {
        source_keys: vec![source.source_key.clone()],
        ..Scope::default()
    };
    let episode = catalog
        .get(&base, "context-episode")
        .await
        .unwrap()
        .unwrap();
    let season = catalog.get(&base, "context-season").await.unwrap().unwrap();
    let series = catalog.get(&base, "context-series").await.unwrap().unwrap();
    let scope = Scope {
        kinds: vec!["episode".into()],
        ..base.clone()
    };
    let ids = [episode.source_item_id];
    let contexts = catalog.item_contexts(&scope, &ids).await.unwrap();
    assert_eq!(
        contexts[&ids[0]].parent_metadata.as_ref().unwrap()["index"],
        2
    );
    assert_eq!(
        contexts[&ids[0]].grandparent_metadata.as_ref().unwrap()["title"],
        "context-series"
    );
    assert!(catalog.item_contexts(&scope, &[]).await.unwrap().is_empty());
    assert!(catalog.item_contexts(&scope, &[ids[0]; 201]).await.is_err());
    for excluded in [
        Scope {
            source_keys: vec!["other".into()],
            ..scope.clone()
        },
        Scope {
            providers: vec!["unsupported".into()],
            ..scope.clone()
        },
        Scope {
            library_ids: vec!["other".into()],
            ..scope.clone()
        },
        Scope {
            kinds: vec!["movie".into()],
            ..scope.clone()
        },
    ] {
        assert!(
            catalog
                .item_contexts(&excluded, &ids)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let other = catalog
        .configure_source("other-context", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    sqlx::query("INSERT INTO catalog.source_items(source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,'foreign-series','{\"title\":\"Wrong series\"}',decode(repeat('00',32),'hex'))")
        .bind(other.id).bind(series.item_id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE catalog.source_items SET state='absent' WHERE id=$1")
        .bind(series.source_item_id)
        .execute(&pool)
        .await
        .unwrap();
    let across = Scope {
        kinds: vec!["episode".into()],
        ..Scope::default()
    };
    assert!(
        catalog.item_contexts(&across, &ids).await.unwrap()[&ids[0]]
            .grandparent_metadata
            .is_none()
    );
    sqlx::query("UPDATE catalog.source_items SET state='enabled' WHERE id=$1")
        .bind(series.source_item_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalog.library_items SET state='absent' WHERE source_item_id=$1")
        .bind(season.source_item_id)
        .execute(&pool)
        .await
        .unwrap();
    let scoped_library = Scope {
        library_ids: vec![library.public_id],
        ..scope.clone()
    };
    assert!(
        catalog.item_contexts(&scoped_library, &ids).await.unwrap()[&ids[0]]
            .parent_metadata
            .is_none()
    );
    sqlx::query("UPDATE catalog.library_items SET state='enabled' WHERE source_item_id=$1")
        .bind(season.source_item_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalog.sources SET state='disabled' WHERE id=$1")
        .bind(source.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        catalog
            .item_contexts(&scope, &ids)
            .await
            .unwrap()
            .is_empty()
    );
    catalog_pool.close().await;
    worker_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_primary_artwork_kind_precedence() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let (name, pool) = database(&admin, &url).await;
    MIGRATOR.run(&pool).await.unwrap();
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let ingest = IngestRepository::new(worker_pool.clone());
    let publisher = MediaRepository::new(worker_pool.clone());
    let media = MediaRepository::new(catalog_pool.clone());
    let source = catalog
        .configure_source(
            "art-ranking",
            "jellyfin",
            &json!({"branding_artwork_ids":["brand"]}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "art-library", "art-library", "Art")
        .await
        .unwrap();
    let policy = operator
        .create_policy(source.id, &policy(json!({"library_ids":["art-library"]})))
        .await
        .unwrap();
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("art-ranking", 60).await.unwrap().unwrap();
    let mut first = page(
        ["movie", "series", "collection"]
            .into_iter()
            .map(|kind| item(kind, kind, "art-library", kind))
            .collect(),
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    let mut season = item("season", "season", "art-library", "season");
    season.parent_public_id = Some("series".into());
    let mut episode = item("episode", "episode", "art-library", "episode");
    episode.parent_public_id = Some("season".into());
    let mut direct = item("direct", "episode", "art-library", "direct");
    direct.parent_public_id = Some("series".into());
    first.items.extend([season, episode, direct]);
    for (target, kind) in [("season", "poster"), ("episode", "thumbnail")] {
        first
            .artwork_inputs
            .push(artwork_input(ArtworkReferenceInput {
                source_artwork_id: format!("{target}-own"),
                kind: kind.into(),
                rights: json!({}),
                item_external_id: Some(target.into()),
                person_external_id: None,
                role: "poster".into(),
                ordinal: 0,
            }));
    }
    for target in ["movie", "series", "collection"] {
        for (key, kind) in [
            ("a-logo", "logo"),
            ("b-backdrop", "backdrop"),
            ("c-thumbnail", "thumbnail"),
            ("y-poster", "poster"),
            ("z-poster", "poster"),
        ] {
            first
                .artwork_inputs
                .push(artwork_input(ArtworkReferenceInput {
                    source_artwork_id: format!("{target}-{key}"),
                    kind: kind.into(),
                    rights: json!({}),
                    item_external_id: Some(target.into()),
                    person_external_id: None,
                    role: "primary".into(),
                    ordinal: 0,
                }));
        }
    }
    for key in ["brand", "person"] {
        first
            .artwork_inputs
            .push(artwork_input(ArtworkReferenceInput {
                source_artwork_id: key.into(),
                kind: "logo".into(),
                rights: json!({}),
                item_external_id: None,
                person_external_id: (key == "person").then(|| "person1".into()),
                role: "primary".into(),
                ordinal: 0,
            }));
    }
    ingest.commit_fixture_page(&claim, &first).await.unwrap();
    let object = verified_object(source.id).await;
    for _ in 0..first.artwork_inputs.len() {
        let claim = ingest.claim("art-publish", 60).await.unwrap().unwrap();
        publisher.publish(&claim, &object).await.unwrap();
    }
    let scope = Scope {
        source_keys: vec![source.source_key.clone()],
        ..Scope::default()
    };
    let mut ids = Vec::new();
    for kind in ["movie", "series", "collection"] {
        ids.push(
            catalog
                .get(&scope, kind)
                .await
                .unwrap()
                .unwrap()
                .source_item_id,
        );
    }
    let assert_kind = async |kind: &str| {
        let selected = media.primary_item_artwork(&scope, &ids).await.unwrap();
        assert_eq!(selected.len(), 3);
        for art in selected.values() {
            assert_eq!(art.kind, kind);
            if kind == "poster" {
                assert!(art.source_artwork_id.ends_with("y-poster"));
            }
        }
    };
    assert_kind("poster").await;
    episode_card_artwork(&catalog, &media, &pool, &scope).await;
    for excluded in [
        Scope {
            source_keys: vec!["other".into()],
            ..Scope::default()
        },
        Scope {
            library_ids: vec!["other".into()],
            ..scope.clone()
        },
        Scope {
            providers: vec!["unsupported".into()],
            ..scope.clone()
        },
        Scope {
            kinds: vec!["episode".into()],
            ..scope.clone()
        },
    ] {
        assert!(
            media
                .primary_item_artwork(&excluded, &ids)
                .await
                .unwrap()
                .is_empty()
        );
    }
    for (kind, next) in [
        ("poster", "thumbnail"),
        ("thumbnail", "backdrop"),
        ("backdrop", "logo"),
    ] {
        sqlx::query("UPDATE media.artwork SET state='absent' WHERE source_id=$1 AND kind=$2")
            .bind(source.id)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
        assert_kind(next).await;
    }
    sqlx::query("UPDATE media.artwork SET state='enabled' WHERE source_id=$1")
        .bind(source.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE media.artwork_generations SET state='pending' WHERE source_id=$1 AND kind='poster'",
    )
    .bind(source.id)
    .execute(&pool)
    .await
    .unwrap();
    assert_kind("thumbnail").await;
    sqlx::query("UPDATE media.artwork_generations SET state='published' WHERE source_id=$1")
        .bind(source.id)
        .execute(&pool)
        .await
        .unwrap();
    // A published label alone cannot bypass required-variant completeness.
    sqlx::query("DELETE FROM media.variants WHERE artwork_id IN (SELECT id FROM media.artwork WHERE source_id=$1 AND kind='poster')").bind(source.id).execute(&pool).await.unwrap();
    assert_kind("thumbnail").await;
    let person = catalog.credits(&scope, "movie", None, 100).await.unwrap()[0].person_id;
    assert_eq!(
        media.person_artwork(&scope, &[person]).await.unwrap()[&person].source_artwork_id,
        "person"
    );
    assert_eq!(
        media.branding(&scope, 8).await.unwrap()[0].source_artwork_id,
        "brand"
    );
    sqlx::query("UPDATE catalog.sources SET state='disabled' WHERE id=$1")
        .bind(source.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        media
            .primary_item_artwork(&scope, &ids)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
    worker_pool.close().await;
    catalog_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

async fn episode_card_artwork(
    catalog: &CatalogRepository,
    media: &MediaRepository,
    pool: &PgPool,
    scope: &Scope,
) {
    let episode = catalog.get(scope, "episode").await.unwrap().unwrap();
    let season = catalog.get(scope, "season").await.unwrap().unwrap();
    let series = catalog.get(scope, "series").await.unwrap().unwrap();
    let direct = catalog.get(scope, "direct").await.unwrap().unwrap();
    let scope = Scope {
        kinds: vec!["episode".into()],
        ..scope.clone()
    };
    let ids = [episode.source_item_id, direct.source_item_id];
    let selected = media.card_item_artwork(&scope, &ids, None).await.unwrap();
    assert_eq!(selected[&ids[0]].source_artwork_id, "season-own");
    assert_eq!(selected[&ids[1]].source_artwork_id, "series-y-poster");
    let own = media
        .card_item_artwork(&scope, &ids, Some(season.item_id))
        .await
        .unwrap();
    assert_eq!(own[&ids[0]].source_artwork_id, "episode-own");
    assert_eq!(own[&ids[1]].source_artwork_id, "series-y-poster");
    assert_eq!(
        media
            .card_item_artwork(&scope, &ids, Some(series.item_id))
            .await
            .unwrap()[&ids[0]]
            .source_artwork_id,
        "season-own"
    );
    assert!(
        media
            .card_item_artwork(&scope, &[], None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .card_item_artwork(&scope, &[ids[0]; 201], None)
            .await
            .is_err()
    );
    for excluded in [
        Scope {
            source_keys: vec!["other".into()],
            ..scope.clone()
        },
        Scope {
            library_ids: vec!["other".into()],
            ..scope.clone()
        },
        Scope {
            providers: vec!["unsupported".into()],
            ..scope.clone()
        },
        Scope {
            kinds: vec!["movie".into()],
            ..scope.clone()
        },
    ] {
        assert!(
            media
                .card_item_artwork(&excluded, &ids, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let other_library = catalog
        .configure_library(
            episode.source_id,
            "ancestor-only",
            "ancestor-only",
            "Ancestors",
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) SELECT $1,source_item_id,source_id,last_seen_run_id FROM catalog.library_items WHERE source_item_id=$2")
        .bind(other_library.id).bind(season.source_item_id).execute(pool).await.unwrap();
    sqlx::query("UPDATE catalog.library_items SET state='absent' WHERE source_item_id=$1 AND library_id<>$2")
        .bind(season.source_item_id).bind(other_library.id).execute(pool).await.unwrap();
    let library_scope = Scope {
        library_ids: vec!["art-library".into()],
        ..scope.clone()
    };
    assert_eq!(
        media
            .card_item_artwork(&library_scope, &ids, None)
            .await
            .unwrap()[&ids[0]]
            .source_artwork_id,
        "series-y-poster"
    );
    assert_eq!(
        media.card_item_artwork(&scope, &ids, None).await.unwrap()[&ids[0]].source_artwork_id,
        "season-own"
    );
    sqlx::query("UPDATE catalog.library_items SET state='enabled' WHERE source_item_id=$1")
        .bind(season.source_item_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalog.libraries SET state='disabled' WHERE id=$1")
        .bind(other_library.id)
        .execute(pool)
        .await
        .unwrap();
    // Ineligible ancestors and non-posters must not mask the series poster.
    for statement in [
        "UPDATE catalog.source_items SET state='absent' WHERE id=$1",
        "UPDATE catalog.library_items SET state='absent' WHERE source_item_id=$1",
    ] {
        sqlx::query(statement)
            .bind(season.source_item_id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            media.card_item_artwork(&scope, &ids, None).await.unwrap()[&ids[0]].source_artwork_id,
            "series-y-poster"
        );
        sqlx::query("UPDATE catalog.source_items SET state='enabled' WHERE id=$1")
            .bind(season.source_item_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE catalog.library_items SET state='enabled' WHERE source_item_id=$1")
            .bind(season.source_item_id)
            .execute(pool)
            .await
            .unwrap();
    }
    for statement in [
        "UPDATE media.artwork SET state='absent' WHERE id=$1",
        "UPDATE media.artwork SET kind='thumbnail' WHERE id=$1",
        "UPDATE media.artwork_generations SET state='pending' WHERE artwork_id=$1",
        "UPDATE media.artwork_generations SET manifest='{\"version\":1,\"variants\":[{\"variant\":\"native\",\"sha256\":[]}]}' WHERE artwork_id=$1",
        "UPDATE media.objects SET state='absent' WHERE id IN (SELECT object_id FROM media.variants WHERE artwork_id=$1)",
    ] {
        let art = selected[&ids[0]].id;
        sqlx::query(statement)
            .bind(art)
            .execute(pool)
            .await
            .unwrap();
        // Objects can be shared across artworks, so readiness uses a separate assertion below.
        if !statement.contains("media.objects") {
            assert_eq!(
                media.card_item_artwork(&scope, &ids, None).await.unwrap()[&ids[0]]
                    .source_artwork_id,
                "series-y-poster"
            );
        } else {
            assert!(
                media
                    .card_item_artwork(&scope, &ids, None)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        sqlx::query("UPDATE media.artwork SET kind='poster',state='enabled' WHERE id=$1")
            .bind(art)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE media.artwork_generations SET state='published' WHERE artwork_id=$1")
            .bind(art)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE media.artwork_generations SET manifest=(SELECT g.manifest FROM media.artwork_generations g JOIN media.artwork a ON a.id=g.artwork_id WHERE a.source_artwork_id='series-y-poster' AND a.source_id=$2) WHERE artwork_id=$1")
            .bind(art).bind(episode.source_id).execute(pool).await.unwrap();
        sqlx::query("UPDATE media.objects SET state='ready' WHERE id IN (SELECT object_id FROM media.variants WHERE artwork_id=$1)").bind(art).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE media.artwork SET state='absent' WHERE source_id=$1 AND kind='poster'")
        .bind(episode.source_id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        media
            .card_item_artwork(&scope, &ids, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        media
            .card_item_artwork(&scope, &ids, Some(season.item_id))
            .await
            .unwrap()[&ids[0]]
            .source_artwork_id,
        "episode-own"
    );
    sqlx::query("UPDATE media.artwork SET state='enabled' WHERE source_id=$1")
        .bind(episode.source_id)
        .execute(pool)
        .await
        .unwrap();

    let mirror = catalog
        .configure_source("ancestor-mirror", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(mirror.id, "art-library", "art-library", "Mirror")
        .await
        .unwrap();
    let ingest = IngestRepository::new(pool.clone());
    let policy = ingest
        .create_policy(mirror.id, &policy(json!({"library_ids":["art-library"]})))
        .await
        .unwrap();
    ingest.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("mirror", 60).await.unwrap().unwrap();
    let mut mirror_season = item("season", "season", "art-library", "mirror season");
    mirror_season.parent_public_id = Some("series".into());
    let mut first = page(
        vec![
            item("series", "series", "art-library", "mirror series"),
            mirror_season,
        ],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    first
        .artwork_inputs
        .push(artwork_input(ArtworkReferenceInput {
            source_artwork_id: "mirror-season".into(),
            kind: "poster".into(),
            rights: json!({}),
            item_external_id: Some("season".into()),
            person_external_id: None,
            role: "primary".into(),
            ordinal: 0,
        }));
    ingest.commit_fixture_page(&claim, &first).await.unwrap();
    let claim = ingest.claim("mirror-art", 60).await.unwrap().unwrap();
    MediaRepository::new(pool.clone())
        .publish(&claim, &verified_object(mirror.id).await)
        .await
        .unwrap();
    sqlx::query("UPDATE media.artwork SET state='absent' WHERE source_id=$1 AND kind='poster'")
        .bind(episode.source_id)
        .execute(pool)
        .await
        .unwrap();
    let across_sources = Scope {
        kinds: vec!["episode".into()],
        ..Scope::default()
    };
    assert!(
        media
            .card_item_artwork(&across_sources, &ids, None)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE media.artwork SET state='enabled' WHERE source_id=$1")
        .bind(episode.source_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn database(admin: &PgPool, url: &str) -> (String, PgPool) {
    let name = format!("screenroom_p_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin)
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(url).unwrap();
    url.set_path(&format!("/{name}"));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(url.as_str())
        .await
        .unwrap();
    (name, pool)
}
async fn runtime_pool(pool: &PgPool, role: &'static str) -> PgPool {
    SearchRepository::new(pool.clone())
        .register_model("test-manual", "1", 1024)
        .await
        .unwrap();
    PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}

async fn verified_object(source: Uuid) -> screenroom_backend::storage::VerifiedObject {
    let bytes = neutral_image::png([64, 96, 128]);
    verified_bytes(source, bytes).await
}
async fn verified_bytes(
    source: Uuid,
    bytes: Vec<u8>,
) -> screenroom_backend::storage::VerifiedObject {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let body = bytes.clone();
    let server = tokio::spawn(async move {
        for method in ["PUT", "HEAD", "GET"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let end = loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(request[..end].to_vec()).unwrap();
            assert!(headers.starts_with(method));
            let length = headers
                .lines()
                .find_map(|s| {
                    s.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() - end < length {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let data = if method == "PUT" {
                &[][..]
            } else {
                body.as_slice()
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
                        data.len(), if body.starts_with(b"\x89PNG") { "image/png" } else { "image/jpeg" }
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            if method != "HEAD" {
                socket.write_all(data).await.unwrap();
            }
        }
    });
    let storage = screenroom_backend::storage::ObjectStorage::new(
        screenroom_backend::storage::StorageConfig {
            endpoint,
            region: "garage".into(),
            bucket: "synthetic-artwork".into(),
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
            timeout: std::time::Duration::from_secs(2),
        },
    )
    .unwrap();
    let verified = storage
        .upload_verified(
            source,
            &screenroom_backend::media::ValidatedImage::raster(bytes).unwrap(),
        )
        .await
        .unwrap();
    server.await.unwrap();
    verified
}
fn policy(selection: Value) -> PolicyWrite {
    PolicyWrite {
        enabled: true,
        selection_json: selection,
        max_items: 100,
        max_requests: 100,
        incremental_seconds: 60,
        reconcile_seconds: 3600,
        enrich_metadata: true,
        enrich_people: true,
        enrich_artwork: true,
    }
}
fn item(id: &str, kind: &str, library: &str, text: &str) -> ItemWrite {
    ItemWrite {
        public_id: id.into(),
        kind: kind.into(),
        parent_public_id: None,
        external_id: id.into(),
        source_revision: Some("r1".into()),
        metadata: json!({"title":id,"provider_references":[{"provider":"jellyfin","status":"available","url":"https://example.com/canonical"}]}),
        provenance: json!({"license":"synthetic-test"}),
        content_hash: input_hash(text).to_vec(),
        library_external_ids: vec![library.into()],
        external_ids: vec![ExternalIdWrite {
            provider: "synthetic".into(),
            entity_kind: kind.into(),
            external_id: id.into(),
            evidence: json!({"trusted":true}),
        }],
        credits: vec![CreditWrite {
            external_id: "person1".into(),
            name: "Synthetic Person".into(),
            metadata: json!({}),
            department: "acting".into(),
            role: "test".into(),
            ordinal: 0,
        }],
        document: text.into(),
        document_title: id.into(),
        document_version: 1,
    }
}
fn page(items: Vec<ItemWrite>, completion: Completion, cursor: Value) -> PageCommit {
    PageCommit {
        libraries: vec![],
        items,
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![],
        checkpoint: CheckpointWrite {
            scope_key: "selected".into(),
            cursor,
            watermark: None,
        },
        requests: 0,
        completion,
    }
}

fn artwork_input(reference: ArtworkReferenceInput) -> ArtworkInputWrite {
    let bytes = neutral_image::png([64, 96, 128]);
    ArtworkInputWrite {
        reference,
        manifest: ArtworkInputManifest {
            version: 1,
            variants: vec![ArtworkVariantInput {
                variant: ArtworkVariant::Native,
                sha256: Sha256::digest(bytes).into(),
            }],
        },
    }
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_request_reservation_contract() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let (name, pool) = database(&admin, &url).await;
    MIGRATOR.run(&pool).await.unwrap();
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let ingest = IngestRepository::new(worker_pool.clone());
    let source = catalog
        .configure_source("reserved", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    let mut write = policy(json!({"library_ids":["sample"]}));
    catalog
        .configure_library(source.id, "sample", "sample", "Sample")
        .await
        .unwrap();
    write.max_requests = 8;
    let policy = operator.create_policy(source.id, &write).await.unwrap();
    let run = operator.start_run(policy.id).await.unwrap();
    let first = ingest.claim("reserve-first", 60).await.unwrap().unwrap();
    let mut outcomes = Vec::new();
    for _ in 0..2 {
        let receipt = ingest
            .reserve_work_request(&first, OperationClass::Page)
            .await
            .unwrap();
        outcomes.push(RequestOutcomeWrite {
            receipt_id: receipt.id,
            outcome: RequestOutcome::Success,
            error_class: None,
        });
    }
    let mut rejected = page(vec![], Completion::Continue, json!({}));
    rejected.requests = -1;
    assert!(
        ingest
            .commit_traversal_page(&first, &rejected, &TraversalWrites::default())
            .await
            .is_err()
    );
    let charged = ingest.run(run.id).await.unwrap().unwrap();
    assert_eq!((charged.requests, charged.budget_requests), (2, 0));
    let mut reserved_page = page(vec![], Completion::Pause, json!({"next":1}));
    reserved_page.requests = 0;
    ingest
        .commit_traversal_page(
            &first,
            &reserved_page,
            &TraversalWrites {
                outcomes,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().requests, 2);
    operator.resume(run.id).await.unwrap();
    let second = ingest.claim("reserve-second", 60).await.unwrap().unwrap();
    assert!(
        ingest
            .reserve_work_request(&first, OperationClass::Page)
            .await
            .is_err()
    );
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().requests, 2);
    ingest
        .reserve_work_request(&second, OperationClass::Page)
        .await
        .unwrap();
    ingest.retry(&second, "test_retry", 0).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE id=$1")
        .bind(second.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&second, OperationClass::Page)
            .await
            .is_err()
    );
    let third = ingest.claim("reserve-third", 60).await.unwrap().unwrap();
    assert!(third.fence > second.fence);
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(third.id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        ingest
            .reserve_work_request(&third, OperationClass::Page)
            .await
            .is_err()
    );
    let fourth = ingest.claim("reserve-fourth", 60).await.unwrap().unwrap();
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(
        ingest
            .reserve_work_request(&fourth, OperationClass::Page)
            .await
            .is_err()
    );
    catalog.set_source_state(source.id, true).await.unwrap();
    sqlx::query("UPDATE ingest.policies SET enabled=false WHERE id=$1")
        .bind(policy.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&fourth, OperationClass::Page)
            .await
            .is_err()
    );
    sqlx::query("UPDATE ingest.policies SET enabled=true,version=version+1 WHERE id=$1")
        .bind(policy.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&fourth, OperationClass::Page)
            .await
            .is_err()
    );
    sqlx::query("UPDATE ingest.policies SET version=version-1 WHERE id=$1")
        .bind(policy.id)
        .execute(&pool)
        .await
        .unwrap();
    // A committed work unit resets only the per-job window, never lifetime usage.
    ingest
        .commit_fixture_page(
            &fourth,
            &page(vec![], Completion::Continue, json!({"next":2})),
        )
        .await
        .unwrap();
    let fifth = ingest.claim("reserve-fifth", 60).await.unwrap().unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().requests, 4);
    for _ in 0..3 {
        ingest
            .reserve_work_request(&fifth, OperationClass::Page)
            .await
            .unwrap();
    }
    let (a, b) = tokio::join!(
        ingest.reserve_work_request(&fifth, OperationClass::Page),
        ingest.reserve_work_request(&fifth, OperationClass::Page)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().requests, 8);
    assert!(
        ingest
            .reserve_work_request(&fifth, OperationClass::Page)
            .await
            .is_err()
    );
    let mut final_page = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    final_page.requests = 0;
    final_page
        .artwork_inputs
        .push(artwork_input(ArtworkReferenceInput {
            source_artwork_id: "reserved-logo".into(),
            kind: "logo".into(),
            rights: json!({"license":"test"}),
            item_external_id: None,
            person_external_id: None,
            role: "poster".into(),
            ordinal: 0,
        }));
    ingest
        .commit_traversal_page(&fifth, &final_page, &TraversalWrites::default())
        .await
        .unwrap();
    let art = ingest.claim("reserve-artwork", 60).await.unwrap().unwrap();
    assert_eq!(art.kind, "artwork");
    assert!(
        ingest
            .reserve_work_request(&art, OperationClass::Artwork)
            .await
            .is_err()
    );
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().requests, 8);
    ingest.complete_job(&art).await.unwrap();
    assert!(
        ingest
            .reserve_work_request(&art, OperationClass::Artwork)
            .await
            .is_err()
    );
    let persisted = IngestRepository::new(worker_pool.clone())
        .run(run.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((persisted.requests, persisted.budget_requests), (8, 0));
    // Cancellation must not refund a reservation.
    let cancelled = operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("reserve-cancel", 60).await.unwrap().unwrap();
    ingest
        .reserve_work_request(&claim, OperationClass::Page)
        .await
        .unwrap();
    operator.cancel_run(cancelled.id).await.unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .is_err()
    );
    assert_eq!(ingest.run(cancelled.id).await.unwrap().unwrap().requests, 1);
    worker_pool.close().await;
    catalog_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_canonical_manual_scheduling_and_non_authoritative_completion() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let (name, pool) = database(&admin, &url).await;
    MIGRATOR.run(&pool).await.unwrap();
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let ingest = IngestRepository::new(worker_pool.clone());
    let source = catalog
        .configure_source("one-shot", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "sample", "sample", "Sample")
        .await
        .unwrap();
    let write = policy(json!({"library_ids":["sample"]}));
    let p = operator.create_policy(source.id, &write).await.unwrap();
    assert!(!p.automatic_runs);
    assert!(ingest.due_policies(100).await.unwrap().is_empty());
    let seed = operator.start_run(p.id).await.unwrap();
    assert!(ingest.due_policies(100).await.unwrap().is_empty());
    let claim = ingest.claim("seed", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &claim,
            &page(
                vec![item("unseen", "movie", "sample", "old")],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        ingest.run(seed.id).await.unwrap().unwrap().state,
        "complete"
    );
    let finite: bool = sqlx::query_scalar("SELECT isfinite(next_due_at) AND next_due_at>now() AND NOT automatic_runs FROM ingest.policies WHERE id=$1").bind(p.id).fetch_one(&pool).await.unwrap();
    assert!(finite);
    let run = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("sample", 60).await.unwrap().unwrap();
    let mut first = page(
        vec![item("sample-item", "movie", "sample", "new")],
        Completion::Continue,
        json!({"next":1}),
    );
    first.followups.push(Followup {
        kind: JobKind::Embedding,
        dedupe_key: "sample-embedding".into(),
        input: json!({"external_id":"sample-item"}),
    });
    first
        .artwork_inputs
        .push(artwork_input(ArtworkReferenceInput {
            source_artwork_id: "sample-logo".into(),
            kind: "logo".into(),
            rights: json!({"license":"synthetic-test"}),
            item_external_id: Some("sample-item".into()),
            person_external_id: None,
            role: "poster".into(),
            ordinal: 0,
        }));
    ingest.commit_fixture_page(&claim, &first).await.unwrap();
    // Keep the ingest job first, leaving both followup kinds pending across pause/resume.
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 hour' WHERE scope_id=$1 AND kind<>'ingest'").bind(run.id).execute(&pool).await.unwrap();
    let claim = ingest.claim("pause", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(&claim, &page(vec![], Completion::Pause, json!({"next":2})))
        .await
        .unwrap();
    assert_eq!(operator.start_run(p.id).await.unwrap().id, run.id);
    assert!(ingest.due_policies(100).await.unwrap().is_empty());
    operator.resume(run.id).await.unwrap();
    let claim = ingest.claim("finish", 60).await.unwrap().unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE scope_id=$1 AND kind<>'ingest'")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    let leased = ingest.claim("leased-followup", 60).await.unwrap().unwrap();
    assert_ne!(leased.kind, "ingest");
    let final_page = page(
        vec![],
        Completion::Complete {
            evidence: json!({"non_authoritative":true,"sample_items":1}),
        },
        json!({"done":true}),
    );
    let mut stale = claim.clone();
    stale.fence += 1;
    assert!(
        ingest
            .commit_fixture_page(&stale, &final_page)
            .await
            .is_err()
    );
    assert!(!ingest.policy(p.id).await.unwrap().unwrap().automatic_runs);
    ingest
        .commit_fixture_page(&claim, &final_page)
        .await
        .unwrap();
    assert!(
        ingest
            .commit_fixture_page(&claim, &final_page)
            .await
            .is_err()
    );
    let completed = ingest.run(run.id).await.unwrap().unwrap();
    assert_eq!(completed.state, "complete");
    assert_eq!(
        completed.coverage,
        json!({"non_authoritative":true,"sample_items":1})
    );
    assert!(
        catalog
            .get(&Scope::default(), "unseen")
            .await
            .unwrap()
            .is_some()
    );
    sqlx::query("UPDATE ingest.policies SET next_due_at=now()-interval '100 years' WHERE id=$1")
        .bind(p.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(ingest.due_policies(100).await.unwrap().is_empty());
    let search = SearchRepository::new(worker_pool.clone());
    let model = search
        .register_model("one-shot-model", "r1", 1024)
        .await
        .unwrap();
    let media = MediaRepository::new(worker_pool.clone());
    let verified = verified_object(source.id).await;
    let pending = ingest.claim("pending-followup", 60).await.unwrap().unwrap();
    assert_ne!(pending.kind, leased.kind);
    for followup in [leased, pending] {
        let mut stale = followup.clone();
        stale.fence += 1;
        assert!(ingest.complete_job(&stale).await.is_err());
        if followup.kind == "artwork" {
            media.publish(&followup, &verified).await.unwrap();
        } else {
            let item = catalog
                .get(&Scope::default(), "sample-item")
                .await
                .unwrap()
                .unwrap();
            let document = search
                .document_for_source_item(item.source_item_id)
                .await
                .unwrap()
                .unwrap();
            let mut vector = vec![0.0; 1024];
            vector[0] = 1.0;
            search
                .commit_embedding(
                    &followup,
                    document.id,
                    model.id,
                    &model.revision,
                    &document.content_hash,
                    &vector,
                )
                .await
                .unwrap();
        }
    }
    let (updated, _) = operator.update_policy(p.id, &write, false).await.unwrap();
    assert!(!updated.automatic_runs && updated.enabled);
    let explicit = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("explicit", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &claim,
            &page(
                vec![],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        ingest.run(explicit.id).await.unwrap().unwrap().state,
        "complete"
    );
    worker_pool.close().await;
    let restarted_pool = runtime_pool(&pool, "screenroom_worker").await;
    let restarted = IngestRepository::new(restarted_pool.clone());
    let persisted = restarted.policy(p.id).await.unwrap().unwrap();
    assert!(persisted.enabled);
    assert!(!persisted.automatic_runs);
    assert!(restarted.due_policies(100).await.unwrap().is_empty());
    assert!(!restarted.policies(None).await.unwrap()[0].automatic_runs);
    restarted_pool.close().await;
    catalog_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_runtime_contract() {
    let url =
        std::env::var("SCREENROOM_TEST_DATABASE_URL").expect("explicit task-owned database URL");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let (name, pool) = database(&admin, &url).await;
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        MIGRATOR.iter().count() as i64
    );
    let version: String = sqlx::query_scalar("SHOW server_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(version.starts_with("18."));
    let vector: String =
        sqlx::query_scalar("SELECT extversion FROM pg_extension WHERE extname='vector'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(vector, "0.8.2");

    let mut altered = MIGRATOR.migrations.to_vec();
    altered[0].checksum = Cow::Owned(vec![0; 48]);
    let changed = Migrator {
        migrations: Cow::Owned(altered),
        ..Migrator::DEFAULT
    };
    let mut connection = pool.acquire().await.unwrap();
    assert!(changed.run(&mut *connection).await.is_err());
    connection.close().await.unwrap();
    let mut bad = MIGRATOR.migrations.to_vec();
    bad.push(Migration::new(
        202611100099,
        "failure-test".into(),
        MigrationType::Simple,
        "CREATE TABLE public.must_rollback(id integer); SELECT missing_column;".into(),
        false,
    ));
    let bad = Migrator {
        migrations: Cow::Owned(bad),
        ..Migrator::DEFAULT
    };
    let mut connection = pool.acquire().await.unwrap();
    assert!(bad.run(&mut *connection).await.is_err());
    connection.close().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.must_rollback')::text")
            .fetch_one(&pool)
            .await
            .unwrap(),
        None
    );

    let (upgrade_name, upgrade) = database(&admin, &url).await;
    let first = Migrator {
        migrations: Cow::Owned(vec![MIGRATOR.migrations[0].clone()]),
        ..Migrator::DEFAULT
    };
    first.run(&upgrade).await.unwrap();
    sqlx::query(
        "INSERT INTO identity.users(public_id,display_name) VALUES('upgrade-user','Upgrade')",
    )
    .execute(&upgrade)
    .await
    .unwrap();
    let old_source: Uuid = sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider) VALUES('mock-jellyfin','jellyfin') RETURNING id").fetch_one(&upgrade).await.unwrap();
    sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) SELECT id,$1,'jf-upgrade' FROM identity.users WHERE public_id='upgrade-user'")
        .bind(old_source).execute(&upgrade).await.unwrap();
    let unresolved = Uuid::new_v4();
    sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) SELECT id,$1,'unresolved' FROM identity.users WHERE public_id='upgrade-user'")
        .bind(unresolved).execute(&upgrade).await.unwrap();
    let mut connection = upgrade.acquire().await.unwrap();
    assert!(MIGRATOR.run(&mut *connection).await.is_err());
    connection.close().await.unwrap();
    sqlx::query("DELETE FROM identity.jellyfin_user_links WHERE source_id=$1")
        .bind(unresolved)
        .execute(&upgrade)
        .await
        .unwrap();
    let old_policy: Uuid = sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,'{\"library_ids\":[\"movies\"]}',100,100,60,3600,true,true,true) RETURNING id").bind(old_source).fetch_one(&upgrade).await.unwrap();
    let old_run: Uuid = sqlx::query_scalar("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,state) VALUES($1,1,'{}','{\"library_ids\":[\"movies\"]}','complete') RETURNING id").bind(old_policy).fetch_one(&upgrade).await.unwrap();
    let legacy_object: Uuid = sqlx::query_scalar("INSERT INTO media.objects(storage_scope,sha256,bucket,object_key,mime,byte_length,width,height) VALUES($1,$2,'legacy','legacy-object','image/png',1,1,1) RETURNING id")
        .bind(old_source).bind([7u8;32].as_slice()).fetch_one(&upgrade).await.unwrap();
    let legacy_native: Uuid = sqlx::query_scalar("INSERT INTO media.objects(storage_scope,sha256,bucket,object_key,mime,byte_length,width,height) VALUES($1,$2,'legacy','legacy-native','image/png',1,1,1) RETURNING id")
        .bind(old_source).bind([8u8;32].as_slice()).fetch_one(&upgrade).await.unwrap();
    let legacy_complete: Uuid = sqlx::query_scalar("INSERT INTO media.artwork(source_id,source_artwork_id,kind,rights,revision) VALUES($1,'legacy-complete','logo','{\"license\":\"legacy\"}',1) RETURNING id")
        .bind(old_source).fetch_one(&upgrade).await.unwrap();
    let legacy_partial: Uuid = sqlx::query_scalar("INSERT INTO media.artwork(source_id,source_artwork_id,kind,rights,revision) VALUES($1,'legacy-partial','logo','{}',1) RETURNING id")
        .bind(old_source).fetch_one(&upgrade).await.unwrap();
    for (art, variants) in [
        (legacy_complete, vec!["original", "native"]),
        (legacy_partial, vec!["native"]),
    ] {
        for variant in variants {
            sqlx::query("INSERT INTO media.variants(artwork_id,source_id,revision,variant,object_id) VALUES($1,$2,1,$3,$4)")
                .bind(art).bind(old_source).bind(variant).bind(if variant == "native" { legacy_native } else { legacy_object }).execute(&upgrade).await.unwrap();
        }
    }
    let old_jobs: String=sqlx::query_scalar("SELECT oid::regclass::text FROM pg_class WHERE relname='jobs' AND relnamespace=(SELECT oid FROM pg_namespace WHERE nspname='ingest')").fetch_one(&upgrade).await.unwrap();
    sqlx::query(&format!("INSERT INTO {old_jobs}(run_id,kind,dedupe_key,input) VALUES($1,'artwork','legacy-unbound-art','{{}}')"))
        .bind(old_run).execute(&upgrade).await.unwrap();
    MIGRATOR.run(&upgrade).await.unwrap();
    assert!(
        IngestRepository::new(upgrade.clone())
            .policy(old_policy)
            .await
            .unwrap()
            .unwrap()
            .automatic_runs
    );
    let legacy_media = MediaRepository::new(upgrade.clone());
    assert_eq!(
        legacy_media
            .list(&Scope::default(), None, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        legacy_media
            .resolve(&Scope::default(), legacy_complete, 1, "native")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        legacy_media
            .resolve(&Scope::default(), legacy_partial, 1, "native")
            .await
            .unwrap()
            .is_none()
    );
    for object in [legacy_object, legacy_native] {
        sqlx::query("UPDATE media.objects SET state='absent' WHERE id=$1")
            .bind(object)
            .execute(&upgrade)
            .await
            .unwrap();
        assert!(
            legacy_media
                .list(&Scope::default(), None, 100)
                .await
                .unwrap()
                .is_empty()
        );
        for variant in ["native", "original"] {
            assert!(
                legacy_media
                    .resolve(&Scope::default(), legacy_complete, 1, variant)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        sqlx::query("UPDATE media.objects SET state='ready' WHERE id=$1")
            .bind(object)
            .execute(&upgrade)
            .await
            .unwrap();
        assert_eq!(
            legacy_media
                .list(&Scope::default(), None, 100)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM queue.jobs WHERE dedupe_key='legacy-unbound-art'"
        )
        .fetch_one(&upgrade)
        .await
        .unwrap(),
        "cancelled"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT source_id FROM ingest.runs WHERE id=$1")
            .bind(old_run)
            .fetch_one(&upgrade)
            .await
            .unwrap(),
        old_source
    );
    let user_pool = runtime_pool(&upgrade, "screenroom_user").await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT source_id FROM identity.jellyfin_user_links")
            .fetch_one(&user_pool)
            .await
            .unwrap(),
        "mock-jellyfin"
    );
    sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) SELECT id,'mock-jellyfin','runtime-write' FROM identity.users WHERE public_id='upgrade-user' ON CONFLICT(user_id,source_id) DO UPDATE SET jellyfin_user_id=EXCLUDED.jellyfin_user_id").execute(&user_pool).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT jellyfin_user_id FROM identity.jellyfin_user_links WHERE source_id='mock-jellyfin'").fetch_one(&user_pool).await.unwrap(), "runtime-write");
    assert!(
        sqlx::query("UPDATE identity.jellyfin_user_links SET source_id=''")
            .execute(&user_pool)
            .await
            .is_err()
    );
    user_pool.close().await;
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT display_name FROM identity.users WHERE public_id='upgrade-user'"
        )
        .fetch_one(&upgrade)
        .await
        .unwrap(),
        "Upgrade"
    );
    upgrade.close().await;
    sqlx::query(&format!("DROP DATABASE {upgrade_name}"))
        .execute(&admin)
        .await
        .unwrap();

    for role in ["screenroom_user", "screenroom_catalog", "screenroom_worker"] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let allowed = if role == "screenroom_user" {
            "SELECT * FROM identity.users"
        } else {
            "SELECT * FROM catalog.sources"
        };
        sqlx::query(allowed).execute(&mut *tx).await.unwrap();
        let forbidden = match role {
            "screenroom_user" => "SELECT * FROM catalog.source_items",
            "screenroom_catalog" => "SELECT * FROM identity.jellyfin_user_links",
            _ => "SELECT * FROM identity.users",
        };
        assert!(sqlx::query(forbidden).execute(&mut *tx).await.is_err());
        tx.rollback().await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        assert!(
            sqlx::query("CREATE TABLE catalog.runtime_ddl(id int)")
                .execute(&mut *tx)
                .await
                .is_err()
        );
        tx.rollback().await.unwrap();
    }

    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let ingest = IngestRepository::new(worker_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let search = SearchRepository::new(worker_pool.clone());
    let search_reader = SearchRepository::new(catalog_pool.clone());
    let source = catalog
        .configure_source("synthetic-jellyfin", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    let movies = catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "tv", "tv", "TV")
        .await
        .unwrap();
    let p = operator
        .create_policy(
            source.id,
            &policy(json!({"kind":"libraries","library_ids":["movies","tv"]})),
        )
        .await
        .unwrap();
    let run = operator.start_run(p.id).await.unwrap();
    assert_eq!(operator.start_run(p.id).await.unwrap().id, run.id);
    assert!(
        sqlx::query("UPDATE ingest.runs SET selection_snapshot='{}' WHERE id=$1")
            .bind(run.id)
            .execute(&pool)
            .await
            .is_err()
    );
    let mut locked = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM queue.jobs WHERE scope_id=$1 FOR UPDATE")
        .bind(run.id)
        .execute(&mut *locked)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            ingest.claim("skip-locked", 60)
        )
        .await
        .unwrap()
        .unwrap()
        .is_none()
    );
    locked.rollback().await.unwrap();
    let c = ingest.claim("worker-one", 60).await.unwrap().unwrap();
    assert!(ingest.claim("worker-two", 60).await.unwrap().is_none());
    ingest.renew(&c, 60).await.unwrap();
    let alpha = item("alpha", "movie", "movies", "rabbit adventure");
    let beta = item("beta", "series", "tv", "spaceship mystery");
    let mut invalid = alpha.clone();
    invalid.credits[0].ordinal = -1;
    let invalid = page(
        vec![beta.clone(), invalid],
        Completion::Continue,
        json!({"next":"bad"}),
    );
    assert!(ingest.commit_fixture_page(&c, &invalid).await.is_err());
    assert!(
        catalog
            .get(&Scope::default(), "beta")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        ingest
            .checkpoint(p.id, "selected", p.version)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(c.id)
    .execute(&pool)
    .await
    .unwrap();
    let newer = ingest.claim("worker-two", 60).await.unwrap().unwrap();
    assert!(newer.fence > c.fence);
    assert!(ingest.renew(&c, 60).await.is_err());
    assert!(
        ingest
            .commit_fixture_page(
                &c,
                &page(vec![alpha.clone()], Completion::Continue, json!({"next":1}))
            )
            .await
            .is_err()
    );
    let mut first_page = page(
        vec![alpha.clone(), beta.clone()],
        Completion::Complete {
            evidence: json!({"authoritative":false}),
        },
        json!({"done":true}),
    );
    for id in ["alpha", "beta"] {
        first_page.followups.push(Followup {
            kind: JobKind::Embedding,
            dedupe_key: format!("embed:{id}"),
            input: json!({"external_id":id,"public_id":id}),
        });
    }
    let artwork = ArtworkReferenceInput {
        source_artwork_id: "synthetic-logo".into(),
        kind: "logo".into(),
        rights: json!({"license":"synthetic-test","attribution":"Synthetic"}),
        item_external_id: Some("alpha".into()),
        person_external_id: Some("person1".into()),
        role: "poster".into(),
        ordinal: 0,
    };
    first_page.artwork_inputs.push(artwork_input(artwork));
    ingest
        .commit_fixture_page(&newer, &first_page)
        .await
        .unwrap();
    let all = catalog.list(&Scope::default(), None, 100).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        all[0].metadata["provider_references"][0]["url"],
        "https://example.com/canonical"
    );
    let movie_scope = Scope {
        library_ids: vec!["movies".into()],
        ..Scope::default()
    };
    assert_eq!(
        catalog.list(&movie_scope, None, 100).await.unwrap().len(),
        1
    );
    assert_eq!(
        catalog
            .credits(&movie_scope, "alpha", None, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        catalog
            .credits(&movie_scope, "beta", None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(catalog.libraries(&Scope::default()).await.unwrap().len(), 2);
    let libraries = catalog
        .library_page(&Scope::default(), None, 1)
        .await
        .unwrap();
    assert_eq!(libraries[0].public_id, "movies");
    let next = catalog
        .library_page(&Scope::default(), Some("movies"), 2)
        .await
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].public_id, "tv");
    assert!(
        catalog
            .children(&Scope::default(), "alpha", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        catalog
            .resolve_reference(&movie_scope, "synthetic-jellyfin", "alpha")
            .await
            .unwrap()
            .unwrap()
            .public_id,
        "alpha"
    );
    let ranked = catalog
        .get_ranked(&movie_scope, &["beta".into(), "alpha".into()])
        .await
        .unwrap();
    assert_eq!(
        ranked
            .iter()
            .map(|row| row.public_id.as_str())
            .collect::<Vec<_>>(),
        ["alpha"]
    );
    let references = catalog
        .resolve_references(
            &movie_scope,
            &[
                ("synthetic-jellyfin".into(), "beta".into()),
                ("synthetic-jellyfin".into(), "alpha".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].0, 1);
    assert_eq!(references[0].1.source_item_id, ranked[0].source_item_id);

    let model = search
        .register_model("synthetic-model", "operator-r1", 1024)
        .await
        .unwrap();
    assert!(search.activate_model(model.id).await.is_err());
    let mut unit = vec![0.0; 1024];
    unit[0] = 1.0;
    let media = screenroom_backend::media::MediaRepository::new(worker_pool.clone());
    let media_reader = screenroom_backend::media::MediaRepository::new(catalog_pool.clone());
    let verified = verified_object(source.id).await;
    for _ in 0..3 {
        let c = ingest.claim("vectors", 60).await.unwrap().unwrap();
        if c.kind == "artwork" {
            let id = media.publish(&c, &verified).await.unwrap();
            assert!(
                media_reader
                    .resolve(&movie_scope, id, 1, "native")
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                media_reader
                    .resolve(
                        &Scope {
                            library_ids: vec!["tv".into()],
                            ..Scope::default()
                        },
                        id,
                        1,
                        "native"
                    )
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                media_reader
                    .resolve(&movie_scope, id, 2, "native")
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                media_reader
                    .resolve(&movie_scope, id, 1, "original")
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                media_reader
                    .list(&movie_scope, None, 100)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            continue;
        }
        let id = c.input["public_id"].as_str().unwrap();
        let item = catalog.get(&Scope::default(), id).await.unwrap().unwrap();
        let d = search
            .document_for_source_item(item.source_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            search
                .commit_embedding(&c, d.id, model.id, &model.revision, &[9; 32], &unit)
                .await
                .is_err()
        );
        let mut v = vec![0.0; 1024];
        v[if id == "alpha" { 0 } else { 1 }] = 1.0;
        assert!(
            search
                .commit_embedding(&c, d.id, model.id, "wrong-revision", &d.content_hash, &v)
                .await
                .is_err()
        );
        search
            .commit_embedding(&c, d.id, model.id, &model.revision, &d.content_hash, &v)
            .await
            .unwrap();
    }
    search.activate_model(model.id).await.unwrap();
    assert_eq!(
        search.active_model().await.unwrap().unwrap().revision,
        "operator-r1"
    );
    assert!(
        search
            .documents_needing_embedding(model.id, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let vectors = search
        .vector(&Scope::default(), &unit, model.id, 100)
        .await
        .unwrap();
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].public_id, "alpha");
    let lexical = search
        .lexical(
            &Scope::default(),
            "spaceship",
            &LexicalMethods {
                full_text: true,
                ..Default::default()
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(lexical.len(), 1);
    assert_eq!(lexical[0].public_id, "beta");
    let exact_title = search
        .lexical(
            &Scope::default(),
            "alpha",
            &LexicalMethods {
                title: true,
                ..Default::default()
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(exact_title.len(), 1);
    let fuzzy = search
        .lexical(
            &Scope::default(),
            "alph",
            &LexicalMethods {
                fuzzy: true,
                ..Default::default()
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(fuzzy.len(), 1);
    let methods = vec!["lexical".into(), "vector".into()];
    let digest = snapshot_digest("spaceship", &Scope::default(), &methods).unwrap();
    let snapshot = search_reader
        .save_snapshot(&SnapshotWrite {
            digest: digest.to_vec(),
            model_revision: Some("operator-r1".into()),
            index_revision: "v1".into(),
            results: reciprocal_rank_fusion(&[lexical, vectors], 100),
            effective_methods: methods,
            degradation_reason: String::new(),
            ttl_seconds: 60,
        })
        .await
        .unwrap();
    let first = search
        .snapshot_page(snapshot, &digest, &Scope::default(), 0, 1)
        .await
        .unwrap();
    assert_eq!(first.results.len(), 1);
    assert_eq!(first.next_offset, Some(1));
    assert!(
        search
            .snapshot_page(snapshot, &[0; 32], &Scope::default(), 0, 10)
            .await
            .is_err()
    );
    let alpha_source = catalog.get(&movie_scope, "alpha").await.unwrap().unwrap();
    let old_document = search
        .document_for_source_item(alpha_source.source_item_id)
        .await
        .unwrap()
        .unwrap();
    let refresh_run = operator.start_run(p.id).await.unwrap();
    assert!(
        ingest
            .run_checkpoint(&refresh_run, "selected")
            .await
            .unwrap()
            .is_none()
    );
    let refresh = ingest.claim("refresh", 60).await.unwrap().unwrap();
    let mut changed_alpha = alpha.clone();
    changed_alpha.document = "a refreshed rabbit adventure".into();
    changed_alpha.content_hash = input_hash(&changed_alpha.document).to_vec();
    let mut refresh_page = page(
        vec![changed_alpha],
        Completion::Complete {
            evidence: json!({"refresh":true}),
        },
        json!({"done":true}),
    );
    refresh_page.followups.push(Followup {
        kind: JobKind::Embedding,
        dedupe_key: "embed:alpha:changed".into(),
        input: json!({"external_id":"alpha"}),
    });
    ingest
        .commit_fixture_page(&refresh, &refresh_page)
        .await
        .unwrap();
    assert_eq!(
        search
            .vector(&Scope::default(), &unit, model.id, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search
            .documents_needing_embedding(model.id, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    let embedding = ingest.claim("refresh-vector", 60).await.unwrap().unwrap();
    assert!(
        search
            .commit_embedding(
                &embedding,
                old_document.id,
                model.id,
                &model.revision,
                &old_document.content_hash,
                &unit
            )
            .await
            .is_err()
    );
    let current = search
        .document_for_source_item(alpha_source.source_item_id)
        .await
        .unwrap()
        .unwrap();
    search
        .commit_embedding(
            &embedding,
            current.id,
            model.id,
            &model.revision,
            &current.content_hash,
            &unit,
        )
        .await
        .unwrap();
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(
        search
            .snapshot_page(snapshot, &digest, &Scope::default(), 0, 100)
            .await
            .unwrap()
            .results
            .is_empty()
    );
    assert!(
        search
            .vector(&Scope::default(), &unit, model.id, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media_reader
            .list(&Scope::default(), None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    catalog.set_source_state(source.id, true).await.unwrap();

    // Completion cannot infer absence in a narrowed traversal.
    let (_, run2) = ingest
        .update_policy(
            p.id,
            &policy(json!({"kind":"libraries","library_ids":["movies"]})),
            true,
        )
        .await
        .unwrap();
    let run2 = run2.unwrap();
    let c = ingest.claim("reconcile", 60).await.unwrap().unwrap();
    assert!(
        ingest
            .commit_fixture_page(
                &c,
                &page(
                    vec![beta.clone()],
                    Completion::Complete {
                        evidence: json!({})
                    },
                    json!({"done":true})
                )
            )
            .await
            .is_err()
    );
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({"done":true}),
            ),
        )
        .await
        .unwrap();
    assert!(
        catalog
            .get(&Scope::default(), "alpha")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        catalog
            .get(&Scope::default(), "beta")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        ingest.run(run2.id).await.unwrap().unwrap().state,
        "complete"
    );

    // A policy update fences a previously valid claim, and a source disable blocks writes.
    let stale_run = ingest.start_run(p.id).await.unwrap();
    let c = ingest.claim("stale-policy", 60).await.unwrap().unwrap();
    ingest.cancel_run(stale_run.id).await.unwrap();
    ingest
        .update_policy(p.id, &policy(json!({"library_ids":["movies"]})), false)
        .await
        .unwrap();
    assert!(ingest.complete_job(&c).await.is_err());
    ingest.start_run(p.id).await.unwrap();
    let c = ingest.claim("disabled-source", 60).await.unwrap().unwrap();
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(
        ingest
            .commit_fixture_page(&c, &page(vec![alpha], Completion::Continue, json!({})))
            .await
            .is_err()
    );
    ingest.cancel_run(c.run_id.unwrap()).await.unwrap();

    // Database constraints independently prohibit cross-source memberships and credits.
    catalog.set_source_state(source.id, true).await.unwrap();
    let other_source = catalog
        .configure_source("constraint-other", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    let source_item: Uuid = sqlx::query_scalar(
        "SELECT id FROM catalog.source_items WHERE source_id=$1 ORDER BY id LIMIT 1",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) VALUES($1,$2,$3,$4)").bind(movies.id).bind(source_item).bind(other_source.id).bind(run.id).execute(&pool).await.is_err());
    catalog.set_source_state(source.id, true).await.unwrap();
    let mut bounded = policy(json!({"library_ids":["movies"]}));
    bounded.max_items = 1;
    bounded.max_requests = 8;
    let bounded = operator.create_policy(source.id, &bounded).await.unwrap();
    let bounded_run = operator.start_run(bounded.id).await.unwrap();
    let c = ingest.claim("budget", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![item("budget-item", "movie", "movies", "budget document")],
                Completion::Pause,
                json!({"next":"durable"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        ingest.run(bounded_run.id).await.unwrap().unwrap().state,
        "paused"
    );
    assert!(ingest.claim("budget", 60).await.unwrap().is_none());
    ingest.resume(bounded_run.id).await.unwrap();
    assert_eq!(
        ingest
            .checkpoint(bounded.id, "selected", bounded.version)
            .await
            .unwrap()
            .unwrap()["next"],
        "durable"
    );
    let c = ingest.claim("budget-resumed", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({"done":true}),
            ),
        )
        .await
        .unwrap();
    let totals = ingest.run(bounded_run.id).await.unwrap().unwrap();
    assert_eq!(totals.items, 1);
    assert_eq!(totals.requests, 2);
    assert_eq!(totals.budget_requests, 0);
    source_run_regressions(&catalog, &operator, &ingest).await;
    retrieval_media_credit_regressions(&catalog, &operator, &ingest, &search, &media, &pool).await;
    artwork_generation_regressions(&catalog, &operator, &ingest, &media, &pool).await;
    required_variant_readiness_regressions(&catalog, &operator, &ingest, &media, &pool).await;
    shared_search_primitives(&catalog, &operator, &ingest, &pool).await;
    metadata_snapshot_regressions(&catalog, &operator, &ingest, &pool).await;
    startup_regressions(&admin, &url).await;
    final_primitives_regressions(&admin, &url).await;
    println!(
        "verified PostgreSQL {version}, pgvector {vector}; migrations/roles/concurrent-startup/jobs/catalog/search/reconciliation/artwork-generations"
    );
    catalog_pool.close().await;
    worker_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

async fn startup_regressions(admin: &PgPool, admin_url: &str) {
    use screenroom_backend::{
        config::{DatabaseConfig, ServiceDatabaseConfig},
        db,
    };
    use std::time::Duration;
    let suffix = Uuid::new_v4().simple().to_string();
    let migration_login = format!("startup_migration_{suffix}");
    let runtime_login = format!("startup_runtime_{suffix}");
    sqlx::query(&format!(
        "CREATE ROLE {migration_login} LOGIN SUPERUSER PASSWORD 'screenroom-startup-disposable'"
    ))
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(&format!("CREATE ROLE {runtime_login} LOGIN IN ROLE screenroom_catalog PASSWORD 'screenroom-startup-disposable'"))
        .execute(admin).await.unwrap();
    let (name, inspect) = database(admin, admin_url).await;
    let mut migration_url = reqwest::Url::parse(admin_url).unwrap();
    migration_url.set_path(&name);
    migration_url.set_username(&migration_login).unwrap();
    migration_url
        .set_password(Some("screenroom-startup-disposable"))
        .unwrap();
    let mut runtime_url = migration_url.clone();
    runtime_url.set_username(&runtime_login).unwrap();
    let config = ServiceDatabaseConfig {
        runtime: DatabaseConfig {
            url: runtime_url.to_string(),
            max_connections: 2,
            acquire_timeout: Duration::from_secs(2),
        },
        migration_url: Some(migration_url.to_string()),
    };
    let mut starts = Vec::new();
    for _ in 0..4 {
        let config = config.clone();
        starts.push(tokio::spawn(
            async move { db::connect_service(&config).await },
        ));
    }
    for start in starts {
        let pool = start.await.unwrap().unwrap();
        db::ready(&pool).await.unwrap();
        assert!(
            sqlx::query("CREATE TABLE catalog.startup_runtime_ddl(id integer)")
                .execute(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SELECT * FROM identity.jellyfin_user_links")
                .execute(&pool)
                .await
                .is_err()
        );
        pool.close().await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations WHERE success")
            .fetch_one(&inspect)
            .await
            .unwrap(),
        MIGRATOR.migrations.len() as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname='catalog'"
        )
        .fetch_one(&inspect)
        .await
        .unwrap(),
        migration_login
    );
    let pool = db::connect_service(&config).await.unwrap();
    pool.close().await;
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    let mut mismatched = config.clone();
    let mut other_database = runtime_url.clone();
    other_database.set_path("postgres");
    mismatched.runtime.url = other_database.to_string();
    assert!(matches!(
        db::connect_service(&mismatched).await,
        Err(db::StartupError::MigrationConfiguration)
    ));
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    let first = &MIGRATOR.migrations[0];
    sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=$2")
        .bind([0u8].as_slice())
        .bind(first.version)
        .execute(&inspect)
        .await
        .unwrap();
    assert!(matches!(
        db::connect_service(&config).await,
        Err(db::StartupError::Migration)
    ));
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=$2")
        .bind(first.checksum.as_ref())
        .bind(first.version)
        .execute(&inspect)
        .await
        .unwrap();
    let pool = tokio::time::timeout(Duration::from_secs(10), db::connect_service(&config))
        .await
        .unwrap()
        .unwrap();
    pool.close().await;
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES(9999999999999,'incompatible future migration',true,$1,0)")
        .bind([1u8].as_slice()).execute(&inspect).await.unwrap();
    assert!(matches!(
        db::connect_service(&config).await,
        Err(db::StartupError::Migration)
    ));
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=9999999999999")
        .execute(&inspect)
        .await
        .unwrap();
    let pool = tokio::time::timeout(Duration::from_secs(10), db::connect_service(&config))
        .await
        .unwrap()
        .unwrap();
    pool.close().await;
    // Failed runtime authentication occurs only after successful migration, with no retained DDL session.
    let mut invalid_runtime = config.clone();
    let mut wrong_url = runtime_url.clone();
    wrong_url
        .set_password(Some("wrong-disposable-password"))
        .unwrap();
    invalid_runtime.runtime.url = wrong_url.to_string();
    assert!(matches!(
        db::connect_service(&invalid_runtime).await,
        Err(db::StartupError::Runtime)
    ));
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    let mut invalid_migration = config.clone();
    let mut wrong_url = migration_url.clone();
    wrong_url
        .set_password(Some("wrong-disposable-password"))
        .unwrap();
    invalid_migration.migration_url = Some(wrong_url.to_string());
    assert!(matches!(
        db::connect_service(&invalid_migration).await,
        Err(db::StartupError::Migration)
    ));
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    // Owner-based local setup can intentionally use its runtime URL for both phases.
    let owner = ServiceDatabaseConfig {
        runtime: DatabaseConfig {
            url: migration_url.to_string(),
            ..config.runtime.clone()
        },
        migration_url: None,
    };
    let pool = db::connect_service(&owner).await.unwrap();
    pool.close().await;
    assert_no_startup_sessions(admin, &migration_login, &runtime_login).await;
    assert_eq!(
        db::StartupError::Migration.to_string(),
        "database startup migration failed"
    );
    inspect.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(admin)
        .await
        .unwrap();
    for login in [&runtime_login, &migration_login] {
        sqlx::query(&format!("DROP ROLE {login}"))
            .execute(admin)
            .await
            .unwrap();
    }
}

async fn assert_no_startup_sessions(admin: &PgPool, migration_login: &str, runtime_login: &str) {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE usename IN ($1,$2)")
            .bind(migration_login)
            .bind(runtime_login)
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(count, 0, "startup pool/session leaked");
}

async fn artwork_generation_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
    media: &MediaRepository,
    pool: &PgPool,
) {
    let source = catalog
        .configure_source(
            "generation-jellyfin",
            "jellyfin",
            &json!({"branding_artwork_ids":["branding","not-present","branding",17]}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let policy = operator
        .create_policy(source.id, &policy(json!({"library_ids":["movies"]})))
        .await
        .unwrap();
    let scope = Scope {
        source_keys: vec![source.source_key.clone()],
        ..Scope::default()
    };
    let bytes_a = neutral_image::png([64, 96, 128]);
    let bytes_b = neutral_image::png([128, 96, 64]);
    let a = verified_bytes(source.id, bytes_a.clone()).await;
    let b = verified_bytes(source.id, bytes_b.clone()).await;
    let inputs = |bytes: &[u8], label: &str| -> Vec<ArtworkInputWrite> {
        ["item", "person", "branding"]
            .into_iter()
            .map(|key| ArtworkInputWrite {
                reference: ArtworkReferenceInput {
                    source_artwork_id: key.into(),
                    kind: "poster".into(),
                    rights: json!({"license":label}),
                    item_external_id: (key == "item").then(|| "generation-item".into()),
                    person_external_id: (key == "person").then(|| "person1".into()),
                    role: "primary".into(),
                    ordinal: 0,
                },
                manifest: ArtworkInputManifest {
                    version: 1,
                    variants: [ArtworkVariant::Original, ArtworkVariant::Native]
                        .into_iter()
                        .map(|variant| ArtworkVariantInput {
                            variant,
                            sha256: Sha256::digest(bytes).into(),
                        })
                        .collect(),
                },
            })
            .collect()
    };
    let inputs_a = inputs(&bytes_a, "a");
    let inputs_b = inputs(&bytes_b, "b");
    let initial = operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("generation-page", 60).await.unwrap().unwrap();
    let mut first = page(
        vec![item(
            "generation-item",
            "movie",
            "movies",
            "generation document",
        )],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({"generation":1}),
    );
    first.artwork_inputs = inputs_a.clone();
    first.artwork_omissions.push(ArtworkOmission {
        source_artwork_id: "missing-optional".into(),
        code: ArtworkOmissionCode::Missing,
    });
    ingest.commit_fixture_page(&claim, &first).await.unwrap();
    let selected = catalog
        .get(&scope, "generation-item")
        .await
        .unwrap()
        .unwrap();
    let person = catalog
        .credits(&scope, "generation-item", None, 100)
        .await
        .unwrap()[0]
        .person_id;
    assert!(
        media
            .primary_item_artwork(&scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
    assert!(
        catalog
            .get(&scope, "generation-item")
            .await
            .unwrap()
            .is_some()
    );
    let omissions = operator
        .artwork_omissions(initial.id, None, 100)
        .await
        .unwrap();
    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].source_artwork_id, "missing-optional");
    assert!(
        operator
            .artwork_omissions(initial.id, Some("missing-optional"), 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.list(&scope, None, 100).await.unwrap().is_empty());
    let mut seen = std::collections::BTreeMap::new();
    for _ in 0..6 {
        let claim = ingest
            .claim("generation-publish", 60)
            .await
            .unwrap()
            .unwrap();
        let job: ArtworkJob = serde_json::from_value(claim.input.clone()).unwrap();
        assert_eq!(job.revision, 1);
        assert!(media.publish(&claim, &b).await.is_err());
        assert!(
            media
                .resolve(&scope, job.artwork_id, 1, job.variant.as_str())
                .await
                .unwrap()
                .is_none()
        );
        media.publish(&claim, &a).await.unwrap();
        assert!(media.publish(&claim, &a).await.is_err());
        let count = seen.entry(job.artwork_id).or_insert(0);
        *count += 1;
        for variant in ["native", "original"] {
            assert_eq!(
                media
                    .resolve(&scope, job.artwork_id, 1, variant)
                    .await
                    .unwrap()
                    .is_some(),
                *count == 2
            );
        }
        let present = match job.source_artwork_id.as_str() {
            "item" => media
                .primary_item_artwork(&scope, &[selected.source_item_id])
                .await
                .unwrap()
                .contains_key(&selected.source_item_id),
            "person" => media
                .person_artwork(&scope, &[person])
                .await
                .unwrap()
                .contains_key(&person),
            "branding" => !media.branding(&scope, 8).await.unwrap().is_empty(),
            _ => unreachable!(),
        };
        assert_eq!(present, *count == 2);
    }
    assert_eq!(media.list(&scope, None, 100).await.unwrap().len(), 3);
    let invalid_run = operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("reject-original-only", 60)
        .await
        .unwrap()
        .unwrap();
    let before_checkpoint = operator
        .checkpoint(policy.id, "selected", policy.version)
        .await
        .unwrap();
    let mut invalid = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({"invalid":true}),
    );
    invalid.artwork_inputs = inputs_b.clone();
    for input in &mut invalid.artwork_inputs {
        input
            .manifest
            .variants
            .retain(|v| v.variant == ArtworkVariant::Original);
    }
    assert!(ingest.commit_fixture_page(&claim, &invalid).await.is_err());
    assert_eq!(
        operator
            .checkpoint(policy.id, "selected", policy.version)
            .await
            .unwrap(),
        before_checkpoint
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE source_id=$1"
        )
        .bind(source.id)
        .fetch_one(pool)
        .await
        .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='artwork'"
        )
        .bind(invalid_run.id)
        .fetch_one(pool)
        .await
        .unwrap(),
        0
    );
    assert!(
        media
            .list(&scope, None, 100)
            .await
            .unwrap()
            .iter()
            .all(|art| art.revision == 1 && art.rights["license"] == "a")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT max(current_generation) FROM media.artwork WHERE source_id=$1"
        )
        .bind(source.id)
        .fetch_one(pool)
        .await
        .unwrap(),
        1
    );
    operator.cancel_run(invalid_run.id).await.unwrap();
    let tv_scope = Scope {
        library_ids: vec!["tv".into()],
        ..scope.clone()
    };
    assert!(
        media
            .primary_item_artwork(&tv_scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&tv_scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(media.branding(&tv_scope, 8).await.unwrap().len(), 1);
    let wrong_source = Scope {
        source_keys: vec!["other-source".into()],
        ..Scope::default()
    };
    assert!(
        media
            .primary_item_artwork(&wrong_source, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&wrong_source, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&wrong_source, 8).await.unwrap().is_empty());
    assert!(
        media
            .primary_item_artwork(&scope, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.person_artwork(&scope, &[]).await.unwrap().is_empty());
    assert!(
        media
            .primary_item_artwork(&scope, &vec![selected.source_item_id; 201])
            .await
            .is_err()
    );
    assert!(
        media
            .person_artwork(&scope, &vec![person; 201])
            .await
            .is_err()
    );
    // Reordered variants still describe exactly the same current input.
    let mut unchanged = first.clone();
    unchanged.items.clear();
    unchanged.artwork_omissions.clear();
    for input in &mut unchanged.artwork_inputs {
        input.manifest.variants.reverse();
    }
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("generation-unchanged", 60)
        .await
        .unwrap()
        .unwrap();
    ingest
        .commit_fixture_page(&claim, &unchanged)
        .await
        .unwrap();
    assert!(
        ingest
            .claim("generation-no-jobs", 60)
            .await
            .unwrap()
            .is_none()
    );
    let generations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM media.artwork_generations WHERE source_id=$1")
            .bind(source.id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(generations, 3);
    for (revision, writes, object, rights) in [(2, &inputs_b, &b, "b"), (3, &inputs_a, &a, "a")] {
        operator.start_run(policy.id).await.unwrap();
        let claim = ingest
            .claim("generation-change", 60)
            .await
            .unwrap()
            .unwrap();
        let mut change = page(
            vec![],
            Completion::Complete {
                evidence: json!({}),
            },
            json!({"generation":revision}),
        );
        change.artwork_inputs = writes.clone();
        ingest.commit_fixture_page(&claim, &change).await.unwrap();
        for row in media.list(&scope, None, 100).await.unwrap() {
            assert_eq!(row.revision, revision - 1);
        }
        let mut seen = std::collections::BTreeMap::new();
        for _ in 0..6 {
            let claim = ingest
                .claim("generation-new-bytes", 60)
                .await
                .unwrap()
                .unwrap();
            let job: ArtworkJob = serde_json::from_value(claim.input.clone()).unwrap();
            assert_eq!(job.revision, revision);
            media.publish(&claim, object).await.unwrap();
            let count = seen.entry(job.artwork_id).or_insert(0);
            *count += 1;
            let rows = media.list(&scope, None, 100).await.unwrap();
            let row = rows.iter().find(|row| row.id == job.artwork_id).unwrap();
            let targeted = match job.source_artwork_id.as_str() {
                "item" => media
                    .primary_item_artwork(&scope, &[selected.source_item_id])
                    .await
                    .unwrap()
                    .remove(&selected.source_item_id)
                    .unwrap(),
                "person" => media
                    .person_artwork(&scope, &[person])
                    .await
                    .unwrap()
                    .remove(&person)
                    .unwrap(),
                "branding" => media.branding(&scope, 8).await.unwrap().remove(0),
                _ => unreachable!(),
            };
            assert_eq!(targeted.revision, row.revision);
            assert_eq!(
                row.revision,
                if *count == 2 { revision } else { revision - 1 }
            );
            assert_eq!(
                row.rights["license"],
                if *count == 2 {
                    rights
                } else if revision == 2 {
                    "a"
                } else {
                    "b"
                }
            );
            for variant in ["native", "original"] {
                assert_eq!(
                    media
                        .resolve(&scope, job.artwork_id, revision, variant)
                        .await
                        .unwrap()
                        .is_some(),
                    *count == 2
                );
                assert!(
                    media
                        .resolve(&scope, job.artwork_id, 1, variant)
                        .await
                        .unwrap()
                        .is_some()
                );
            }
        }
    }
    // A leased older generation cannot publish after a newer page supersedes it.
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("generation-four", 60).await.unwrap().unwrap();
    let mut change = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({"generation":4}),
    );
    change.artwork_inputs = inputs_b.clone();
    ingest.commit_fixture_page(&claim, &change).await.unwrap();
    let old = ingest
        .claim("generation-old-lease", 60)
        .await
        .unwrap()
        .unwrap();
    let newer = operator.start_run(policy.id).await.unwrap();
    // Ready older art jobs precede the next ingestion job in the queue.
    let mut older = vec![old];
    let page_claim = loop {
        let c = ingest.claim("generation-five", 60).await.unwrap().unwrap();
        if c.kind == "ingest" {
            break c;
        }
        older.push(c);
    };
    assert_eq!(page_claim.run_id, Some(newer.id));
    change.artwork_inputs = inputs_a;
    change.checkpoint.cursor = json!({"generation":5});
    ingest
        .commit_fixture_page(&page_claim, &change)
        .await
        .unwrap();
    for c in older {
        assert!(matches!(
            media.publish(&c, &b).await,
            Err(screenroom_backend::Error::Stale)
        ));
        ingest.complete_job(&c).await.unwrap();
    }
    for _ in 0..6 {
        let claim = ingest
            .claim("generation-current", 60)
            .await
            .unwrap()
            .unwrap();
        let job: ArtworkJob = serde_json::from_value(claim.input.clone()).unwrap();
        assert_eq!(job.revision, 5);
        media.publish(&claim, &a).await.unwrap();
    }
    assert!(
        media
            .list(&scope, None, 100)
            .await
            .unwrap()
            .iter()
            .all(|row| row.revision == 5)
    );
    let rollback_run = operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("generation-rollback", 60)
        .await
        .unwrap()
        .unwrap();
    let mut rollback = change.clone();
    rollback.artwork_inputs[0].reference.source_artwork_id = "rollback-art".into();
    rollback.followups.push(Followup {
        kind: JobKind::Artwork,
        dedupe_key: "not-allowed".into(),
        input: json!({}),
    });
    assert!(ingest.commit_fixture_page(&claim, &rollback).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM media.artwork WHERE source_id=$1 AND source_artwork_id='rollback-art'").bind(source.id).fetch_one(pool).await.unwrap();
    assert_eq!(count, 0);
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='artwork'")
            .bind(rollback_run.id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(jobs, 0);
    assert!(
        operator
            .run_checkpoint(
                &operator.run(rollback_run.id).await.unwrap().unwrap(),
                "selected"
            )
            .await
            .unwrap()
            .is_none()
    );
    rollback.artwork_inputs.clear();
    rollback.followups.clear();
    rollback.artwork_omissions = vec![ArtworkOmission {
        source_artwork_id: "invalid-optional".into(),
        code: ArtworkOmissionCode::Invalid,
    }];
    ingest.commit_fixture_page(&claim, &rollback).await.unwrap();
    assert_eq!(
        operator
            .artwork_omissions(rollback_run.id, None, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    // A later successful ingestion repairs a previously omitted optional input.
    let repair_run = operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("generation-repair", 60)
        .await
        .unwrap()
        .unwrap();
    let mut repair = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({"repair":true}),
    );
    let mut repaired = first.artwork_inputs[2].clone();
    repaired.reference.source_artwork_id = "missing-optional".into();
    repaired.manifest.variants[1].sha256 = Sha256::digest(&bytes_b).into();
    repair.artwork_inputs.push(repaired);
    ingest.commit_fixture_page(&claim, &repair).await.unwrap();
    assert!(
        operator
            .artwork_omissions(repair_run.id, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    for _ in 0..2 {
        let claim = ingest
            .claim("generation-distinct-variants", 60)
            .await
            .unwrap()
            .unwrap();
        let job: ArtworkJob = serde_json::from_value(claim.input.clone()).unwrap();
        assert_eq!(job.revision, 1);
        let (correct, wrong) = match job.variant {
            ArtworkVariant::Original => (&a, &b),
            ArtworkVariant::Native => (&b, &a),
        };
        assert!(media.publish(&claim, wrong).await.is_err());
        media.publish(&claim, correct).await.unwrap();
    }
    let repaired = media
        .list(&scope, None, 100)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.source_artwork_id == "missing-optional")
        .unwrap();
    assert!(
        media
            .resolve(&scope, repaired.id, 1, "original")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        media
            .resolve(&scope, repaired.id, 1, "native")
            .await
            .unwrap()
            .is_some()
    );
    let configured = vec![
        "configured-10",
        "configured-08",
        "configured-02",
        "configured-10",
        "configured-07",
        "configured-03",
        "configured-11",
        "configured-01",
        "configured-09",
        "configured-06",
        "configured-00",
    ];
    let mut configuration: Value =
        sqlx::query_scalar("SELECT configuration FROM catalog.sources WHERE id=$1")
            .bind(source.id)
            .fetch_one(pool)
            .await
            .unwrap();
    configuration["branding_artwork_ids"] = json!(configured);
    catalog
        .configure_source(&source.source_key, "jellyfin", &configuration, &json!({}))
        .await
        .unwrap();
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("branding-inputs", 60).await.unwrap().unwrap();
    let mut branding_page = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({"branding":true}),
    );
    for index in 0..12 {
        let mut input = first.artwork_inputs[2].clone();
        input.reference.source_artwork_id = format!("configured-{index:02}");
        input
            .manifest
            .variants
            .retain(|v| v.variant == ArtworkVariant::Native);
        branding_page.artwork_inputs.push(input);
    }
    ingest
        .commit_fixture_page(&claim, &branding_page)
        .await
        .unwrap();
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
    for _ in 0..12 {
        let claim = ingest.claim("branding-publish", 60).await.unwrap().unwrap();
        media.publish(&claim, &a).await.unwrap();
    }
    assert_eq!(media.list(&scope, None, 100).await.unwrap().len(), 16);
    let names: Vec<String> = media
        .branding(&scope, 8)
        .await
        .unwrap()
        .into_iter()
        .map(|art| art.source_artwork_id)
        .collect();
    assert_eq!(
        names,
        vec![
            "configured-10",
            "configured-08",
            "configured-02",
            "configured-07",
            "configured-03",
            "configured-11",
            "configured-01",
            "configured-09"
        ]
    );
    assert_eq!(
        media.branding(&scope, 1).await.unwrap()[0].source_artwork_id,
        "configured-10"
    );
    assert!(media.branding(&scope, 9).await.is_err());
    assert!(media.branding(&scope, 0).await.is_err());
    catalog
        .configure_source(
            &source.source_key,
            "jellyfin",
            &json!({"branding_artwork_ids":{ "invalid":"array expected" }}),
            &json!({}),
        )
        .await
        .unwrap();
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
    // Configuration cannot authorize linked artwork outside its eligible association.
    catalog
        .configure_source(
            &source.source_key,
            "jellyfin",
            &json!({"branding_artwork_ids":["item","person","branding"]}),
            &json!({}),
        )
        .await
        .unwrap();
    assert_eq!(media.branding(&scope, 8).await.unwrap().len(), 3);
    assert_eq!(media.branding(&tv_scope, 8).await.unwrap().len(), 1);
    let wrong_provider = Scope {
        providers: vec!["unsupported".into()],
        ..scope.clone()
    };
    assert!(
        media
            .primary_item_artwork(&wrong_provider, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&wrong_provider, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&wrong_provider, 8).await.unwrap().is_empty());
    sqlx::query("UPDATE catalog.libraries SET state='disabled' WHERE source_id=$1")
        .bind(source.id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        media
            .primary_item_artwork(&scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(media.branding(&scope, 8).await.unwrap().len(), 1);
    sqlx::query("UPDATE catalog.libraries SET state='enabled' WHERE source_id=$1")
        .bind(source.id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalog.source_items SET state='absent' WHERE id=$1")
        .bind(selected.source_item_id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        media
            .primary_item_artwork(&scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(media.branding(&scope, 8).await.unwrap().len(), 1);
    sqlx::query("UPDATE catalog.source_items SET state='enabled' WHERE id=$1")
        .bind(selected.source_item_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE media.objects SET state='absent' WHERE storage_scope=$1")
        .bind(source.id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        media
            .primary_item_artwork(&scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
    assert!(media.list(&scope, None, 100).await.unwrap().is_empty());
    assert!(
        media
            .resolve(&scope, repaired.id, 1, "original")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE media.objects SET state='ready' WHERE storage_scope=$1")
        .bind(source.id)
        .execute(pool)
        .await
        .unwrap();
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(
        media
            .primary_item_artwork(&scope, &[selected.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&scope, 8).await.unwrap().is_empty());
}

async fn final_primitives_regressions(admin: &PgPool, url: &str) {
    let (name, pool) = database(admin, url).await;
    MIGRATOR.run(&pool).await.unwrap();
    let catalog_pool = runtime_pool(&pool, "screenroom_catalog").await;
    let worker_pool = runtime_pool(&pool, "screenroom_worker").await;
    let catalog = CatalogRepository::new(catalog_pool.clone());
    let operator = IngestRepository::new(catalog_pool.clone());
    let ingest = IngestRepository::new(worker_pool.clone());
    let search = SearchRepository::new(worker_pool.clone());
    let media = MediaRepository::new(worker_pool.clone());
    for mode in ["retry", "expiry", "legacy"] {
        let key = format!("failed-followups-{mode}");
        let source = catalog
            .configure_source(&key, "jellyfin", &json!({}), &json!({}))
            .await
            .unwrap();
        catalog
            .configure_library(source.id, "movies", "movies", "Movies")
            .await
            .unwrap();
        let p = operator
            .create_policy(source.id, &policy(json!({"library_ids":["movies"]})))
            .await
            .unwrap();
        let run = operator.start_run(p.id).await.unwrap();
        let first = ingest.claim("partial-page", 300).await.unwrap().unwrap();
        let ids = [format!("{key}-leased"), format!("{key}-pending")];
        let mut partial = page(
            ids.iter()
                .map(|id| item(id, "movie", "movies", id))
                .collect(),
            Completion::Continue,
            json!({"page":1}),
        );
        partial.followups = ids
            .iter()
            .map(|id| Followup {
                kind: JobKind::Embedding,
                dedupe_key: format!("embed:{id}"),
                input: json!({"external_id":id}),
            })
            .collect();
        partial.artwork_inputs = ids
            .iter()
            .enumerate()
            .map(|(ordinal, id)| {
                artwork_input(ArtworkReferenceInput {
                    source_artwork_id: if ordinal == 0 { "leased" } else { "pending" }.into(),
                    kind: "poster".into(),
                    rights: json!({"license":"synthetic-test"}),
                    item_external_id: Some(id.clone()),
                    person_external_id: None,
                    role: "primary".into(),
                    ordinal: 0,
                })
            })
            .collect();
        ingest.commit_fixture_page(&first, &partial).await.unwrap();
        // Leave one delayed ready and one leased job of each follow-up kind.
        sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp()+interval '1 day' WHERE scope_id=$1 AND state='ready'").bind(run.id).execute(&pool).await.unwrap();
        sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp()-interval '1 second' WHERE scope_id=$1 AND ((kind='artwork' AND input->>'source_artwork_id'='leased') OR (kind='embedding' AND input->>'external_id'=$2))")
            .bind(run.id).bind(&ids[0]).execute(&pool).await.unwrap();
        let mut leased = Vec::new();
        for _ in 0..2 {
            leased.push(ingest.claim("old-followup", 300).await.unwrap().unwrap());
        }
        let old_art = leased.iter().find(|c| c.kind == "artwork").unwrap();
        let old_embedding = leased.iter().find(|c| c.kind == "embedding").unwrap();
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind IN ('artwork','embedding') AND state='ready' AND available_at>clock_timestamp()").bind(run.id).fetch_one(&pool).await.unwrap(),2);
        sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp()-interval '1 second',max_attempts=1 WHERE scope_id=$1 AND kind='ingest'").bind(run.id).execute(&pool).await.unwrap();
        let final_claim = ingest
            .claim("final-ingest-attempt", 300)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(final_claim.kind, "ingest");
        assert_eq!(final_claim.attempts, 1);
        match mode {
            "retry" => ingest
                .retry(&final_claim, "permanent_failure", 0)
                .await
                .unwrap(),
            "expiry" => {
                sqlx::query("UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(final_claim.id).execute(&pool).await.unwrap();
                assert!(ingest.claim("reap-expired", 300).await.unwrap().is_none());
            }
            _ => {
                // Reproduce a pre-fix failure with active delayed/leased siblings.
                sqlx::query("UPDATE queue.jobs SET state='failed',lease_owner=NULL,lease_until=NULL WHERE id=$1").bind(final_claim.id).execute(&pool).await.unwrap();
                sqlx::query("UPDATE ingest.runs SET state='failed',finished_at=now() WHERE id=$1")
                    .bind(run.id)
                    .execute(&pool)
                    .await
                    .unwrap();
                assert!(
                    ingest
                        .claim("repair-old-failure", 300)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        }
        assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "failed");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind IN ('artwork','embedding') AND state IN ('ready','leased')").bind(run.id).fetch_one(&pool).await.unwrap(),4);
        for old in &leased {
            let fence: i64 = sqlx::query_scalar("SELECT fence FROM queue.jobs WHERE id=$1")
                .bind(old.id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(fence > old.fence);
            assert!(matches!(
                ingest.renew(old, 30).await,
                Err(screenroom_backend::Error::Stale)
            ));
        }
        // Failure retains observed catalog/library state; it is not absence evidence.
        let scope = Scope {
            source_keys: vec![key.clone()],
            ..Scope::default()
        };
        assert_eq!(catalog.list(&scope, None, 100).await.unwrap().len(), 2);
        let object = verified_object(source.id).await;
        let model = search
            .register_model(&key, "failure-r1", 1024)
            .await
            .unwrap();
        let document = search
            .document_for_reference(&key, &ids[0])
            .await
            .unwrap()
            .unwrap();
        let mut vector = vec![0f32; 1024];
        vector[0] = 1.;
        assert!(matches!(
            media.publish(old_art, &object).await,
            Err(screenroom_backend::Error::Stale)
        ));
        assert!(matches!(
            search
                .commit_embedding(
                    old_embedding,
                    document.id,
                    model.id,
                    &model.revision,
                    &document.content_hash,
                    &vector
                )
                .await,
            Err(screenroom_backend::Error::Stale)
        ));
        let next = operator.start_run(p.id).await.unwrap();
        assert_ne!(next.id, run.id);
        let new_claim = ingest.claim("new-ingestion", 300).await.unwrap().unwrap();
        partial.completion = Completion::Complete {
            evidence: json!({}),
        };
        ingest
            .commit_fixture_page(&new_claim, &partial)
            .await
            .unwrap();
        assert_eq!(
            ingest.run(next.id).await.unwrap().unwrap().state,
            "complete"
        );
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind IN ('artwork','embedding') AND state='ready'").bind(next.id).fetch_one(&pool).await.unwrap(),4);
        let mut completed = (0, 0);
        for _ in 0..4 {
            let claim = ingest
                .claim("replacement-followup", 300)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(claim.run_id, Some(next.id));
            if claim.kind == "artwork" {
                media.publish(&claim, &object).await.unwrap();
                completed.0 += 1;
            } else {
                let external = claim.input["external_id"].as_str().unwrap();
                let doc = search
                    .document_for_reference(&key, external)
                    .await
                    .unwrap()
                    .unwrap();
                search
                    .commit_embedding(
                        &claim,
                        doc.id,
                        model.id,
                        &model.revision,
                        &doc.content_hash,
                        &vector,
                    )
                    .await
                    .unwrap();
                completed.1 += 1;
            }
        }
        assert_eq!(completed, (2, 2));
        assert!(ingest.claim("finished", 300).await.unwrap().is_none());
        assert!(matches!(
            media.publish(old_art, &object).await,
            Err(screenroom_backend::Error::Stale)
        ));
        assert!(matches!(
            ingest.commit_fixture_page(&final_claim, &partial).await,
            Err(screenroom_backend::Error::Stale)
        ));
    }
    let source = catalog
        .configure_source("completed-followups", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let p = operator
        .create_policy(source.id, &policy(json!({"library_ids":["movies"]})))
        .await
        .unwrap();
    let run = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("complete-ingest", 300).await.unwrap().unwrap();
    let mut done = page(
        vec![],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    done.followups = ["expired", "healthy"]
        .iter()
        .map(|key| Followup {
            kind: JobKind::Embedding,
            dedupe_key: format!("completed:{key}"),
            input: json!({"external_id":key}),
        })
        .collect();
    ingest.commit_fixture_page(&claim, &done).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp()+interval '1 day' WHERE scope_id=$1 AND state='ready'").bind(run.id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp(),max_attempts=1 WHERE scope_id=$1 AND dedupe_key='completed:expired'").bind(run.id).execute(&pool).await.unwrap();
    let expired = ingest
        .claim("expired-followup", 300)
        .await
        .unwrap()
        .unwrap();
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expired.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp() WHERE scope_id=$1 AND dedupe_key='completed:healthy'").bind(run.id).execute(&pool).await.unwrap();
    let healthy = ingest
        .claim("healthy-followup", 300)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(expired.id, healthy.id);
    assert_eq!(healthy.run_id, Some(run.id));
    ingest.complete_job(&healthy).await.unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM queue.jobs WHERE id=$1")
            .bind(expired.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "failed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND state='cancelled'"
        )
        .bind(run.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    top_level_regressions(&catalog, &operator, &ingest).await;
    catalog_pool.close().await;
    worker_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(admin)
        .await
        .unwrap();
}

async fn top_level_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
) {
    let expected = [
        "roots-a-season",
        "roots-b-episode",
        "roots-m-movie",
        "roots-n-season",
        "roots-y-episode",
        "roots-z-series",
    ];
    for (key, provider, library) in [
        ("roots-primary", "jellyfin", "movies"),
        ("roots-secondary", "jellyfin", "tv"),
    ] {
        let source = catalog
            .configure_source(key, provider, &json!({}), &json!({}))
            .await
            .unwrap();
        catalog
            .configure_library(source.id, library, library, library)
            .await
            .unwrap();
        let p = operator
            .create_policy(source.id, &policy(json!({"library_ids":[library]})))
            .await
            .unwrap();
        operator.start_run(p.id).await.unwrap();
        let claim = ingest.claim("roots", 300).await.unwrap().unwrap();
        let mut items = Vec::new();
        for (id, kind, parent) in [
            (expected[2], "movie", None),
            (expected[5], "series", None),
            (expected[0], "season", Some(expected[5])),
            (expected[3], "season", Some(expected[5])),
            (expected[1], "episode", Some(expected[0])),
            (expected[4], "episode", Some(expected[0])),
        ] {
            let mut write = item(id, kind, library, id);
            write.parent_public_id = parent.map(str::to_owned);
            write.metadata["source"] = json!(key);
            items.push(write);
        }
        ingest
            .commit_fixture_page(
                &claim,
                &page(
                    items,
                    Completion::Complete {
                        evidence: json!({}),
                    },
                    json!({}),
                ),
            )
            .await
            .unwrap();
    }
    let scope = Scope {
        source_keys: vec!["roots-primary".into(), "roots-secondary".into()],
        ..Scope::default()
    };
    let mut after = None;
    let mut roots = Vec::new();
    loop {
        let page = catalog
            .list_top_level(&scope, after.as_deref(), 1)
            .await
            .unwrap();
        let Some(root) = page.first() else { break };
        assert!(root.parent_public_id.is_none());
        assert_eq!(root.source_key, "roots-primary");
        after = Some(root.public_id.clone());
        roots.push(root.public_id.clone());
    }
    assert_eq!(roots, vec![expected[2], expected[5]]);
    let unrestricted = catalog.list(&scope, None, 100).await.unwrap();
    assert_eq!(
        unrestricted
            .iter()
            .map(|i| i.public_id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    for filtered in [
        Scope {
            library_ids: vec!["tv".into()],
            ..scope.clone()
        },
        Scope {
            providers: vec!["jellyfin".into()],
            source_keys: vec!["roots-secondary".into()],
            ..scope.clone()
        },
        Scope {
            source_keys: vec!["roots-secondary".into()],
            ..Scope::default()
        },
    ] {
        let roots = catalog.list_top_level(&filtered, None, 100).await.unwrap();
        assert_eq!(
            roots
                .iter()
                .map(|i| i.public_id.as_str())
                .collect::<Vec<_>>(),
            vec![expected[2], expected[5]]
        );
        assert!(roots.iter().all(|i| i.source_key == "roots-secondary"));
    }
    let seasons = Scope {
        kinds: vec!["season".into()],
        ..scope.clone()
    };
    assert!(
        catalog
            .list_top_level(&seasons, None, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(catalog.list(&seasons, None, 100).await.unwrap().len(), 2);
    for (parent, children) in [
        (expected[5], vec![expected[0], expected[3]]),
        (expected[0], vec![expected[1], expected[4]]),
    ] {
        let mut found = Vec::new();
        let mut after = None;
        loop {
            let page = catalog
                .children(&scope, parent, after.as_deref(), 1)
                .await
                .unwrap();
            let Some(child) = page.first() else { break };
            assert_eq!(child.parent_public_id.as_deref(), Some(parent));
            assert_eq!(
                catalog
                    .get(&scope, &child.public_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .public_id,
                child.public_id
            );
            after = Some(child.public_id.clone());
            found.push(child.public_id.clone());
        }
        assert_eq!(found, children);
    }
    assert!(catalog.list_top_level(&scope, None, 0).await.is_err());
}

async fn metadata_snapshot_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
    pool: &PgPool,
) {
    use screenroom_backend::chrono::SecondsFormat;
    assert!(
        catalog
            .metadata_snapshot(&Scope::default())
            .await
            .unwrap()
            .is_none()
    );
    let scope = Scope {
        source_keys: vec!["snapshot-primary".into(), "snapshot-secondary".into()],
        ..Scope::default()
    };
    assert!(catalog.metadata_snapshot(&scope).await.unwrap().is_none());
    let invalid = vec![
        json!(""),
        json!(null),
        json!(42),
        json!(true),
        json!({}),
        json!([]),
        json!("infinity"),
        json!("-infinity"),
        json!("NaN"),
        json!("2999-02-30T00:00:00Z"),
        json!("2999-13-01T00:00:00Z"),
        json!("2999-01-01T25:00:00Z"),
        json!("2999-01-01T24:00:00Z"),
        json!("2999-01-01 00:00:00Z"),
        json!("2999-01-01"),
        json!("2999-01-01T00:00:00"),
        json!("2999-01-01T00:00:00+00"),
        json!(" 2999-01-01T00:00:00Z"),
    ];
    let mut sources = Vec::new();
    for (key, provider, date) in [
        (
            "snapshot-primary",
            "jellyfin",
            "2026-03-01T23:00:00.500+13:00",
        ),
        (
            "snapshot-secondary",
            "jellyfin",
            "2026-03-01T08:00:00.125-03:00",
        ),
    ] {
        let source = catalog
            .configure_source(key, provider, &json!({}), &json!({}))
            .await
            .unwrap();
        catalog
            .configure_library(source.id, "movies", "movies", "Movies")
            .await
            .unwrap();
        catalog
            .configure_library(source.id, "tv", "tv", "TV")
            .await
            .unwrap();
        let policy = operator
            .create_policy(source.id, &policy(json!({"library_ids":["movies","tv"]})))
            .await
            .unwrap();
        operator.start_run(policy.id).await.unwrap();
        let claim = ingest
            .claim("metadata-snapshot", 60)
            .await
            .unwrap()
            .unwrap();
        let mut primary = item("snapshot-canonical", "movie", "movies", "provider snapshot");
        primary.metadata["metadata_retrieved_at"] = json!(date);
        let mut items = vec![primary];
        if provider == "jellyfin" {
            for (index, date) in invalid.iter().enumerate() {
                let mut write = item(
                    &format!("snapshot-invalid-{index}"),
                    "movie",
                    "movies",
                    "invalid timestamp",
                );
                write.metadata["metadata_retrieved_at"] = date.clone();
                items.push(write);
            }
            let mut unknown = item("snapshot-unknown", "movie", "movies", "no provider date");
            unknown.metadata["other_date"] = json!("2999-01-01T00:00:00Z");
            items.push(unknown);
            let mut series = item("snapshot-series", "series", "tv", "older provider snapshot");
            series.metadata["metadata_retrieved_at"] = json!("2001-01-01T00:00:00Z");
            items.push(series);
        }
        ingest
            .commit_fixture_page(
                &claim,
                &page(
                    items,
                    Completion::Complete {
                        evidence: json!({}),
                    },
                    json!({}),
                ),
            )
            .await
            .unwrap();
        sources.push(source);
    }
    let wire = |date: Option<
        screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc>,
    >| date.map(|d| d.to_rfc3339_opts(SecondsFormat::AutoSi, true));
    let latest = Some("2026-03-01T11:00:00.125Z".to_owned());
    assert_eq!(
        wire(catalog.metadata_snapshot(&Scope::default()).await.unwrap()),
        latest
    );
    assert_eq!(
        wire(catalog.metadata_snapshot(&scope).await.unwrap()),
        latest
    );
    // Display precedence keeps Jellyfin metadata; aggregation includes the newer secondary source.
    let display = catalog
        .get(&scope, "snapshot-canonical")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(display.source_key, "snapshot-primary");
    assert_eq!(
        display.metadata["metadata_retrieved_at"],
        "2026-03-01T23:00:00.500+13:00"
    );
    let primary = Scope {
        providers: vec!["jellyfin".into()],
        source_keys: vec!["snapshot-primary".into()],
        ..scope.clone()
    };
    assert_eq!(
        wire(catalog.metadata_snapshot(&primary).await.unwrap()),
        Some("2026-03-01T10:00:00.500Z".into())
    );
    let secondary = Scope {
        source_keys: vec!["snapshot-secondary".into()],
        ..scope.clone()
    };
    assert_eq!(
        wire(catalog.metadata_snapshot(&secondary).await.unwrap()),
        latest
    );
    let television = Scope {
        library_ids: vec!["tv".into()],
        ..scope.clone()
    };
    let series = Scope {
        kinds: vec!["series".into()],
        ..scope.clone()
    };
    for filtered in [&television, &series] {
        assert_eq!(
            wire(catalog.metadata_snapshot(filtered).await.unwrap()),
            Some("2001-01-01T00:00:00Z".into())
        );
    }
    let no_match = Scope {
        kinds: vec!["episode".into()],
        ..scope.clone()
    };
    assert!(
        catalog
            .metadata_snapshot(&no_match)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.source_items SET state='absent' WHERE source_id=$1")
        .bind(sources[1].id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        wire(catalog.metadata_snapshot(&scope).await.unwrap()),
        Some("2026-03-01T10:00:00.500Z".into())
    );
    assert!(
        catalog
            .metadata_snapshot(&secondary)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.source_items SET state='enabled' WHERE source_id=$1")
        .bind(sources[1].id)
        .execute(pool)
        .await
        .unwrap();
    catalog
        .set_source_state(sources[1].id, false)
        .await
        .unwrap();
    assert_eq!(
        wire(catalog.metadata_snapshot(&scope).await.unwrap()),
        Some("2026-03-01T10:00:00.500Z".into())
    );
    assert!(
        catalog
            .metadata_snapshot(&secondary)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.libraries SET state='disabled' WHERE source_id=$1")
        .bind(sources[0].id)
        .execute(pool)
        .await
        .unwrap();
    assert!(catalog.metadata_snapshot(&scope).await.unwrap().is_none());
    assert!(
        catalog
            .metadata_snapshot(&television)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.libraries SET state='enabled' WHERE source_id=$1")
        .bind(sources[0].id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE catalog.source_items SET state='absent' WHERE source_id=$1 AND external_id IN ('snapshot-canonical','snapshot-series')").bind(sources[0].id).execute(pool).await.unwrap();
    // Invalid/unknown dates still have eligible records, but must not become a request-clock fallback.
    assert!(catalog.metadata_snapshot(&scope).await.unwrap().is_none());
    assert!(
        catalog
            .metadata_snapshot(&Scope {
                library_ids: vec!["all".into()],
                ..Scope::default()
            })
            .await
            .is_err()
    );
}

async fn shared_search_primitives(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
    pool: &PgPool,
) {
    let mut documents = Vec::new();
    for (key, provider, title, text) in [
        (
            "primitives-primary",
            "jellyfin",
            "Unrelated Primary",
            "nothing here",
        ),
        (
            "primitives-secondary",
            "jellyfin",
            "Midnight Lantern",
            "cobaltquasar overview only",
        ),
    ] {
        let source = catalog
            .configure_source(key, provider, &json!({}), &json!({}))
            .await
            .unwrap();
        catalog
            .configure_library(source.id, "movies", "movies", "Movies")
            .await
            .unwrap();
        let p = operator
            .create_policy(source.id, &policy(json!({"library_ids":["movies"]})))
            .await
            .unwrap();
        operator.start_run(p.id).await.unwrap();
        let claim = ingest
            .claim("search-primitives", 60)
            .await
            .unwrap()
            .unwrap();
        let mut write = item("primitives-canonical", "movie", "movies", text);
        write.document_title = title.into();
        ingest
            .commit_fixture_page(
                &claim,
                &page(
                    vec![write],
                    Completion::Complete {
                        evidence: json!({}),
                    },
                    json!({}),
                ),
            )
            .await
            .unwrap();
        documents.push(
            SearchRepository::new(pool.clone())
                .document_for_reference(key, "primitives-canonical")
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let runtime = runtime_pool(pool, "screenroom_catalog").await;
    let search = SearchRepository::new(runtime.clone());
    let scope = Scope {
        source_keys: vec!["primitives-primary".into(), "primitives-secondary".into()],
        ..Scope::default()
    };
    let full = LexicalMethods {
        full_text: true,
        ..LexicalMethods::default()
    };
    assert_eq!(
        search
            .lexical(&scope, "cobaltquasar", &full, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        search
            .lexical_titles(&scope, "cobaltquasar", &full, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search
            .lexical_titles(&scope, "lantern", &full, 100)
            .await
            .unwrap()[0]
            .source_item_id,
        documents[1].source_item_id
    );
    assert_eq!(
        search
            .lexical_titles(
                &scope,
                "Midnight Lanter",
                &LexicalMethods {
                    fuzzy: true,
                    ..LexicalMethods::default()
                },
                100
            )
            .await
            .unwrap()
            .len(),
        1
    );
    let primary = Scope {
        providers: vec!["jellyfin".into()],
        source_keys: vec!["primitives-primary".into()],
        ..scope.clone()
    };
    assert!(
        search
            .lexical_titles(&primary, "lantern", &full, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let outside = Scope {
        library_ids: vec!["tv".into()],
        ..scope.clone()
    };
    assert!(
        search
            .lexical_titles(&outside, "lantern", &full, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let model = SearchRepository::new(pool.clone())
        .register_model("primitive-model", "snapshot-r1", 1024)
        .await
        .unwrap();
    sqlx::query("UPDATE search.embedding_models SET state='retired' WHERE state='active'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE search.embedding_models SET state='active' WHERE id=$1")
        .bind(model.id)
        .execute(pool)
        .await
        .unwrap();
    let mut query = vec![0f32; 1024];
    query[0] = 1.;
    let vector = format!(
        "[{}]",
        query
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    sqlx::query("INSERT INTO search.embeddings(document_id,model_id,input_hash,vector) VALUES($1,$2,$3,$4::text::vector)")
        .bind(documents[1].id).bind(model.id).bind(&documents[1].content_hash).bind(&vector).execute(pool).await.unwrap();
    let result = search
        .vector_with_coverage(&scope, &query, model.id, 100)
        .await
        .unwrap();
    assert_eq!(
        result.coverage,
        EmbeddingCoverage {
            eligible_documents: 2,
            current_embeddings: 1,
            complete: false
        }
    );
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(
        result.candidates[0].source_item_id,
        documents[1].source_item_id
    );
    assert!(matches!(
        search
            .vector_with_coverage(&scope, &query, Uuid::new_v4(), 100)
            .await,
        Err(screenroom_backend::Error::NotFound)
    ));
    assert!(
        search
            .vector_with_coverage(&scope, &query[..10], model.id, 100)
            .await
            .is_err()
    );
    assert!(
        search
            .vector_with_coverage(&outside, &query, model.id, 100)
            .await
            .unwrap()
            .coverage
            .complete
    );
    let inactive = SearchRepository::new(pool.clone())
        .register_model("primitive-model", "building-r2", 1024)
        .await
        .unwrap();
    let result = search
        .vector_with_coverage(&scope, &query, inactive.id, 100)
        .await
        .unwrap();
    assert!(!result.coverage.complete && result.candidates.is_empty());
    // RLS test gate blocks inside the statement, after its snapshot has been taken.
    sqlx::query("CREATE FUNCTION public.screenroom_snapshot_gate() RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM pg_advisory_xact_lock(785902140); RETURN true; END $$").execute(pool).await.unwrap();
    sqlx::query("ALTER TABLE search.documents ENABLE ROW LEVEL SECURITY")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE POLICY snapshot_gate ON search.documents FOR SELECT TO screenroom_catalog USING(public.screenroom_snapshot_gate())").execute(pool).await.unwrap();
    let mut lock = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(785902140)")
        .execute(&mut *lock)
        .await
        .unwrap();
    let concurrent = {
        let search = search.clone();
        let scope = scope.clone();
        let query = query.clone();
        tokio::spawn(async move {
            search
                .vector_with_coverage(&scope, &query, model.id, 100)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let waiting:bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks WHERE locktype='advisory' AND objid=785902140 AND NOT granted)").fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    // Invalidate, retrieve-equivalent gap, then re-embed while the reader is deterministically blocked.
    let new_hash = input_hash("updated document");
    sqlx::query("UPDATE search.documents SET content_hash=$2 WHERE id=$1")
        .bind(documents[1].id)
        .bind(new_hash.as_slice())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO search.embeddings(document_id,model_id,input_hash,vector) VALUES($1,$2,$3,$4::text::vector)")
        .bind(documents[0].id).bind(model.id).bind(&documents[0].content_hash).bind(&vector).execute(pool).await.unwrap();
    sqlx::query("UPDATE search.embeddings SET input_hash=$2 WHERE document_id=$1 AND model_id=$3")
        .bind(documents[1].id)
        .bind(new_hash.as_slice())
        .bind(model.id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_unlock(785902140)")
        .execute(&mut *lock)
        .await
        .unwrap();
    let result = concurrent.await.unwrap().unwrap();
    assert_eq!(
        result.coverage,
        EmbeddingCoverage {
            eligible_documents: 2,
            current_embeddings: 1,
            complete: false
        }
    );
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(
        result.candidates[0].source_item_id,
        documents[1].source_item_id
    );
    let fresh = search
        .vector_with_coverage(&scope, &query, model.id, 100)
        .await
        .unwrap();
    assert!(fresh.coverage.complete);
    assert_eq!(fresh.candidates.len(), 2);
    let filtered = search
        .vector_with_coverage(&primary, &query, model.id, 100)
        .await
        .unwrap();
    assert_eq!(filtered.coverage.eligible_documents, 1);
    assert!(filtered.coverage.complete);
    assert_eq!(filtered.candidates.len(), 1);
    let limited = search
        .vector_with_coverage(&scope, &query, model.id, 1)
        .await
        .unwrap();
    assert!(limited.coverage.complete);
    assert_eq!(limited.coverage.current_embeddings, 2);
    assert_eq!(limited.candidates.len(), 1);
    sqlx::query("DROP POLICY snapshot_gate ON search.documents")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE search.documents DISABLE ROW LEVEL SECURITY")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION public.screenroom_snapshot_gate()")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE search.documents SET document_version=2 WHERE id=$1")
        .bind(documents[1].id)
        .execute(pool)
        .await
        .unwrap();
    let invalidated = search
        .vector_with_coverage(&scope, &query, model.id, 100)
        .await
        .unwrap();
    assert!(!invalidated.coverage.complete);
    assert_eq!(invalidated.candidates.len(), 1);
    assert_eq!(
        invalidated.candidates[0].source_item_id,
        documents[0].source_item_id
    );
    runtime.close().await;
}

async fn required_variant_readiness_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
    media: &MediaRepository,
    pool: &PgPool,
) {
    let source = catalog
        .configure_source(
            "required-set-jellyfin",
            "jellyfin",
            &json!({"branding_artwork_ids":["required-art"]}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let policy = operator
        .create_policy(source.id, &policy(json!({"library_ids":["movies"]})))
        .await
        .unwrap();
    let scope = Scope {
        source_keys: vec![source.source_key.clone()],
        ..Scope::default()
    };
    let native_bytes = neutral_image::png([64, 96, 128]);
    let original_bytes = neutral_image::png([128, 96, 64]);
    let native_hash: [u8; 32] = Sha256::digest(&native_bytes).into();
    let original_hash: [u8; 32] = Sha256::digest(&original_bytes).into();
    assert_ne!(native_hash, original_hash);
    let native = verified_bytes(source.id, native_bytes).await;
    let original = verified_bytes(source.id, original_bytes).await;
    operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("required-set-ingest", 60)
        .await
        .unwrap()
        .unwrap();
    let mut page = page(
        vec![item("required-set-item", "movie", "movies", "required set")],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    page.artwork_inputs.push(ArtworkInputWrite {
        reference: ArtworkReferenceInput {
            source_artwork_id: "required-art".into(),
            kind: "poster".into(),
            rights: json!({"license":"test"}),
            item_external_id: Some("required-set-item".into()),
            person_external_id: Some("person1".into()),
            role: "primary".into(),
            ordinal: 0,
        },
        manifest: ArtworkInputManifest {
            version: 1,
            variants: vec![
                ArtworkVariantInput {
                    variant: ArtworkVariant::Original,
                    sha256: original_hash,
                },
                ArtworkVariantInput {
                    variant: ArtworkVariant::Native,
                    sha256: native_hash,
                },
            ],
        },
    });
    ingest.commit_fixture_page(&claim, &page).await.unwrap();
    let mut artwork = Uuid::nil();
    for _ in 0..2 {
        let claim = ingest
            .claim("required-set-publish", 60)
            .await
            .unwrap()
            .unwrap();
        let job: ArtworkJob = serde_json::from_value(claim.input.clone()).unwrap();
        artwork = media
            .publish(
                &claim,
                if job.variant == ArtworkVariant::Native {
                    &native
                } else {
                    &original
                },
            )
            .await
            .unwrap();
    }
    let target = catalog
        .get(&scope, "required-set-item")
        .await
        .unwrap()
        .unwrap()
        .source_item_id;
    let person = catalog
        .credits(&scope, "required-set-item", None, 100)
        .await
        .unwrap()[0]
        .person_id;
    assert_complete_art_reads(media, &scope, target, person, artwork, true).await;
    for hash in [original_hash, native_hash] {
        let changed = sqlx::query(
            "UPDATE media.objects SET state='absent' WHERE storage_scope=$1 AND sha256=$2",
        )
        .bind(source.id)
        .bind(hash.as_slice())
        .execute(pool)
        .await
        .unwrap()
        .rows_affected();
        assert_eq!(changed, 1);
        assert_complete_art_reads(media, &scope, target, person, artwork, false).await;
        sqlx::query("UPDATE media.objects SET state='ready' WHERE storage_scope=$1 AND sha256=$2")
            .bind(source.id)
            .bind(hash.as_slice())
            .execute(pool)
            .await
            .unwrap();
        assert_complete_art_reads(media, &scope, target, person, artwork, true).await;
    }
    // Ready state alone cannot substitute different bytes for a required digest.
    sqlx::query("UPDATE media.objects SET sha256=$3 WHERE storage_scope=$1 AND sha256=$2")
        .bind(source.id)
        .bind(original_hash.as_slice())
        .bind([42u8; 32].as_slice())
        .execute(pool)
        .await
        .unwrap();
    assert_complete_art_reads(media, &scope, target, person, artwork, false).await;
    sqlx::query("UPDATE media.objects SET sha256=$3 WHERE storage_scope=$1 AND sha256=$2")
        .bind(source.id)
        .bind([42u8; 32].as_slice())
        .bind(original_hash.as_slice())
        .execute(pool)
        .await
        .unwrap();
    assert_complete_art_reads(media, &scope, target, person, artwork, true).await;
}

async fn assert_complete_art_reads(
    media: &MediaRepository,
    scope: &Scope,
    item: Uuid,
    person: Uuid,
    artwork: Uuid,
    present: bool,
) {
    assert_eq!(
        media.list(scope, None, 100).await.unwrap().len(),
        usize::from(present)
    );
    assert_eq!(
        media
            .primary_item_artwork(scope, &[item])
            .await
            .unwrap()
            .contains_key(&item),
        present
    );
    assert_eq!(
        media
            .person_artwork(scope, &[person])
            .await
            .unwrap()
            .contains_key(&person),
        present
    );
    assert_eq!(
        media.branding(scope, 8).await.unwrap().len(),
        usize::from(present)
    );
    for variant in ["native", "original"] {
        assert_eq!(
            media
                .resolve(scope, artwork, 1, variant)
                .await
                .unwrap()
                .is_some(),
            present
        );
    }
}

async fn source_run_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
) {
    for provider in ["jellyfin"] {
        let source = catalog
            .configure_source(
                &format!("exclusion-{provider}"),
                provider,
                &json!({}),
                &json!({}),
            )
            .await
            .unwrap();
        catalog
            .configure_library(source.id, "movies", "movies", "Movies")
            .await
            .unwrap();
        let selection = json!({"library_ids":["movies"]});
        let a = operator
            .create_policy(source.id, &policy(selection.clone()))
            .await
            .unwrap();
        let b = operator
            .create_policy(source.id, &policy(selection.clone()))
            .await
            .unwrap();
        let (ar, br) = tokio::join!(operator.start_run(a.id), operator.start_run(b.id));
        let (run, winner, loser) = match (ar, br) {
            (Ok(run), Err(screenroom_backend::Error::SourceBusy { source_id, run_id })) => {
                assert_eq!(source_id, source.id);
                assert_eq!(run_id, run.id);
                (run, a, b)
            }
            (Err(screenroom_backend::Error::SourceBusy { source_id, run_id }), Ok(run)) => {
                assert_eq!(source_id, source.id);
                assert_eq!(run_id, run.id);
                (run, b, a)
            }
            other => panic!("exactly one concurrent source run must start: {other:?}"),
        };
        assert_eq!(run.source_id, source.id);
        assert!(sqlx::query("INSERT INTO ingest.runs(source_id,policy_id,policy_version,policy_snapshot,selection_snapshot) VALUES($1,$2,1,'{}','{}')")
            .bind(source.id).bind(loser.id).execute(catalog.pool()).await.is_err());
        let id = format!("exclusion-{provider}");
        let document = item(&id, "movie", "movies", "observed document");
        let observed = |completion| page(vec![document.clone()], completion, json!({"next":true}));
        let complete = || Completion::Complete {
            evidence: json!({}),
        };
        let c = ingest.claim("exclusion", 60).await.unwrap().unwrap();
        assert_eq!(c.run_id, Some(run.id));
        ingest
            .commit_fixture_page(&c, &observed(Completion::Continue))
            .await
            .unwrap();
        assert!(matches!(
            operator.start_run(loser.id).await,
            Err(screenroom_backend::Error::SourceBusy { .. })
        ));
        let c = ingest.claim("exclusion", 60).await.unwrap().unwrap();
        ingest
            .commit_fixture_page(&c, &page(vec![], Completion::Pause, json!({})))
            .await
            .unwrap();
        assert!(matches!(
            operator
                .update_policy(loser.id, &policy(selection), true)
                .await,
            Err(screenroom_backend::Error::SourceBusy { .. })
        ));
        assert_eq!(
            operator.policy(loser.id).await.unwrap().unwrap().version,
            loser.version
        );
        assert!(
            !operator
                .due_policies(100)
                .await
                .unwrap()
                .iter()
                .any(|p| p.source_id == source.id)
        );
        ingest.resume(run.id).await.unwrap();
        assert!(matches!(
            operator.start_run(loser.id).await,
            Err(screenroom_backend::Error::SourceBusy { .. })
        ));
        let c = ingest.claim("exclusion", 60).await.unwrap().unwrap();
        ingest
            .commit_fixture_page(&c, &page(vec![], complete(), json!({})))
            .await
            .unwrap();
        let second = operator.start_run(loser.id).await.unwrap();
        let c = ingest.claim("exclusion", 60).await.unwrap().unwrap();
        assert_eq!(c.run_id, Some(second.id));
        ingest
            .commit_fixture_page(&c, &observed(Completion::Continue))
            .await
            .unwrap();
        assert!(matches!(
            operator.start_run(winner.id).await,
            Err(screenroom_backend::Error::SourceBusy { .. })
        ));
        let c = ingest.claim("exclusion", 60).await.unwrap().unwrap();
        ingest
            .commit_fixture_page(&c, &page(vec![], complete(), json!({})))
            .await
            .unwrap();
        let scoped = Scope {
            source_keys: vec![source.source_key.clone()],
            ..Scope::default()
        };
        assert!(catalog.get(&scoped, &id).await.unwrap().is_some());
        operator.start_run(winner.id).await.unwrap();
        let c = ingest
            .claim("concurrent-commits", 60)
            .await
            .unwrap()
            .unwrap();
        let mut continued = page(vec![], Completion::Continue, json!({}));
        continued.followups.push(Followup {
            kind: JobKind::Embedding,
            dedupe_key: format!("concurrent-{provider}"),
            input: json!({"external_id":provider}),
        });
        ingest.commit_fixture_page(&c, &continued).await.unwrap();
        let first = ingest
            .claim("concurrent-commits", 60)
            .await
            .unwrap()
            .unwrap();
        let second = ingest
            .claim("concurrent-commits", 60)
            .await
            .unwrap()
            .unwrap();
        let (traversal, followup) = if first.kind == "ingest" {
            (first, second)
        } else {
            (second, first)
        };
        let terminal = page(
            vec![],
            Completion::Complete {
                evidence: json!({}),
            },
            json!({}),
        );
        let results = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                ingest.commit_fixture_page(&traversal, &terminal),
                ingest.complete_job(&followup)
            )
        })
        .await
        .unwrap();
        results.0.unwrap();
        results.1.unwrap();
    }
}

async fn retrieval_media_credit_regressions(
    catalog: &CatalogRepository,
    operator: &IngestRepository,
    ingest: &IngestRepository,
    search: &SearchRepository,
    media: &screenroom_backend::media::MediaRepository,
    pool: &PgPool,
) {
    let jf = catalog
        .configure_source("reg-jellyfin", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    let secondary_source = catalog
        .configure_source("reg-secondary", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(jf.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let jp = operator
        .create_policy(jf.id, &policy(json!({"library_ids":["movies"]})))
        .await
        .unwrap();
    catalog
        .configure_library(secondary_source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let tp = operator
        .create_policy(
            secondary_source.id,
            &policy(json!({"library_ids":["movies"]})),
        )
        .await
        .unwrap();
    let scope = Scope {
        source_keys: vec![jf.source_key.clone(), secondary_source.source_key.clone()],
        ..Scope::default()
    };
    let mut jf_item = item("reg-shared", "movie", "movies", "local library synopsis");
    jf_item.credits = vec![
        CreditWrite {
            external_id: "tied-person".into(),
            name: "Person".into(),
            metadata: json!({}),
            department: "cast".into(),
            role: "first".into(),
            ordinal: 0,
        },
        CreditWrite {
            external_id: "tied-person".into(),
            name: "Person".into(),
            metadata: json!({}),
            department: "cast".into(),
            role: "second".into(),
            ordinal: 0,
        },
        CreditWrite {
            external_id: "other-person".into(),
            name: "Other".into(),
            metadata: json!({}),
            department: "cast".into(),
            role: "third".into(),
            ordinal: 0,
        },
    ];
    let mut secondary_item = jf_item.clone();
    secondary_item.document = "quasarexclusive discovery text".into();
    secondary_item.metadata["title"] = json!("alternate display");
    let refs = vec![
        ArtworkReferenceInput {
            source_artwork_id: "linked-item".into(),
            kind: "poster".into(),
            rights: json!({}),
            item_external_id: Some("reg-shared".into()),
            person_external_id: None,
            role: "poster".into(),
            ordinal: 0,
        },
        ArtworkReferenceInput {
            source_artwork_id: "linked-person".into(),
            kind: "portrait".into(),
            rights: json!({}),
            item_external_id: None,
            person_external_id: Some("tied-person".into()),
            role: "portrait".into(),
            ordinal: 0,
        },
        ArtworkReferenceInput {
            source_artwork_id: "branding".into(),
            kind: "logo".into(),
            rights: json!({}),
            item_external_id: None,
            person_external_id: None,
            role: "logo".into(),
            ordinal: 0,
        },
    ];
    let run = operator.start_run(jp.id).await.unwrap();
    let c = ingest.claim("regression", 60).await.unwrap().unwrap();
    let mut initial = page(
        vec![jf_item.clone()],
        Completion::Complete {
            evidence: json!({}),
        },
        json!({}),
    );
    initial.artwork_inputs = refs.iter().cloned().map(artwork_input).collect();
    ingest.commit_fixture_page(&c, &initial).await.unwrap();
    // Completed-run followups remain publishable while the next ingestion run exists.
    let next = operator.start_run(jp.id).await.unwrap();
    let verified = verified_object(jf.id).await;
    let mut artwork = vec![];
    for _ in &refs {
        let c = ingest.claim("reg-art", 60).await.unwrap().unwrap();
        assert_eq!(c.run_id, Some(run.id));
        let job: ArtworkJob = serde_json::from_value(c.input.clone()).unwrap();
        let index = refs
            .iter()
            .position(|r| r.source_artwork_id == job.source_artwork_id)
            .unwrap();
        let id = media.publish(&c, &verified).await.unwrap();
        artwork.push((id, index));
    }
    operator.cancel_run(next.id).await.unwrap();
    let mut cursor = None;
    let mut credits = vec![];
    loop {
        let page = catalog
            .credits(&scope, "reg-shared", cursor.as_ref(), 1)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        let position = page[0].cursor();
        assert!(!credits.contains(&position));
        let json = serde_json::to_string(&position).unwrap();
        cursor = Some(serde_json::from_str::<CreditCursor>(&json).unwrap());
        credits.push(position);
    }
    assert_eq!(credits.len(), 3);
    assert!(credits.iter().all(|c| c.ordinal == 0));
    operator.start_run(tp.id).await.unwrap();
    let c = ingest.claim("reg-secondary", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![secondary_item],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        catalog
            .get(&scope, "reg-shared")
            .await
            .unwrap()
            .unwrap()
            .source_id,
        jf.id
    );
    let lexical = search
        .lexical(
            &scope,
            "quasarexclusive",
            &LexicalMethods {
                full_text: true,
                ..Default::default()
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(lexical.len(), 1);
    let alternate = catalog
        .get(
            &Scope {
                source_keys: vec![secondary_source.source_key.clone()],
                ..Scope::default()
            },
            "reg-shared",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lexical[0].source_item_id, alternate.source_item_id);
    let model = search.active_model().await.unwrap().unwrap();
    assert_eq!(
        search.embedding_coverage(&scope, model.id).await.unwrap(),
        EmbeddingCoverage {
            eligible_documents: 2,
            current_embeddings: 0,
            complete: false,
        }
    );
    assert!(matches!(
        search.embedding_coverage(&scope, Uuid::new_v4()).await,
        Err(screenroom_backend::Error::NotFound)
    ));
    let mut unit = vec![0.0; 1024];
    unit[0] = 1.0;
    // Simulate the already-validated worker writes for both documents, then stale the display source.
    for key in [&jf.source_key, &secondary_source.source_key] {
        let item = catalog
            .get(
                &Scope {
                    source_keys: vec![key.clone()],
                    ..Scope::default()
                },
                "reg-shared",
            )
            .await
            .unwrap()
            .unwrap();
        let doc = search
            .document_for_source_item(item.source_item_id)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("INSERT INTO search.embeddings(document_id,model_id,input_hash,vector) VALUES($1,$2,$3,$4::vector)")
            .bind(doc.id).bind(model.id).bind(&doc.content_hash).bind(pgvector::Vector::from(unit.clone())).execute(pool).await.unwrap();
        if key == &jf.source_key {
            assert_eq!(
                search.embedding_coverage(&scope, model.id).await.unwrap(),
                EmbeddingCoverage {
                    eligible_documents: 2,
                    current_embeddings: 1,
                    complete: false,
                }
            );
        }
    }
    assert!(
        search
            .embedding_coverage(&scope, model.id)
            .await
            .unwrap()
            .complete
    );
    let secondary = search
        .document_for_reference(&secondary_source.source_key, "reg-shared")
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE search.documents SET document_version=document_version+1 WHERE id=$1")
        .bind(secondary.id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        search
            .embedding_coverage(&scope, model.id)
            .await
            .unwrap()
            .current_embeddings,
        1
    );
    assert_eq!(
        search
            .vector(&scope, &unit, model.id, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("UPDATE search.documents SET document_version=document_version-1 WHERE id=$1")
        .bind(secondary.id)
        .execute(pool)
        .await
        .unwrap();
    let both = search.vector(&scope, &unit, model.id, 100).await.unwrap();
    assert_eq!(both.len(), 2);
    assert_eq!(reciprocal_rank_fusion(&[both], 100).len(), 1);
    jf_item.document = "changed local synopsis".into();
    operator.start_run(jp.id).await.unwrap();
    let c = ingest.claim("reg-stale", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![jf_item],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    let vectors = search.vector(&scope, &unit, model.id, 100).await.unwrap();
    assert_eq!(
        search.embedding_coverage(&scope, model.id).await.unwrap(),
        EmbeddingCoverage {
            eligible_documents: 2,
            current_embeddings: 1,
            complete: false,
        }
    );
    assert_eq!(vectors.len(), 1);
    assert_eq!(vectors[0].source_item_id, alternate.source_item_id);
    let fused = reciprocal_rank_fusion(&[lexical, vectors], 100);
    assert_eq!(fused.len(), 1);
    let digest = snapshot_digest(
        "quasarexclusive",
        &scope,
        &["lexical".into(), "vector".into()],
    )
    .unwrap();
    let snapshot = search
        .save_snapshot(&SnapshotWrite {
            digest: digest.to_vec(),
            model_revision: Some(model.revision),
            index_revision: "regression".into(),
            results: fused,
            effective_methods: vec!["lexical".into(), "vector".into()],
            degradation_reason: String::new(),
            ttl_seconds: 60,
        })
        .await
        .unwrap();
    let result = search
        .snapshot_page(snapshot, &digest, &scope, 0, 200)
        .await
        .unwrap();
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].source_item_id, alternate.source_item_id);
    let jf_scope = Scope {
        source_keys: vec![jf.source_key.clone()],
        ..Scope::default()
    };
    for (id, _) in &artwork {
        assert!(
            media
                .resolve(&jf_scope, *id, 1, "native")
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(media.list(&jf_scope, None, 100).await.unwrap().len(), 3);
    operator.start_run(jp.id).await.unwrap();
    let c = ingest.claim("reg-absence", 60).await.unwrap().unwrap();
    ingest
        .commit_fixture_page(
            &c,
            &page(
                vec![],
                Completion::Complete {
                    evidence: json!({}),
                },
                json!({}),
            ),
        )
        .await
        .unwrap();
    assert!(
        catalog
            .get(&jf_scope, "reg-shared")
            .await
            .unwrap()
            .is_some()
    );
    // Historical absent memberships remain readable without a mutation API for absence.
    sqlx::query("UPDATE catalog.library_items SET state='absent' WHERE source_id=$1")
        .bind(jf.id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        catalog
            .get(&jf_scope, "reg-shared")
            .await
            .unwrap()
            .is_none()
    );
    let hidden_document = search
        .document_for_reference(&jf.source_key, "reg-shared")
        .await
        .unwrap()
        .unwrap();
    let rebuild = search
        .register_model("local-embedding", "absent-membership-rebuild", 1024)
        .await
        .unwrap();
    assert!(
        search
            .documents_needing_embedding(rebuild.id, 1000)
            .await
            .unwrap()
            .iter()
            .any(|d| d.id == hidden_document.id)
    );
    assert_eq!(
        search.embedding_coverage(&scope, model.id).await.unwrap(),
        EmbeddingCoverage {
            eligible_documents: 1,
            current_embeddings: 1,
            complete: true,
        }
    );
    assert_eq!(
        search.embedding_coverage(&scope, rebuild.id).await.unwrap(),
        EmbeddingCoverage {
            eligible_documents: 1,
            current_embeddings: 0,
            complete: false,
        }
    );
    for (key, external) in [
        ("", "reg-shared"),
        ("reg-jellyfin", ""),
        (&"x".repeat(201), "reg-shared"),
        ("reg-jellyfin", &"x".repeat(201)),
    ] {
        assert!(matches!(
            search.document_for_reference(key, external).await,
            Err(screenroom_backend::Error::Invalid(_))
        ));
    }
    assert!(
        search
            .document_for_reference(&jf.source_key, "missing")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.source_items SET state='absent' WHERE id=$1")
        .bind(hidden_document.source_item_id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        search
            .document_for_reference(&jf.source_key, "reg-shared")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !search
            .documents_needing_embedding(rebuild.id, 1000)
            .await
            .unwrap()
            .iter()
            .any(|d| d.id == hidden_document.id)
    );
    sqlx::query("UPDATE catalog.source_items SET state='enabled' WHERE id=$1")
        .bind(hidden_document.source_item_id)
        .execute(pool)
        .await
        .unwrap();
    catalog.set_source_state(jf.id, false).await.unwrap();
    assert!(
        search
            .document_for_reference(&jf.source_key, "reg-shared")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !search
            .documents_needing_embedding(rebuild.id, 1000)
            .await
            .unwrap()
            .iter()
            .any(|d| d.id == hidden_document.id)
    );
    catalog.set_source_state(jf.id, true).await.unwrap();
    for (id, index) in &artwork {
        for read_scope in [&Scope::default(), &jf_scope] {
            assert_eq!(
                media
                    .resolve(read_scope, *id, 1, "native")
                    .await
                    .unwrap()
                    .is_some(),
                *index == 2
            );
            assert_eq!(
                media
                    .list(read_scope, None, 100)
                    .await
                    .unwrap()
                    .iter()
                    .any(|a| a.id == *id),
                *index == 2
            );
        }
        assert!(
            media
                .resolve(
                    &Scope {
                        providers: vec!["unsupported".into()],
                        ..Scope::default()
                    },
                    *id,
                    1,
                    "native"
                )
                .await
                .unwrap()
                .is_none()
        );
    }
    let removed_scope = Scope {
        source_keys: vec![jf.source_key],
        ..Scope::default()
    };
    assert!(
        search
            .snapshot_page(snapshot, &digest, &removed_scope, 0, 200)
            .await
            .unwrap()
            .results
            .is_empty()
    );
    catalog
        .set_source_state(secondary_source.id, false)
        .await
        .unwrap();
    assert!(
        search
            .document_for_reference(&secondary_source.source_key, "reg-shared")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        search.embedding_coverage(&scope, model.id).await.unwrap(),
        EmbeddingCoverage {
            eligible_documents: 0,
            current_embeddings: 0,
            complete: true,
        }
    );
    assert!(
        search
            .snapshot_page(snapshot, &digest, &scope, 0, 200)
            .await
            .unwrap()
            .results
            .is_empty()
    );
}
