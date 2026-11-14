use crate::{
    Error, Result,
    catalog::{ELIGIBLE, Scope},
    embedding::{DOCUMENT_VERSION, validate_vector},
    ingest::{Claim, finish, guard},
};
use chrono::{DateTime, Utc};
use pgvector::Vector;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use std::collections::BTreeMap;
use uuid::Uuid;
const CURRENT_VECTOR: &str = "m.state='active' AND m.dimensions=1024 AND m.document_version=d.document_version AND v.input_hash=d.content_hash";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RankedHit {
    pub public_id: String,
    pub source_item_id: Uuid,
    pub score: f64,
}
#[derive(Debug, Clone, Default)]
pub struct LexicalMethods {
    pub full_text: bool,
    pub title: bool,
    pub fuzzy: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Model {
    pub id: Uuid,
    pub name: String,
    pub revision: String,
    pub dimensions: i32,
    pub distance_metric: String,
    pub document_version: i32,
    pub state: String,
}
#[derive(Debug, Clone, FromRow)]
pub struct Document {
    pub id: Uuid,
    pub source_item_id: Uuid,
    pub document_version: i32,
    pub content_hash: Vec<u8>,
    pub text: String,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct EmbeddingCoverage {
    pub eligible_documents: i64,
    pub current_embeddings: i64,
    pub complete: bool,
}
#[derive(Debug, Clone)]
pub struct VectorCoverage {
    pub candidates: Vec<RankedHit>,
    pub coverage: EmbeddingCoverage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotWrite {
    pub digest: Vec<u8>,
    pub model_revision: Option<String>,
    pub index_revision: String,
    pub results: Vec<RankedHit>,
    pub effective_methods: Vec<String>,
    pub degradation_reason: String,
    pub ttl_seconds: i32,
}
#[derive(Debug, Clone)]
pub struct SnapshotPage {
    pub id: Uuid,
    pub results: Vec<RankedHit>,
    pub effective_methods: Vec<String>,
    pub degradation_reason: String,
    pub expires_at: DateTime<Utc>,
    pub next_offset: Option<u32>,
}
#[derive(Clone)]
pub struct SearchRepository {
    pool: PgPool,
}
impl SearchRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn lexical(
        &self,
        scope: &Scope,
        query: &str,
        methods: &LexicalMethods,
        limit: u32,
    ) -> Result<Vec<RankedHit>> {
        self.lexical_query(scope, query, methods, limit, false)
            .await
    }
    /// Restricts every lexical engine to title text, without semantic retrieval.
    pub async fn lexical_titles(
        &self,
        scope: &Scope,
        query: &str,
        methods: &LexicalMethods,
        limit: u32,
    ) -> Result<Vec<RankedHit>> {
        self.lexical_query(scope, query, methods, limit, true).await
    }
    async fn lexical_query(
        &self,
        scope: &Scope,
        query: &str,
        methods: &LexicalMethods,
        limit: u32,
        titles_only: bool,
    ) -> Result<Vec<RankedHit>> {
        validate_query(scope, query, limit)?;
        if !methods.full_text && !methods.title && !methods.fuzzy {
            return Ok(vec![]);
        }
        let lexical = if titles_only {
            "to_tsvector('english',d.title)"
        } else {
            "d.lexical"
        };
        let sql=format!("{ELIGIBLE} SELECT e.public_id,e.id AS source_item_id,
         (CASE WHEN $6 THEN ts_rank_cd({lexical},websearch_to_tsquery('english',$5)) ELSE 0 END +
        CASE WHEN $7 AND lower(d.title)=lower($5) THEN 2 ELSE 0 END +
        CASE WHEN $8 THEN similarity(d.title,$5) ELSE 0 END)::float8 AS score
        FROM eligible e JOIN search.documents d ON d.source_item_id=e.id WHERE
         (($6 AND {lexical} @@ websearch_to_tsquery('english',$5)) OR ($7 AND lower(d.title)=lower($5)) OR ($8 AND similarity(d.title,$5)>=0.3)) ORDER BY score DESC,e.public_id,e.id LIMIT $9");
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(query)
            .bind(methods.full_text)
            .bind(methods.title)
            .bind(methods.fuzzy)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
    /// Exact cosine distance over the complete eligible set. No ANN index or lexical shortlist.
    pub async fn vector(
        &self,
        scope: &Scope,
        query: &[f32],
        model: Uuid,
        limit: u32,
    ) -> Result<Vec<RankedHit>> {
        scope.validate()?;
        validate_vector(query)?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql=format!("{ELIGIBLE} SELECT e.public_id,e.id AS source_item_id,(1-(v.vector <=> $5::vector))::float8 AS score
        FROM eligible e JOIN search.documents d ON d.source_item_id=e.id JOIN search.embeddings v ON v.document_id=d.id
         JOIN search.embedding_models m ON m.id=v.model_id WHERE m.id=$6 AND {CURRENT_VECTOR}
        ORDER BY v.vector <=> $5::vector,e.public_id,e.id LIMIT $7");
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(Vector::from(query.to_vec()))
            .bind(model)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn active_model(&self) -> Result<Option<Model>> {
        Ok(
            sqlx::query_as("SELECT * FROM search.embedding_models WHERE state='active'")
                .fetch_optional(&self.pool)
                .await?,
        )
    }
    /// Candidates and coverage share one PostgreSQL statement snapshot and eligible-document set.
    /// Unknown models fail; inactive models return no candidates and incomplete coverage.
    pub async fn vector_with_coverage(
        &self,
        scope: &Scope,
        query: &[f32],
        model: Uuid,
        limit: u32,
    ) -> Result<VectorCoverage> {
        scope.validate()?;
        validate_vector(query)?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql = format!("{ELIGIBLE}, docs AS MATERIALIZED (
            SELECT e.public_id,e.id AS source_item_id,d.id,d.document_version,d.content_hash FROM eligible e
            JOIN search.documents d ON d.source_item_id=e.id
        ), current_vectors AS MATERIALIZED (
            SELECT d.public_id,d.source_item_id,v.vector FROM docs d
            JOIN search.embeddings v ON v.document_id=d.id
            JOIN search.embedding_models m ON m.id=v.model_id
            WHERE m.id=$6 AND {CURRENT_VECTOR}
        ) SELECT m.state,(SELECT count(*) FROM docs),(SELECT count(*) FROM current_vectors),
            (SELECT coalesce(jsonb_agg(to_jsonb(h) ORDER BY h.score DESC,h.public_id,h.source_item_id),'[]'::jsonb)
             FROM (SELECT public_id,source_item_id,(1-(vector <=> $5::vector))::float8 AS score
                   FROM current_vectors ORDER BY vector <=> $5::vector,public_id,source_item_id LIMIT $7) h)
        FROM search.embedding_models m WHERE m.id=$6");
        let (state, eligible, current, hits): (String, i64, i64, Value) = sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(Vector::from(query.to_vec()))
            .bind(model)
            .bind(i64::from(limit))
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        Ok(VectorCoverage {
            candidates: serde_json::from_value(hits)
                .map_err(|_| Error::Incompatible("vector results"))?,
            coverage: EmbeddingCoverage {
                eligible_documents: eligible,
                current_embeddings: current,
                complete: state == "active" && eligible == current,
            },
        })
    }
    /// Query-time coverage over all eligible source documents, before canonical deduplication.
    /// Only active-model vectors with the current document version and hash count as current.
    pub async fn embedding_coverage(
        &self,
        scope: &Scope,
        model: Uuid,
    ) -> Result<EmbeddingCoverage> {
        scope.validate()?;
        let sql = format!("{ELIGIBLE} SELECT coverage.*,
            (m.state='active' AND coverage.eligible_documents=coverage.current_embeddings) AS complete
            FROM search.embedding_models m CROSS JOIN LATERAL (
                SELECT count(*) AS eligible_documents,count(v.document_id) AS current_embeddings
                FROM eligible e JOIN search.documents d ON d.source_item_id=e.id
                LEFT JOIN search.embeddings v ON v.document_id=d.id AND v.model_id=m.id
                     AND {CURRENT_VECTOR}
            ) coverage WHERE m.id=$5");
        sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(model)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)
    }
    pub async fn register_model(
        &self,
        name: &str,
        revision: &str,
        dimensions: i32,
    ) -> Result<Model> {
        if name.is_empty()
            || revision.is_empty()
            || dimensions != 1024
            || name.len() > 200
            || revision.len() > 200
        {
            return Err(Error::Invalid("model"));
        }
        sqlx::query_as("INSERT INTO search.embedding_models(name,revision,dimensions,document_version) VALUES($1,$2,$3,$4) ON CONFLICT(name,revision) DO UPDATE SET name=EXCLUDED.name WHERE search.embedding_models.dimensions=EXCLUDED.dimensions AND search.embedding_models.document_version=EXCLUDED.document_version RETURNING *")
            .bind(name).bind(revision).bind(dimensions).bind(DOCUMENT_VERSION).fetch_optional(&self.pool).await?.ok_or(Error::Invalid("model revision configuration mismatch"))
    }
    /// Explicit rebuild switch: activation fails until every enabled document has a current vector.
    pub async fn activate_model(&self, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(78590211)")
            .execute(&mut *tx)
            .await?;
        let model: Model =
            sqlx::query_as("SELECT * FROM search.embedding_models WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?;
        // Prevent document/model edits between coverage validation and activation.
        sqlx::query("LOCK TABLE search.documents,search.embeddings IN SHARE MODE")
            .execute(&mut *tx)
            .await?;
        if model.state == "retired" {
            return Err(Error::Invalid("register a new revision for a rebuild"));
        }
        let missing:i64=sqlx::query_scalar("SELECT count(*) FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN catalog.sources s ON s.id=si.source_id WHERE si.state='enabled' AND s.state='enabled' AND (d.document_version<>$2 OR NOT EXISTS(SELECT FROM search.embeddings e WHERE e.document_id=d.id AND e.model_id=$1 AND e.input_hash=d.content_hash))")
            .bind(id).bind(model.document_version).fetch_one(&mut *tx).await?;
        if missing > 0 {
            return Err(Error::Invalid("model rebuild incomplete"));
        }
        sqlx::query(
            "UPDATE search.embedding_models SET state='retired' WHERE state='active' AND id<>$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE search.embedding_models SET state='active' WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn documents_needing_embedding(
        &self,
        model: Uuid,
        limit: u32,
    ) -> Result<Vec<Document>> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        Ok(sqlx::query_as("SELECT d.id,d.source_item_id,d.document_version,d.content_hash,d.text,d.title FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN catalog.sources s ON s.id=si.source_id JOIN search.embedding_models m ON m.id=$1 WHERE si.state='enabled' AND s.state='enabled' AND m.state IN ('building','active') AND d.document_version=m.document_version AND NOT EXISTS(SELECT FROM search.embeddings e WHERE e.model_id=$1 AND e.document_id=d.id AND e.input_hash=d.content_hash) ORDER BY d.id LIMIT $2")
            .bind(model).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn document_for_source_item(&self, id: Uuid) -> Result<Option<Document>> {
        Ok(sqlx::query_as("SELECT id,source_item_id,document_version,content_hash,text,title FROM search.documents WHERE source_item_id=$1").bind(id).fetch_optional(&self.pool).await?)
    }
    /// Internal rebuild lookup: library visibility does not constrain enabled source documents.
    pub async fn document_for_reference(
        &self,
        source_key: &str,
        external_id: &str,
    ) -> Result<Option<Document>> {
        if [source_key, external_id]
            .iter()
            .any(|id| id.is_empty() || id.len() > 200)
        {
            return Err(Error::Invalid("document reference"));
        }
        Ok(sqlx::query_as("SELECT d.id,d.source_item_id,d.document_version,d.content_hash,d.text,d.title FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN catalog.sources s ON s.id=si.source_id WHERE s.source_key=$1 AND si.external_id=$2 AND s.state='enabled' AND si.state='enabled'")
            .bind(source_key).bind(external_id).fetch_optional(&self.pool).await?)
    }
    /// Vector network work happens first; the guarded commit rejects stale documents and revisions.
    pub async fn commit_embedding(
        &self,
        claim: &Claim,
        document: Uuid,
        model: Uuid,
        expected_revision: &str,
        expected_hash: &[u8],
        vector: &[f32],
    ) -> Result<()> {
        self.commit_embedding_with_outcomes(
            claim,
            document,
            model,
            expected_revision,
            expected_hash,
            vector,
            &[],
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_embedding_with_outcomes(
        &self,
        claim: &Claim,
        document: Uuid,
        model: Uuid,
        expected_revision: &str,
        expected_hash: &[u8],
        vector: &[f32],
        outcomes: &[crate::ingest::RequestOutcomeWrite],
    ) -> Result<()> {
        validate_vector(vector)?;
        if expected_hash.len() != 32 || expected_revision.is_empty() || claim.kind != "embedding" {
            return Err(Error::Invalid("embedding commit"));
        }
        let hash = claim.input["input_hash"]
            .as_str()
            .ok_or(Error::Invalid("embedding input hash"))?;
        let expected: String = expected_hash.iter().map(|b| format!("{b:02x}")).collect();
        if hash != expected {
            return Err(Error::Stale);
        }
        let mut tx = self.pool.begin().await?;
        let (_, policy) = guard(&mut tx, claim).await?;
        sqlx::query("SELECT d.id FROM search.documents d WHERE d.id=$1 FOR SHARE")
            .bind(document)
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT id FROM search.embedding_models WHERE id=$1 FOR SHARE")
            .bind(model)
            .execute(&mut *tx)
            .await?;
        let n=sqlx::query("INSERT INTO search.embeddings(document_id,model_id,input_hash,vector) SELECT d.id,m.id,d.content_hash,$4 FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN search.embedding_models m ON m.id=$2 WHERE d.id=$1 AND d.content_hash=$3 AND si.source_id=$5 AND si.state='enabled' AND m.revision=$6 AND m.document_version=d.document_version AND m.state IN ('active','building') ON CONFLICT(document_id,model_id) DO UPDATE SET input_hash=EXCLUDED.input_hash,vector=EXCLUDED.vector,created_at=now()")
            .bind(document).bind(model).bind(expected_hash).bind(Vector::from(vector.to_vec())).bind(policy.source_id).bind(expected_revision).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Stale);
        }
        crate::ingest::write_outcomes(&mut tx, claim, outcomes).await?;
        finish(&mut tx, claim).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn save_snapshot(&self, s: &SnapshotWrite) -> Result<Uuid> {
        if s.digest.len() != 32
            || s.results.len() > 1000
            || !(1..=900).contains(&s.ttl_seconds)
            || s.results.iter().any(|h| !h.score.is_finite())
            || s.effective_methods.len() > 5
            || s.degradation_reason.len() > 200
        {
            return Err(Error::Invalid("snapshot"));
        }
        let results = serde_json::to_value(&s.results).map_err(|_| Error::Invalid("snapshot"))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(78590212)")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM search.result_snapshots WHERE expires_at<=now()")
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM search.result_snapshots")
            .fetch_one(&mut *tx)
            .await?;
        if count >= 10000 {
            return Err(Error::Unavailable);
        }
        let id=sqlx::query_scalar("INSERT INTO search.result_snapshots(digest,model_revision,index_revision,results,effective_methods,degradation_reason,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+make_interval(secs=>$7)) RETURNING id")
            .bind(&s.digest).bind(&s.model_revision).bind(&s.index_revision).bind(results).bind(serde_json::to_value(&s.effective_methods).map_err(|_|Error::Invalid("methods"))?).bind(&s.degradation_reason).bind(s.ttl_seconds).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn snapshot_page(
        &self,
        id: Uuid,
        digest: &[u8],
        scope: &Scope,
        offset: u32,
        limit: u32,
    ) -> Result<SnapshotPage> {
        scope.validate()?;
        if digest.len() != 32 || !(1..=200).contains(&limit) || offset > 1000 {
            return Err(Error::Invalid("snapshot page"));
        }
        let row:Option<(Value,Value,String,DateTime<Utc>)>=sqlx::query_as("SELECT results,effective_methods,degradation_reason,expires_at FROM search.result_snapshots WHERE id=$1 AND digest=$2 AND expires_at>now()")
            .bind(id).bind(digest).fetch_optional(&self.pool).await?;
        let (results, methods, reason, expires) = row.ok_or(Error::NotFound)?;
        let results: Vec<RankedHit> =
            serde_json::from_value(results).map_err(|_| Error::Incompatible("snapshot results"))?;
        let methods: Vec<String> =
            serde_json::from_value(methods).map_err(|_| Error::Incompatible("snapshot methods"))?;
        let end = (offset as usize + limit as usize).min(results.len());
        let start = (offset as usize).min(end);
        let page = &results[start..end];
        let ids: Vec<Uuid> = page.iter().map(|h| h.source_item_id).collect();
        let sql = format!("{ELIGIBLE} SELECT id FROM eligible WHERE id=ANY($5)");
        let allowed: Vec<Uuid> = sqlx::query_scalar(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(ids)
            .fetch_all(&self.pool)
            .await?;
        Ok(SnapshotPage {
            id,
            results: page
                .iter()
                .filter(|h| allowed.contains(&h.source_item_id))
                .cloned()
                .collect(),
            effective_methods: methods,
            degradation_reason: reason,
            expires_at: expires,
            next_offset: (end < results.len()).then_some(end as u32),
        })
    }
}

fn validate_query(scope: &Scope, query: &str, limit: u32) -> Result<()> {
    scope.validate()?;
    if query.trim().is_empty() || query.chars().count() > 8000 || !(1..=1000).contains(&limit) {
        return Err(Error::Invalid("search query"));
    }
    Ok(())
}
/// Fixed k=60 RRF; input order is rank, not raw cross-method scores.
/// Fuse source documents first, then retain each canonical identity's highest scoring
/// document (ties: public ID, source-item UUID). Multiple sources never add a popularity boost.
pub fn reciprocal_rank_fusion(rankings: &[Vec<RankedHit>], limit: usize) -> Vec<RankedHit> {
    let mut results: BTreeMap<Uuid, RankedHit> = BTreeMap::new();
    for ranking in rankings {
        let mut seen = std::collections::BTreeSet::new();
        for (rank, hit) in ranking.iter().take(1000).enumerate() {
            if !seen.insert(hit.source_item_id) {
                continue;
            }
            let entry = results
                .entry(hit.source_item_id)
                .or_insert_with(|| RankedHit {
                    score: 0.0,
                    ..hit.clone()
                });
            entry.score += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut results: Vec<_> = results.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.public_id.cmp(&b.public_id))
            .then(a.source_item_id.cmp(&b.source_item_id))
    });
    let mut seen = std::collections::BTreeSet::new();
    results.retain(|hit| seen.insert(hit.public_id.clone()));
    results.truncate(limit.min(1000));
    results
}

pub fn snapshot_digest(query: &str, scope: &Scope, methods: &[String]) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    scope.validate()?;
    let bytes = serde_json::to_vec(&(crate::embedding::preprocess(query), scope, methods))
        .map_err(|_| Error::Invalid("snapshot digest"))?;
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_rrf_ties_and_duplicates() {
        let a = RankedHit {
            public_id: "a".into(),
            source_item_id: Uuid::nil(),
            score: 99.0,
        };
        let b = RankedHit {
            public_id: "b".into(),
            source_item_id: Uuid::from_u128(1),
            ..a.clone()
        };
        let result =
            reciprocal_rank_fusion(&[vec![a.clone(), b.clone(), a.clone()], vec![b, a]], 10);
        assert_eq!(result[0].public_id, "a");
        assert_eq!(result[0].score, result[1].score);
    }
    #[test]
    fn canonical_score_is_best_document_not_sum_of_sources() {
        let a = RankedHit {
            public_id: "a".into(),
            source_item_id: Uuid::from_u128(1),
            score: 0.0,
        };
        let alternate = RankedHit {
            source_item_id: Uuid::from_u128(2),
            ..a.clone()
        };
        let b = RankedHit {
            public_id: "b".into(),
            source_item_id: Uuid::from_u128(3),
            score: 0.0,
        };
        let result = reciprocal_rank_fusion(
            &[
                vec![a.clone(), alternate.clone(), b.clone()],
                vec![alternate, b, a],
            ],
            10,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].public_id, "a");
        assert_eq!(result[0].source_item_id, Uuid::from_u128(2));
        assert_eq!(result[0].score, 1.0 / 62.0 + 1.0 / 61.0);
    }
}
