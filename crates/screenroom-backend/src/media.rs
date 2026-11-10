use crate::{
    Error, Result,
    catalog::{ELIGIBLE, Scope},
    ingest::{Claim, Followup, JobKind, enqueue, finish, guard},
    storage::{ObjectLocation, VerifiedObject},
};
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::{collections::BTreeMap, io::Cursor};
use uuid::Uuid;

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
#[derive(Clone)]
pub struct ValidatedImage {
    bytes: Vec<u8>,
    mime: String,
    width: u32,
    height: u32,
}
impl ValidatedImage {
    pub fn raster(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(Error::Invalid("image byte limit"));
        }
        let format = image::guess_format(&bytes).map_err(|_| Error::Invalid("image format"))?;
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            _ => return Err(Error::Invalid("PNG or JPEG required")),
        };
        let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
            .into_dimensions()
            .map_err(|_| Error::Invalid("image dimensions"))?;
        validate_dimensions(width, height)?;
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let _ = reader
            .decode()
            .map_err(|_| Error::Invalid("image decode"))?;
        Ok(Self {
            bytes,
            mime: mime.into(),
            width,
            height,
        })
    }
    /// Retain attribution originals without rasterizing. Reject active/external SVG content.
    pub fn svg_original(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(Error::Invalid("SVG byte limit"));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| Error::Invalid("SVG UTF-8"))?;
        let doc = roxmltree::Document::parse(text).map_err(|_| Error::Invalid("SVG XML"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "svg" {
            return Err(Error::Invalid("SVG root"));
        }
        for node in doc.descendants().filter(|n| n.is_element()) {
            if !matches!(
                node.tag_name().name(),
                "svg"
                    | "defs"
                    | "style"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "title"
                    | "desc"
                    | "g"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "polygon"
                    | "polyline"
                    | "line"
            ) {
                return Err(Error::Invalid("SVG active element"));
            }
            for attribute in node.attributes() {
                let name = attribute.name().to_ascii_lowercase();
                let value = attribute.value().to_ascii_lowercase();
                if name.starts_with("on")
                    || name == "href"
                    || value.contains("javascript:")
                    || value.contains("data:")
                    || value.contains('@')
                    || value.contains('\\')
                    || value.contains("http:")
                    || value.contains("https:")
                    || value.split("url(").skip(1).any(|s| !s.starts_with('#'))
                {
                    return Err(Error::Invalid("SVG active attribute"));
                }
            }
            if node.tag_name().name() == "style" {
                let text = node.text().unwrap_or("").to_ascii_lowercase();
                if text.contains('@')
                    || text.contains('\\')
                    || text.contains("http")
                    || text.contains("data:")
                    || text.split("url(").skip(1).any(|s| !s.starts_with('#'))
                {
                    return Err(Error::Invalid("SVG external style"));
                }
            }
        }
        let values: Vec<f64> = root
            .attribute("viewBox")
            .ok_or(Error::Invalid("SVG viewBox"))?
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<f64>()
                    .map_err(|_| Error::Invalid("SVG dimensions"))
            })
            .collect::<Result<_>>()?;
        if values.len() != 4
            || values.iter().any(|v| !v.is_finite())
            || values[2] <= 0.0
            || values[3] <= 0.0
        {
            return Err(Error::Invalid("SVG dimensions"));
        }
        let width = values[2].ceil() as u32;
        let height = values[3].ceil() as u32;
        validate_dimensions(width, height)?;
        Ok(Self {
            bytes,
            mime: "image/svg+xml".into(),
            width,
            height,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn mime(&self) -> &str {
        &self.mime
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
}
fn validate_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(Error::Invalid("image pixel limit"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkReferenceInput {
    pub source_artwork_id: String,
    pub kind: String,
    pub rights: Value,
    pub item_external_id: Option<String>,
    pub person_external_id: Option<String>,
    pub role: String,
    pub ordinal: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkVariant {
    Original,
    Native,
}
impl ArtworkVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Native => "native",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtworkVariantInput {
    pub variant: ArtworkVariant,
    pub sha256: [u8; 32],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkInputManifest {
    pub version: u32,
    /// Every listed variant is required before this generation is publicly deliverable.
    pub variants: Vec<ArtworkVariantInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkInputWrite {
    pub reference: ArtworkReferenceInput,
    pub manifest: ArtworkInputManifest,
}
impl ArtworkInputWrite {
    pub fn fingerprint(&self) -> Result<[u8; 32]> {
        let r = &self.reference;
        if r.source_artwork_id.is_empty()
            || r.source_artwork_id.len() > 200
            || r.kind.is_empty()
            || r.kind.len() > 200
            || !r.rights.is_object()
            || r.rights.to_string().len() > 16384
            || r.ordinal < 0
            || self.manifest.version != 1
            || self.manifest.variants.is_empty()
            || self.manifest.variants.len() > 2
            || !self
                .manifest
                .variants
                .iter()
                .any(|v| v.variant == ArtworkVariant::Native)
        {
            return Err(Error::Invalid("artwork input"));
        }
        let mut variants = self.manifest.variants.clone();
        variants.sort_by_key(|v| v.variant);
        if variants.windows(2).any(|v| v[0].variant == v[1].variant) {
            return Err(Error::Invalid("duplicate artwork variant"));
        }
        let mut value =
            serde_json::json!({"version":1,"kind":r.kind,"rights":r.rights,"variants":variants});
        canonicalize(&mut value);
        Ok(Sha256::digest(
            serde_json::to_vec(&value).map_err(|_| Error::Invalid("artwork manifest"))?,
        )
        .into())
    }
}
fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                canonicalize(value);
            }
            map.sort_keys();
        }
        Value::Array(array) => array.iter_mut().for_each(canonicalize),
        _ => {}
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkJob {
    pub source_key: String,
    pub source_artwork_id: String,
    pub artwork_id: Uuid,
    pub revision: i64,
    pub fingerprint: [u8; 32],
    pub variant: ArtworkVariant,
    pub expected_sha256: [u8; 32],
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtworkOmissionCode {
    Missing,
    Invalid,
    Unavailable,
}
impl ArtworkOmissionCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkOmission {
    pub source_artwork_id: String,
    pub code: ArtworkOmissionCode,
}
#[derive(Debug, Clone, FromRow)]
pub struct ArtworkRecord {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_artwork_id: String,
    pub kind: String,
    pub rights: Value,
    pub revision: i64,
}
#[derive(FromRow)]
struct TargetedArtwork {
    target_id: Uuid,
    #[sqlx(flatten)]
    artwork: ArtworkRecord,
}
#[derive(Debug, Clone)]
pub struct ArtworkObject {
    pub artwork_id: Uuid,
    pub revision: i64,
    pub variant: String,
    pub location: ObjectLocation,
    pub width: i32,
    pub height: i32,
}
#[derive(FromRow)]
struct ObjectRow {
    bucket: String,
    object_key: String,
    sha256: Vec<u8>,
    mime: String,
    byte_length: i64,
    width: i32,
    height: i32,
}
#[derive(Clone)]
pub struct MediaRepository {
    pool: PgPool,
}
impl MediaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn publish(&self, claim: &Claim, object: &VerifiedObject) -> Result<Uuid> {
        if claim.kind != "artwork" {
            return Err(Error::Invalid("artwork publication"));
        }
        let mut tx = self.pool.begin().await?;
        let (_, policy) = guard(&mut tx, claim).await?;
        let input: Value = sqlx::query_scalar("SELECT input FROM ingest.jobs WHERE id=$1")
            .bind(claim.id)
            .fetch_one(&mut *tx)
            .await?;
        let job: ArtworkJob =
            serde_json::from_value(input).map_err(|_| Error::Invalid("artwork job"))?;
        if object.source != policy.source_id {
            return Err(Error::Invalid("object source"));
        }
        if object.location.sha256.as_slice() != job.expected_sha256
            || (job.variant == ArtworkVariant::Native
                && !matches!(object.location.mime.as_str(), "image/png" | "image/jpeg"))
        {
            return Err(Error::Invalid("artwork expected bytes"));
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT current_generation FROM media.artwork WHERE id=$1 AND source_id=$2 AND source_artwork_id=$3 FOR UPDATE")
            .bind(job.artwork_id).bind(policy.source_id).bind(&job.source_artwork_id).fetch_optional(&mut *tx).await?;
        if current != Some(job.revision) {
            return Err(Error::Stale);
        }
        let generation: Option<(Vec<u8>, Value, String)> = sqlx::query_as("SELECT fingerprint,manifest,state FROM media.artwork_generations WHERE artwork_id=$1 AND revision=$2 AND source_id=$3")
            .bind(job.artwork_id).bind(job.revision).bind(policy.source_id).fetch_optional(&mut *tx).await?;
        let (fingerprint, manifest, state) = generation.ok_or(Error::Stale)?;
        let manifest: ArtworkInputManifest =
            serde_json::from_value(manifest).map_err(|_| Error::Invalid("artwork manifest"))?;
        if fingerprint.as_slice() != job.fingerprint
            || state == "superseded"
            || manifest.version != 1
            || !manifest
                .variants
                .iter()
                .any(|v| v.variant == job.variant && v.sha256 == job.expected_sha256)
        {
            return Err(Error::Stale);
        }
        let art = job.artwork_id;
        let id:Uuid=sqlx::query_scalar("INSERT INTO media.objects(storage_scope,sha256,bucket,object_key,mime,byte_length,width,height) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(storage_scope,sha256) DO UPDATE SET state='ready' WHERE media.objects.object_key=EXCLUDED.object_key AND media.objects.byte_length=EXCLUDED.byte_length AND media.objects.mime=EXCLUDED.mime RETURNING id")
            .bind(object.source).bind(&object.location.sha256).bind(&object.location.bucket).bind(&object.location.object_key).bind(&object.location.mime).bind(object.location.byte_length).bind(object.width as i32).bind(object.height as i32).fetch_optional(&mut *tx).await?.ok_or(Error::Invalid("object metadata mismatch"))?;
        let n=sqlx::query("INSERT INTO media.variants(artwork_id,source_id,revision,variant,object_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(artwork_id,revision,variant) DO UPDATE SET object_id=EXCLUDED.object_id WHERE media.variants.object_id=EXCLUDED.object_id")
            .bind(art).bind(object.source).bind(job.revision).bind(job.variant.as_str()).bind(id).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Invalid("artwork revision bytes are immutable"));
        }
        for required in &manifest.variants {
            let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM media.variants v JOIN media.objects o ON o.id=v.object_id WHERE v.artwork_id=$1 AND v.revision=$2 AND v.variant=$3 AND o.sha256=$4 AND o.state='ready')")
                .bind(art).bind(job.revision).bind(required.variant.as_str()).bind(required.sha256.as_slice()).fetch_one(&mut *tx).await?;
            if !ready {
                finish(&mut tx, claim).await?;
                tx.commit().await?;
                return Ok(art);
            }
        }
        let ready: bool = sqlx::query_scalar(&format!("SELECT {FULL_GENERATION_READY} FROM media.artwork a JOIN media.artwork_generations g ON g.artwork_id=a.id WHERE a.id=$1 AND g.revision=$2"))
            .bind(art).bind(job.revision).fetch_one(&mut *tx).await?;
        if !ready {
            return Err(Error::Invalid(
                "artwork generation requires ready native variant",
            ));
        }
        sqlx::query("UPDATE media.artwork_generations SET state='published' WHERE artwork_id=$1 AND revision=$2")
            .bind(art).bind(job.revision).execute(&mut *tx).await?;
        sqlx::query("UPDATE media.artwork a SET revision=g.revision,kind=g.kind,rights=g.rights,state='enabled' FROM media.artwork_generations g WHERE a.id=$1 AND g.artwork_id=a.id AND g.revision=$2 AND a.current_generation=g.revision")
            .bind(art).bind(job.revision).execute(&mut *tx).await?;
        finish(&mut tx, claim).await?;
        tx.commit().await?;
        Ok(art)
    }
    pub async fn resolve(
        &self,
        scope: &Scope,
        artwork: Uuid,
        revision: i64,
        variant: &str,
    ) -> Result<Option<ArtworkObject>> {
        scope.validate()?;
        if !matches!(variant, "original" | "native") || revision < 1 {
            return Err(Error::Invalid("artwork variant"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT o.bucket,o.object_key,o.sha256,o.mime,o.byte_length,o.width,o.height FROM media.artwork a JOIN catalog.sources s ON s.id=a.source_id JOIN media.variants v ON v.artwork_id=a.id JOIN media.artwork_generations g ON g.artwork_id=v.artwork_id AND g.revision=v.revision AND g.state='published' JOIN media.objects o ON o.id=v.object_id WHERE a.id=$5 AND v.revision=$6 AND v.variant=$7 AND a.state='enabled' AND s.state='enabled' AND o.state='ready' AND (cardinality($1::text[])=0 OR s.source_key=ANY($1)) AND (cardinality($3::text[])=0 OR s.provider=ANY($3)) AND {FULL_GENERATION_READY} AND {ARTWORK_AVAILABLE}"
        );
        let row: Option<ObjectRow> = sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(artwork)
            .bind(revision)
            .bind(variant)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| ArtworkObject {
            artwork_id: artwork,
            revision,
            variant: variant.into(),
            location: ObjectLocation {
                bucket: row.bucket,
                object_key: row.object_key,
                sha256: row.sha256,
                mime: row.mime,
                byte_length: row.byte_length,
            },
            width: row.width,
            height: row.height,
        }))
    }
    pub async fn list(
        &self,
        scope: &Scope,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<ArtworkRecord>> {
        scope.validate()?;
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT a.id,a.source_id,a.source_artwork_id,a.kind,a.rights,a.revision FROM media.artwork a JOIN catalog.sources s ON s.id=a.source_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE a.state='enabled' AND s.state='enabled' AND ($5::uuid IS NULL OR a.id>$5) AND (cardinality($1::text[])=0 OR s.source_key=ANY($1)) AND (cardinality($3::text[])=0 OR s.provider=ANY($3)) AND {FULL_GENERATION_READY} AND {ARTWORK_AVAILABLE} ORDER BY a.id LIMIT $6"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(after)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn primary_item_artwork(
        &self,
        scope: &Scope,
        source_item_ids: &[Uuid],
    ) -> Result<BTreeMap<Uuid, ArtworkRecord>> {
        scope.validate()?;
        if source_item_ids.len() > 200 {
            return Err(Error::Invalid("artwork target batch"));
        }
        if source_item_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let sql = format!(
            "{ELIGIBLE} SELECT DISTINCT ON (ia.source_item_id) ia.source_item_id AS target_id,a.id,a.source_id,a.source_artwork_id,a.kind,a.rights,a.revision FROM eligible e JOIN media.item_artwork ia ON ia.source_item_id=e.id JOIN media.artwork a ON a.id=ia.artwork_id JOIN catalog.sources s ON s.id=a.source_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=ANY($5) AND a.state='enabled' AND s.state='enabled' AND {FULL_GENERATION_READY} ORDER BY ia.source_item_id,CASE ia.role WHEN 'primary' THEN 0 WHEN 'poster' THEN 1 WHEN 'thumbnail' THEN 2 ELSE 3 END,ia.ordinal,ia.role,a.source_artwork_id,a.id"
        );
        let rows: Vec<TargetedArtwork> = sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(source_item_ids)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.target_id, row.artwork))
            .collect())
    }
    pub async fn person_artwork(
        &self,
        scope: &Scope,
        person_ids: &[Uuid],
    ) -> Result<BTreeMap<Uuid, ArtworkRecord>> {
        scope.validate()?;
        if person_ids.len() > 200 {
            return Err(Error::Invalid("artwork target batch"));
        }
        if person_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let sql = format!(
            "{ELIGIBLE} SELECT DISTINCT ON (pa.person_id) pa.person_id AS target_id,a.id,a.source_id,a.source_artwork_id,a.kind,a.rights,a.revision FROM media.person_artwork pa JOIN media.artwork a ON a.id=pa.artwork_id JOIN catalog.sources s ON s.id=a.source_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE pa.person_id=ANY($5) AND a.state='enabled' AND s.state='enabled' AND EXISTS(SELECT FROM catalog.credits c JOIN eligible e ON e.id=c.source_item_id WHERE c.person_id=pa.person_id) AND {FULL_GENERATION_READY} ORDER BY pa.person_id,CASE pa.role WHEN 'primary' THEN 0 WHEN 'portrait' THEN 1 ELSE 2 END,pa.role,a.source_artwork_id,a.id"
        );
        let rows: Vec<TargetedArtwork> = sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(person_ids)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.target_id, row.artwork))
            .collect())
    }
    pub async fn branding(&self, scope: &Scope, limit: u32) -> Result<Vec<ArtworkRecord>> {
        scope.validate()?;
        if !(1..=8).contains(&limit) {
            return Err(Error::Invalid("branding limit"));
        }
        let sql = format!(
            "{ELIGIBLE} SELECT a.id,a.source_id,a.source_artwork_id,a.kind,a.rights,a.revision FROM catalog.sources s CROSS JOIN LATERAL (SELECT entry.value #>> '{{}}' AS source_artwork_id,min(entry.ordinality) AS ordinal FROM jsonb_array_elements(CASE WHEN jsonb_typeof(s.configuration->'branding_artwork_ids')='array' THEN s.configuration->'branding_artwork_ids' ELSE '[]'::jsonb END) WITH ORDINALITY entry(value,ordinality) WHERE jsonb_typeof(entry.value)='string' GROUP BY entry.value) configured JOIN media.artwork a ON a.source_id=s.id AND a.source_artwork_id=configured.source_artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE s.state='enabled' AND a.state='enabled' AND (cardinality($1::text[])=0 OR s.source_key=ANY($1)) AND (cardinality($3::text[])=0 OR s.provider=ANY($3)) AND {FULL_GENERATION_READY} AND {ARTWORK_AVAILABLE} ORDER BY s.source_key,configured.ordinal,a.source_artwork_id,a.id LIMIT $5"
        );
        Ok(sqlx::query_as(&sql)
            .bind(&scope.source_keys)
            .bind(&scope.library_ids)
            .bind(&scope.providers)
            .bind(&scope.kinds)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }
}

