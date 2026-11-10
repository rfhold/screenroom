use screenroom_backend::{
    catalog::{CatalogRepository, SourceInitialization},
    config::ServiceDatabaseConfig,
    ingest::{LibraryWrite, PolicyWrite},
};
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Installation {
    sources: Vec<Source>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    key: String,
    provider: String,
    configuration: Value,
    capabilities: Value,
    #[serde(default)]
    libraries: Vec<Library>,
    policy: Option<PolicyWrite>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    id: String,
    external_id: String,
    title: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: screenroom-catalog-init installation.json")?;
    let mut input = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Err("installation exceeds 1 MiB".into());
    }
    let installation: Installation = serde_json::from_slice(&input)?;
    if installation.sources.len() > 32 {
        return Err("at most 32 installation sources".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    for s in &installation.sources {
        if s.key.is_empty()
            || s.key.len() > 200
            || !keys.insert(&s.key)
            || !matches!(s.provider.as_str(), "jellyfin" | "tmdb")
        {
            return Err("invalid or duplicate installation source".into());
        }
        if s.libraries.len() > 100 {
            return Err("at most 100 libraries per source".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut external_ids = std::collections::BTreeSet::new();
        for l in &s.libraries {
            if l.id.is_empty()
                || l.id == "all"
                || l.id.len() > 200
                || l.external_id.is_empty()
                || !ids.insert(&l.id)
                || !external_ids.insert(&l.external_id)
            {
                return Err("invalid or duplicate installation library".into());
            }
        }
        if let Some(policy) = &s.policy {
            screenroom_catalog::validate_policy_write(&s.provider, policy)?;
        }
    }
    let pool = screenroom_backend::db::connect_service(&ServiceDatabaseConfig::from_env()?).await?;
    let sources: Vec<_> = installation
        .sources
        .into_iter()
        .map(|source| SourceInitialization {
            source_key: source.key,
            provider: source.provider,
            configuration: source.configuration,
            capabilities: source.capabilities,
            libraries: source
                .libraries
                .into_iter()
                .map(|library| LibraryWrite {
                    public_id: library.id,
                    external_id: library.external_id,
                    title: library.title,
                })
                .collect(),
            initial_policy: source.policy,
        })
        .collect();
    for initialized in CatalogRepository::new(pool).initialize(&sources).await? {
        for policy in initialized.policies {
            println!("{} {}", initialized.source.source_key, policy.id);
        }
    }
    Ok(())
}
