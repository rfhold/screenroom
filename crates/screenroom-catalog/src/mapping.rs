use screenroom_backend::{
    catalog::{CatalogItem, ItemContext},
    media::ArtworkRecord,
};
use screenroom_proto::catalog::v1::*;
use serde_json::Value;

impl crate::Catalog {
    pub(crate) async fn hydrate_items(
        &self,
        scope: &screenroom_backend::catalog::Scope,
        rows: Vec<CatalogItem>,
        user: Option<&str>,
    ) -> Result<Vec<Item>, tonic::Status> {
        self.hydrate_items_in_season(scope, rows, None, user).await
    }

    pub(crate) async fn hydrate_items_in_season(
        &self,
        scope: &screenroom_backend::catalog::Scope,
        rows: Vec<CatalogItem>,
        season_parent: Option<screenroom_backend::Uuid>,
        user: Option<&str>,
    ) -> Result<Vec<Item>, tonic::Status> {
        let user = user
            .map(screenroom_backend::Uuid::parse_str)
            .transpose()
            .map_err(|_| tonic::Status::invalid_argument("user selector must be UUID"))?;
        let revision = match user {
            Some(user) => self.watched.revision(user).await.map_err(crate::status)?,
            None => None,
        };
        let mut items = Vec::with_capacity(rows.len());
        for batch in rows.chunks(200) {
            let ids: Vec<_> = batch.iter().map(|row| row.source_item_id).collect();
            let watched = match user {
                Some(user) => self.watched.read(user, &ids).await.map_err(crate::status)?,
                None => Default::default(),
            };
            let artwork = self
                .media
                .card_item_artwork(scope, &ids, season_parent)
                .await
                .map_err(crate::status)?;
            let episode_ids: Vec<_> = batch
                .iter()
                .filter(|row| row.kind == "episode")
                .map(|row| row.source_item_id)
                .collect();
            let contexts = self
                .catalog
                .item_contexts(scope, &episode_ids)
                .await
                .map_err(crate::status)?;
            for row in batch {
                let mut item = item(row.clone());
                item.watched = watched.get(&row.source_item_id).copied();
                item.subtitle = episode_subtitle(row, contexts.get(&row.source_item_id));
                item.artwork = artwork
                    .get(&row.source_item_id)
                    .cloned()
                    .map(|row| self::artwork(row, &self.origin));
                items.push(item);
            }
        }
        if let Some(user) = user
            && self.watched.revision(user).await.map_err(crate::status)? != revision
        {
            for item in &mut items {
                item.watched = None;
            }
        }
        Ok(items)
    }
}

fn episode_subtitle(row: &CatalogItem, context: Option<&ItemContext>) -> String {
    let fallback = || string(&row.metadata, "subtitle");
    if row.kind != "episode" {
        return fallback();
    }
    let Some(context) = context else {
        return fallback();
    };
    let Some(episode) = number(&row.metadata, "index") else {
        return fallback();
    };
    let Some(season) = context
        .parent_metadata
        .as_ref()
        .filter(|_| context.parent_kind.as_deref() == Some("season"))
        .and_then(|metadata| number(metadata, "index"))
    else {
        return fallback();
    };
    let Some(series) = context
        .grandparent_metadata
        .as_ref()
        .filter(|_| context.grandparent_kind.as_deref() == Some("series"))
        .and_then(|metadata| metadata["title"].as_str())
        .filter(|title| !title.trim().is_empty())
    else {
        return fallback();
    };
    format!("{series} · S{season:02}E{episode:02}")
}

