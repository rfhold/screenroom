use crate::{
    Error, Result,
    config::required,
    media::{MAX_BYTES, ValidatedImage},
};
use aws_sigv4::http_request::{
    PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest, SigningSettings,
    UriPathNormalizationMode, sign,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream};
use reqwest::{Method, Url};
use screenroom_observability::transport::{
    BodyDirection, RequestKind, RequestObservation, RequestOutcome, TransportObservation,
};
use sha2::{Digest, Sha256};
use std::pin::Pin;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

#[derive(Clone)]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub timeout: Duration,
}
impl StorageConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            endpoint: required("SCREENROOM_S3_ENDPOINT")?,
            region: required("SCREENROOM_S3_REGION")?,
            bucket: required("SCREENROOM_S3_BUCKET")?,
            access_key_id: required("SCREENROOM_S3_ACCESS_KEY_ID")?,
            secret_access_key: required("SCREENROOM_S3_SECRET_ACCESS_KEY")?,
            timeout: Duration::from_secs(30),
        })
    }
}
#[derive(Debug, Clone)]
pub struct ObjectLocation {
    pub bucket: String,
    pub object_key: String,
    pub sha256: Vec<u8>,
    pub mime: String,
    pub byte_length: i64,
}
pub type ObjectByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send + 'static>>;
pub struct ObjectRead {
    pub location: ObjectLocation,
    pub body: ObjectByteStream,
}
struct ReadState {
    observation: RequestObservation,
    observer: Option<std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    upstream: Pin<Box<dyn Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send>>,
    pending: Bytes,
    received: usize,
    expected: usize,
    digest: Sha256,
    expected_digest: Vec<u8>,
    finished: bool,
    deadline: tokio::time::Instant,
}
/// Only upload + HEAD + byte-digest verification can create this publication proof.
#[derive(Debug, Clone)]
pub struct VerifiedObject {
    pub(crate) source: Uuid,
    pub(crate) location: ObjectLocation,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
impl VerifiedObject {
    pub fn location(&self) -> &ObjectLocation {
        &self.location
    }
    pub fn source_id(&self) -> Uuid {
        self.source
    }
}
#[derive(Clone)]
pub struct ObjectStorage {
    transport_observer: TransportObservation,
    download_observer: Option<std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    client: reqwest::Client,
    config: StorageConfig,
    endpoint: Url,
}
impl ObjectStorage {
    pub fn new(config: StorageConfig) -> Result<Self> {
        let endpoint = Url::parse(&config.endpoint).map_err(|_| Error::Invalid("S3 endpoint"))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || config.bucket.is_empty()
            || !config
                .bucket
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            || config.region.is_empty()
            || config.access_key_id.is_empty()
            || config.secret_access_key.is_empty()
            || config.timeout.is_zero()
            || config.timeout > Duration::from_secs(120)
        {
            return Err(Error::Invalid("S3 configuration"));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Unavailable)?;
        Ok(Self {
            transport_observer: Default::default(),
            download_observer: None,
            client,
            config,
            endpoint,
        })
    }
    /// Observes consumed response-body chunks, including chunks later rejected by verification.
    pub fn with_download_observer(
        mut self,
        observer: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Self {
        self.download_observer = Some(observer);
        self
    }
    pub fn with_transport_observer(mut self, observer: TransportObservation) -> Self {
        self.transport_observer = observer;
        self
    }
    fn url(&self, key: &str) -> Result<Url> {
        if key.is_empty()
            || key.len() > 1000
            || key.split('/').any(|part| {
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            })
        {
            return Err(Error::Invalid("object key"));
        }
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Invalid("endpoint"))?
            .pop_if_empty()
            .push(&self.config.bucket)
            .extend(key.split('/'));
        Ok(url)
    }
    async fn request(
        &self,
        method: Method,
        key: &str,
        body: Option<Vec<u8>>,
        mime: Option<&str>,
    ) -> Result<(reqwest::Response, RequestObservation)> {
        let url = self.url(key)?;
        let bytes = body.as_deref().unwrap_or_default();
        let identity = aws_credential_types::Credentials::new(
            &self.config.access_key_id,
            &self.config.secret_access_key,
            None,
            None,
            "screenroom-explicit-env",
        )
        .into();
        let mut settings = SigningSettings::default();
        settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        let params = aws_sigv4::sign::v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.config.region)
            .name("s3")
            .time(SystemTime::now())
            .settings(settings)
            .build()
            .map_err(|_| Error::Invalid("S3 signing"))?
            .into();
        let signable = SignableRequest::new(
            method.as_str(),
            url.as_str(),
            std::iter::empty(),
            SignableBody::Bytes(bytes),
        )
        .map_err(|_| Error::Invalid("S3 request"))?;
        let (instructions, _) = sign(signable, &params)
            .map_err(|_| Error::Invalid("S3 signing"))?
            .into_parts();
        let kind = match method {
            Method::GET => RequestKind::StorageRead,
            Method::HEAD => RequestKind::StorageHead,
            _ => RequestKind::StorageUpload,
        };
        let mut observation = self.transport_observer.start(kind);
        let mut request = self.client.request(method, url);
        for (name, value) in instructions.headers() {
            request = request.header(name, value);
        }
        if let Some(mime) = mime {
            request = request.header(reqwest::header::CONTENT_TYPE, mime);
        }
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_LENGTH, body.len())
                .body(crate::embedding::observed_upload(body, &observation));
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                observation.finish(RequestOutcome::Failed);
                return Err(Error::Unavailable);
            }
        };
        if !response.status().is_success() {
            observation.finish(RequestOutcome::Failed);
            return Err(Error::Unavailable);
        }
        Ok((response, observation))
    }
    pub async fn upload_verified(
        &self,
        source: Uuid,
        image: &ValidatedImage,
    ) -> Result<VerifiedObject> {
        let digest = Sha256::digest(image.bytes());
        let key = object_key(source, digest.as_slice());
        let (_, mut observation) = self
            .request(
                Method::PUT,
                &key,
                Some(image.bytes().to_vec()),
                Some(image.mime()),
            )
            .await?;
        observation.finish(RequestOutcome::Succeeded);
        let location = ObjectLocation {
            bucket: self.config.bucket.clone(),
            object_key: key,
            sha256: digest.to_vec(),
            mime: image.mime().into(),
            byte_length: image.bytes().len() as i64,
        };
        self.head(&location).await?;
        let _ = self.read(&location).await?;
        Ok(VerifiedObject {
            source,
            location,
            width: image.width(),
            height: image.height(),
        })
    }
    pub async fn head(&self, object: &ObjectLocation) -> Result<()> {
        self.validate_location(object)?;
        let (response, mut observation) = self
            .request(Method::HEAD, &object.object_key, None, None)
            .await?;
        let length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok());
        if length != Some(object.byte_length) {
            observation.finish(RequestOutcome::Failed);
            return Err(Error::Incompatible("S3 object length"));
        }
        observation.finish(RequestOutcome::Succeeded);
        Ok(())
    }
    /// Bounded download; dropping this future cancels transport. Never returns an S3 URL.
    pub async fn read(&self, object: &ObjectLocation) -> Result<Vec<u8>> {
        let mut read = self.read_stream(object, MAX_BYTES).await?;
        let mut bytes = Vec::with_capacity(object.byte_length as usize);
        while let Some(chunk) = read.body.next().await {
            bytes.extend_from_slice(&chunk?);
        }
        Ok(bytes)
    }
    /// No prefetch task or complete-body buffer. Dropping the stream drops the upstream response.
    /// Chunks are at most 64 KiB; the final upstream chunk is withheld until length/digest/EOF verify.
    /// The total deadline includes body consumption and backpressure time.
    pub async fn read_stream(
        &self,
        object: &ObjectLocation,
        max_bytes: usize,
    ) -> Result<ObjectRead> {
        self.validate_location(object)?;
        if !(1..=MAX_BYTES).contains(&max_bytes) || object.byte_length as usize > max_bytes {
            return Err(Error::Invalid("object read limit"));
        }
        let deadline = tokio::time::Instant::now() + self.config.timeout;
        let (response, mut observation) = self
            .request(Method::GET, &object.object_key, None, None)
            .await?;
        let length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok());
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok());
        if response.status() != reqwest::StatusCode::OK
            || length != Some(object.byte_length)
            || mime != Some(object.mime.as_str())
        {
            observation.finish(RequestOutcome::Failed);
            return Err(Error::Incompatible("S3 object status, length or MIME"));
        }
        let state = ReadState {
            observation,
            observer: self.download_observer.clone(),
            upstream: Box::pin(response.bytes_stream()),
            pending: Bytes::new(),
            received: 0,
            expected: object.byte_length as usize,
            digest: Sha256::new(),
            expected_digest: object.sha256.clone(),
            finished: false,
            deadline,
        };
        let body = stream::try_unfold(state, |mut state| async move {
            let result = async {
                loop {
                    if state.finished && state.pending.is_empty() {
                        state.observation.finish(RequestOutcome::Succeeded);
                        return Ok(None);
                    }
                    if tokio::time::Instant::now() >= state.deadline {
                        return Err(Error::Unavailable);
                    }
                    if !state.pending.is_empty() {
                        let count = state.pending.len().min(64 * 1024);
                        return Ok(Some(state.pending.split_to(count)));
                    }
                    let chunk = tokio::time::timeout_at(state.deadline, state.upstream.next())
                        .await
                        .map_err(|_| Error::Unavailable)?
                        .transpose()
                        .map_err(|_| Error::Unavailable)?
                        .ok_or(Error::Incompatible("S3 truncated object"))?;
                    state
                        .observation
                        .body()
                        .chunk(BodyDirection::DownloadConsumed, chunk.len());
                    if let Some(observer) = &state.observer {
                        observer(chunk.len() as u64);
                    }
                    state.received = state
                        .received
                        .checked_add(chunk.len())
                        .ok_or(Error::Incompatible("S3 object length"))?;
                    if state.received > state.expected {
                        return Err(Error::Incompatible("S3 object length"));
                    }
                    state.digest.update(&chunk);
                    if state.received == state.expected {
                        let end = tokio::time::timeout_at(state.deadline, state.upstream.next())
                            .await
                            .map_err(|_| Error::Unavailable)?;
                        if let Some(Ok(chunk)) = &end {
                            state
                                .observation
                                .body()
                                .chunk(BodyDirection::DownloadConsumed, chunk.len());
                        }
                        if let (Some(Ok(chunk)), Some(observer)) = (&end, &state.observer) {
                            observer(chunk.len() as u64);
                        }
                        if end.is_some()
                            || std::mem::take(&mut state.digest).finalize().as_slice()
                                != state.expected_digest
                        {
                            return Err(Error::Incompatible("S3 object digest or length"));
                        }
                        state.finished = true;
                        state.observation.finish(RequestOutcome::Succeeded);
                    }
                    state.pending = chunk;
                }
            }
            .await;
            match result {
                Ok(Some(chunk)) => Ok(Some((chunk, state))),
                Ok(None) => Ok(None),
                Err(error) => {
                    state.observation.finish(RequestOutcome::Failed);
                    Err(error)
                }
            }
        });
        Ok(ObjectRead {
            location: object.clone(),
            body: Box::pin(body),
        })
    }
    fn validate_location(&self, o: &ObjectLocation) -> Result<()> {
        if o.bucket != self.config.bucket
            || o.sha256.len() != 32
            || o.byte_length < 1
            || o.byte_length > MAX_BYTES as i64
            || o.mime.is_empty()
            || o.mime.len() > 200
            || !o.mime.is_ascii()
            || o.mime.bytes().any(|b| b.is_ascii_control())
        {
            return Err(Error::Invalid("object location"));
        }
        Ok(())
    }
}
pub fn object_key(source: Uuid, digest: &[u8]) -> String {
    let hash = hex::encode(digest);
    let prefix = hash.get(..2).unwrap_or("00");
    format!("artwork/v1/sources/{source}/sha256/{prefix}/{hash}/original")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_scoped_keys() {
        let digest = [7; 32];
        assert_ne!(
            object_key(Uuid::nil(), &digest),
            object_key(Uuid::new_v4(), &digest)
        );
        assert!(object_key(Uuid::nil(), &digest).contains("/0707"));
    }
    #[test]
    fn private_path_style_no_query() {
        let s = ObjectStorage::new(StorageConfig {
            endpoint: "http://localhost:3900".into(),
            region: "garage".into(),
            bucket: "artwork".into(),
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
            timeout: Duration::from_secs(2),
        })
        .unwrap();
        assert_eq!(
            s.url("a/b").unwrap().as_str(),
            "http://localhost:3900/artwork/a/b"
        );
        assert!(s.url("a/../b").is_err());
        assert!(s.url("https://example.com").is_err());
    }
}
