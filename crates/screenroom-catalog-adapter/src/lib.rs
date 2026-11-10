//! Offline provider inputs. No transport, persistence, authentication, or retry ownership.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

pub const JELLYFIN_SOURCE: &str = "mock-jellyfin";
pub const TMDB_SOURCE: &str = "mock-tmdb";
pub const MOVIES_LIBRARY: &str = "movies";
pub const TV_LIBRARY: &str = "tv";
pub const MOCK_USER_ALICE: &str = "mock-jellyfin-alice";
pub const MOCK_USER_BOB: &str = "mock-jellyfin-bob";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Fixture(String),
    InvalidScope,
    InvalidCursor,
    CursorExpired,
    RateLimited { retry_after_seconds: u32 },
    Unavailable,
    MissingArtwork,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Movie,
    Series,
    Season,
    Episode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderId {
    pub provider: String,
    pub kind: MediaKind,
    pub id: u64,
}

/// Display provenance only; neither trusted identity nor connection/playback availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderReference {
    pub provider: String,
    pub status: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rights {
    pub label: String,
    pub note: String,
    pub source: String,
    pub license_url: String,
    pub attribution: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rating {
    pub provider: String,
    pub value: Option<f64>,
    pub scale: u32,
    pub votes: Option<u64>,
    pub retrieved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    pub seconds: Option<u64>,
    pub minimum_seconds: Option<u64>,
    pub maximum_seconds: Option<u64>,
    pub note: String,
    pub display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credit {
    pub id: String,
    pub name: String,
    pub role: String,
    pub department: String,
    #[serde(default)]
    pub ordinal: usize,
    #[serde(default)]
    pub provider_person_id: Option<u64>,
    #[serde(rename = "artwork", skip_serializing)]
    artwork_input: Option<ArtworkInput>,
    #[serde(default)]
    pub artwork_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtworkInput {
    id: String,
}

/// Shared metadata deliberately has no watch state or user fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub kind: MediaKind,
    pub parent_id: Option<String>,
    pub year: Option<u32>,
    pub subtitle: String,
    pub overview: String,
    pub tags: Vec<String>,
    pub colors: Vec<String>,
    pub release_date: Option<String>,
    pub added_order: u32,
    pub title_rights: Rights,
    pub metadata_rights: Rights,
    pub rating: Option<Rating>,
    pub runtime: Option<Runtime>,
    pub metadata_retrieved_at: String,
    pub provenance_notes: Vec<String>,
    #[serde(default)]
    pub provider_references: Vec<ProviderReference>,
    #[serde(default)]
    pub provider_ids: Vec<ProviderId>,
    #[serde(default)]
    pub credits: Vec<Credit>,
    #[serde(default)]
    pub artwork_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    pub id: String,
    pub file: String,
    pub render_file: Option<String>,
    pub derivation_note: Option<String>,
    pub label: String,
    pub creator: String,
    pub license: String,
    pub source: String,
    pub download: String,
    pub license_url: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub section: String,
    pub title: String,
    pub text: String,
    pub link_label: String,
    pub link_url: String,
}

/// All fault state belongs to this adapter instance, never to global fixture files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Faults {
    pub duplicate_page: Option<usize>,
    pub rate_limit_page: Option<usize>,
    pub unavailable_page: Option<usize>,
    pub partial_page: Option<usize>,
    pub loop_cursor_page: Option<usize>,
    pub expire_cursors: bool,
    pub removed_item: Option<String>,
    pub missing_artwork: Option<String>,
    pub updated_title: Option<(String, String)>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    pub source: String,
    pub external_id: String,
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub reference: ItemRef,
    pub library_id: Option<String>,
    /// Zero-based ordinal within this library selection or feed window.
    pub ordinal: usize,
    pub item: Item,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    InProgress,
    /// Only this evidence permits selected-library absence reconciliation.
    SelectedLibrariesComplete,
    /// Never authorizes title deletion, even at the end of a configured feed.
    BoundedWindowComplete,
    Partial,
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub coverage: Coverage,
    pub evidence: CoverageEvidence,
}

#[derive(Debug, Clone)]
pub struct CoverageEvidence {
    pub source: String,
    pub scope: String,
    pub snapshot: String,
    pub traversed: usize,
    pub total_in_scope: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feed {
    MoviePopular,
    MovieUpcoming,
    MovieNowPlaying,
    MovieTrending,
    TvPopular,
    TvOnTheAir,
    TvTrending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendingWindow {
    Day,
    Week,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedPolicy {
    pub feed: Feed,
    pub max_pages: usize,
    pub page_size: usize,
    pub locale: String,
    pub region: Option<String>,
    pub trending_window: Option<TrendingWindow>,
}

impl FeedPolicy {
    pub fn media_kind(&self) -> MediaKind {
        match self.feed {
            Feed::MoviePopular
            | Feed::MovieUpcoming
            | Feed::MovieNowPlaying
            | Feed::MovieTrending => MediaKind::Movie,
            _ => MediaKind::Series,
        }
    }

    fn validate(&self) -> Result<()> {
        let trending = matches!(self.feed, Feed::MovieTrending | Feed::TvTrending);
        let region_supported = matches!(
            self.feed,
            Feed::MoviePopular | Feed::MovieUpcoming | Feed::MovieNowPlaying
        );
        if self.max_pages == 0
            || self.max_pages > 500
            || self.page_size == 0
            || self.page_size > 100
            || self.locale.is_empty()
            || trending != self.trending_window.is_some()
            || (self.region.is_some() && !region_supported)
            || self
                .region
                .as_ref()
                .is_some_and(|r| r.len() != 2 || !r.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err(Error::InvalidScope);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UserProgress {
    pub reference: ItemRef,
    pub ordinal: usize,
    pub position_seconds: u64,
    pub runtime_seconds: u64,
    pub played: bool,
    pub updated_at: String,
}

#[derive(Deserialize)]
struct Fixture {
    items: Vec<Item>,
    credits: BTreeMap<String, Vec<Credit>>,
}

#[derive(Deserialize)]
struct Tmdb {
    #[serde(rename = "retrievedAt")]
    retrieved_at: String,
    items: BTreeMap<String, TmdbItem>,
}

#[derive(Deserialize)]
struct TmdbItem {
    id: u64,
    cast: Vec<Person>,
    crew: Vec<Person>,
}

#[derive(Deserialize)]
struct Person {
    id: u64,
}

#[derive(Debug, Clone)]
pub struct MockAdapters {
    root: PathBuf,
    items: Vec<Item>,
    artwork: Vec<Artwork>,
    notices: Vec<Notice>,
    faults: Faults,
    snapshot: String,
}

impl MockAdapters {
    pub fn load(root: impl AsRef<Path>, faults: Faults) -> Result<Self> {
        let root = root.as_ref().canonicalize().map_err(fixture_error)?;
        let fixture_bytes = std::fs::read(root.join("fixture.json")).map_err(fixture_error)?;
        let tmdb_bytes = std::fs::read(root.join("tmdb.json")).map_err(fixture_error)?;
        let artwork_bytes = std::fs::read(root.join("artwork.json")).map_err(fixture_error)?;
        let notice_bytes = std::fs::read(root.join("notices.json")).map_err(fixture_error)?;
        let mut fixture: Fixture = serde_json::from_slice(&fixture_bytes).map_err(fixture_error)?;
        let tmdb: Tmdb = serde_json::from_slice(&tmdb_bytes).map_err(fixture_error)?;
        let artwork: Vec<Artwork> =
            serde_json::from_slice(&artwork_bytes).map_err(fixture_error)?;
        let notices: Vec<Notice> = serde_json::from_slice(&notice_bytes).map_err(fixture_error)?;
        let ids: BTreeSet<_> = fixture.items.iter().map(|i| i.id.clone()).collect();
        if ids.len() != fixture.items.len() || !notices.iter().any(|n| n.id == "tmdb-attribution") {
            return Err(Error::Fixture(
                "duplicate items or missing attribution".into(),
            ));
        }
        let mut art_ids = BTreeSet::new();
        for art in &artwork {
            if !art_ids.insert(&art.id)
                || art.license.is_empty()
                || art.creator.is_empty()
                || !art.source.starts_with("https://")
                || !art.download.starts_with("https://")
            {
                return Err(Error::Fixture("invalid artwork provenance".into()));
            }
            validate_optional_art_path(&root, &art.file)?;
            if let Some(render) = &art.render_file {
                validate_optional_art_path(&root, render)?;
                if art.derivation_note.as_ref().is_none_or(String::is_empty) {
                    return Err(Error::Fixture("render without derivation".into()));
                }
            }
        }
        for item in &mut fixture.items {
            let raw = tmdb
                .items
                .get(&item.id)
                .ok_or_else(|| Error::Fixture("missing provider identity".into()))?;
            if raw.id == 0
                || item.metadata_retrieved_at != tmdb.retrieved_at
                || item.metadata_rights.source.is_empty()
                || item.title_rights.source.is_empty()
                || item.parent_id.as_ref().is_some_and(|id| !ids.contains(id))
            {
                return Err(Error::Fixture(
                    "invalid metadata provenance or hierarchy".into(),
                ));
            }
            item.provider_ids = vec![ProviderId {
                provider: "tmdb".into(),
                kind: item.kind,
                id: raw.id,
            }];
            item.credits = fixture.credits.remove(&item.id).unwrap_or_default();
            let trusted_people: BTreeSet<_> =
                raw.cast.iter().chain(&raw.crew).map(|p| p.id).collect();
            for (ordinal, credit) in item.credits.iter_mut().enumerate() {
                let id = credit
                    .id
                    .strip_prefix("tmdb:")
                    .and_then(|id| id.parse().ok());
                if id.is_none_or(|id| !trusted_people.contains(&id)) {
                    return Err(Error::Fixture("untrusted person identity".into()));
                }
                credit.ordinal = ordinal;
                credit.provider_person_id = id;
                credit.artwork_key = credit.artwork_input.take().map(|a| a.id);
                if credit
                    .artwork_key
                    .as_ref()
                    .is_some_and(|id| !art_ids.contains(id))
                {
                    return Err(Error::Fixture("missing person artwork provenance".into()));
                }
            }
            item.artwork_keys = artwork
                .iter()
                .filter(|a| a.id == item.id)
                .map(|a| a.id.clone())
                .collect();
        }
        validate_hierarchy(&fixture.items)?;
        let mut fingerprint = fixture_bytes;
        fingerprint.extend(tmdb_bytes);
        fingerprint.extend(artwork_bytes);
        fingerprint.extend(notice_bytes);
        fingerprint.extend(serde_json::to_vec(&faults).map_err(fixture_error)?);
        let snapshot = format!("{:016x}", hash(&fingerprint));
        if let Some(id) = &faults.removed_item {
            let mut removed = BTreeSet::from([id.clone()]);
            loop {
                let previous = removed.len();
                for item in &fixture.items {
                    if item.parent_id.as_ref().is_some_and(|p| removed.contains(p)) {
                        removed.insert(item.id.clone());
                    }
                }
                if removed.len() == previous {
                    break;
                }
            }
            fixture.items.retain(|i| !removed.contains(&i.id));
        }
        if let Some((id, title)) = &faults.updated_title
            && let Some(item) = fixture.items.iter_mut().find(|i| &i.id == id)
        {
            item.title.clone_from(title);
        }
        Ok(Self {
            root,
            items: fixture.items,
            artwork,
            notices,
            faults,
            snapshot,
        })
    }

    pub fn libraries(&self) -> Vec<Library> {
        [(MOVIES_LIBRARY, "Movies"), (TV_LIBRARY, "TV Shows")]
            .into_iter()
            .map(|(id, name)| Library {
                id: id.into(),
                name: name.into(),
                source: JELLYFIN_SOURCE.into(),
            })
            .collect()
    }

    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }
    pub fn artwork(&self) -> &[Artwork] {
        &self.artwork
    }

    /// Reads original bytes only on demand; native=true selects the approved PNG logo rendering.
    pub fn read_artwork(&self, key: &str, native: bool) -> Result<Vec<u8>> {
        if self.faults.missing_artwork.as_deref() == Some(key) {
            return Err(Error::MissingArtwork);
        }
        let art = self
            .artwork
            .iter()
            .find(|a| a.id == key)
            .ok_or(Error::MissingArtwork)?;
        let file = if native {
            art.render_file.as_deref().unwrap_or(&art.file)
        } else {
            &art.file
        };
        std::fs::read(safe_art_path(&self.root, file)?).map_err(artwork_io_error)
    }

    pub fn jellyfin_item(&self, external_id: &str) -> Option<CatalogEntry> {
        self.items
            .iter()
            .find(|i| i.id == external_id)
            .map(|i| self.entry(i, JELLYFIN_SOURCE, 0))
    }

    pub fn tmdb_lookup(&self, id: &ProviderId) -> Option<CatalogEntry> {
        self.items
            .iter()
            .find(|i| i.provider_ids.contains(id))
            .map(|i| self.entry(i, TMDB_SOURCE, 0))
    }

    /// Immediate canonical children only. Completion covers this parent scope, not TMDB.
    pub fn tmdb_children(
        &self,
        parent: &ProviderId,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<Page<CatalogEntry>> {
        if page_size == 0 || page_size > 100 {
            return Err(Error::InvalidScope);
        }
        let scope = format!(
            "children:{}:{page_size}",
            serde_json::to_string(parent).map_err(fixture_error)?
        );
        let canonical_parent = self.items.iter().find(|i| i.provider_ids.contains(parent));
        let child_kind = match parent.kind {
            MediaKind::Series => Some(MediaKind::Season),
            MediaKind::Season => Some(MediaKind::Episode),
            _ => None,
        };
        let mut children: Vec<_> = self
            .items
            .iter()
            .filter(|item| {
                canonical_parent.is_some_and(|p| item.parent_id.as_ref() == Some(&p.id))
                    && Some(item.kind) == child_kind
            })
            .collect();
        children.sort_by_key(|i| (i.added_order, &i.id));
        self.page(
            TMDB_SOURCE,
            &scope,
            page_size,
            cursor,
            children.len(),
            false,
            |index| self.entry(children[index], TMDB_SOURCE, index),
        )
    }

    pub fn jellyfin_page(
        &self,
        selected_libraries: &[String],
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<Page<CatalogEntry>> {
        let scope = self.library_scope(selected_libraries, page_size)?;
        let items: Vec<_> = self
            .items
            .iter()
            .filter(|i| selected_libraries.iter().any(|l| l == library_for(i)))
            .collect();
        self.page(
            JELLYFIN_SOURCE,
            &scope,
            page_size,
            cursor,
            items.len(),
            true,
            |index| self.entry(items[index], JELLYFIN_SOURCE, index),
        )
    }

    pub fn tmdb_page(
        &self,
        policy: &FeedPolicy,
        cursor: Option<&str>,
    ) -> Result<Page<CatalogEntry>> {
        policy.validate()?;
        let scope = serde_json::to_string(policy).map_err(fixture_error)?;
        let mut items: Vec<_> = self
            .items
            .iter()
            .filter(|i| i.kind == policy.media_kind())
            .collect();
        let rotation = match policy.feed {
            Feed::MoviePopular | Feed::TvPopular => 0,
            Feed::MovieUpcoming => 2,
            Feed::MovieNowPlaying | Feed::TvOnTheAir => 4,
            _ => {
                if policy.trending_window == Some(TrendingWindow::Day) {
                    1
                } else {
                    3
                }
            }
        };
        if !items.is_empty() {
            let n = items.len();
            items.rotate_left(rotation % n);
        }
        let total = items.len().min(policy.max_pages * policy.page_size);
        self.page(
            TMDB_SOURCE,
            &scope,
            policy.page_size,
            cursor,
            total,
            false,
            |index| self.entry(items[index], TMDB_SOURCE, index),
        )
    }

    /// The caller resolves its local UUID to this external user ID. Unknown IDs return no state.
    pub fn continue_watching(
        &self,
        external_user_id: &str,
        selected_libraries: &[String],
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<Page<UserProgress>> {
        let library_scope = self.library_scope(selected_libraries, page_size)?;
        let scope = format!("progress:{external_user_id}:{library_scope}");
        let states: &[(&str, u64, u64)] = match external_user_id {
            MOCK_USER_ALICE => &[
                ("big-buck-bunny", 240, 600),
                ("beverly-hillbillies-s1-e1", 450, 1500),
                ("sintel", 180, 888),
            ],
            MOCK_USER_BOB => &[("his-girl-friday", 2760, 5520), ("big-buck-bunny", 60, 600)],
            _ => &[],
        };
        let states: Vec<_> = states
            .iter()
            .filter(|(id, _, _)| {
                self.items
                    .iter()
                    .any(|i| i.id == *id && selected_libraries.iter().any(|l| l == library_for(i)))
            })
            .collect();
        self.page(
            JELLYFIN_SOURCE,
            &scope,
            page_size,
            cursor,
            states.len(),
            false,
            |index| {
                let (id, position, runtime) = states[index];
                UserProgress {
                    reference: ItemRef {
                        source: JELLYFIN_SOURCE.into(),
                        external_id: (*id).into(),
                    },
                    ordinal: index,
                    position_seconds: *position,
                    runtime_seconds: *runtime,
                    played: false,
                    updated_at: "2026-11-07T20:23:53.449Z".into(),
                }
            },
        )
    }

    fn library_scope(&self, libraries: &[String], page_size: usize) -> Result<String> {
        let unique: BTreeSet<_> = libraries.iter().map(String::as_str).collect();
        if unique.is_empty()
            || unique.len() != libraries.len()
            || page_size == 0
            || page_size > 100
            || unique
                .iter()
                .any(|id| ![MOVIES_LIBRARY, TV_LIBRARY].contains(id))
        {
            return Err(Error::InvalidScope);
        }
        Ok(format!(
            "{}:{page_size}",
            unique.into_iter().collect::<Vec<_>>().join(",")
        ))
    }

    fn entry(&self, item: &Item, source: &str, ordinal: usize) -> CatalogEntry {
        let external_id = if source == TMDB_SOURCE {
            let p = &item.provider_ids[0];
            format!("{}:{}", kind_key(p.kind), p.id)
        } else {
            item.id.clone()
        };
        CatalogEntry {
            reference: ItemRef {
                source: source.into(),
                external_id,
            },
            library_id: (source == JELLYFIN_SOURCE).then(|| library_for(item).into()),
            ordinal,
            item: item.clone(),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "pagination scope, bounds, coverage, and projection are coupled"
    )]
    fn page<T>(
        &self,
        source: &str,
        scope: &str,
        size: usize,
        cursor: Option<&str>,
        total: usize,
        library_complete: bool,
        get: impl Fn(usize) -> T,
    ) -> Result<Page<T>> {
        let token = |offset| {
            format!(
                "{:016x}-{:016x}",
                offset,
                hash(format!("{}:{source}:{scope}:{offset}", self.snapshot).as_bytes())
            )
        };
        let offset = if let Some(cursor) = cursor {
            if self.faults.expire_cursors {
                return Err(Error::CursorExpired);
            }
            let offset = cursor
                .split_once('-')
                .and_then(|(n, _)| usize::from_str_radix(n, 16).ok())
                .ok_or(Error::InvalidCursor)?;
            if offset >= total || offset % size != 0 || token(offset) != cursor {
                return Err(Error::InvalidCursor);
            }
            offset
        } else {
            0
        };
        let number = offset / size;
        if self.faults.rate_limit_page == Some(number) {
            return Err(Error::RateLimited {
                retry_after_seconds: 2,
            });
        }
        if self.faults.unavailable_page == Some(number) {
            return Err(Error::Unavailable);
        }
        let end = (offset + size).min(total);
        let duplicate = self.faults.duplicate_page == Some(number) && number > 0;
        let start = if duplicate { offset - size } else { offset };
        let items = (start..(start + (end - offset))).map(get).collect();
        let partial = self.faults.partial_page == Some(number);
        let looping = self.faults.loop_cursor_page == Some(number);
        let tainted = partial
            || looping
            || self
                .faults
                .duplicate_page
                .is_some_and(|p| p > 0 && p <= number)
            || self.faults.loop_cursor_page.is_some_and(|p| p <= number)
            || self.faults.partial_page.is_some_and(|p| p <= number);
        let coverage = if tainted {
            Coverage::Partial
        } else if end < total {
            Coverage::InProgress
        } else if library_complete {
            Coverage::SelectedLibrariesComplete
        } else {
            Coverage::BoundedWindowComplete
        };
        let next_cursor = if partial {
            None
        } else if looping {
            Some(token(offset))
        } else if end < total {
            Some(token(end))
        } else {
            None
        };
        Ok(Page {
            items,
            next_cursor,
            coverage,
            evidence: CoverageEvidence {
                source: source.into(),
                scope: scope.into(),
                snapshot: self.snapshot.clone(),
                traversed: if tainted { start } else { end },
                total_in_scope: total,
            },
        })
    }
}

fn kind_key(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Movie => "movie",
        MediaKind::Series => "tv",
        MediaKind::Season => "season",
        MediaKind::Episode => "episode",
    }
}
fn library_for(item: &Item) -> &'static str {
    if item.kind == MediaKind::Movie {
        MOVIES_LIBRARY
    } else {
        TV_LIBRARY
    }
}
fn fixture_error(error: impl std::fmt::Display) -> Error {
    Error::Fixture(error.to_string())
}
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn safe_art_path(root: &Path, file: &str) -> Result<PathBuf> {
    if Path::new(file)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || file.is_empty()
        || file.contains('\0')
    {
        return Err(Error::Fixture("unsafe artwork path".into()));
    }
    let base = resolve_art_path(root, Path::new("artwork"), true)?;
    resolve_art_path(&base, Path::new(file), false)
}
fn resolve_art_path(root: &Path, relative: &Path, directory: bool) -> Result<PathBuf> {
    let mut pending: VecDeque<_> = relative
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();
    let mut path = root.to_path_buf();
    let mut symlinks = 0;
    while let Some(component) = pending.pop_front() {
        if component == "." {
            continue;
        }
        if component == ".." {
            if path == root || !path.pop() {
                return Err(Error::Fixture("artwork escaped fixture root".into()));
            }
            continue;
        }
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path).map_err(artwork_io_error)?;
        if metadata.file_type().is_symlink() {
            symlinks += 1;
            if symlinks > 40 {
                return Err(Error::Fixture("artwork symlink loop".into()));
            }
            let target = std::fs::read_link(&path).map_err(artwork_io_error)?;
            let target = path.parent().unwrap().join(target);
            let relative = target
                .strip_prefix(root)
                .map_err(|_| Error::Fixture("artwork escaped fixture root".into()))?;
            let mut target_components: VecDeque<_> = relative
                .components()
                .map(|c| c.as_os_str().to_owned())
                .collect();
            target_components.append(&mut pending);
            pending = target_components;
            path = root.to_path_buf();
        } else if (pending.is_empty()
            && if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file()
            })
            || (!pending.is_empty() && !metadata.is_dir())
        {
            return Err(Error::Fixture("invalid artwork path type".into()));
        }
    }
    let metadata = std::fs::metadata(&path).map_err(artwork_io_error)?;
    if if directory {
        !metadata.is_dir()
    } else {
        !metadata.is_file()
    } {
        return Err(Error::Fixture("invalid artwork path type".into()));
    }
    Ok(path)
}
fn artwork_io_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::MissingArtwork
    } else {
        fixture_error(error)
    }
}
fn validate_optional_art_path(root: &Path, file: &str) -> Result<()> {
    match safe_art_path(root, file) {
        Ok(_) | Err(Error::MissingArtwork) => Ok(()),
        Err(error) => Err(error),
    }
}
fn validate_hierarchy(items: &[Item]) -> Result<()> {
    for item in items {
        let parent = item
            .parent_id
            .as_ref()
            .and_then(|id| items.iter().find(|i| &i.id == id));
        let valid = match item.kind {
            MediaKind::Movie | MediaKind::Series => parent.is_none(),
            MediaKind::Season => parent.is_some_and(|p| p.kind == MediaKind::Series),
            MediaKind::Episode => parent.is_some_and(|p| p.kind == MediaKind::Season),
        };
        if !valid {
            return Err(Error::Fixture("invalid hierarchy".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
