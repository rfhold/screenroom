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
            client,
            config,
            endpoint,
        })
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
    ) -> Result<reqwest::Response> {
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
        let mut request = self.client.request(method, url);
        for (name, value) in instructions.headers() {
            request = request.header(name, value);
        }
        if let Some(mime) = mime {
            request = request.header(reqwest::header::CONTENT_TYPE, mime);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(|_| Error::Unavailable)?;
        if !response.status().is_success() {
            return Err(Error::Unavailable);
        }
        Ok(response)
    }
    pub async fn upload_verified(
        &self,
        source: Uuid,
        image: &ValidatedImage,
    ) -> Result<VerifiedObject> {
        let digest = Sha256::digest(image.bytes());
        let key = object_key(source, digest.as_slice());
        self.request(
            Method::PUT,
            &key,
            Some(image.bytes().to_vec()),
            Some(image.mime()),
        )
        .await?;
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
        let response = self
            .request(Method::HEAD, &object.object_key, None, None)
            .await?;
        let length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok());
        if length != Some(object.byte_length) {
            return Err(Error::Incompatible("S3 object length"));
        }
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
        let response = self
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
            return Err(Error::Incompatible("S3 object status, length or MIME"));
        }
        let state = ReadState {
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
            loop {
                if state.finished && state.pending.is_empty() {
                    return Ok(None);
                }
                if tokio::time::Instant::now() >= state.deadline {
                    return Err(Error::Unavailable);
                }
                if !state.pending.is_empty() {
                    let count = state.pending.len().min(64 * 1024);
                    return Ok(Some((state.pending.split_to(count), state)));
                }
                let chunk = tokio::time::timeout_at(state.deadline, state.upstream.next())
                    .await
                    .map_err(|_| Error::Unavailable)?
                    .transpose()
                    .map_err(|_| Error::Unavailable)?
                    .ok_or(Error::Incompatible("S3 truncated object"))?;
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
                    if end.is_some()
                        || std::mem::take(&mut state.digest).finalize().as_slice()
                            != state.expected_digest
                    {
                        return Err(Error::Incompatible("S3 object digest or length"));
                    }
                    state.finished = true;
                }
                state.pending = chunk;
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
