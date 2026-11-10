use screenroom_backend::{catalog::CatalogItem, media::ArtworkRecord};
use screenroom_proto::catalog::v1::*;
use serde_json::Value;

impl crate::Catalog {
    pub(crate) async fn hydrate_items(
        &self,
        scope: &screenroom_backend::catalog::Scope,
        rows: Vec<CatalogItem>,
    ) -> Result<Vec<Item>, tonic::Status> {
        let mut items = Vec::with_capacity(rows.len());
        for batch in rows.chunks(200) {
            let ids: Vec<_> = batch.iter().map(|row| row.source_item_id).collect();
            let artwork = self
                .media
                .primary_item_artwork(scope, &ids)
                .await
                .map_err(crate::status)?;
            for row in batch {
                let mut item = item(row.clone());
                item.artwork = artwork
                    .get(&row.source_item_id)
                    .cloned()
                    .map(|row| self::artwork(row, &self.origin));
                items.push(item);
            }
        }
        Ok(items)
    }
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
        provider_references: m["provider_references"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| ProviderReference {
                        provider: string(v, "provider"),
                        status: string(v, "status"),
                        url: string(v, "url"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        progress_percent: None,
        provenance_notes: strings(m, "provenance_notes"),
        metadata_retrieved_at: string(m, "metadata_retrieved_at"),
    }
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
