//! Backend-only provider access. Metadata reads and private playback use separate transports.

use crate::MediaKind;
use reqwest::{Client, Url, header};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

mod connection;
mod delta;
mod network;
pub mod playback;
mod revision;
mod secret;
mod watched;
pub use connection::{ApiKey, ConnectionConfig, ConnectionProbe, ServerIdentity};
pub use delta::{
    DeltaError, DeltaItem, DeltaPage, MetadataRevision, ServerCapabilities, ServerVersion,
};
pub use revision::ConfigurationRevision;

#[derive(Clone)]
pub struct JellyfinConfig {
    pub source_key: String,
    pub base_url: String,
    pub secret_file: PathBuf,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
}
impl std::fmt::Debug for JellyfinConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JellyfinConfig")
            .field("source_key", &self.source_key)
            .field("base_url", &"[REDACTED]")
            .field("secret_file", &"[REDACTED]")
            .field("timeout_ms", &self.timeout_ms)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidConfig,
    Secret,
    InvalidScope,
    Transport,
    Resolution,
    DestinationRejected,
    Deadline,
    TooLarge,
    Unauthorized,
    NotFound,
    RateLimited { retry_after_seconds: Option<u32> },
    Http(u16),
    InvalidResponse,
    InvalidServerIdentity,
    UnsupportedCollection,
    Hierarchy,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct LiveJellyfin {
    transport_observer: screenroom_observability::transport::TransportObservation,
    config: JellyfinConfig,
    base: Url,
    client: Client,
    authorization: header::HeaderValue,
}
impl std::fmt::Debug for LiveJellyfin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveJellyfin")
            .field("source_key", &self.config.source_key)
            .field("authorization", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryType {
    Movies,
    Television,
    Collections,
    Unsupported(String),
}
#[derive(Debug, Clone)]
pub struct Library {
    pub id: String,
    pub external_id: String,
    pub name: String,
    pub kind: LibraryType,
}
#[derive(Debug, Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub hidden: bool,
}
#[derive(Debug, Clone)]
pub struct Artwork {
    pub item_id: String,
    pub kind: String,
    pub tag: String,
}
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub external_id: String,
    pub parent_external_id: Option<String>,
    pub kind: MediaKind,
    pub title: String,
    pub overview: Option<String>,
    pub year: Option<u32>,
    pub index: Option<u32>,
    pub provider_ids: BTreeMap<String, String>,
    pub artwork: Vec<Artwork>,
    pub runtime_seconds: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct Collection {
    pub id: String,
    pub external_id: String,
    pub title: String,
    pub overview: Option<String>,
    pub year: Option<u32>,
    pub provider_ids: BTreeMap<String, String>,
    pub artwork: Vec<Artwork>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberKind {
    Media(MediaKind),
    Collection,
    Unsupported(String),
}
/// Collection membership is not canonical media ancestry.
#[derive(Debug, Clone)]
pub struct CollectionMember {
    pub id: String,
    pub external_id: String,
    pub kind: MemberKind,
    pub parent_external_id: Option<String>,
}
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u32,
    pub next_offset: Option<u32>,
    pub reported_total: u32,
    /// Always false: even an exhausted sample cannot authorize absence reconciliation.
    pub authoritative: bool,
}
#[derive(Debug, Clone)]
pub struct Progress {
    pub external_item_id: String,
    pub position_seconds: u64,
    pub runtime_seconds: Option<u64>,
    pub played: bool,
    pub updated_at: Option<String>,
}