// Unattached source branding is distinct from artwork with item/person associations.
const ARTWORK_AVAILABLE: &str = "((NOT EXISTS(SELECT FROM media.item_artwork ia WHERE ia.artwork_id=a.id) AND NOT EXISTS(SELECT FROM media.person_artwork pa WHERE pa.artwork_id=a.id)) OR EXISTS(SELECT FROM media.item_artwork ia JOIN eligible e ON e.id=ia.source_item_id WHERE ia.artwork_id=a.id) OR EXISTS(SELECT FROM media.person_artwork pa JOIN catalog.credits c ON c.person_id=pa.person_id JOIN eligible e ON e.id=c.source_item_id WHERE pa.artwork_id=a.id))";
const FULL_GENERATION_READY: &str = r#"
EXISTS(
 SELECT FROM media.variants nv JOIN media.objects no ON no.id=nv.object_id
 WHERE nv.artwork_id=a.id AND nv.revision=g.revision AND nv.variant='native'
 AND no.state='ready' AND no.mime IN ('image/png','image/jpeg')
) AND CASE g.manifest->>'version'
 WHEN '0' THEN EXISTS(
  SELECT FROM media.variants ov JOIN media.objects oo ON oo.id=ov.object_id
  WHERE ov.artwork_id=a.id AND ov.revision=g.revision AND ov.variant='original' AND oo.state='ready'
 )
 WHEN '1' THEN EXISTS(
  SELECT FROM jsonb_array_elements(CASE WHEN jsonb_typeof(g.manifest->'variants')='array'
   THEN g.manifest->'variants' ELSE '[]'::jsonb END) required WHERE required->>'variant'='native'
 ) AND NOT EXISTS(
  SELECT FROM jsonb_array_elements(CASE WHEN jsonb_typeof(g.manifest->'variants')='array'
   THEN g.manifest->'variants' ELSE '[]'::jsonb END) required
  WHERE NOT EXISTS(
   SELECT FROM media.variants rv JOIN media.objects ro ON ro.id=rv.object_id
   WHERE rv.artwork_id=a.id AND rv.revision=g.revision AND rv.variant=required->>'variant'
   AND ro.state='ready'
   AND to_jsonb(ARRAY(SELECT get_byte(ro.sha256,n) FROM generate_series(0,31) n))=required->'sha256'
  )
 )
 ELSE false END
