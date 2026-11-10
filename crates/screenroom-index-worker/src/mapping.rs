use screenroom_backend::{
    Error, Result,
    catalog::{CreditWrite, ExternalIdWrite, ItemWrite},
    embedding::{DOCUMENT_VERSION, MAX_INPUT_CHARS, input_hash, preprocess},
    media::{
        ArtworkInputManifest, ArtworkInputWrite, ArtworkOmission, ArtworkOmissionCode,
        ArtworkReferenceInput, ArtworkVariant, ArtworkVariantInput, ValidatedImage,
    },
};
use screenroom_catalog_adapter::{CatalogEntry, MediaKind, MockAdapters};
use serde_json::json;
use sha2::{Digest, Sha256};

pub fn kind(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Movie => "movie",
        MediaKind::Series => "series",
        MediaKind::Season => "season",
        MediaKind::Episode => "episode",
    }
}

pub fn item(entry: &CatalogEntry, snapshot: &str, people: bool) -> Result<ItemWrite> {
    let i = &entry.item;
    let metadata = serde_json::to_value(i).map_err(|_| Error::Invalid("adapter metadata"))?;
    let document = preprocess(&format!(
        "{} {} {} {} {} {}",
        i.title,
        i.subtitle,
        i.overview,
        i.year.map(|y| y.to_string()).unwrap_or_default(),
        i.tags.join(" "),
        i.credits
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    ))
    .chars()
    .take(MAX_INPUT_CHARS)
    .collect::<String>();
    Ok(ItemWrite {
        public_id: i.id.clone(),
        kind: kind(i.kind).into(),
        parent_public_id: i.parent_id.clone(),
        external_id: entry.reference.external_id.clone(),
        source_revision: Some(snapshot.into()),
        content_hash: input_hash(&metadata.to_string()).to_vec(),
        metadata,
        provenance: json!({"snapshot":snapshot,"title_rights":i.title_rights,
            "metadata_rights":i.metadata_rights,"provider_references":i.provider_references,
            "notes":i.provenance_notes,"retrieved_at":i.metadata_retrieved_at}),
        library_external_ids: entry.library_id.iter().cloned().collect(),
        external_ids: i
            .provider_ids
            .iter()
            .map(|id| ExternalIdWrite {
                provider: id.provider.clone(),
                entity_kind: kind(id.kind).into(),
                external_id: id.id.to_string(),
                evidence: json!({"snapshot":snapshot,"retrieved_at":i.metadata_retrieved_at}),
            })
            .collect(),
        credits: if people {
            i.credits
                .iter()
                .map(|c| {
                    Ok(CreditWrite {
                        external_id: c.id.clone(),
                        name: c.name.clone(),
                        metadata: serde_json::to_value(c).map_err(|_| Error::Invalid("credit"))?,
                        department: c.department.clone(),
                        role: c.role.clone(),
                        ordinal: c.ordinal as i32,
                    })
                })
                .collect::<Result<_>>()?
        } else {
            vec![]
        },
        document_title: i.title.clone(),
        document,
        document_version: DOCUMENT_VERSION,
    })
}

pub fn artwork(
    adapters: &MockAdapters,
    entry: &CatalogEntry,
    people: bool,
) -> Result<(Vec<ArtworkInputWrite>, Vec<ArtworkOmission>)> {
    let mut refs = Vec::new();
    let mut omissions = Vec::new();
    let associations = entry.item.artwork_keys.iter().map(|key| (key, None)).chain(
        entry
            .item
            .credits
            .iter()
            .filter(|_| people)
            .filter_map(|c| c.artwork_key.as_ref().map(|key| (key, Some(c.id.as_str())))),
    );
    for (key, person) in associations {
        let Some(art) = adapters.artwork().iter().find(|a| &a.id == key) else {
            omissions.push(ArtworkOmission {
                source_artwork_id: key.clone(),
                code: ArtworkOmissionCode::Missing,
            });
            continue;
        };
        let reference = ArtworkReferenceInput {
            source_artwork_id: key.clone(),
            kind: art.kind.clone(),
            rights: serde_json::to_value(art).map_err(|_| Error::Invalid("artwork rights"))?,
            item_external_id: person
                .is_none()
                .then(|| entry.reference.external_id.clone()),
            person_external_id: person.map(str::to_owned),
            role: if person.is_some() {
                "portrait"
            } else {
                "primary"
            }
            .into(),
            ordinal: 0,
        };
        match prepare_artwork(adapters, reference) {
            Ok(input) => refs.push(input),
            Err(code) => omissions.push(ArtworkOmission {
                source_artwork_id: key.clone(),
                code,
            }),
        }
    }
    Ok((refs, omissions))
}

pub fn branding(adapters: &MockAdapters) -> Result<(Vec<ArtworkInputWrite>, Vec<ArtworkOmission>)> {
    let Some(art) = adapters.artwork().iter().find(|art| art.id == "tmdb") else {
        return Ok((
            vec![],
            vec![ArtworkOmission {
                source_artwork_id: "tmdb".into(),
                code: ArtworkOmissionCode::Missing,
            }],
        ));
    };
    let reference = ArtworkReferenceInput {
        source_artwork_id: art.id.clone(),
        kind: art.kind.clone(),
        rights: serde_json::to_value(art)
            .map_err(|_| Error::Invalid("provider branding rights"))?,
        item_external_id: None,
        person_external_id: None,
        role: "attribution".into(),
        ordinal: 0,
    };
    Ok(match prepare_artwork(adapters, reference) {
        Ok(input) => (vec![input], vec![]),
        Err(code) => (
            vec![],
            vec![ArtworkOmission {
                source_artwork_id: art.id.clone(),
                code,
            }],
        ),
    })
}

fn prepare_artwork(
    adapters: &MockAdapters,
    reference: ArtworkReferenceInput,
) -> std::result::Result<ArtworkInputWrite, ArtworkOmissionCode> {
    let mut variants = vec![];
    for variant in [ArtworkVariant::Original, ArtworkVariant::Native] {
        let image = artwork_image(adapters, &reference.source_artwork_id, variant).map_err(
            |e| match e {
                Error::NotFound => ArtworkOmissionCode::Missing,
                Error::Unavailable => ArtworkOmissionCode::Unavailable,
                _ => ArtworkOmissionCode::Invalid,
            },
        )?;
        variants.push(ArtworkVariantInput {
            variant,
            sha256: Sha256::digest(image.bytes()).into(),
        });
    }
    Ok(ArtworkInputWrite {
        reference,
        manifest: ArtworkInputManifest {
            version: 1,
            variants,
        },
    })
}

pub fn artwork_image(
    adapters: &MockAdapters,
    key: &str,
    variant: ArtworkVariant,
) -> Result<ValidatedImage> {
    let art = adapters
        .artwork()
        .iter()
        .find(|a| a.id == key)
        .ok_or(Error::NotFound)?;
    let bytes = adapters
        .read_artwork(key, variant == ArtworkVariant::Native)
        .map_err(|e| match e {
            screenroom_catalog_adapter::Error::MissingArtwork => Error::NotFound,
            screenroom_catalog_adapter::Error::Unavailable => Error::Unavailable,
            _ => Error::Invalid("artwork input"),
        })?;
    if variant == ArtworkVariant::Original && art.file.ends_with(".svg") {
        ValidatedImage::svg_original(bytes)
    } else {
        ValidatedImage::raster(bytes)
    }
}