impl LiveJellyfin {
    pub async fn from_config(config: JellyfinConfig) -> Result<Self> {
        let base = validate_config(&config)?;
        Self::load_secret(config, base).await
    }
    async fn load_secret(config: JellyfinConfig, base: Url) -> Result<Self> {
        let path = config.secret_file.clone();
        let key = tokio::task::spawn_blocking(move || secret::read_key(&path))
            .await
            .map_err(|_| Error::Secret)??;
        Self::build(config, base, key)
    }
    fn build(config: JellyfinConfig, base: Url, key: ApiKey) -> Result<Self> {
        let mut authorization =
            header::HeaderValue::from_str(&format!("MediaBrowser Token=\"{}\"", key.value()))
                .map_err(|_| Error::Secret)?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .no_proxy()
            .dns_resolver(std::sync::Arc::new(network::GuardedResolver::new(
                config.timeout_ms,
            )))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| Error::InvalidConfig)?;
        Ok(Self {
            transport_observer: Default::default(),
            config,
            base,
            client,
            authorization,
        })
    }
    pub fn source_key(&self) -> &str {
        &self.config.source_key
    }
    pub fn with_transport_observer(
        mut self,
        observer: screenroom_observability::transport::TransportObservation,
    ) -> Self {
        self.transport_observer = observer;
        self
    }
    pub fn public_id(&self, external: &str) -> String {
        let encoded: String = external
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("jellyfin:{}:{encoded}", self.source_key())
    }
    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        {
            let mut path = url.path_segments_mut().map_err(|_| Error::InvalidConfig)?;
            path.pop_if_empty();
            for segment in segments {
                valid_id(segment)?;
                path.push(segment);
            }
        }
        Ok(url)
    }
    async fn bytes(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Vec<u8>> {
        let url = self.url(segments)?;
        use screenroom_observability::transport::{BodyDirection, RequestKind, RequestOutcome};
        let mut observation = self.transport_observer.start(RequestKind::Provider);
        let result = tokio::time::timeout(Duration::from_millis(self.config.timeout_ms), async {
            let mut response = self
                .client
                .get(url)
                .query(query)
                .header(header::AUTHORIZATION, self.authorization.clone())
                .send()
                .await
                .map_err(transport)?;
            let status = response.status().as_u16();
            match status {
                200 => {}
                401 | 403 => return Err(Error::Unauthorized),
                404 => return Err(Error::NotFound),
                429 => {
                    return Err(Error::RateLimited {
                        retry_after_seconds: response
                            .headers()
                            .get(header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse().ok()),
                    });
                }
                _ => return Err(Error::Http(status)),
            }
            if response
                .content_length()
                .is_some_and(|n| n > self.config.max_response_bytes as u64)
            {
                return Err(Error::TooLarge);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(transport)? {
                observation
                    .body()
                    .chunk(BodyDirection::DownloadConsumed, chunk.len());
                if chunk.len() > self.config.max_response_bytes - bytes.len() {
                    return Err(Error::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .unwrap_or(Err(Error::Deadline));
        observation.finish(if result.is_ok() {
            RequestOutcome::Succeeded
        } else {
            RequestOutcome::Failed
        });
        result
    }
    async fn json<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<T> {
        serde_json::from_slice(&self.bytes(segments, query).await?)
            .map_err(|_| Error::InvalidResponse)
    }
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let rows: Vec<LibraryDto> = self.json(&["Library", "VirtualFolders"], &[]).await?;
        rows.into_iter()
            .map(|r| {
                valid_id(&r.item_id)?;
                Ok(Library {
                    id: self.public_id(&r.item_id),
                    external_id: r.item_id,
                    name: clean(r.name),
                    kind: match r.collection_type.as_deref() {
                        Some("movies") => LibraryType::Movies,
                        Some("tvshows") => LibraryType::Television,
                        Some("boxsets") => LibraryType::Collections,
                        other => LibraryType::Unsupported(other.unwrap_or("unknown").into()),
                    },
                })
            })
            .collect()
    }
    pub async fn profiles(&self) -> Result<Vec<Profile>> {
        let rows: Vec<ProfileDto> = self.json(&["Users"], &[]).await?;
        rows.into_iter()
            .filter(|p| !p.policy.is_disabled)
            .map(|p| {
                valid_id(&p.id)?;
                Ok(Profile {
                    id: p.id,
                    name: clean(p.name),
                    hidden: p.policy.is_hidden,
                })
            })
            .collect()
    }
    pub async fn validate_profile(&self, id: &str) -> Result<Profile> {
        valid_id(id)?;
        self.profiles()
            .await?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or(Error::InvalidScope)
    }
    pub async fn library_page(
        &self,
        library: &Library,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Item>> {
        if !matches!(library.kind, LibraryType::Movies | LibraryType::Television) {
            return Err(Error::UnsupportedCollection);
        }
        let mut query = page_query(&library.external_id, offset, limit)?;
        query.push(("CollapseBoxSetItems", "false".into()));
        let rows: ItemsDto = self.json(&["Items"], &query).await?;
        let (next_offset, total) = rows.bounds(offset, limit)?;
        let items = rows
            .items
            .into_iter()
            .map(|i| self.item(i))
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            offset,
            next_offset,
            reported_total: total,
            authoritative: false,
        })
    }
    pub async fn collection_page(
        &self,
        library: &Library,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Collection>> {
        if library.kind != LibraryType::Collections {
            return Err(Error::UnsupportedCollection);
        }
        let mut query = page_query(&library.external_id, offset, limit)?;
        for (key, value) in &mut query {
            if *key == "IncludeItemTypes" {
                *value = "BoxSet".into();
            }
        }
        query.push(("CollapseBoxSetItems", "false".into()));
        let rows: ItemsDto = self.json(&["Items"], &query).await?;
        let (next_offset, reported_total) = rows.bounds(offset, limit)?;
        let items = rows
            .items
            .into_iter()
            .map(|dto| {
                if dto.kind != "BoxSet" {
                    return Err(Error::InvalidResponse);
                }
                valid_id(&dto.id)?;
                let title = title(dto.name.clone())?;
                let artwork = artwork(&dto);
                Ok(Collection {
                    id: self.public_id(&dto.id),
                    external_id: dto.id,
                    title,
                    overview: dto.overview.map(clean),
                    year: dto.production_year,
                    provider_ids: dto.provider_ids,
                    artwork,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            offset,
            next_offset,
            reported_total,
            authoritative: false,
        })
    }
    /// Requires an explicit operator-selected profile to include linked members.
    /// Unknown member types remain visible as Unsupported, never an empty successful page.
    pub async fn collection_members_page(
        &self,
        profile_id: &str,
        collection_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<CollectionMember>> {
        valid_id(profile_id)?;
        let mut query = page_query(collection_id, offset, limit)?;
        query.retain(|(key, _)| *key != "IncludeItemTypes");
        for (key, value) in &mut query {
            if *key == "Recursive" {
                *value = "false".into();
            }
        }
        query.extend([
            ("UserId", profile_id.into()),
            ("EnableUserData", "false".into()),
            ("CollapseBoxSetItems", "false".into()),
        ]);
        let rows: ItemsDto = self.json(&["Items"], &query).await?;
        let (next_offset, reported_total) = rows.bounds(offset, limit)?;
        let items = rows
            .items
            .into_iter()
            .map(|dto| {
                valid_id(&dto.id)?;
                if dto.id == collection_id {
                    return Err(Error::Hierarchy);
                }
                if let Some(parent) = &dto.parent_id {
                    valid_id(parent)?;
                }
                let kind = match dto.kind.as_str() {
                    "Movie" => MemberKind::Media(MediaKind::Movie),
                    "Series" => MemberKind::Media(MediaKind::Series),
                    "Season" => MemberKind::Media(MediaKind::Season),
                    "Episode" => MemberKind::Media(MediaKind::Episode),
                    "BoxSet" => MemberKind::Collection,
                    _ => MemberKind::Unsupported(clean(dto.kind)),
                };
                Ok(CollectionMember {
                    id: self.public_id(&dto.id),
                    external_id: dto.id,
                    kind,
                    parent_external_id: dto.parent_id,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            offset,
            next_offset,
            reported_total,
            authoritative: false,
        })
    }
    pub async fn item_by_external_id(&self, id: &str) -> Result<Item> {
        valid_id(id)?;
        let mut rows: LookupDto = self
            .json(
                &["Items"],
                &[
                    ("Ids", id.into()),
                    ("Fields", "Overview,ProviderIds,ParentId".into()),
                    ("Limit", "1".into()),
                ],
            )
            .await?;
        if rows.items.is_empty() {
            return Err(Error::NotFound);
        }
        if rows.items.len() != 1 {
            return Err(Error::InvalidResponse);
        }
        let row = rows.items.pop().ok_or(Error::InvalidResponse)?;
        if row.id != id {
            return Err(Error::Hierarchy);
        }
        self.item(row)
    }
    /// Returns ancestors root-first, excluding the item. The caller accounts these against its persistence cap.
    pub async fn ancestors(&self, item: &Item, max_depth: usize) -> Result<Vec<Item>> {
        if !(1..=32).contains(&max_depth) {
            return Err(Error::InvalidScope);
        }
        let mut seen = BTreeSet::from([item.external_id.clone()]);
        let mut parent = item.parent_external_id.clone();
        let mut result = Vec::new();
        while let Some(id) = parent {
            if result.len() >= max_depth || !seen.insert(id.clone()) {
                return Err(Error::Hierarchy);
            }
            let item = self.item_by_external_id(&id).await?;
            if item.external_id != id {
                return Err(Error::Hierarchy);
            }
            parent = item.parent_external_id.clone();
            result.push(item);
        }
        result.reverse();
        Ok(result)
    }
    pub async fn artwork_bytes(&self, artwork: &Artwork) -> Result<Vec<u8>> {
        if !["Primary", "Backdrop", "Logo", "Thumb", "Banner"].contains(&artwork.kind.as_str()) {
            return Err(Error::InvalidScope);
        }
        self.bytes(
            &["Items", &artwork.item_id, "Images", &artwork.kind],
            &[("tag", artwork.tag.clone())],
        )
        .await
    }
    pub async fn resume_page(
        &self,
        profile_id: &str,
        library_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Progress>> {
        valid_id(profile_id)?;
        let mut query = page_query(library_id, offset, limit)?;
        query.push(("MediaTypes", "Video".into()));
        let rows: ItemsDto = self
            .json(&["Users", profile_id, "Items", "Resume"], &query)
            .await?;
        let (next_offset, total) = rows.bounds(offset, limit)?;
        let items = rows
            .items
            .into_iter()
            .map(|i| {
                valid_id(&i.id)?;
                let user = i.user_data.ok_or(Error::InvalidResponse)?;
                if i.run_time_ticks
                    .is_some_and(|runtime| runtime <= 0 || user.playback_position_ticks > runtime)
                {
                    return Err(Error::InvalidResponse);
                }
                let position = ticks(user.playback_position_ticks)?;
                let runtime = i.run_time_ticks.map(ticks).transpose()?;
                if runtime.is_some_and(|r| r == 0 || position > r) {
                    return Err(Error::InvalidResponse);
                }
                Ok(Progress {
                    external_item_id: i.id,
                    position_seconds: position,
                    runtime_seconds: runtime,
                    played: user.played,
                    updated_at: user.last_played_date,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            offset,
            next_offset,
            reported_total: total,
            authoritative: false,
        })
    }
    fn item(&self, dto: ItemDto) -> Result<Item> {
        valid_id(&dto.id)?;
        let artwork = artwork(&dto);
        let kind = match dto.kind.as_str() {
            "Movie" => MediaKind::Movie,
            "Series" => MediaKind::Series,
            "Season" => MediaKind::Season,
            "Episode" => MediaKind::Episode,
            _ => return Err(Error::InvalidResponse),
        };
        let parent = match kind {
            MediaKind::Movie | MediaKind::Series => None,
            _ => {
                let p = dto.parent_id.ok_or(Error::Hierarchy)?;
                valid_id(&p)?;
                Some(p)
            }
        };
        Ok(Item {
            id: self.public_id(&dto.id),
            external_id: dto.id,
            parent_external_id: parent,
            kind,
            title: title(dto.name)?,
            overview: dto.overview.map(clean),
            year: dto.production_year,
            index: dto.index_number,
            provider_ids: dto.provider_ids,
            artwork,
            runtime_seconds: dto.run_time_ticks.map(ticks).transpose()?,
        })
    }
}
fn validate_config(config: &JellyfinConfig) -> Result<Url> {
    if config.base_url.len() > 8192
        || config.base_url.trim() != config.base_url
        || config.base_url.contains('\\')
        || config.base_url.chars().any(char::is_control)
        || config.base_url.as_bytes().windows(3).any(|triplet| {
            triplet[0] == b'%'
                && std::str::from_utf8(&triplet[1..])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .is_some_and(|byte| byte.is_ascii_control())
        })
    {
        return Err(Error::InvalidConfig);
    }
    let (scheme, rest) = config
        .base_url
        .split_once("://")
        .ok_or(Error::InvalidConfig)?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or(Error::InvalidConfig)?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || authority.contains(['@', '%'])
        || authority.ends_with(':')
    {
        return Err(Error::InvalidConfig);
    }
    let base = Url::parse(&config.base_url).map_err(|_| Error::InvalidConfig)?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || config.base_url.chars().any(char::is_control)
        || config.source_key.is_empty()
        || config.source_key.len() > 128
        || !config
            .source_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || !(1..=120_000).contains(&config.timeout_ms)
        || !(1..=32 * 1024 * 1024).contains(&config.max_response_bytes)
    {
        return Err(Error::InvalidConfig);
    }
    network::validate_url(&base)?;
    Ok(base)
}
fn transport(error: reqwest::Error) -> Error {
    if let Some(classified) = network::classify(&error) {
        classified
    } else if error.is_timeout() {
        Error::Deadline
    } else {
        Error::Transport
    }
}
fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 1024
        || id.chars().any(char::is_control)
        || id == "."
        || id == ".."
    {
        Err(Error::InvalidScope)
    } else {
        Ok(())
    }
}
fn clean(s: String) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}
fn title(s: String) -> Result<String> {
    let value = clean(s);
    if value.trim().is_empty() {
        Err(Error::InvalidResponse)
    } else {
        Ok(value)
    }
}
/// At most five references: direct primary/logo/thumb/banner and the first backdrop.
fn artwork(dto: &ItemDto) -> Vec<Artwork> {
    let mut result: Vec<_> = dto
        .image_tags
        .iter()
        .filter(|(kind, tag)| {
            ["Primary", "Logo", "Thumb", "Banner"].contains(&kind.as_str())
                && !tag.is_empty()
                && tag.len() <= 1024
                && !tag.chars().any(char::is_control)
        })
        .map(|(kind, tag)| Artwork {
            item_id: dto.id.clone(),
            kind: kind.clone(),
            tag: tag.clone(),
        })
        .collect();
    if let Some(tag) = dto
        .backdrop_image_tags
        .first()
        .filter(|tag| !tag.is_empty() && tag.len() <= 1024 && !tag.chars().any(char::is_control))
    {
        result.push(Artwork {
            item_id: dto.id.clone(),
            kind: "Backdrop".into(),
            tag: tag.clone(),
        });
    }
    result
}
fn ticks(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map(|n| n / 10_000_000)
        .map_err(|_| Error::InvalidResponse)
}
fn page_query(library: &str, offset: u32, limit: u32) -> Result<Vec<(&'static str, String)>> {
    valid_id(library)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(Error::InvalidScope);
    }
    Ok(vec![
        ("ParentId", library.into()),
        ("Recursive", "true".into()),
        ("StartIndex", offset.to_string()),
        ("Limit", limit.to_string()),
        ("IncludeItemTypes", "Movie,Series,Season,Episode".into()),
        ("Fields", "Overview,ProviderIds,ParentId".into()),
        ("EnableTotalRecordCount", "true".into()),
        ("SortBy", "SortName,Id".into()),
        ("SortOrder", "Ascending".into()),
    ])
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LibraryDto {
    item_id: String,
    name: String,
    collection_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProfileDto {
    id: String,
    name: String,
    policy: PolicyDto,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PolicyDto {
    is_disabled: bool,
    is_hidden: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ItemsDto {
    items: Vec<ItemDto>,
    total_record_count: u32,
    start_index: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LookupDto {
    items: Vec<ItemDto>,
}
impl ItemsDto {
    fn bounds(&self, offset: u32, limit: u32) -> Result<(Option<u32>, u32)> {
        if self.items.len() > limit as usize || self.start_index.is_some_and(|n| n != offset) {
            return Err(Error::InvalidResponse);
        }
        let mut ids = BTreeSet::new();
        if self.items.iter().any(|item| !ids.insert(&item.id)) {
            return Err(Error::InvalidResponse);
        }
        let end = offset
            .checked_add(self.items.len() as u32)
            .ok_or(Error::InvalidResponse)?;
        if end > self.total_record_count
            || (self.items.is_empty() && offset < self.total_record_count)
        {
            return Err(Error::InvalidResponse);
        }
        Ok((
            (end < self.total_record_count).then_some(end),
            self.total_record_count,
        ))
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ItemDto {
    id: String,
    name: String,
    #[serde(rename = "Type")]
    kind: String,
    parent_id: Option<String>,
    overview: Option<String>,
    production_year: Option<u32>,
    index_number: Option<u32>,
    run_time_ticks: Option<i64>,
    #[serde(default, deserialize_with = "null_default")]
    provider_ids: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "null_default")]
    image_tags: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "null_default")]
    backdrop_image_tags: Vec<String>,
    user_data: Option<UserDto>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserDto {
    playback_position_ticks: i64,
    played: bool,
    last_played_date: Option<String>,
}
fn null_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests;