"#;

pub(crate) async fn stage_input(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    run: Uuid,
    input: &ArtworkInputWrite,
) -> Result<Uuid> {
    let fingerprint = input.fingerprint()?;
    let r = &input.reference;
    let (art, current): (Uuid, i64) = sqlx::query_as("INSERT INTO media.artwork(source_id,source_artwork_id,kind,rights,revision,state) VALUES($1,$2,$3,'{}',NULL,'absent') ON CONFLICT(source_id,source_artwork_id) DO UPDATE SET source_artwork_id=EXCLUDED.source_artwork_id RETURNING id,current_generation")
        .bind(source).bind(&r.source_artwork_id).bind(&r.kind).fetch_one(&mut **tx).await?;
    let previous: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT fingerprint FROM media.artwork_generations WHERE artwork_id=$1 AND revision=$2",
    )
    .bind(art)
    .bind(current)
    .fetch_optional(&mut **tx)
    .await?;
    let revision = if previous.as_deref() == Some(fingerprint.as_slice()) {
        current
    } else {
        let revision = current
            .checked_add(1)
            .ok_or(Error::Invalid("artwork revision exhausted"))?;
        sqlx::query("UPDATE media.artwork_generations SET state='superseded' WHERE artwork_id=$1 AND state='pending'")
            .bind(art).execute(&mut **tx).await?;
        sqlx::query("INSERT INTO media.artwork_generations(artwork_id,source_id,revision,fingerprint,manifest,kind,rights) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(art).bind(source).bind(revision).bind(fingerprint.as_slice())
            .bind(serde_json::to_value(&input.manifest).map_err(|_| Error::Invalid("artwork manifest"))?)
            .bind(&r.kind).bind(&r.rights).execute(&mut **tx).await?;
        sqlx::query("UPDATE media.artwork SET current_generation=$2 WHERE id=$1")
            .bind(art)
            .bind(revision)
            .execute(&mut **tx)
            .await?;
        revision
    };
    if let Some(item) = &r.item_external_id {
        let n=sqlx::query("INSERT INTO media.item_artwork(source_item_id,artwork_id,source_id,role,ordinal) SELECT id,$2,$3,$4,$5 FROM catalog.source_items WHERE source_id=$3 AND external_id=$1 ON CONFLICT(source_item_id,artwork_id,role) DO UPDATE SET ordinal=EXCLUDED.ordinal")
            .bind(item).bind(art).bind(source).bind(&r.role).bind(r.ordinal).execute(&mut **tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Invalid("artwork item"));
        }
    }
    if let Some(person) = &r.person_external_id {
        let n=sqlx::query("INSERT INTO media.person_artwork(person_id,artwork_id,source_id,role) SELECT id,$2,$3,$4 FROM catalog.people WHERE source_id=$3 AND external_id=$1 ON CONFLICT DO NOTHING")
            .bind(person).bind(art).bind(source).bind(&r.role).execute(&mut **tx).await?.rows_affected();
        if n == 0 {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT FROM catalog.people WHERE source_id=$1 AND external_id=$2)",
            )
            .bind(source)
            .bind(person)
            .fetch_one(&mut **tx)
            .await?;
            if !exists {
                return Err(Error::Invalid("artwork person"));
            }
        }
    }
    let source_key: String =
        sqlx::query_scalar("SELECT source_key FROM catalog.sources WHERE id=$1")
            .bind(source)
            .fetch_one(&mut **tx)
            .await?;
    for variant in &input.manifest.variants {
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM media.variants v JOIN media.objects o ON o.id=v.object_id WHERE v.artwork_id=$1 AND v.revision=$2 AND v.variant=$3 AND o.sha256=$4 AND o.state='ready')")
            .bind(art).bind(revision).bind(variant.variant.as_str()).bind(variant.sha256.as_slice()).fetch_one(&mut **tx).await?;
        if !ready {
            let job = ArtworkJob {
                source_key: source_key.clone(),
                source_artwork_id: r.source_artwork_id.clone(),
                artwork_id: art,
                revision,
                fingerprint,
                variant: variant.variant,
                expected_sha256: variant.sha256,
            };
            enqueue(
                tx,
                run,
                &Followup {
                    kind: JobKind::Artwork,
                    dedupe_key: format!("art:{art}:{revision}:{}", variant.variant.as_str()),
                    input: serde_json::to_value(job).map_err(|_| Error::Invalid("artwork job"))?,
                },
            )
            .await?;
        }
    }
    sqlx::query("DELETE FROM ingest.artwork_omissions WHERE run_id=$1 AND source_artwork_id=$2")
        .bind(run)
        .bind(&r.source_artwork_id)
        .execute(&mut **tx)
        .await?;
    Ok(art)
}

