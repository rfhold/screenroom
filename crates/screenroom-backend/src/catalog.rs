use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    pub source_keys: Vec<String>,
    pub library_ids: Vec<String>,
    pub providers: Vec<String>,
    pub kinds: Vec<String>,
}
impl Scope {
    pub fn validate(&self) -> Result<()> {
        for values in [
            &self.source_keys,
            &self.library_ids,
            &self.providers,
            &self.kinds,
        ] {
            if values.len() > 100 || values.iter().any(|s| s.is_empty() || s.len() > 200) {
                return Err(Error::Invalid("scope"));
            }
        }
        if self.library_ids.iter().any(|s| s == "all") {
            return Err(Error::Invalid("expand all into movies/tv or empty scope"));
        }
        Ok(())
    }
}

// All filters precede source precedence. Empty scope selects every enabled source.
pub(crate) const ELIGIBLE: &str = "WITH eligible AS (
 SELECT si.*, i.public_id, i.kind, parent.public_id AS parent_public_id, s.source_key, s.provider,
 row_number() OVER (PARTITION BY i.id ORDER BY CASE WHEN s.provider='jellyfin' THEN 0 ELSE 1 END,s.source_key,si.id) AS precedence
 FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id
 LEFT JOIN catalog.items parent ON parent.id=i.parent_id JOIN catalog.sources s ON s.id=si.source_id
 WHERE si.state='enabled' AND s.state='enabled'
 AND (s.provider<>'jellyfin' OR EXISTS (SELECT 1 FROM catalog.library_items live JOIN catalog.libraries lib ON lib.id=live.library_id WHERE live.source_item_id=si.id AND live.state='enabled' AND lib.state='enabled'))
 AND (cardinality($1::text[])=0 OR s.source_key=ANY($1))
 AND (cardinality($2::text[])=0 OR EXISTS (SELECT 1 FROM catalog.library_items li JOIN catalog.libraries l ON l.id=li.library_id
 WHERE li.source_item_id=si.id AND li.state='enabled' AND l.state='enabled' AND l.public_id=ANY($2)))
 AND (cardinality($3::text[])=0 OR s.provider=ANY($3))
 AND (cardinality($4::text[])=0 OR i.kind=ANY($4))
) ";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CatalogItem {
    pub source_item_id: Uuid,
    pub item_id: Uuid,
    pub public_id: String,
    pub parent_public_id: Option<String>,
    pub kind: String,
    pub source_id: Uuid,
    pub source_key: String,
    pub provider: String,
    pub external_id: String,
    pub metadata: Value,
    pub provenance: Value,
}
const ITEM_COLUMNS: &str = "id AS source_item_id,item_id,public_id,parent_public_id,kind,source_id,source_key,provider,external_id,metadata,provenance";

