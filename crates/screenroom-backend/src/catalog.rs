use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
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
#[derive(Debug, Clone, FromRow)]
pub struct ItemContext {
    pub source_item_id: Uuid,
    pub parent_kind: Option<String>,
    pub parent_metadata: Option<Value>,
    pub grandparent_kind: Option<String>,
    pub grandparent_metadata: Option<Value>,
}
#[derive(Debug, Clone)]
pub struct CollectionMembershipWrite {
    pub container_external_id: String,
    pub member_external_id: String,
    pub ordinal: Option<i32>,
    pub observation_receipt_id: Uuid,
}
#[derive(Debug, Clone, FromRow)]
pub struct CollectionMember {
    #[sqlx(flatten)]
    pub item: CatalogItem,
    pub ordinal: Option<i32>,
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
    /// Same-source, scope-visible canonical ancestors; kind filters apply only to targets.
    pub async fn item_contexts(
        &self,
        scope: &Scope,
        source_item_ids: &[Uuid],
    ) -> Result<BTreeMap<Uuid, ItemContext>> {
        scope.validate()?;
        if source_item_ids.len() > 200 {
            return Err(Error::Invalid("item context target batch"));
        }
        if source_item_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let sql = format!(
            r#"{ELIGIBLE}
            SELECT target.id AS source_item_id, parent.kind AS parent_kind,
                parent.metadata AS parent_metadata, grandparent.kind AS grandparent_kind,
                grandparent.metadata AS grandparent_metadata
            FROM eligible target JOIN catalog.items i ON i.id=target.item_id
            LEFT JOIN catalog.items p ON p.id=i.parent_id
            LEFT JOIN LATERAL (SELECT e.* FROM eligible e WHERE e.item_id=p.id AND e.source_id=target.source_id ORDER BY e.id LIMIT 1) parent ON true
            LEFT JOIN LATERAL (SELECT e.* FROM eligible e WHERE e.item_id=p.parent_id AND e.source_id=target.source_id ORDER BY e.id LIMIT 1) grandparent ON true
            WHERE target.id=ANY($5) AND (cardinality($6::text[])=0 OR target.kind=ANY($6))"#
        );
        let rows: Vec<ItemContext> = sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(Vec::<String>::new())
            .bind(source_item_ids)
            .bind(&scope.kinds)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.source_item_id, row))
            .collect())
    }
    /// Worker-only source-qualified membership upserts; parent edges remain unchanged.
    pub async fn upsert_collection_members(
        &self,
        claim: &crate::ingest::Claim,
        members: &[CollectionMembershipWrite],
    ) -> Result<()> {
        if members.len() > 2000 {
            return Err(Error::Invalid("collection membership bounds"));
        }
        let mut tx = self.pool.begin().await?;
        let (run, _) = crate::ingest::guard(&mut tx, claim).await?;
        if claim.kind != "ingest" || run.state != "running" {
            return Err(Error::Stale);
        }
        for member in members {
            write_collection_member(&mut tx, &run, member).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Public-ID keyset order; membership eligibility precedes source precedence.
    pub async fn collection_members(
        &self,
        scope: &Scope,
        container: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<CollectionMember>> {
        scope.validate()?;
        if !(1..=200).contains(&limit)
            || container.is_empty()
            || container.len() > 200
            || after.is_some_and(|id| id.len() > 200)
        {
            return Err(Error::Invalid("collection member page"));
        }
        let query = format!(
            "{ELIGIBLE} , members AS (SELECT e.*, cm.ordinal, row_number() OVER(PARTITION BY e.item_id ORDER BY e.source_key,e.source_id,e.id) AS member_precedence FROM eligible e JOIN catalog.collection_members cm ON cm.member_id=e.id AND cm.source_id=e.source_id JOIN eligible c ON c.id=cm.container_id AND c.source_id=cm.source_id WHERE c.public_id=$5 AND c.kind='collection' AND (cardinality($8::text[])=0 OR e.kind=ANY($8))) SELECT {ITEM_COLUMNS},ordinal FROM members WHERE member_precedence=1 AND ($6::text IS NULL OR public_id>$6) ORDER BY public_id LIMIT $7"
        );
        Ok(sqlx::query_as(&query)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(Vec::<String>::new())
            .bind(container)
            .bind(after)
            .bind(i64::from(limit))
            .bind(&scope.kinds)
            .fetch_all(&self.pool)
            .await?)
    }
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

pub(crate) async fn write_collection_member(
    tx: &mut Transaction<'_, Postgres>,
    run: &crate::ingest::Run,
    member: &CollectionMembershipWrite,
) -> Result<()> {
    if [&member.container_external_id, &member.member_external_id]
        .iter()
        .any(|id| id.is_empty() || id.len() > 200)
        || member.ordinal.is_some_and(|ordinal| ordinal < 0)
    {
        return Err(Error::Invalid("collection membership"));
    }
    let selected = crate::ingest::selected_library_ids(&run.selection_snapshot)?;
    let n = sqlx::query("INSERT INTO catalog.collection_members(source_id,container_id,member_id,ordinal,observed_run_id,observation_receipt_id) SELECT c.source_id,c.id,m.id,$4,$5,$7 FROM catalog.source_items c JOIN catalog.items ci ON ci.id=c.item_id JOIN catalog.source_items m ON m.source_id=c.source_id WHERE c.source_id=$1 AND c.external_id=$2 AND m.external_id=$3 AND c.state='enabled' AND m.state='enabled' AND ci.kind='collection' AND ci.parent_id IS NULL AND c.id<>m.id AND EXISTS(SELECT FROM catalog.library_items li JOIN catalog.libraries l ON l.id=li.library_id WHERE li.source_item_id=c.id AND li.state='enabled' AND l.state='enabled' AND l.public_id=ANY($6)) AND EXISTS(SELECT FROM catalog.library_items li JOIN catalog.libraries l ON l.id=li.library_id WHERE li.source_item_id=m.id AND li.state='enabled' AND l.state='enabled' AND l.public_id=ANY($6)) AND EXISTS(SELECT FROM ingest.request_receipts r WHERE r.id=$7 AND r.run_id=$5 AND r.operation='collection' AND r.outcome='success' AND r.is_request) AND NOT EXISTS(SELECT FROM ingest.request_receipts prior WHERE prior.run_id=$5 AND prior.operation='collection' AND NOT prior.is_request AND prior.operation_key='membership:'||c.id::text||':'||m.id::text) ON CONFLICT(container_id,member_id) DO UPDATE SET ordinal=EXCLUDED.ordinal,observed_run_id=EXCLUDED.observed_run_id,observation_receipt_id=EXCLUDED.observation_receipt_id")
        .bind(run.source_id).bind(&member.container_external_id).bind(&member.member_external_id)
        .bind(member.ordinal).bind(run.id).bind(selected).bind(member.observation_receipt_id).execute(&mut **tx).await?.rows_affected();
    if n != 1 {
        let repeated:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.request_receipts r JOIN catalog.source_items c ON c.source_id=$2 AND c.external_id=$3 JOIN catalog.source_items m ON m.source_id=c.source_id AND m.external_id=$4 WHERE r.run_id=$1 AND r.operation='collection' AND NOT r.is_request AND r.operation_key='membership:'||c.id::text||':'||m.id::text)")
            .bind(run.id).bind(run.source_id).bind(&member.container_external_id).bind(&member.member_external_id).fetch_one(&mut **tx).await?;
        if repeated {
            return Err(Error::Invalid("repeated collection member"));
        }
        return Err(Error::Invalid("collection endpoints outside run scope"));
    }
    let observed = sqlx::query("INSERT INTO ingest.request_receipts(run_id,job_id,fence,operation,outcome,is_request,operation_key) SELECT j.scope_id,j.id,j.fence,'collection','success',false,'membership:'||c.id::text||':'||m.id::text FROM queue.jobs j JOIN catalog.source_items c ON c.source_id=$2 AND c.external_id=$3 JOIN catalog.source_items m ON m.source_id=c.source_id AND m.external_id=$4 WHERE j.scope_id=$1 AND j.kind='ingest' AND j.state='leased' ON CONFLICT(run_id,operation,operation_key) WHERE operation_key IS NOT NULL DO NOTHING")
        .bind(run.id).bind(run.source_id).bind(&member.container_external_id).bind(&member.member_external_id).execute(&mut **tx).await?.rows_affected();
    if observed != 1 {
        return Err(Error::Invalid("repeated collection member"));
    }
    Ok(())
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
    if item.kind != "collection" {
        let hash = crate::embedding::input_hash(&item.document);
        sqlx::query("INSERT INTO search.documents(source_item_id,document_version,content_hash,text,title) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_item_id) DO UPDATE SET document_version=EXCLUDED.document_version,content_hash=EXCLUDED.content_hash,text=EXCLUDED.text,title=EXCLUDED.title,updated_at=now()")
            .bind(id).bind(item.document_version).bind(hash.as_slice()).bind(&item.document).bind(&item.document_title).execute(&mut **tx).await?;
    }
    Ok(id)
}
