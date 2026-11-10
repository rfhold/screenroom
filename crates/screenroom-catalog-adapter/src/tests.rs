use super::*;

fn adapters(faults: Faults) -> MockAdapters {
    MockAdapters::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join("fixtures/catalog/fixture.json").is_file())
            .unwrap()
            .join("fixtures/catalog"),
        faults,
    )
    .unwrap()
}
fn libraries() -> Vec<String> {
    vec![MOVIES_LIBRARY.into(), TV_LIBRARY.into()]
}
fn policy(feed: Feed, pages: usize) -> FeedPolicy {
    FeedPolicy {
        feed,
        max_pages: pages,
        page_size: 2,
        locale: "en-US".into(),
        region: None,
        trending_window: matches!(feed, Feed::MovieTrending | Feed::TvTrending)
            .then_some(TrendingWindow::Day),
    }
}

struct CopiedFixture {
    root: PathBuf,
}

impl CopiedFixture {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let source = adapters(Faults::default()).root;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "screenroom-adapter-fixture-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("artwork")).unwrap();
        for directory in [Path::new(""), Path::new("artwork")] {
            for entry in std::fs::read_dir(source.join(directory)).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    std::fs::copy(entry.path(), root.join(directory).join(entry.file_name()))
                        .unwrap();
                }
            }
        }
        Self {
            root: root.canonicalize().unwrap(),
        }
    }

    fn load(&self) -> Result<MockAdapters> {
        MockAdapters::load(&self.root, Faults::default())
    }

    fn set_logo_file(&self, file: &str) {
        let manifest = self.root.join("artwork.json");
        let mut artwork: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        artwork
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["id"] == "tmdb")
            .unwrap()["file"] = file.into();
        std::fs::write(manifest, serde_json::to_vec(&artwork).unwrap()).unwrap();
    }
}