#[derive(Debug, Clone, FromRow)]
pub struct Source {
    pub id: Uuid,
    pub source_key: String,
    pub provider: String,
    pub state: String,
    pub configuration: Value,
    pub capabilities: Value,
}
#[derive(Debug, Clone, FromRow)]
pub struct Library {
    pub id: Uuid,
    pub source_id: Uuid,
    pub public_id: String,
    pub external_id: String,
    pub title: String,
    pub state: String,
}
#[derive(Debug, Clone)]
pub struct SourceInitialization {
    pub source_key: String,
    pub provider: String,
    pub configuration: Value,
    pub capabilities: Value,
    pub libraries: Vec<crate::ingest::LibraryWrite>,
    pub initial_policy: Option<crate::ingest::PolicyWrite>,
}
#[derive(Debug, Clone)]
pub struct InitializedSource {
    pub source: Source,
    /// Requested libraries, including their existing operator-controlled state.
    pub libraries: Vec<Library>,
    /// Existing policies (at most 1000), or the newly installed initial policy.
    pub policies: Vec<crate::ingest::Policy>,
}
#[derive(Debug, Clone, FromRow)]
pub struct Credit {
    pub person_id: Uuid,
    pub external_id: String,
    pub name: String,
    pub metadata: Value,
    pub department: String,
    pub role: String,
    pub ordinal: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditCursor {
    pub ordinal: i32,
    pub person_id: Uuid,
    pub department: String,
    pub role: String,
}
impl Credit {
    pub fn cursor(&self) -> CreditCursor {
        CreditCursor {
            ordinal: self.ordinal,
            person_id: self.person_id,
            department: self.department.clone(),
            role: self.role.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CatalogRepository {
    pool: PgPool,
}
impl CatalogRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    /// Newest valid provider timestamp across eligible source items, before canonical precedence.
    pub async fn metadata_snapshot(
        &self,
        scope: &Scope,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        scope.validate()?;
        let sql = format!("{ELIGIBLE}, snapshots AS (
            SELECT CASE WHEN jsonb_typeof(metadata->'metadata_retrieved_at')='string'
                AND metadata->>'metadata_retrieved_at' ~ '^[0-9]{{4}}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:([0-5][0-9]|60)([.][0-9]+)?([Zz]|[+-]([01][0-9]|2[0-3]):[0-5][0-9])$'
                AND pg_input_is_valid(metadata->>'metadata_retrieved_at','timestamp with time zone')
                THEN (metadata->>'metadata_retrieved_at')::timestamptz END AS snapshot
            FROM eligible
        ) SELECT max(snapshot) FILTER (WHERE isfinite(snapshot)) FROM snapshots");
        Ok(sqlx::query_scalar(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .fetch_one(&self.pool)
            .await?)
    }
    /// Explicit initialization only: existing source/library configuration and policies are preserved.
    /// Missing entries are installed atomically; output is ordered by source key and library ID.
    pub async fn initialize(
        &self,
        inputs: &[SourceInitialization],
    ) -> Result<Vec<InitializedSource>> {
        if inputs.len() > 32 {
            return Err(Error::Invalid("initialization source count"));
        }
        let mut ordered: Vec<_> = inputs.iter().collect();
        ordered.sort_by(|a, b| a.source_key.cmp(&b.source_key));
        let mut seen = std::collections::BTreeSet::new();
        for input in &ordered {
            if input.source_key.is_empty()
                || input.source_key.len() > 200
                || !seen.insert(&input.source_key)
                || !matches!(input.provider.as_str(), "jellyfin" | "tmdb")
                || !input.configuration.is_object()
                || input.configuration.to_string().len() > 65536
                || !input.capabilities.is_object()
                || input.capabilities.to_string().len() > 32768
                || input.libraries.len() > 100
            {
                return Err(Error::Invalid("source initialization"));
            }
            let mut public = std::collections::BTreeSet::new();
            let mut external = std::collections::BTreeSet::new();
            for library in &input.libraries {
                if library.public_id.is_empty()
                    || library.public_id == "all"
                    || library.public_id.len() > 200
                    || library.external_id.is_empty()
                    || library.external_id.len() > 200
                    || library.title.len() > 1000
                    || !public.insert(&library.public_id)
                    || !external.insert(&library.external_id)
                {
                    return Err(Error::Invalid("initialization library"));
                }
            }
            if let Some(policy) = &input.initial_policy {
                policy.validate()?;
            }
        }
        let mut tx = self.pool.begin().await?;
        let mut output = Vec::with_capacity(inputs.len());
        for input in ordered {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,78590213))")
                .bind(&input.source_key)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO catalog.sources(source_key,provider,configuration,capabilities) VALUES($1,$2,$3,$4) ON CONFLICT(source_key) DO NOTHING")
                .bind(&input.source_key).bind(&input.provider).bind(&input.configuration).bind(&input.capabilities).execute(&mut *tx).await?;
            let source: Source = sqlx::query_as(
                "SELECT * FROM catalog.sources WHERE source_key=$1 FOR NO KEY UPDATE",
            )
            .bind(&input.source_key)
            .fetch_one(&mut *tx)
            .await?;
            if source.provider != input.provider {
                return Err(Error::Invalid("initialization provider mismatch"));
            }
            for library in &input.libraries {
                sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$3,$4) ON CONFLICT(source_id,external_id) DO NOTHING")
                    .bind(source.id).bind(&library.public_id).bind(&library.external_id).bind(&library.title).execute(&mut *tx).await?;
            }
            let mut policies: Vec<crate::ingest::Policy> = sqlx::query_as(
                "SELECT * FROM ingest.policies WHERE source_id=$1 ORDER BY id LIMIT 1000",
            )
            .bind(source.id)
            .fetch_all(&mut *tx)
            .await?;
            if policies.is_empty()
                && let Some(policy) = &input.initial_policy
            {
                policies.push(crate::ingest::insert_policy(&mut tx, source.id, policy).await?);
            }
            let libraries = sqlx::query_as(
                "SELECT * FROM catalog.libraries WHERE source_id=$1 AND external_id=ANY($2) ORDER BY public_id,external_id",
            )
            .bind(source.id)
            .bind(input.libraries.iter().map(|l| &l.external_id).collect::<Vec<_>>())
            .fetch_all(&mut *tx)
            .await?;
            output.push(InitializedSource {
                source,
                libraries,
                policies,
            });
        }
        tx.commit().await?;
        Ok(output)
    }