pub(crate) fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().into()
}
fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn number(value: &Value, key: &str) -> Option<u32> {
    value[key].as_u64().and_then(|n| n.try_into().ok())
}
fn rights(v: &Value) -> Option<Rights> {
    v.is_object().then(|| Rights {
        label: string(v, "label"),
        note: string(v, "note"),
        source: string(v, "source"),
        license_url: string(v, "license_url"),
        attribution: string(v, "attribution"),
    })
}
pub(crate) fn item(row: CatalogItem) -> Item {
    let m = &row.metadata;
    let provider_references = m["provider_references"]
        .as_array()
        .map(|references| {
            references
                .iter()
                .map(|reference| {
                    let provider = string(reference, "provider");
                    let mut url = string(reference, "url");
                    if url.is_empty()
                        && let Some(id) = m["provider_ids"].get(&provider).and_then(Value::as_str)
                    {
                        url = reference_url(&provider, id, &row.kind).unwrap_or_default();
                    }
                    ProviderReference {
                        provider,
                        status: string(reference, "status"),
                        url,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Item {
        id: row.public_id,
        title: string(m, "title"),
        kind: match row.kind.as_str() {
            "movie" => "Movie".into(),
            "series" => "Series".into(),
            "season" => "Season".into(),
            "episode" => "Episode".into(),
            _ => row.kind,
        },
        parent_id: row.parent_public_id,
        year: number(m, "year").unwrap_or_default(),
        subtitle: string(m, "subtitle"),
        overview: string(m, "overview"),
        tags: strings(m, "tags"),
        colors: strings(m, "colors"),
        release_date: string(m, "release_date"),
        added_order: number(m, "added_order").unwrap_or_default(),
        title_rights: rights(&m["title_rights"]),
        metadata_rights: rights(&m["metadata_rights"]),
        artwork: None,
        rating: m["rating"]["value"].as_f64().map(|value| Rating {
            value,
            provider: string(&m["rating"], "provider"),
            scale: m["rating"]["scale"].as_f64().unwrap_or_default(),
            votes: m["rating"]["votes"].as_u64(),
            retrieved_at: string(&m["rating"], "retrieved_at"),
        }),
        runtime: m["runtime"].is_object().then(|| Runtime {
            seconds: number(&m["runtime"], "seconds"),
            minimum_seconds: number(&m["runtime"], "minimum_seconds"),
            maximum_seconds: number(&m["runtime"], "maximum_seconds"),
            note: string(&m["runtime"], "note"),
            display: string(&m["runtime"], "display"),
        }),
        provider_references,
        progress_percent: None,
        watched: None,
        provenance_notes: strings(m, "provenance_notes"),
        metadata_retrieved_at: string(m, "metadata_retrieved_at"),
    }
}
// Keep validation equivalent to the worker's source-mapping reference_url.
fn reference_url(provider: &str, id: &str, kind: &str) -> Option<String> {
    let digits = |value: &str| {
        !value.is_empty()
            && value.len() <= 20
            && value.bytes().all(|b| b.is_ascii_digit())
            && value.bytes().any(|b| b != b'0')
    };
    if provider.eq_ignore_ascii_case("imdb") && id.strip_prefix("tt").is_some_and(digits) {
        return Some(format!("https://www.imdb.com/title/{id}/"));
    }
    if provider.eq_ignore_ascii_case("tmdb") && digits(id) {
        let entity = match kind {
            "movie" => "movie",
            "series" => "tv",
            _ => return None,
        };
        return Some(format!("https://www.themoviedb.org/{entity}/{id}"));
    }
    None
}

pub(crate) fn artwork(row: ArtworkRecord, origin: &str) -> Artwork {
    let r = &row.rights;
    Artwork {
        id: row.id.to_string(),
        url: format!(
            "{origin}/artwork/{}/revisions/{}/native",
            row.id, row.revision
        ),
        kind: row.kind,
        label: string(r, "label"),
        creator: string(r, "creator"),
        license: string(r, "license"),
        source: string(r, "source"),
        license_url: r["licenseUrl"]
            .as_str()
            .or_else(|| r["license_url"].as_str())
            .unwrap_or_default()
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenroom_backend::Uuid;
    use serde_json::json;

    #[test]
    fn persisted_reference_projection_preserves_existing_urls_and_provenance() {
        let row = CatalogItem {
            source_item_id: Uuid::nil(),
            item_id: Uuid::nil(),
            source_id: Uuid::nil(),
            public_id: "tt9999999".into(),
            parent_public_id: None,
            kind: "movie".into(),
            source_key: "source".into(),
            provider: "jellyfin".into(),
            external_id: "private".into(),
            provenance: json!({"provider":"jellyfin"}),
            metadata: json!({
                "title":"tt8888888",
                "provider_ids":{"Imdb":"tt0123456","Tmdb":"123","Unknown":"456","imdb":"tt7654321"},
                "provider_references":[
                    {"provider":"Imdb","status":"source_supplied","url":""},
                    {"provider":"Tmdb","status":"source_supplied","url":""},
                    {"provider":"Unknown","status":"unverified","url":""},
                    {"provider":"imdb","status":"original","url":"https://example.com/original"},
                    {"provider":"IMDB","status":"case-mismatch","url":""}
                ]
            }),
        };
        let projected = item(row.clone());
        let refs = projected.provider_references;
        assert_eq!(refs[0].url, "https://www.imdb.com/title/tt0123456/");
        assert_eq!(refs[1].url, "https://www.themoviedb.org/movie/123");
        assert_eq!(refs[2].url, "");
        assert_eq!(refs[3].url, "https://example.com/original");
        assert_eq!(refs[4].url, "");
        for (reference, original) in refs
            .iter()
            .zip(row.metadata["provider_references"].as_array().unwrap())
        {
            assert_eq!(reference.provider, original["provider"]);
            assert_eq!(reference.status, original["status"]);
        }
        assert_eq!(row.metadata["provider_references"][0]["url"], "");
        for (kind, expected) in [
            ("series", "https://www.themoviedb.org/tv/123"),
            ("episode", ""),
            ("season", ""),
            ("other", ""),
        ] {
            let mut changed = row.clone();
            changed.kind = kind.into();
            assert_eq!(item(changed).provider_references[1].url, expected);
        }
        for ids in [
            json!(null),
            json!([]),
            json!({"Imdb":123}),
            json!({"Imdb":"tt/secret"}),
        ] {
            let mut changed = row.clone();
            changed.metadata["provider_ids"] = ids;
            let refs = item(changed).provider_references;
            assert_eq!(refs[0].url, "");
            assert_eq!(refs[3].url, "https://example.com/original");
        }
    }

    #[test]
    fn reference_validation_matches_worker_vectors() {
        for kind in ["movie", "series", "episode"] {
            assert_eq!(
                reference_url("IMDb", "tt0123456", kind).as_deref(),
                Some("https://www.imdb.com/title/tt0123456/")
            );
        }
        assert_eq!(
            reference_url("Tmdb", "123", "movie").as_deref(),
            Some("https://www.themoviedb.org/movie/123")
        );
        assert_eq!(
            reference_url("tmdb", "123", "series").as_deref(),
            Some("https://www.themoviedb.org/tv/123")
        );
        for kind in ["episode", "season"] {
            assert!(reference_url("Tmdb", "123", kind).is_none());
        }
        for id in [
            "",
            "0",
            "-1",
            "123/456",
            "123?token=secret",
            "１２３",
            " 123",
            "123 ",
            "123456789012345678901",
        ] {
            assert!(reference_url("Tmdb", id, "movie").is_none(), "{id}");
            assert!(
                reference_url("Imdb", &format!("tt{id}"), "movie").is_none(),
                "{id}"
            );
        }
        for id in ["123", "nm123", "TT123", "https://imdb.com/title/tt123"] {
            assert!(reference_url("Imdb", id, "movie").is_none());
        }
        assert!(reference_url("unknown", "123", "movie").is_none());
    }

    #[test]
    fn episode_identity_subtitles_require_complete_context() {
        let mut row = CatalogItem {
            source_item_id: Uuid::nil(),
            item_id: Uuid::nil(),
            source_id: Uuid::nil(),
            public_id: "episode".into(),
            parent_public_id: Some("season".into()),
            kind: "episode".into(),
            source_key: "source".into(),
            provider: "jellyfin".into(),
            external_id: "episode".into(),
            provenance: json!({}),
            metadata: json!({"title":"Episode title", "index":5, "subtitle":"Existing mock subtitle"}),
        };
        let mut context = ItemContext {
            source_item_id: row.source_item_id,
            parent_kind: Some("season".into()),
            parent_metadata: Some(json!({"index":2})),
            grandparent_kind: Some("series".into()),
            grandparent_metadata: Some(json!({"title":"Series title"})),
        };
        assert_eq!(
            episode_subtitle(&row, Some(&context)),
            "Series title · S02E05"
        );
        row.metadata["index"] = json!(0);
        context.parent_metadata = Some(json!({"index":0}));
        assert_eq!(
            episode_subtitle(&row, Some(&context)),
            "Series title · S00E00"
        );
        row.metadata["index"] = json!(105);
        context.parent_metadata = Some(json!({"index":12}));
        assert_eq!(
            episode_subtitle(&row, Some(&context)),
            "Series title · S12E105"
        );
        for index in [
            json!(null),
            json!(-1),
            json!("5"),
            json!(1.5),
            json!(4294967296u64),
        ] {
            row.metadata["index"] = index;
            assert_eq!(
                episode_subtitle(&row, Some(&context)),
                "Existing mock subtitle"
            );
        }
        row.metadata["index"] = json!(5);
        context.parent_metadata = Some(json!({}));
        assert_eq!(
            episode_subtitle(&row, Some(&context)),
            "Existing mock subtitle"
        );
        context.parent_metadata = Some(json!({"index":2}));
        for series in [None, Some(json!({})), Some(json!({"title":" "}))] {
            context.grandparent_metadata = series;
            assert_eq!(
                episode_subtitle(&row, Some(&context)),
                "Existing mock subtitle"
            );
        }
        context.parent_kind = Some("series".into());
        context.parent_metadata = Some(json!({"title":"Direct series", "index":2}));
        assert_eq!(
            episode_subtitle(&row, Some(&context)),
            "Existing mock subtitle"
        );
        assert_eq!(episode_subtitle(&row, None), "Existing mock subtitle");
        row.metadata["subtitle"] = json!("");
        assert_eq!(episode_subtitle(&row, None), "");
        row.kind = "movie".into();
        row.metadata["subtitle"] = json!("Movie subtitle");
        assert_eq!(episode_subtitle(&row, Some(&context)), "Movie subtitle");
    }
}