impl Drop for CopiedFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn assert_metadata_available(adapter: &MockAdapters) {
    assert!(adapter.jellyfin_item("nosferatu").is_some());
    assert_eq!(
        adapter
            .jellyfin_page(&libraries(), 100, None)
            .unwrap()
            .items
            .len(),
        12
    );
    assert!(
        !adapter
            .tmdb_page(&policy(Feed::MoviePopular, 1), None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn physical_missing_optional_artwork_does_not_block_metadata_or_fall_back() {
    for (file, native) in [("tmdb.svg", false), ("tmdb.png", true)] {
        let fixture = CopiedFixture::new();
        std::fs::remove_file(fixture.root.join("artwork").join(file)).unwrap();
        let adapter = fixture.load().unwrap();
        assert_metadata_available(&adapter);
        assert!(matches!(
            adapter.read_artwork("tmdb", native),
            Err(Error::MissingArtwork)
        ));
        assert!(adapter.read_artwork("tmdb", !native).is_ok());

        let fixture = CopiedFixture::new();
        let adapter = fixture.load().unwrap();
        std::fs::remove_file(fixture.root.join("artwork").join(file)).unwrap();
        assert_metadata_available(&adapter);
        assert!(matches!(
            adapter.read_artwork("tmdb", native),
            Err(Error::MissingArtwork)
        ));
    }
    let fixture = CopiedFixture::new();
    std::fs::rename(
        fixture.root.join("artwork"),
        fixture.root.join("omitted-artwork"),
    )
    .unwrap();
    let adapter = fixture.load().unwrap();
    assert_metadata_available(&adapter);
    assert!(matches!(
        adapter.read_artwork("tmdb", false),
        Err(Error::MissingArtwork)
    ));
    assert!(matches!(
        adapter.read_artwork("tmdb", true),
        Err(Error::MissingArtwork)
    ));
    fixture.set_logo_file("../fixture.json");
    assert!(matches!(fixture.load(), Err(Error::Fixture(_))));
}

#[test]
fn missing_artwork_does_not_relax_manifest_path_or_provenance_validation() {
    for file in [
        "",
        "../fixture.json",
        "/tmdb.svg",
        "./tmdb.svg",
        "bad\0path",
    ] {
        let fixture = CopiedFixture::new();
        fixture.set_logo_file(file);
        assert!(matches!(fixture.load(), Err(Error::Fixture(_))), "{file:?}");
    }
    let fixture = CopiedFixture::new();
    std::fs::write(fixture.root.join("artwork.json"), b"invalid JSON").unwrap();
    assert!(matches!(fixture.load(), Err(Error::Fixture(_))));
    let fixture = CopiedFixture::new();
    let manifest = fixture.root.join("artwork.json");
    let mut artwork: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    artwork[0]["license"] = "".into();
    std::fs::write(manifest, serde_json::to_vec(&artwork).unwrap()).unwrap();
    assert!(matches!(fixture.load(), Err(Error::Fixture(_))));
}

#[cfg(unix)]
#[test]
fn artwork_symlinks_cannot_escape_canonical_directory_or_fixture_root() {
    use std::os::unix::fs::symlink;

    for target in ["fixture.json", "absent.svg"] {
        let fixture = CopiedFixture::new();
        let logo = fixture.root.join("artwork/tmdb.svg");
        std::fs::remove_file(&logo).unwrap();
        symlink(fixture.root.join(target), &logo).unwrap();
        assert!(matches!(fixture.load(), Err(Error::Fixture(_))));
    }
    for missing in [false, true] {
        let fixture = CopiedFixture::new();
        let outside = CopiedFixture::new();
        std::fs::rename(
            fixture.root.join("artwork"),
            fixture.root.join("original-artwork"),
        )
        .unwrap();
        symlink(
            outside
                .root
                .join(if missing { "absent" } else { "artwork" }),
            fixture.root.join("artwork"),
        )
        .unwrap();
        assert!(matches!(fixture.load(), Err(Error::Fixture(_))));
    }
    let fixture = CopiedFixture::new();
    let adapter = fixture.load().unwrap();
    let logo = fixture.root.join("artwork/tmdb.svg");
    std::fs::remove_file(&logo).unwrap();
    symlink(fixture.root.join("fixture.json"), &logo).unwrap();
    assert!(matches!(
        adapter.read_artwork("tmdb", false),
        Err(Error::Fixture(_))
    ));

    let fixture = CopiedFixture::new();
    let before = fixture.load().unwrap().read_artwork("tmdb", false).unwrap();
    std::fs::rename(
        fixture.root.join("artwork"),
        fixture.root.join("original-artwork"),
    )
    .unwrap();
    symlink(
        fixture.root.join("original-artwork"),
        fixture.root.join("artwork"),
    )
    .unwrap();
    let logo = fixture.root.join("original-artwork/tmdb.svg");
    std::fs::rename(&logo, fixture.root.join("original-artwork/source.svg")).unwrap();
    symlink("source.svg", &logo).unwrap();
    assert_eq!(
        fixture.load().unwrap().read_artwork("tmdb", false).unwrap(),
        before
    );
}

#[test]
fn selected_libraries_complete_only_after_full_traversal() {
    let a = adapters(Faults::default());
    assert_eq!(a.libraries().len(), 2);
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let p = a.jellyfin_page(&libraries(), 5, cursor.as_deref()).unwrap();
        ids.extend(p.items.iter().map(|i| i.item.id.clone()));
        assert_eq!(p.evidence.traversed, ids.len());
        if p.next_cursor.is_none() {
            assert_eq!(p.coverage, Coverage::SelectedLibrariesComplete);
            assert_eq!(p.evidence.total_in_scope, ids.len());
            break;
        }
        assert_eq!(p.coverage, Coverage::InProgress);
        cursor = p.next_cursor;
    }
    assert_eq!(ids.len(), 12);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 12);
    let movies = a
        .jellyfin_page(&[MOVIES_LIBRARY.into()], 100, None)
        .unwrap();
    assert_eq!(movies.items.len(), 8);
    assert!(movies.items.iter().all(|i| i.item.kind == MediaKind::Movie));
}

#[test]
fn feed_windows_are_ordered_bounded_and_never_full_catalog_coverage() {
    let a = adapters(Faults::default());
    for feed in [
        Feed::MoviePopular,
        Feed::MovieUpcoming,
        Feed::MovieNowPlaying,
        Feed::MovieTrending,
        Feed::TvPopular,
        Feed::TvOnTheAir,
        Feed::TvTrending,
    ] {
        let policy = policy(feed, 1);
        let p = a.tmdb_page(&policy, None).unwrap();
        assert!(p.items.len() <= 2);
        assert_eq!(p.coverage, Coverage::BoundedWindowComplete);
        assert!(p.next_cursor.is_none());
        assert!(p.items.iter().all(|i| i.item.kind == policy.media_kind()));
        assert_eq!(
            p.items.iter().map(|i| i.ordinal).collect::<Vec<_>>(),
            (0..p.items.len()).collect::<Vec<_>>()
        );
    }
    let popular = a.tmdb_page(&policy(Feed::MoviePopular, 2), None).unwrap();
    let upcoming = a.tmdb_page(&policy(Feed::MovieUpcoming, 2), None).unwrap();
    let second = a
        .tmdb_page(
            &policy(Feed::MoviePopular, 2),
            popular.next_cursor.as_deref(),
        )
        .unwrap();
    assert_eq!(second.items[0].reference, upcoming.items[0].reference);
    assert_eq!(second.items[0].ordinal, 2);
    assert_eq!(second.coverage, Coverage::BoundedWindowComplete);
    let mut bad = policy(Feed::TvPopular, 1);
    bad.region = Some("US".into());
    assert!(matches!(a.tmdb_page(&bad, None), Err(Error::InvalidScope)));
    bad = policy(Feed::MovieTrending, 1);
    bad.trending_window = None;
    assert!(matches!(a.tmdb_page(&bad, None), Err(Error::InvalidScope)));
}

#[test]
fn cursors_are_deterministic_and_bound_to_snapshot_source_scope_user_and_size() {
    let a = adapters(Faults::default());
    let c = a
        .jellyfin_page(&libraries(), 2, None)
        .unwrap()
        .next_cursor
        .unwrap();
    assert_eq!(
        c,
        a.jellyfin_page(&libraries(), 2, None)
            .unwrap()
            .next_cursor
            .unwrap()
    );
    assert!(matches!(
        a.jellyfin_page(&[MOVIES_LIBRARY.into()], 2, Some(&c)),
        Err(Error::InvalidCursor)
    ));
    assert!(matches!(
        a.jellyfin_page(&libraries(), 3, Some(&c)),
        Err(Error::InvalidCursor)
    ));
    assert!(matches!(
        a.tmdb_page(&policy(Feed::MoviePopular, 2), Some(&c)),
        Err(Error::InvalidCursor)
    ));
    let b = adapters(Faults {
        revision: 1,
        ..Faults::default()
    });
    assert!(matches!(
        b.jellyfin_page(&libraries(), 2, Some(&c)),
        Err(Error::InvalidCursor)
    ));
    let b = adapters(Faults {
        expire_cursors: true,
        ..Faults::default()
    });
    assert!(matches!(
        b.jellyfin_page(&libraries(), 2, Some(&c)),
        Err(Error::CursorExpired)
    ));
    let user = a
        .continue_watching(MOCK_USER_ALICE, &libraries(), 1, None)
        .unwrap()
        .next_cursor
        .unwrap();
    assert!(matches!(
        a.continue_watching(MOCK_USER_BOB, &libraries(), 1, Some(&user)),
        Err(Error::InvalidCursor)
    ));
}

#[test]
fn duplicate_loop_partial_and_rate_failures_do_not_claim_coverage() {
    let looping = adapters(Faults {
        loop_cursor_page: Some(0),
        ..Faults::default()
    });
    let first_loop = looping.jellyfin_page(&libraries(), 2, None).unwrap();
    let repeat = looping
        .jellyfin_page(&libraries(), 2, first_loop.next_cursor.as_deref())
        .unwrap();
    assert_eq!(first_loop.next_cursor, repeat.next_cursor);
    assert_eq!(first_loop.items[0].reference, repeat.items[0].reference);
    assert_eq!(repeat.coverage, Coverage::Partial);
    let a = adapters(Faults {
        duplicate_page: Some(1),
        ..Faults::default()
    });
    let first = a.jellyfin_page(&libraries(), 6, None).unwrap();
    let second = a
        .jellyfin_page(&libraries(), 6, first.next_cursor.as_deref())
        .unwrap();
    assert_eq!(first.items[0].reference, second.items[0].reference);
    assert_eq!(second.coverage, Coverage::Partial);
    let a = adapters(Faults {
        loop_cursor_page: Some(1),
        ..Faults::default()
    });
    let first = a.jellyfin_page(&libraries(), 2, None).unwrap();
    let second = a
        .jellyfin_page(&libraries(), 2, first.next_cursor.as_deref())
        .unwrap();
    assert_eq!(first.next_cursor, second.next_cursor);
    assert_eq!(second.coverage, Coverage::Partial);
    let a = adapters(Faults {
        partial_page: Some(0),
        ..Faults::default()
    });
    let p = a.jellyfin_page(&libraries(), 2, None).unwrap();
    assert_eq!(p.coverage, Coverage::Partial);
    assert!(p.next_cursor.is_none());
    let a = adapters(Faults {
        rate_limit_page: Some(0),
        ..Faults::default()
    });
    assert!(matches!(
        a.jellyfin_page(&libraries(), 2, None),
        Err(Error::RateLimited {
            retry_after_seconds: 2
        })
    ));
    let a = adapters(Faults {
        unavailable_page: Some(0),
        ..Faults::default()
    });
    assert!(matches!(
        a.jellyfin_page(&libraries(), 2, None),
        Err(Error::Unavailable)
    ));
}

#[test]
fn progress_is_private_mapped_user_state_never_shared_metadata() {
    let a = adapters(Faults::default());
    let alice = a
        .continue_watching(MOCK_USER_ALICE, &libraries(), 100, None)
        .unwrap();
    let bob = a
        .continue_watching(MOCK_USER_BOB, &libraries(), 100, None)
        .unwrap();
    assert_eq!(alice.items[0].reference, bob.items[1].reference);
    assert_ne!(
        alice.items[0].position_seconds,
        bob.items[1].position_seconds
    );
    assert!(
        a.continue_watching("unmapped", &libraries(), 100, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        a.continue_watching(MOCK_USER_ALICE, &[TV_LIBRARY.into()], 100, None)
            .unwrap()
            .items
            .len(),
        1
    );
    for entry in a.jellyfin_page(&libraries(), 100, None).unwrap().items {
        let serialized = serde_json::to_string(&entry.item).unwrap();
        assert!(!serialized.contains("progress"));
        assert!(!serialized.contains("position_seconds"));
    }
}

#[test]
fn tmdb_children_traverse_typed_hierarchy_without_jellyfin_enumeration() {
    let a = adapters(Faults::default());
    let roots = a.tmdb_page(&policy(Feed::TvPopular, 1), None).unwrap();
    let series = &roots.items[0].item.provider_ids[0];
    let seasons = a.tmdb_children(series, 1, None).unwrap();
    assert_eq!(seasons.coverage, Coverage::BoundedWindowComplete);
    assert_eq!(seasons.items.len(), 1);
    assert_eq!(seasons.items[0].reference.source, TMDB_SOURCE);
    assert_eq!(seasons.items[0].item.id, "beverly-hillbillies-s1");
    assert_eq!(
        seasons.items[0].item.parent_id.as_deref(),
        Some("beverly-hillbillies")
    );
    let season = &seasons.items[0].item.provider_ids[0];
    let first = a.tmdb_children(season, 1, None).unwrap();
    assert_eq!(first.coverage, Coverage::InProgress);
    assert_eq!(first.items[0].item.id, "beverly-hillbillies-s1-e1");
    assert_eq!(first.items[0].ordinal, 0);
    assert_eq!(first.evidence.source, TMDB_SOURCE);
    let cursor = first.next_cursor.as_deref();
    let second = a.tmdb_children(season, 1, cursor).unwrap();
    assert_eq!(second.items[0].item.id, "beverly-hillbillies-s1-e2");
    assert_eq!(second.items[0].ordinal, 1);
    assert_eq!(second.coverage, Coverage::BoundedWindowComplete);
    assert_eq!(second.evidence.traversed, 2);
    assert_eq!(second.evidence.total_in_scope, 2);
    assert!(second.next_cursor.is_none());
    assert_eq!(first.evidence.snapshot, second.evidence.snapshot);
    assert_eq!(first.evidence.scope, second.evidence.scope);
    assert_eq!(
        first.next_cursor,
        a.tmdb_children(season, 1, None).unwrap().next_cursor
    );
    let combined = a.tmdb_children(season, 2, None).unwrap();
    assert_eq!(combined.items[0].reference, first.items[0].reference);
    assert_eq!(combined.items[1].reference, second.items[0].reference);
    for entry in seasons
        .items
        .iter()
        .chain(first.items.iter())
        .chain(second.items.iter())
    {
        let lookup = a.tmdb_lookup(&entry.item.provider_ids[0]).unwrap();
        assert_eq!(
            serde_json::to_value(&entry.item).unwrap(),
            serde_json::to_value(&lookup.item).unwrap()
        );
    }
    assert!(matches!(
        a.tmdb_children(series, 1, cursor),
        Err(Error::InvalidCursor)
    ));
    assert!(matches!(
        a.tmdb_children(season, 2, cursor),
        Err(Error::InvalidCursor)
    ));
    assert!(matches!(
        a.tmdb_page(&policy(Feed::TvPopular, 1), cursor),
        Err(Error::InvalidCursor)
    ));
    assert!(matches!(
        a.tmdb_children(season, 0, None),
        Err(Error::InvalidScope)
    ));
    assert!(matches!(
        a.tmdb_children(season, 101, None),
        Err(Error::InvalidScope)
    ));
    for parent in [
        ProviderId {
            provider: "tmdb".into(),
            kind: MediaKind::Series,
            id: 999999,
        },
        ProviderId {
            provider: "other".into(),
            ..series.clone()
        },
        ProviderId {
            provider: "tmdb".into(),
            kind: MediaKind::Movie,
            id: 653,
        },
        first.items[0].item.provider_ids[0].clone(),
    ] {
        let empty = a.tmdb_children(&parent, 1, None).unwrap();
        assert!(empty.items.is_empty());
        assert_eq!(empty.coverage, Coverage::BoundedWindowComplete);
        assert_eq!(empty.evidence.total_in_scope, 0);
    }
}

#[test]
fn tmdb_children_faults_are_partial_and_cursors_are_snapshot_bound() {
    let parent = ProviderId {
        provider: "tmdb".into(),
        kind: MediaKind::Season,
        id: 5593,
    };
    let normal = adapters(Faults::default());
    let cursor = normal
        .tmdb_children(&parent, 1, None)
        .unwrap()
        .next_cursor
        .unwrap();
    let changed = adapters(Faults {
        revision: 1,
        ..Faults::default()
    });
    assert!(matches!(
        changed.tmdb_children(&parent, 1, Some(&cursor)),
        Err(Error::InvalidCursor)
    ));
    let expired = adapters(Faults {
        expire_cursors: true,
        ..Faults::default()
    });
    assert!(matches!(
        expired.tmdb_children(&parent, 1, Some(&cursor)),
        Err(Error::CursorExpired)
    ));
    for faults in [
        Faults {
            duplicate_page: Some(1),
            ..Faults::default()
        },
        Faults {
            partial_page: Some(1),
            ..Faults::default()
        },
        Faults {
            loop_cursor_page: Some(1),
            ..Faults::default()
        },
    ] {
        let a = adapters(faults.clone());
        let first = a.tmdb_children(&parent, 1, None).unwrap();
        let second = a
            .tmdb_children(&parent, 1, first.next_cursor.as_deref())
            .unwrap();
        assert_eq!(second.coverage, Coverage::Partial);
        if faults.duplicate_page.is_some() {
            assert_eq!(second.items[0].reference, first.items[0].reference);
        }
        if faults.partial_page.is_some() {
            assert!(second.next_cursor.is_none());
        }
        if faults.loop_cursor_page.is_some() {
            assert_eq!(second.next_cursor, first.next_cursor);
        }
    }
    let limited = adapters(Faults {
        rate_limit_page: Some(1),
        ..Faults::default()
    });
    let first = limited.tmdb_children(&parent, 1, None).unwrap();
    assert!(matches!(
        limited.tmdb_children(&parent, 1, first.next_cursor.as_deref()),
        Err(Error::RateLimited { .. })
    ));
}

#[test]
fn provider_display_references_preserve_canonical_hierarchy_status_and_json() {
    let a = adapters(Faults::default());
    let canonical: serde_json::Value =
        serde_json::from_slice(&std::fs::read(a.root.join("fixture.json")).unwrap()).unwrap();
    for (id, url) in [
        ("nosferatu", "https://www.themoviedb.org/movie/653"),
        (
            "beverly-hillbillies-s1",
            "https://www.themoviedb.org/tv/1930/season/1",
        ),
        (
            "beverly-hillbillies-s1-e1",
            "https://www.themoviedb.org/tv/1930/season/1/episode/1",
        ),
    ] {
        let item = a.jellyfin_item(id).unwrap().item;
        assert_eq!(
            item.provider_references,
            vec![
                ProviderReference {
                    provider: "TMDB".into(),
                    status: "Listed".into(),
                    url: url.into(),
                },
                ProviderReference {
                    provider: "Jellyfin".into(),
                    status: "Not connected".into(),
                    url: String::new(),
                },
            ]
        );
        let serialized = serde_json::to_value(&item).unwrap();
        let original = canonical["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .unwrap();
        assert_eq!(
            serialized["provider_references"],
            original["provider_references"]
        );
        let roundtrip: Item = serde_json::from_value(serialized).unwrap();
        assert_eq!(roundtrip.provider_references, item.provider_references);
        let enriched = a.tmdb_lookup(&item.provider_ids[0]).unwrap().item;
        assert_eq!(enriched.provider_references, item.provider_references);
    }
}

#[test]
fn original_artwork_rights_trusted_ids_hierarchy_and_null_counts_survive() {
    let a = adapters(Faults::default());
    let season = a.jellyfin_item("beverly-hillbillies-s1").unwrap().item;
    assert_eq!(season.parent_id.as_deref(), Some("beverly-hillbillies"));
    assert_eq!(season.rating.unwrap().votes, None);
    assert_eq!(season.provider_ids[0].id, 5593);
    let id = ProviderId {
        provider: "tmdb".into(),
        kind: MediaKind::Movie,
        id: 653,
    };
    assert_eq!(a.tmdb_lookup(&id).unwrap().item.id, "nosferatu");
    assert!(
        a.tmdb_lookup(&ProviderId {
            kind: MediaKind::Series,
            ..id
        })
        .is_none()
    );
    let bunny = a.jellyfin_item("big-buck-bunny").unwrap().item;
    assert_eq!(bunny.credits[1].name, "Sacha Goedegebure");
    assert_eq!(bunny.credits[1].ordinal, 1);
    assert_eq!(bunny.credits[1].provider_person_id, Some(64936));
    assert_eq!(
        bunny.credits[1].artwork_key.as_deref(),
        Some("Sacha Goedegebure")
    );
    assert_eq!(bunny.title_rights.label, "CC BY 3.0");
    let svg = a.read_artwork("tmdb", false).unwrap();
    let png = a.read_artwork("tmdb", true).unwrap();
    assert!(String::from_utf8(svg.clone()).unwrap().contains("<svg"));
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(svg, std::fs::read(a.root.join("artwork/tmdb.svg")).unwrap());
    assert_eq!(png, std::fs::read(a.root.join("artwork/tmdb.png")).unwrap());
    assert!(
        a.artwork()
            .iter()
            .find(|a| a.id == "tmdb")
            .unwrap()
            .derivation_note
            .is_some()
    );
    assert!(a.notices().iter().any(|n| n.id == "tmdb-attribution"));
    assert!(safe_art_path(&a.root, "../fixture.json").is_err());
    let rating: Rating = serde_json::from_str(
        r#"{"provider":"TMDB","value":null,"scale":10,"votes":null,"retrieved_at":"snapshot"}"#,
    )
    .unwrap();
    assert!(rating.value.is_none());
}

#[test]
fn scenario_changes_are_isolated_and_removal_is_an_authoritative_library_snapshot() {
    let normal = adapters(Faults::default());
    let changed = adapters(Faults {
        removed_item: Some("nosferatu".into()),
        missing_artwork: Some("tmdb".into()),
        updated_title: Some(("sintel".into(), "Updated Sintel".into())),
        ..Faults::default()
    });
    assert!(changed.jellyfin_item("nosferatu").is_none());
    assert!(normal.jellyfin_item("nosferatu").is_some());
    assert_eq!(
        changed.jellyfin_item("sintel").unwrap().item.title,
        "Updated Sintel"
    );
    assert_eq!(normal.jellyfin_item("sintel").unwrap().item.title, "Sintel");
    assert!(matches!(
        changed.read_artwork("tmdb", true),
        Err(Error::MissingArtwork)
    ));
    assert!(normal.read_artwork("tmdb", true).is_ok());
    let p = changed.jellyfin_page(&libraries(), 100, None).unwrap();
    assert_eq!(p.items.len(), 11);
    assert_eq!(p.coverage, Coverage::SelectedLibrariesComplete);
}