    pub async fn configure_source(
        &self,
        key: &str,
        provider: &str,
        configuration: &Value,
        capabilities: &Value,
    ) -> Result<Source> {
        if key.is_empty() || key.len() > 200 || !matches!(provider, "jellyfin" | "tmdb") {
            return Err(Error::Invalid("source"));
        }
        Ok(sqlx::query_as("INSERT INTO catalog.sources(source_key,provider,configuration,capabilities) VALUES($1,$2,$3,$4) ON CONFLICT(source_key) DO UPDATE SET configuration=EXCLUDED.configuration,capabilities=EXCLUDED.capabilities RETURNING *")
            .bind(key).bind(provider).bind(configuration).bind(capabilities).fetch_one(&self.pool).await?)
    }
    pub async fn set_source_state(&self, source: Uuid, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE catalog.sources SET state=$2 WHERE id=$1")
            .bind(source)
            .bind(if enabled { "enabled" } else { "disabled" })
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn sources(&self) -> Result<Vec<Source>> {
        Ok(
            sqlx::query_as("SELECT * FROM catalog.sources ORDER BY source_key")
                .fetch_all(&self.pool)
                .await?,
        )
    }
    pub async fn configure_library(
        &self,
        source: Uuid,
        public_id: &str,
        external_id: &str,
        title: &str,
    ) -> Result<Library> {
        if public_id.is_empty()
            || public_id == "all"
            || public_id.len() > 200
            || external_id.is_empty()
        {
            return Err(Error::Invalid("library"));
        }
        Ok(sqlx::query_as("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$3,$4) ON CONFLICT(source_id,external_id) DO UPDATE SET title=EXCLUDED.title,public_id=EXCLUDED.public_id RETURNING *")
            .bind(source).bind(public_id).bind(external_id).bind(title).fetch_one(&self.pool).await?)
    }
    pub async fn libraries(&self, scope: &Scope) -> Result<Vec<Library>> {
        scope.validate()?;
        Ok(sqlx::query_as("SELECT l.* FROM catalog.libraries l JOIN catalog.sources s ON s.id=l.source_id WHERE s.state='enabled' AND l.state='enabled' AND (cardinality($1::text[])=0 OR s.source_key=ANY($1)) AND (cardinality($2::text[])=0 OR l.public_id=ANY($2)) AND (cardinality($3::text[])=0 OR s.provider=ANY($3)) ORDER BY l.public_id,s.source_key")
            .bind(&scope.source_keys).bind(&scope.library_ids).bind(&scope.providers).fetch_all(&self.pool).await?)
    }
    pub async fn list(
        &self,
        scope: &Scope,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<CatalogItem>> {
        self.list_query(scope, after, limit, false).await
    }
    pub async fn list_top_level(
        &self,
        scope: &Scope,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<CatalogItem>> {
        self.list_query(scope, after, limit, true).await
    }
    async fn list_query(
        &self,
        scope: &Scope,
        after: Option<&str>,
        limit: u32,
        roots_only: bool,
    ) -> Result<Vec<CatalogItem>> {
        scope.validate()?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT {ITEM_COLUMNS} FROM eligible WHERE precedence=1 AND (NOT $7::bool OR parent_public_id IS NULL) AND ($5::text IS NULL OR public_id>$5) ORDER BY public_id LIMIT $6"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(after)
            .bind(i64::from(limit))
            .bind(roots_only)
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn get(&self, scope: &Scope, public_id: &str) -> Result<Option<CatalogItem>> {
        scope.validate()?;
        let sql = format!(
            "{ELIGIBLE} SELECT {ITEM_COLUMNS} FROM eligible WHERE precedence=1 AND public_id=$5"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn resolve_reference(
        &self,
        scope: &Scope,
        source_key: &str,
        external_id: &str,
    ) -> Result<Option<CatalogItem>> {
        scope.validate()?;
        let sql = format!(
            "{ELIGIBLE} SELECT {ITEM_COLUMNS} FROM eligible WHERE source_key=$5 AND external_id=$6"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(source_key)
            .bind(external_id)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn children(
        &self,
        scope: &Scope,
        parent: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<CatalogItem>> {
        scope.validate()?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT {ITEM_COLUMNS} FROM eligible WHERE precedence=1 AND parent_public_id=$5 AND ($6::text IS NULL OR public_id>$6) ORDER BY public_id LIMIT $7"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(parent)
            .bind(after)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn credits(
        &self,
        scope: &Scope,
        public_id: &str,
        after: Option<&CreditCursor>,
        limit: u32,
    ) -> Result<Vec<Credit>> {
        scope.validate()?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT p.id AS person_id,p.external_id,p.name,p.metadata,c.department,c.role,c.ordinal FROM eligible e JOIN catalog.credits c ON c.source_item_id=e.id JOIN catalog.people p ON p.id=c.person_id WHERE e.precedence=1 AND e.public_id=$5 AND ($6::integer IS NULL OR (c.ordinal,p.id,c.department,c.role)>($6,$7,$8,$9)) ORDER BY c.ordinal,p.id,c.department,c.role LIMIT $10"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(public_id)
            .bind(after.map(|c| c.ordinal))
            .bind(after.map(|c| c.person_id))
            .bind(after.map(|c| c.department.as_str()))
            .bind(after.map(|c| c.role.as_str()))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdWrite {
    pub provider: String,
    pub entity_kind: String,
    pub external_id: String,
    pub evidence: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditWrite {
    pub external_id: String,
    pub name: String,
    pub metadata: Value,
    pub department: String,
    pub role: String,
    pub ordinal: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemWrite {
    pub public_id: String,
    pub kind: String,
    pub parent_public_id: Option<String>,
    pub external_id: String,
    pub source_revision: Option<String>,
    pub metadata: Value,
    pub provenance: Value,
    pub content_hash: Vec<u8>,
    pub library_external_ids: Vec<String>,
    pub external_ids: Vec<ExternalIdWrite>,
    pub credits: Vec<CreditWrite>,
    pub document: String,
    pub document_title: String,
    pub document_version: i32,
}

pub(crate) async fn write_item(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    run: Uuid,
    item: &ItemWrite,
) -> Result<Uuid> {
    if item.public_id.is_empty()
        || item.public_id.len() > 200
        || item.external_id.is_empty()
        || item.content_hash.len() != 32
        || item.document.len() > 32000
        || item.credits.len() > 1000
        || item.metadata.to_string().len() > 262144
        || item.provenance.to_string().len() > 65536
        || !item.metadata.is_object()
        || item.metadata.get("progress_percent").is_some()
    {
        return Err(Error::Invalid("item"));
    }
    let parent: Option<Uuid> = if let Some(parent) = &item.parent_public_id {
        Some(
            sqlx::query_scalar("SELECT id FROM catalog.items WHERE public_id=$1")
                .bind(parent)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(Error::Invalid("parent must be ingested first"))?,
        )
    } else {
        None
    };
    let canonical: Uuid=sqlx::query_scalar("INSERT INTO catalog.items(public_id,kind,parent_id) VALUES($1,$2,$3) ON CONFLICT(public_id) DO UPDATE SET updated_at=now() WHERE catalog.items.kind=EXCLUDED.kind AND catalog.items.parent_id IS NOT DISTINCT FROM EXCLUDED.parent_id RETURNING id")
        .bind(&item.public_id).bind(&item.kind).bind(parent).fetch_optional(&mut **tx).await?.ok_or(Error::Invalid("canonical identity conflict"))?;
    let id: Uuid=sqlx::query_scalar("INSERT INTO catalog.source_items(source_id,item_id,external_id,source_revision,metadata,provenance,content_hash) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(source_id,external_id) DO UPDATE SET source_revision=EXCLUDED.source_revision,metadata=EXCLUDED.metadata,provenance=EXCLUDED.provenance,content_hash=EXCLUDED.content_hash,state='enabled',updated_at=now() WHERE catalog.source_items.item_id=EXCLUDED.item_id RETURNING id")
        .bind(source).bind(canonical).bind(&item.external_id).bind(&item.source_revision).bind(&item.metadata).bind(&item.provenance).bind(&item.content_hash).fetch_optional(&mut **tx).await?.ok_or(Error::Invalid("source identity conflict"))?;
    for library in &item.library_external_ids {
        let affected=sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) SELECT id,$2,$3,$4 FROM catalog.libraries WHERE source_id=$3 AND external_id=$1 AND state='enabled' ON CONFLICT(library_id,source_item_id) DO UPDATE SET last_seen_run_id=EXCLUDED.last_seen_run_id,state='enabled'")
            .bind(library).bind(id).bind(source).bind(run).execute(&mut **tx).await?.rows_affected();
        if affected != 1 {
            return Err(Error::Invalid("unknown or disabled library"));
        }
    }
    sqlx::query("DELETE FROM catalog.external_ids WHERE source_item_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for external in &item.external_ids {
        sqlx::query(
            "INSERT INTO catalog.external_ids VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(&external.provider)
        .bind(&external.entity_kind)
        .bind(&external.external_id)
        .bind(&external.evidence)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query("DELETE FROM catalog.credits WHERE source_item_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for credit in &item.credits {
        let person: Uuid=sqlx::query_scalar("INSERT INTO catalog.people(source_id,external_id,name,metadata) VALUES($1,$2,$3,$4) ON CONFLICT(source_id,external_id) DO UPDATE SET name=EXCLUDED.name,metadata=EXCLUDED.metadata RETURNING id").bind(source).bind(&credit.external_id).bind(&credit.name).bind(&credit.metadata).fetch_one(&mut **tx).await?;
        sqlx::query("INSERT INTO catalog.credits VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(person)
            .bind(source)
            .bind(&credit.department)
            .bind(&credit.role)
            .bind(credit.ordinal)
            .execute(&mut **tx)
            .await?;
    }
    let hash = crate::embedding::input_hash(&item.document);
    sqlx::query("INSERT INTO search.documents(source_item_id,document_version,content_hash,text,title) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_item_id) DO UPDATE SET document_version=EXCLUDED.document_version,content_hash=EXCLUDED.content_hash,text=EXCLUDED.text,title=EXCLUDED.title,updated_at=now()")
        .bind(id).bind(item.document_version).bind(hash.as_slice()).bind(&item.document).bind(&item.document_title).execute(&mut **tx).await?;
    Ok(id)
}