pub(crate) async fn write_omission(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
    omission: &ArtworkOmission,
) -> Result<()> {
    if omission.source_artwork_id.is_empty() || omission.source_artwork_id.len() > 200 {
        return Err(Error::Invalid("artwork omission"));
    }
    sqlx::query("INSERT INTO ingest.artwork_omissions(run_id,source_artwork_id,code) VALUES($1,$2,$3) ON CONFLICT(run_id,source_artwork_id) DO UPDATE SET code=EXCLUDED.code,observed_at=now()")
        .bind(run).bind(&omission.source_artwork_id).bind(omission.code.as_str()).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_manifest_fingerprint_is_canonical_and_versioned() {
        let mut input = ArtworkInputWrite {
            reference: ArtworkReferenceInput {
                source_artwork_id: "art".into(),
                kind: "poster".into(),
                rights: serde_json::json!({"license":"test","nested":{"z":1,"a":2}}),
                item_external_id: None,
                person_external_id: None,
                role: "primary".into(),
                ordinal: 0,
            },
            manifest: ArtworkInputManifest {
                version: 1,
                variants: vec![
                    ArtworkVariantInput {
                        variant: ArtworkVariant::Original,
                        sha256: [1; 32],
                    },
                    ArtworkVariantInput {
                        variant: ArtworkVariant::Native,
                        sha256: [2; 32],
                    },
                ],
            },
        };
        let fingerprint = input.fingerprint().unwrap();
        let mut native_only = input.clone();
        native_only
            .manifest
            .variants
            .retain(|v| v.variant == ArtworkVariant::Native);
        assert!(native_only.fingerprint().is_ok());
        let mut original_only = input.clone();
        original_only
            .manifest
            .variants
            .retain(|v| v.variant == ArtworkVariant::Original);
        assert!(original_only.fingerprint().is_err());
        input.manifest.variants.reverse();
        input.reference.rights =
            serde_json::from_str(r#"{"nested":{"a":2,"z":1},"license":"test"}"#).unwrap();
        input.reference.item_external_id = Some("other-association".into());
        assert_eq!(input.fingerprint().unwrap(), fingerprint);
        input.manifest.variants[0].sha256 = [3; 32];
        assert_ne!(input.fingerprint().unwrap(), fingerprint);
        input.manifest.variants[0].variant = ArtworkVariant::Original;
        assert!(input.fingerprint().is_err());
        input.manifest.variants.pop();
        input.manifest.version = 2;
        assert!(input.fingerprint().is_err());
    }
    #[test]
    fn fixture_images_and_svg_are_retained() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/catalog/artwork");
        for name in ["tmdb.png", "his-girl-friday.jpg"] {
            let bytes = std::fs::read(path.join(name)).unwrap();
            let image = ValidatedImage::raster(bytes.clone()).unwrap();
            assert_eq!(image.bytes(), bytes);
        }
        let bytes = std::fs::read(path.join("tmdb.svg")).unwrap();
        assert_eq!(
            ValidatedImage::svg_original(bytes.clone()).unwrap().bytes(),
            bytes
        );
        assert!(
            ValidatedImage::svg_original(b"<svg viewBox='0 0 1 1'><script/></svg>".to_vec())
                .is_err()
        );
        assert!(ValidatedImage::raster(vec![0; 20]).is_err());
    }
}
