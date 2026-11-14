use crate::{Error, Result, config::required};
use futures_util::StreamExt;
use screenroom_observability::transport::{
    BodyDirection, RequestKind, RequestObservation, RequestOutcome, TransportObservation,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const DIMENSIONS: usize = 1024;
pub const DOCUMENT_VERSION: i32 = 1;
pub const MAX_INPUT_CHARS: usize = 8000;
pub const MAX_BATCH: usize = 16;

#[derive(Clone)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub dimensions: usize,
    pub revision: String,
    pub timeout: Duration,
}
impl EmbeddingConfig {
    pub fn optional_from_env() -> Result<Option<Self>> {
        let configured = [
            "SCREENROOM_EMBEDDING_BASE_URL",
            "SCREENROOM_EMBEDDING_MODEL",
            "SCREENROOM_EMBEDDING_API_KEY",
            "SCREENROOM_EMBEDDING_DIMENSIONS",
            "SCREENROOM_EMBEDDING_REVISION",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some());
        if configured {
            Self::from_env().map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            base_url: required("SCREENROOM_EMBEDDING_BASE_URL")?,
            model: required("SCREENROOM_EMBEDDING_MODEL")?,
            api_key: std::env::var("SCREENROOM_EMBEDDING_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            dimensions: required("SCREENROOM_EMBEDDING_DIMENSIONS")?
                .parse()
                .map_err(|_| Error::Invalid("embedding dimensions"))?,
            revision: required("SCREENROOM_EMBEDDING_REVISION")?,
            timeout: Duration::from_secs(30),
        })
    }
    pub fn validate(&self) -> Result<()> {
        let url =
            reqwest::Url::parse(&self.base_url).map_err(|_| Error::Invalid("embedding URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || self.model.is_empty()
            || self.model.len() > 200
            || self.revision.is_empty()
            || self.revision.len() > 200
            || self.dimensions != DIMENSIONS
            || self.timeout.is_zero()
            || self.timeout > Duration::from_secs(120)
        {
            return Err(Error::Invalid(
                "embedding configuration; expected 1024 dimensions and declared revision",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct EmbeddingClient {
    transport_observer: TransportObservation,
    client: reqwest::Client,
    config: EmbeddingConfig,
}
#[derive(Debug, Clone)]
pub struct EmbeddingBatch {
    pub vectors: Vec<Vec<f32>>,
    pub response_model: String,
    pub revision: String,
}
impl EmbeddingClient {
    pub fn new(config: EmbeddingConfig) -> Result<Self> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Unavailable)?;
        Ok(Self {
            client,
            config,
            transport_observer: Default::default(),
        })
    }
    pub fn with_transport_observer(mut self, observer: TransportObservation) -> Self {
        self.transport_observer = observer;
        self
    }
    pub fn config(&self) -> &EmbeddingConfig {
        &self.config
    }
    pub async fn embed(&self, inputs: &[String]) -> Result<EmbeddingBatch> {
        if inputs.is_empty() || inputs.len() > MAX_BATCH {
            return Err(Error::Invalid("embedding batch"));
        }
        let inputs: Vec<String> = inputs.iter().map(|s| preprocess(s)).collect();
        if inputs
            .iter()
            .any(|s| s.is_empty() || s.chars().count() > MAX_INPUT_CHARS)
        {
            return Err(Error::Invalid("embedding input length"));
        }
        let mut request=self.client.post(format!("{}/embeddings",self.config.base_url.trim_end_matches('/'))).json(&serde_json::json!({"model":self.config.model,"input":inputs,"encoding_format":"float"}));
        if let Some(key) = &self.config.api_key {
            request = request.bearer_auth(key);
        }
        let mut observation = self.transport_observer.start(RequestKind::Embedding);
        let result = async {
            let mut request = request.build().map_err(|_| Error::Unavailable)?;
            if observation.is_enabled()
                && let Some(bytes) = request.body().and_then(reqwest::Body::as_bytes)
            {
                let bytes = bytes.to_vec();
                request
                    .headers_mut()
                    .insert(reqwest::header::CONTENT_LENGTH, bytes.len().into());
                *request.body_mut() = Some(observed_upload(bytes, &observation));
            }
            let response = self
                .client
                .execute(request)
                .await
                .map_err(|_| Error::Unavailable)?;
            if !response.status().is_success() {
                return Err(
                    if response.status().is_server_error() || response.status().as_u16() == 429 {
                        Error::Unavailable
                    } else {
                        Error::Incompatible("embedding HTTP response")
                    },
                );
            }
            bounded_body(response, 2 * 1024 * 1024, &observation).await
        }
        .await;
        observation.finish(if result.is_ok() {
            RequestOutcome::Succeeded
        } else {
            RequestOutcome::Failed
        });
        decode(&result?, inputs.len(), &self.config.revision)
    }
}

pub fn preprocess(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn input_hash(input: &str) -> [u8; 32] {
    Sha256::digest(preprocess(input).as_bytes()).into()
}
pub fn validate_vector(vector: &[f32]) -> Result<()> {
    if vector.len() != DIMENSIONS
        || vector.iter().any(|v| !v.is_finite())
        || !vector.iter().any(|v| *v != 0.0)
    {
        return Err(Error::Incompatible(
            "embedding vector dimensions, finite values or norm",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct Response {
    data: Vec<Row>,
    model: String,
}
#[derive(Deserialize)]
struct Row {
    index: usize,
    embedding: Vec<f32>,
}
fn decode(bytes: &[u8], count: usize, revision: &str) -> Result<EmbeddingBatch> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| Error::Incompatible("embedding JSON"))?;
    if response.data.len() != count || response.model.is_empty() {
        return Err(Error::Incompatible("embedding row count or model"));
    }
    let mut rows = vec![None; count];
    for row in response.data {
        if row.index >= count || rows[row.index].is_some() {
            return Err(Error::Incompatible("embedding duplicate or invalid index"));
        }
        validate_vector(&row.embedding)?;
        rows[row.index] = Some(row.embedding);
    }
    let vectors = rows
        .into_iter()
        .map(|r| r.ok_or(Error::Incompatible("embedding missing index")))
        .collect::<Result<_>>()?;
    Ok(EmbeddingBatch {
        vectors,
        response_model: response.model,
        revision: revision.into(),
    })
}
async fn bounded_body(
    response: reqwest::Response,
    max: usize,
    observation: &RequestObservation,
) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(Error::Incompatible("response byte limit"));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Unavailable)?;
        observation
            .body()
            .chunk(BodyDirection::DownloadConsumed, chunk.len());
        if bytes.len() + chunk.len() > max {
            return Err(Error::Incompatible("response byte limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn observed_upload(bytes: Vec<u8>, observation: &RequestObservation) -> reqwest::Body {
    if !observation.is_enabled() {
        return reqwest::Body::from(bytes);
    }
    let observer = observation.body();
    reqwest::Body::wrap_stream(futures_util::stream::unfold(
        bytes::Bytes::from(bytes),
        move |mut remaining| {
            let observer = observer.clone();
            async move {
                if remaining.is_empty() {
                    return None;
                }
                let chunk = remaining.split_to(remaining.len().min(64 * 1024));
                observer.chunk(BodyDirection::UploadBodyConsumed, chunk.len());
                Some((Ok::<_, std::io::Error>(chunk), remaining))
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preprocessing_and_hash_are_shared() {
        assert_eq!(preprocess(" one\n two \t"), "one two");
        assert_eq!(input_hash("one\ntwo"), input_hash("one two"));
    }
    #[test]
    fn reorders_and_rejects_bad_rows() {
        let mut first = vec![0.0; DIMENSIONS];
        first[0] = 1.0;
        let mut second = first.clone();
        second[1] = 2.0;
        let bytes=serde_json::to_vec(&serde_json::json!({"model":"alias-target","data":[{"index":1,"embedding":second},{"index":0,"embedding":first}]})).unwrap();
        let batch = decode(&bytes, 2, "declared-r1").unwrap();
        assert_eq!(batch.vectors[1][1], 2.0);
        assert_eq!(batch.revision, "declared-r1");
        assert!(decode(&bytes, 1, "r").is_err());
        assert!(validate_vector(&vec![f32::NAN; DIMENSIONS]).is_err());
        assert!(validate_vector(&vec![0.0; DIMENSIONS]).is_err());
        let bad=serde_json::to_vec(&serde_json::json!({"model":"m","data":[{"index":0,"embedding":first},{"index":0,"embedding":first}]})).unwrap();
        assert!(decode(&bad, 2, "r").is_err());
    }
    #[tokio::test]
    #[ignore = "explicit opt-in, bounded integration to the authorized endpoint only"]
    async fn authorized_live_embedding() {
        let config = EmbeddingConfig::from_env().unwrap();
        assert_eq!(
            config.base_url.trim_end_matches('/'),
            "https://agent-gateway.holdenitdown.net/v1"
        );
        assert_eq!(config.model, "local-embedding");
        let response = EmbeddingClient::new(config)
            .unwrap()
            .embed(&[
                "A friendly animated rabbit adventure".into(),
                "A silent science fiction journey".into(),
            ])
            .await
            .unwrap();
        assert_eq!(response.vectors.len(), 2);
        assert_eq!(response.vectors[0].len(), DIMENSIONS);
    }
}
