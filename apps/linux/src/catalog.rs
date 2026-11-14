use screenroom_client_core as core;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Series,
    Season,
    Episode,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Movie => "Movie",
            Self::Series => "Series",
            Self::Season => "Season",
            Self::Episode => "Episode",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub year: u16,
    pub meta: String,
    pub overview: String,
    pub colors: (String, String),
    pub parent: Option<String>,
    pub progress: Option<u8>,
    pub watched: Option<bool>,
    pub rights: Value,
    pub metadata: Value,
    pub art: Option<Value>,
}
impl Item {
    pub fn is_watched(&self) -> bool {
        self.watched == Some(true)
    }
    pub fn playback_accessibility(&self, progress: Option<u8>) -> String {
        let mut labels = Vec::new();
        if self.is_watched() {
            labels.push("Watched".to_owned());
        }
        if let Some(progress) = progress {
            labels.push(format!("Playback progress {progress}%"));
        }
        labels.join(", ")
    }
    pub fn card_accessibility(&self, progress: Option<u8>) -> String {
        [
            self.title.clone(),
            self.kind.label().into(),
            self.meta.clone(),
            self.playback_accessibility(progress),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
    }
}
#[derive(Clone, Default)]
pub struct Catalog {
    pub items: Vec<Item>,
    pub libraries: Vec<core::MediaLibrary>,
    pub members: HashMap<String, Vec<String>>,
    pub children: HashMap<String, Vec<String>>,
    pub homes: HashMap<String, Vec<(String, Vec<(String, Option<u8>)>)>>,
    pub home_errors: HashMap<String, String>,
}
thread_local! { static DATA: RefCell<Catalog> = RefCell::default(); }
pub fn install(mut data: Catalog) {
    for ids in data.members.values_mut().chain(data.children.values_mut()) {
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
    }
    DATA.with(|d| d.replace(data));
}
pub fn merge_items(items: Vec<core::Item>) {
    DATA.with(|data| {
        let mut data = data.borrow_mut();
        for item in items {
            if let Some(existing) = data.items.iter_mut().find(|i| i.id == item.id) {
                *existing = map_item(item, vec![]);
            } else {
                data.items.push(map_item(item, vec![]));
            }
        }
    });
}
pub fn catalog() -> Vec<Item> {
    DATA.with(|d| d.borrow().items.clone())
}
pub fn home_rows(library: &str) -> Vec<(String, Vec<(String, Option<u8>)>)> {
    DATA.with(|d| d.borrow().homes.get(library).cloned().unwrap_or_default())
}
pub fn libraries() -> Vec<core::MediaLibrary> {
    DATA.with(|d| d.borrow().libraries.clone())
}
pub fn members(library: &str) -> Vec<String> {
    DATA.with(|d| d.borrow().members.get(library).cloned().unwrap_or_default())
}
pub fn children(parent: &str) -> Vec<String> {
    DATA.with(|d| d.borrow().children.get(parent).cloned().unwrap_or_default())
}
pub fn home_error(title: &str) -> Option<String> {
    DATA.with(|d| d.borrow().home_errors.get(title).cloned())
}
pub fn replace_home(library: &str, rows: Vec<core::HomeRow>) {
    merge_items(rows.iter().flat_map(|r| r.items.clone()).collect());
    DATA.with(|d| {
        let mut data = d.borrow_mut();
        data.home_errors.clear();
        for row in &rows {
            if let Some(error) = &row.read_error {
                data.home_errors.insert(
                    row.title.clone(),
                    crate::presentation::row_error(&error.code).into(),
                );
            }
        }
        data.homes.insert(
            library.into(),
            rows.into_iter()
                .map(|r| {
                    (
                        r.title,
                        r.items
                            .into_iter()
                            .map(|i| (i.id, i.progress_percent.map(|p| p.min(100) as u8)))
                            .collect(),
                    )
                })
                .collect(),
        );
    });
}
pub fn append_page(library: &str, parent: Option<&str>, ids: Vec<String>, items: Vec<core::Item>) {
    merge_items(items);
    DATA.with(|d| {
        let mut data = d.borrow_mut();
        let entries = if let Some(parent) = parent {
            data.children.entry(parent.into()).or_default()
        } else {
            data.members.entry(library.into()).or_default()
        };
        for id in ids {
            if !entries.contains(&id) {
                entries.push(id);
            }
        }
    });
}
pub fn art_for(id: &str) -> Option<Value> {
    catalog()
        .into_iter()
        .find(|i| i.id == id)
        .and_then(|i| i.art)
}
pub fn person_art(person: &Value) -> Option<Value> {
    person["artwork"]
        .as_object()
        .map(|_| person["artwork"].clone())
}
pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn rating_labels(metadata: &Value) -> (String, String) {
    let provider = text(metadata, "ratingProvider");
    let provider = if provider.is_empty() {
        "Rating"
    } else {
        provider
    };
    let scale = metadata["ratingScale"].as_f64();
    match (metadata["rating"].as_f64(), metadata["votes"].as_u64()) {
        (Some(rating), votes) => (
            if votes == Some(0) {
                format!("{provider} · Unrated")
            } else {
                match scale {
                    Some(scale) => format!("{provider} {rating:.1}/{scale}"),
                    None => format!("{provider} {rating:.1}"),
                }
            },
            format!(
                "{} · Snapshot {}",
                votes
                    .map(|votes| format!("{votes} votes"))
                    .unwrap_or_else(|| "Vote count unavailable".into()),
                text(metadata, "retrievedAt")
            ),
        ),
        _ => (
            format!("{provider} —"),
            "Rating snapshot not imported.".into(),
        ),
    }
}
pub fn art_record(art: &core::Artwork) -> Value {
    json!({"id":art.id,"file":art.url,"label":art.label,"creator":art.creator,"license":art.license,"licenseUrl":art.license_url,"source":art.source,"type":art.kind})
}
pub fn provider_info(metadata: &Value) -> String {
    metadata["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            format!(
                "{} · {}{}",
                text(r, "provider"),
                text(r, "status"),
                if text(r, "url").is_empty() {
                    String::new()
                } else {
                    format!(" · {}", text(r, "url"))
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn map_item(item: core::Item, credits: Vec<core::Credit>) -> Item {
    let people = |cast: bool| {
        credits.iter().filter(|c| c.department.eq_ignore_ascii_case("cast") == cast).map(|c| json!({"id":c.id,"name":c.name,"role":c.role,"artwork":c.artwork.as_ref().map(art_record)})).collect::<Vec<_>>()
    };
    let rating = item.rating.as_ref();
    let rights = item.title_rights.unwrap_or_default();
    Item {
        id: item.id,
        title: item.title,
        kind: match item.kind.as_str() {
            "Series" => Kind::Series,
            "Season" => Kind::Season,
            "Episode" => Kind::Episode,
            _ => Kind::Movie,
        },
        year: item.year as u16,
        meta: item.subtitle,
        overview: item.overview,
        colors: (
            item.colors.first().cloned().unwrap_or("#303038".into()),
            item.colors.get(1).cloned().unwrap_or("#505058".into()),
        ),
        parent: item.parent_id,
        progress: item.progress_percent.map(|v| v.min(100) as u8),
        watched: item.watched,
        rights: json!({"rights":rights.label,"note":rights.note,"source":rights.source,"license":rights.license_url,"attribution":rights.attribution}),
        metadata: json!({"rating":rating.map(|r| r.value),"ratingProvider":rating.map(|r| r.provider.clone()),"ratingScale":rating.map(|r| r.scale),"votes":rating.and_then(|r| r.votes),"retrievedAt":rating.map(|r| r.retrieved_at.clone()),"date":item.release_date,"providers":item.provider_references,"rights":item.metadata_rights,"metadataRetrievedAt":item.metadata_retrieved_at,"notes":item.provenance_notes,"cast":people(true),"crew":people(false)}),
        art: item.artwork.as_ref().map(art_record),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watched_state_is_explicit_and_independent_of_progress() {
        for watched in [None, Some(false), Some(true), Some(false), None, Some(true)] {
            let mapped = map_item(
                core::Item {
                    id: "stable-item".into(),
                    watched,
                    progress_percent: Some(100),
                    ..Default::default()
                },
                vec![],
            );
            assert_eq!(mapped.watched, watched);
            assert_eq!(mapped.is_watched(), watched == Some(true));
            let label = mapped.card_accessibility(mapped.progress);
            assert_eq!(
                label.matches("Watched").count(),
                usize::from(watched == Some(true))
            );
            assert!(label.contains("Playback progress 100%"));
            assert_eq!(mapped.id, "stable-item");
        }
        let mapped = map_item(
            core::Item {
                watched: Some(true),
                ..Default::default()
            },
            vec![],
        );
        assert_eq!(mapped.playback_accessibility(None), "Watched");
    }
    #[test]
    fn arbitrary_provider_scale_and_metadata_survive_mapping() {
        let mapped = map_item(
            core::Item {
                id: "unfamiliar-title".into(),
                colors: vec!["#123456".into(), "#abcdef".into()],
                rating: Some(core::Rating {
                    provider: "Local Review Board".into(),
                    value: 3.5,
                    scale: 5.0,
                    votes: None,
                    retrieved_at: "rating-date".into(),
                }),
                provider_references: vec![core::ProviderReference {
                    provider: "Other catalog".into(),
                    status: "Archived".into(),
                    url: "https://example.org/item".into(),
                }],
                metadata_rights: Some(core::Rights {
                    label: "Separate metadata terms".into(),
                    ..Default::default()
                }),
                metadata_retrieved_at: "metadata-date".into(),
                provenance_notes: vec!["Server correction".into()],
                ..Default::default()
            },
            vec![],
        );
        assert_eq!(mapped.colors, ("#123456".into(), "#abcdef".into()));
        assert_eq!(
            rating_labels(&mapped.metadata).0,
            "Local Review Board 3.5/5"
        );
        assert_eq!(
            mapped.metadata["rights"]["label"],
            "Separate metadata terms"
        );
        assert_eq!(mapped.metadata["metadataRetrievedAt"], "metadata-date");
        assert_eq!(mapped.metadata["notes"][0], "Server correction");
        let providers = provider_info(&mapped.metadata);
        assert_eq!(
            providers,
            "Other catalog · Archived · https://example.org/item"
        );
        assert!(!providers.contains("Jellyfin"));
    }
    #[test]
    fn absent_rating_keeps_independent_metadata_provenance() {
        let mapped = map_item(
            core::Item {
                metadata_retrieved_at: "metadata-only-date".into(),
                metadata_rights: Some(core::Rights {
                    note: "Metadata only".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            vec![],
        );
        assert_eq!(rating_labels(&mapped.metadata).0, "Rating —");
        assert_eq!(mapped.metadata["metadataRetrievedAt"], "metadata-only-date");
        assert_eq!(mapped.metadata["rights"]["note"], "Metadata only");
        assert_eq!(provider_info(&mapped.metadata), "");
    }
}
