use super::*;

const PLAYLIST_LIMIT: usize = 4 * 1024 * 1024;
const SUBTITLE_LIMIT: usize = 2 * 1024 * 1024;
const RESOURCE_LIMIT: usize = 8192;
const NESTING_LIMIT: u8 = 4;
const CHUNK_LIMIT: usize = 256 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ResourceScope {
    pub device: GatewayDeviceId,
    pub server: String,
    pub profile: String,
    pub item: String,
    pub source: String,
    pub play_session: String,
}
impl std::fmt::Debug for ResourceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResourceScope([REDACTED])")
    }
}
impl ResourceScope {
    pub fn check_auth(&self, auth: &SessionAuth) -> Result<()> {
        if self.device != auth.intent.device
            || self.server != auth.intent.server
            || self.profile != auth.intent.personal_profile
        {
            return Err(fail(Failure::Identity));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    DirectMedia,
    Playlist,
    Segment,
    Subtitle,
    Initialization,
}
/// Not constructible from client input. Only negotiation and validated playlist expansion mint
/// references. Gateway maps these under its exact session/transport revision and rechecks fences.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceRef {
    pub(super) url: Url,
    pub(super) scope: ResourceScope,
    pub(super) depth: u8,
    pub(super) kind: ResourceKind,
}
impl std::hash::Hash for ResourceRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.url.hash(state);
        self.scope.device.as_private_str().hash(state);
        self.scope.server.hash(state);
        self.scope.profile.hash(state);
        self.scope.item.hash(state);
        self.scope.source.hash(state);
        self.scope.play_session.hash(state);
        self.depth.hash(state);
        self.kind.hash(state);
    }
}
impl std::fmt::Debug for ResourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceRef")
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .finish()
    }
}
impl ResourceRef {
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    pub fn nesting_depth(&self) -> u8 {
        self.depth
    }
}

/// One validated HTTP byte range, including open-ended and suffix ranges.
#[derive(Debug, Clone)]
pub struct ByteRange(String);
impl ByteRange {
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() > 64 {
            return Err(fail(Failure::InvalidInput));
        }
        let range = value
            .strip_prefix("bytes=")
            .ok_or_else(|| fail(Failure::InvalidInput))?;
        let (start, end) = range
            .split_once('-')
            .ok_or_else(|| fail(Failure::InvalidInput))?;
        let number = |s: &str| -> Result<Option<u64>> {
            if s.is_empty() {
                return Ok(None);
            }
            if !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(fail(Failure::InvalidInput));
            }
            s.parse().map(Some).map_err(|_| fail(Failure::InvalidInput))
        };
        match (number(start)?, number(end)?) {
            (None, None) | (None, Some(0)) => return Err(fail(Failure::InvalidInput)),
            (Some(s), Some(e)) if s > e => return Err(fail(Failure::InvalidInput)),
            _ => {}
        }
        Ok(Self(value.into()))
    }
}
#[derive(Debug, Default)]
pub struct MediaRequest {
    pub head: bool,
    pub range: Option<ByteRange>,
    pub if_range: Option<String>,
}
/// Pull-driven: one bounded chunk per await, no background body reader or whole-film deadline.
/// Dropping the response cancels consumption; an idle error permanently closes it.
/// Retains at most one upstream frame; its backing allocation is transport-owned.
pub struct MediaStream {
    pub status: u16,
    pub headers: header::HeaderMap,
    response: Option<reqwest::Response>,
    pending: bytes::Bytes,
    idle_timeout: Duration,
}
async fn bounded_frame_chunk(
    pending: &mut bytes::Bytes,
    fetch: impl Future<Output = Result<Option<bytes::Bytes>>>,
) -> Result<Option<Vec<u8>>> {
    if pending.is_empty() {
        let Some(frame) = fetch.await? else {
            return Ok(None);
        };
        *pending = frame;
    }
    Ok(Some(
        pending.split_to(pending.len().min(CHUNK_LIMIT)).to_vec(),
    ))
}
impl std::fmt::Debug for MediaStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaStream")
            .field("status", &self.status)
            .finish()
    }
}
impl MediaStream {
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(response) = self.response.as_mut() else {
            return Ok(None);
        };
        let next = bounded_frame_chunk(&mut self.pending, async {
            tokio::time::timeout(self.idle_timeout, response.chunk())
                .await
                .map_err(|_| fail(Failure::Deadline))
                .and_then(|r| r.map_err(|e| remote_error(e, false)))
        })
        .await;
        match next {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            Ok(None) => {
                self.response = None;
                self.pending = bytes::Bytes::new();
                Ok(None)
            }
            Err(error) => {
                self.response = None;
                self.pending = bytes::Bytes::new();
                Err(error)
            }
        }
    }
    pub async fn bounded_body(self, limit: usize) -> Result<Vec<u8>> {
        if limit == 0 || limit > SUBTITLE_LIMIT {
            return Err(fail(Failure::Bounds));
        }
        self.bounded_resource_body(limit).await
    }
    async fn bounded_resource_body(mut self, limit: usize) -> Result<Vec<u8>> {
        if limit == 0 || limit > PLAYLIST_LIMIT {
            return Err(fail(Failure::Bounds));
        }
        let mut body = Vec::new();
        while let Some(chunk) = self.next_chunk().await? {
            if chunk.len() > limit - body.len() {
                return Err(fail(Failure::Bounds));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}
impl PlaybackAdapter {
    pub(super) fn base_directory(&self) -> Result<Url> {
        let mut base = self.admin.base.clone();
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Ok(base)
    }
    pub(super) fn resource(
        &self,
        scope: &ResourceScope,
        parent: &Url,
        raw: &str,
        depth: u8,
    ) -> Result<ResourceRef> {
        if raw.is_empty()
            || raw.len() > 8192
            || raw.contains(['\\', '#'])
            || raw.chars().any(char::is_control)
            || raw.trim() != raw
        {
            return Err(fail(Failure::ResourceRejected));
        }
        // Reject normalization tricks before URL::join erases dot segments. Encoded slashes,
        // double encodings, and dot segments have no place in Jellyfin's generated media paths.
        let base = self.base_directory()?;
        let inspected = raw
            .strip_prefix(base.as_str())
            .or_else(|| raw.strip_prefix(base.path()))
            .unwrap_or(raw);
        let path = inspected.split('?').next().unwrap_or("");
        if path.split('/').any(|s| s == "." || s == "..")
            || path.contains('%')
            || raw.starts_with("//")
        {
            return Err(fail(Failure::ResourceRejected));
        }
        let mut url = parent
            .join(raw)
            .map_err(|_| fail(Failure::ResourceRejected))?;
        if url.origin() != base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !url.path().starts_with(base.path())
        {
            return Err(fail(Failure::ResourceRejected));
        }
        network::validate_url(&url).map_err(|_| fail(Failure::ResourceRejected))?;
        let relative = url
            .path()
            .strip_prefix(base.path())
            .ok_or_else(|| fail(Failure::ResourceRejected))?;
        let parts: Vec<_> = relative.split('/').collect();
        if parts.len() < 3
            || !parts[0].eq_ignore_ascii_case("Videos")
            || !same_id(parts[1], &scope.item)
        {
            return Err(fail(Failure::ResourceRejected));
        }
        let filename = parts.last().copied().unwrap_or("").to_ascii_lowercase();
        let kind = if filename.ends_with(".m3u8") {
            ResourceKind::Playlist
        } else if filename.ends_with(".vtt") {
            ResourceKind::Subtitle
        } else if filename.ends_with(".ts") || filename.ends_with(".m4s") {
            ResourceKind::Segment
        } else if filename.ends_with(".mp4")
            && (filename.starts_with("init")
                || parts.iter().any(|s| s.eq_ignore_ascii_case("hls1")))
        {
            ResourceKind::Initialization
        } else if filename == "stream.mp4" && parts.len() == 3 {
            ResourceKind::DirectMedia
        } else {
            return Err(fail(Failure::ResourceRejected));
        };
        if kind == ResourceKind::Playlist && depth > NESTING_LIMIT {
            return Err(fail(Failure::Bounds));
        }
        let subtitle_path = parts.iter().any(|s| s.eq_ignore_ascii_case("Subtitles"));
        if subtitle_path && (parts.len() < 6 || !same_id(parts[2], &scope.source)) {
            return Err(fail(Failure::ResourceRejected));
        }
        if kind == ResourceKind::Subtitle && !subtitle_path {
            return Err(fail(Failure::ResourceRejected));
        }
        let mut query = Vec::new();
        let mut keys = std::collections::BTreeSet::new();
        for (key, value) in url.query_pairs() {
            let lower = key.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "api_key" | "apikey" | "access_token" | "token" | "x-emby-token" | "authorization"
            ) {
                continue;
            }
            if key.len() > 64
                || value.len() > 2048
                || value.chars().any(char::is_control)
                || !keys.insert(lower.clone())
            {
                return Err(fail(Failure::ResourceRejected));
            }
            if matches!(
                lower.as_str(),
                "url" | "path" | "secret" | "params" | "livestreamid" | "opentoken"
            ) || (lower == "itemid" && !same_id(&value, &scope.item))
                || (lower == "mediasourceid" && !same_id(&value, &scope.source))
                || (lower == "deviceid" && value != scope.device.0)
                || (lower == "playsessionid" && value != scope.play_session)
                || (lower == "userid" && value != scope.profile)
            {
                return Err(fail(Failure::ResourceRejected));
            }
            if !subtitle_path
                && matches!(
                    lower.as_str(),
                    "mediasourceid" | "deviceid" | "playsessionid"
                )
            {
                continue;
            }
            query.push((key.into_owned(), value.into_owned()));
        }
        if !subtitle_path {
            // URL joining drops a parent's query. Bind supported video/HLS endpoints
            // explicitly instead of allowing their provider-side default source/session.
            query.extend([
                ("mediaSourceId".into(), scope.source.clone()),
                ("deviceId".into(), scope.device.0.clone()),
                ("playSessionId".into(), scope.play_session.clone()),
            ]);
        }
        url.set_query(None);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(ResourceRef {
            url,
            scope: scope.clone(),
            depth,
            kind,
        })
    }
    pub async fn stream(
        &self,
        auth: &SessionAuth,
        resource: &ResourceRef,
        request: MediaRequest,
    ) -> Result<MediaStream> {
        resource.scope.check_auth(auth)?;
        self.check_binding(&auth.intent)?;
        // Revalidate the adapter's configured destination too: refs cannot migrate across servers.
        self.resource(
            &resource.scope,
            &resource.url,
            resource.url.as_str(),
            resource.depth,
        )?;
        if request
            .if_range
            .as_ref()
            .is_some_and(|s| s.len() > 256 || !s.is_ascii() || s.chars().any(char::is_control))
        {
            return Err(fail(Failure::InvalidInput));
        }
        let method = if request.head {
            Method::HEAD
        } else {
            Method::GET
        };
        let mut outbound = self
            .client
            .request(method, resource.url.clone())
            .header(
                header::AUTHORIZATION,
                self.authorization(&auth.intent.device, Some(&auth.token))?,
            )
            .header(header::ACCEPT_ENCODING, "identity");
        if let Some(range) = request.range {
            outbound = outbound.header(header::RANGE, range.0);
        }
        if let Some(validator) = request.if_range {
            outbound = outbound.header(header::IF_RANGE, validator);
        }
        self.check_dispatch(
            DispatchPurpose::Active,
            false,
            Some(&resource.scope.play_session),
        )
        .await?;
        let response = tokio::time::timeout(self.header_timeout, outbound.send())
            .await
            .map_err(|_| fail(Failure::Deadline))?
            .map_err(|e| remote_error(e, false))?;
        let status = response.status().as_u16();
        if !matches!(status, 200 | 206 | 416) {
            return Err(fail(Failure::Http(status)));
        }
        let mut headers = header::HeaderMap::new();
        for name in [
            header::CONTENT_TYPE,
            header::CONTENT_LENGTH,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::ETAG,
            header::LAST_MODIFIED,
        ] {
            if let Some(value) = response.headers().get(&name) {
                if value.len() > 512 || value.to_str().is_err() {
                    return Err(fail(Failure::InvalidResponse));
                }
                headers.insert(name, value.clone());
            }
        }
        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err(fail(Failure::InvalidResponse));
        }
        if status != 416 {
            let mime = headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let valid = match resource.kind {
                ResourceKind::Playlist => matches!(
                    mime.as_str(),
                    "application/vnd.apple.mpegurl"
                        | "application/x-mpegurl"
                        | "audio/mpegurl"
                        | "audio/x-mpegurl"
                ),
                ResourceKind::Subtitle => mime == "text/vtt",
                ResourceKind::Segment => matches!(
                    mime.as_str(),
                    "video/mp2t" | "video/mp4" | "audio/mp4" | "application/octet-stream"
                ),
                ResourceKind::Initialization | ResourceKind::DirectMedia => {
                    matches!(mime.as_str(), "video/mp4" | "audio/mp4")
                }
            };
            if !valid {
                return Err(fail(Failure::Unsupported));
            }
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_str(&mime).map_err(|_| fail(Failure::InvalidResponse))?,
            );
        }
        if matches!(status, 206 | 416) && !headers.contains_key(header::CONTENT_RANGE) {
            return Err(fail(Failure::InvalidResponse));
        }
        if let Some(range) = headers.get(header::CONTENT_RANGE) {
            validate_content_range(
                range.to_str().map_err(|_| fail(Failure::InvalidResponse))?,
                status,
            )?;
        }
        if headers
            .get(header::ACCEPT_RANGES)
            .is_some_and(|value| value != "bytes" && value != "none")
            || headers.values().any(|value| {
                value
                    .to_str()
                    .is_ok_and(|v| v.contains(auth.token.expose_for_private_store()))
            })
        {
            return Err(fail(Failure::InvalidResponse));
        }
        if status == 416 {
            headers.insert(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("0"),
            );
            headers.remove(header::CONTENT_TYPE);
        }
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        Ok(MediaStream {
            status,
            headers,
            response: if request.head || status == 416 {
                None
            } else {
                Some(response)
            },
            idle_timeout: self.idle_timeout,
            pending: bytes::Bytes::new(),
        })
    }
    pub async fn playlist(
        &self,
        auth: &SessionAuth,
        resource: &ResourceRef,
        budget: &mut PlaylistBudget,
        map: impl FnMut(ResourceRef) -> Result<String>,
    ) -> Result<String> {
        if resource.kind != ResourceKind::Playlist {
            return Err(fail(Failure::InvalidInput));
        }
        let body = tokio::time::timeout(self.control_timeout, async {
            self.stream(auth, resource, MediaRequest::default())
                .await?
                .bounded_resource_body(PLAYLIST_LIMIT)
                .await
        })
        .await
        .map_err(|_| fail(Failure::Deadline))??;
        self.rewrite_playlist(auth, resource, &body, budget, map)
    }
    pub async fn subtitle(&self, auth: &SessionAuth, resource: &ResourceRef) -> Result<Vec<u8>> {
        if resource.kind != ResourceKind::Subtitle {
            return Err(fail(Failure::InvalidInput));
        }
        let body = tokio::time::timeout(self.control_timeout, async {
            self.stream(auth, resource, MediaRequest::default())
                .await?
                .bounded_body(SUBTITLE_LIMIT)
                .await
        })
        .await
        .map_err(|_| fail(Failure::Deadline))??;
        let text = std::str::from_utf8(&body).map_err(|_| fail(Failure::InvalidResponse))?;
        if !text.starts_with("WEBVTT")
            || text.contains(auth.token.expose_for_private_store())
            || text.contains("X-TIMESTAMP-MAP")
        {
            return Err(fail(Failure::Unsupported));
        }
        Ok(body)
    }
}
fn validate_content_range(value: &str, status: u16) -> Result<()> {
    let range = value
        .strip_prefix("bytes ")
        .ok_or_else(|| fail(Failure::InvalidResponse))?;
    let (range, total) = range
        .split_once('/')
        .ok_or_else(|| fail(Failure::InvalidResponse))?;
    let number = |s: &str| -> Result<u64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(fail(Failure::InvalidResponse));
        }
        s.parse().map_err(|_| fail(Failure::InvalidResponse))
    };
    if status == 416 {
        if range != "*" {
            return Err(fail(Failure::InvalidResponse));
        }
        number(total)?;
    } else {
        let (start, end) = range
            .split_once('-')
            .ok_or_else(|| fail(Failure::InvalidResponse))?;
        let (start, end) = (number(start)?, number(end)?);
        if start > end || (total != "*" && end >= number(total)?) {
            return Err(fail(Failure::InvalidResponse));
        }
    }
    Ok(())
}

/// One budget per transport graph, not per playlist fetch. Repeated fetches of the same
/// sanitized locator do not consume additional capacity.
#[derive(Default, Clone)]
pub struct PlaylistBudget {
    resources: usize,
    seen: std::collections::BTreeSet<String>,
}
impl std::fmt::Debug for PlaylistBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaylistBudget")
            .field("resources", &self.resources)
            .finish()
    }
}
impl PlaylistBudget {
    pub fn resources(&self) -> usize {
        self.resources
    }
}
impl PlaybackAdapter {
    /// Rewrites only the supported VOD subset; rejects unknown tags rather than guessing whether
    /// an extension can load URLs. The mapper must return gateway-only opaque URLs.
    pub fn rewrite_playlist(
        &self,
        auth: &SessionAuth,
        parent: &ResourceRef,
        body: &[u8],
        budget: &mut PlaylistBudget,
        map: impl FnMut(ResourceRef) -> Result<String>,
    ) -> Result<String> {
        // Validate the entire manifest and resource graph before touching caller mappings/budget.
        let mut validation_budget = PlaylistBudget {
            resources: budget.resources,
            seen: budget.seen.clone(),
        };
        self.rewrite_playlist_inner(auth, parent, body, &mut validation_budget, |_| {
            Ok("/playback/v1/media/".into())
        })?;
        self.rewrite_playlist_inner(auth, parent, body, budget, map)
    }
    fn rewrite_playlist_inner(
        &self,
        auth: &SessionAuth,
        parent: &ResourceRef,
        body: &[u8],
        budget: &mut PlaylistBudget,
        mut map: impl FnMut(ResourceRef) -> Result<String>,
    ) -> Result<String> {
        parent.scope.check_auth(auth)?;
        if body.len() > PLAYLIST_LIMIT || parent.kind != ResourceKind::Playlist {
            return Err(fail(Failure::Bounds));
        }
        let text = std::str::from_utf8(body).map_err(|_| fail(Failure::InvalidResponse))?;
        if text.lines().next() != Some("#EXTM3U") {
            return Err(fail(Failure::Unsupported));
        }
        let master = validate_playlist_form(text)?;
        let mut output = String::new();
        for line in text.lines() {
            if line.len() > 8192 || line.chars().any(|c| c.is_control() && c != '\r') {
                return Err(fail(Failure::ResourceRejected));
            }
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let rewrite = |raw: &str,
                           budget: &mut PlaylistBudget,
                           map: &mut dyn FnMut(ResourceRef) -> Result<String>|
             -> Result<String> {
                let resource = self.resource(&parent.scope, &parent.url, raw, parent.depth + 1)?;
                // Segments do not deepen recursion. Only playlists consume nesting depth.
                let resource = ResourceRef {
                    depth: if resource.kind == ResourceKind::Playlist {
                        parent.depth + 1
                    } else {
                        parent.depth
                    },
                    ..resource
                };
                if !budget.seen.contains(resource.url.as_str()) {
                    if budget.resources >= RESOURCE_LIMIT {
                        return Err(fail(Failure::Bounds));
                    }
                    budget.seen.insert(resource.url.as_str().into());
                    budget.resources += 1;
                }
                let mapped = map(resource)?;
                if !mapped.starts_with("/playback/v1/media/")
                    || mapped.contains(['"', '\r', '\n', '\\'])
                    || mapped.contains(auth.token.expose_for_private_store())
                    || mapped.len() > 8192
                {
                    return Err(fail(Failure::ResourceRejected));
                }
                Ok(mapped)
            };
            let rewritten = if !line.starts_with('#') {
                let resource = self.resource(&parent.scope, &parent.url, line, parent.depth + 1)?;
                if (resource.kind == ResourceKind::Playlist) != master {
                    return Err(fail(Failure::Unsupported));
                }
                rewrite(line, budget, &mut map)?
            } else {
                let tag = line.split(':').next().unwrap_or("");
                match tag {
                    "#EXTM3U"
                    | "#EXT-X-ENDLIST"
                    | "#EXT-X-DISCONTINUITY"
                    | "#EXT-X-INDEPENDENT-SEGMENTS" => {
                        if tag != line {
                            return Err(fail(Failure::Unsupported));
                        }
                        line.to_owned()
                    }
                    "#EXTINF" => {
                        let duration = line
                            .strip_prefix("#EXTINF:")
                            .and_then(|s| s.split_once(','))
                            .map(|(s, _)| s)
                            .ok_or_else(|| fail(Failure::Unsupported))?;
                        if duration.len() > 32
                            || !duration.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                        {
                            return Err(fail(Failure::Unsupported));
                        }
                        format!("#EXTINF:{duration},")
                    }
                    "#EXT-X-VERSION"
                    | "#EXT-X-TARGETDURATION"
                    | "#EXT-X-MEDIA-SEQUENCE"
                    | "#EXT-X-DISCONTINUITY-SEQUENCE" => {
                        let value = line.split_once(':').map(|(_, v)| v).unwrap_or("");
                        if value.is_empty()
                            || value.len() > 20
                            || !value.bytes().all(|b| b.is_ascii_digit())
                        {
                            return Err(fail(Failure::Unsupported));
                        }
                        line.to_owned()
                    }
                    "#EXT-X-PLAYLIST-TYPE" => {
                        if line != "#EXT-X-PLAYLIST-TYPE:VOD" {
                            return Err(fail(Failure::Unsupported));
                        }
                        line.to_owned()
                    }
                    "#EXT-X-ALLOW-CACHE" => {
                        if !matches!(line, "#EXT-X-ALLOW-CACHE:YES" | "#EXT-X-ALLOW-CACHE:NO") {
                            return Err(fail(Failure::Unsupported));
                        }
                        line.to_owned()
                    }
                    "#EXT-X-BYTERANGE" => {
                        let value = line.split_once(':').map(|(_, v)| v).unwrap_or("");
                        if value.is_empty()
                            || value.len() > 64
                            || !value.bytes().all(|b| b.is_ascii_digit() || b == b'@')
                        {
                            return Err(fail(Failure::Unsupported));
                        }
                        line.to_owned()
                    }
                    "#EXT-X-MEDIA"
                    | "#EXT-X-I-FRAME-STREAM-INF"
                    | "#EXT-X-MAP"
                    | "#EXT-X-STREAM-INF" => {
                        let attrs = line
                            .split_once(':')
                            .map(|(_, v)| v)
                            .ok_or_else(|| fail(Failure::Unsupported))?;
                        let mut attributes = parse_attributes(attrs)?;
                        let mut uri = false;
                        for (key, value, quoted) in &mut attributes {
                            if key == "URI" {
                                if !*quoted {
                                    return Err(fail(Failure::Unsupported));
                                }
                                *value = rewrite(value, budget, &mut map)?;
                                uri = true;
                            } else if !matches!(
                                key.as_str(),
                                "TYPE"
                                    | "GROUP-ID"
                                    | "NAME"
                                    | "LANGUAGE"
                                    | "ASSOC-LANGUAGE"
                                    | "DEFAULT"
                                    | "AUTOSELECT"
                                    | "FORCED"
                                    | "INSTREAM-ID"
                                    | "CHARACTERISTICS"
                                    | "CHANNELS"
                                    | "BANDWIDTH"
                                    | "AVERAGE-BANDWIDTH"
                                    | "VIDEO-RANGE"
                                    | "CODECS"
                                    | "RESOLUTION"
                                    | "FRAME-RATE"
                                    | "AUDIO"
                                    | "VIDEO"
                                    | "SUBTITLES"
                                    | "CLOSED-CAPTIONS"
                                    | "BYTERANGE"
                            ) || (key == "VIDEO-RANGE" && (value != "SDR" || *quoted))
                            {
                                return Err(fail(Failure::Unsupported));
                            } else if value.contains(auth.token.expose_for_private_store())
                                || value.to_ascii_lowercase().contains("api_key")
                                || value.contains("://")
                            {
                                return Err(fail(Failure::ResourceRejected));
                            }
                        }
                        if !uri && matches!(tag, "#EXT-X-MAP" | "#EXT-X-I-FRAME-STREAM-INF") {
                            return Err(fail(Failure::Unsupported));
                        }
                        format!(
                            "{tag}:{}",
                            attributes
                                .into_iter()
                                .map(|(k, v, q)| if q {
                                    format!("{k}=\"{v}\"")
                                } else {
                                    format!("{k}={v}")
                                })
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                    // Includes encryption, session keys, variables, low-latency hints,
                    // external session data, date ranges, and unknown URI-bearing extensions.
                    _ => return Err(fail(Failure::Unsupported)),
                }
            };
            if output.len() + rewritten.len() + 1 > PLAYLIST_LIMIT * 2 {
                return Err(fail(Failure::Bounds));
            }
            output.push_str(&rewritten);
            output.push('\n');
        }
        Ok(output)
    }
}
fn validate_playlist_form(text: &str) -> Result<bool> {
    let mut master = false;
    let mut media = false;
    let mut pending_variant = false;
    let mut pending_segment = false;
    let mut variants = 0usize;
    let mut segments = 0usize;
    let mut end = false;
    for line in text.lines().skip(1).filter(|line| !line.is_empty()) {
        if end {
            return Err(fail(Failure::Unsupported));
        }
        let tag = line.split(':').next().unwrap_or("");
        match tag {
            "#EXT-X-STREAM-INF" => {
                if pending_variant || pending_segment {
                    return Err(fail(Failure::Unsupported));
                }
                master = true;
                pending_variant = true;
            }
            "#EXT-X-MEDIA" | "#EXT-X-I-FRAME-STREAM-INF" => {
                if pending_variant || pending_segment {
                    return Err(fail(Failure::Unsupported));
                }
                master = true;
                if tag == "#EXT-X-I-FRAME-STREAM-INF" {
                    variants += 1;
                }
            }
            "#EXTINF" => {
                if pending_variant || pending_segment {
                    return Err(fail(Failure::Unsupported));
                }
                media = true;
                pending_segment = true;
            }
            "#EXT-X-ENDLIST" => {
                media = true;
                end = true;
            }
            "#EXT-X-MAP"
            | "#EXT-X-TARGETDURATION"
            | "#EXT-X-MEDIA-SEQUENCE"
            | "#EXT-X-DISCONTINUITY-SEQUENCE"
            | "#EXT-X-DISCONTINUITY"
            | "#EXT-X-PLAYLIST-TYPE"
            | "#EXT-X-ALLOW-CACHE"
            | "#EXT-X-BYTERANGE" => {
                if pending_variant {
                    return Err(fail(Failure::Unsupported));
                }
                media = true;
            }
            "#EXT-X-VERSION" | "#EXT-X-INDEPENDENT-SEGMENTS" => {
                if pending_variant || pending_segment {
                    return Err(fail(Failure::Unsupported));
                }
            }
            _ if !line.starts_with('#') => {
                if pending_variant {
                    pending_variant = false;
                    variants += 1;
                } else if pending_segment {
                    pending_segment = false;
                    segments += 1;
                } else {
                    return Err(fail(Failure::Unsupported));
                }
            }
            _ => return Err(fail(Failure::Unsupported)),
        }
        if master && media {
            return Err(fail(Failure::Unsupported));
        }
    }
    if pending_variant
        || pending_segment
        || (!master && (!media || !end || segments == 0))
        || (master && variants == 0)
    {
        return Err(fail(Failure::Unsupported));
    }
    Ok(master)
}
fn parse_attributes(input: &str) -> Result<Vec<(String, String, bool)>> {
    let mut result = Vec::new();
    let mut rest = input;
    let mut keys = std::collections::BTreeSet::new();
    while !rest.is_empty() {
        if result.len() >= 32 {
            return Err(fail(Failure::Bounds));
        }
        let (key, value) = rest
            .split_once('=')
            .ok_or_else(|| fail(Failure::Unsupported))?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_uppercase() || b == b'-')
            || !keys.insert(key.to_owned())
        {
            return Err(fail(Failure::Unsupported));
        }
        let (value, tail, quoted) = if let Some(value) = value.strip_prefix('"') {
            let (value, tail) = value
                .split_once('"')
                .ok_or_else(|| fail(Failure::Unsupported))?;
            (value, tail, true)
        } else {
            let (value, tail) = value
                .split_once(',')
                .map(|(v, t)| (v, &rest[rest.len() - t.len() - 1..]))
                .unwrap_or((value, ""));
            (value, tail, false)
        };
        if value.is_empty() || value.contains(['"', '\\']) {
            return Err(fail(Failure::Unsupported));
        }
        result.push((key.into(), value.into(), quoted));
        if tail.is_empty() {
            break;
        }
        rest = tail
            .strip_prefix(',')
            .filter(|v| !v.is_empty())
            .ok_or_else(|| fail(Failure::Unsupported))?;
    }
    Ok(result)
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[tokio::test]
    async fn oversized_frame_drains_before_fetching_again_and_empty_frames_yield() {
        let original: Vec<_> = (0..CHUNK_LIMIT * 3 + 17).map(|n| n as u8).collect();
        let mut pending = bytes::Bytes::new();
        let mut polls = 0;
        let mut reconstructed = Vec::new();
        for expected_polls in [1, 1, 1, 1, 2] {
            let next = bounded_frame_chunk(&mut pending, async {
                polls += 1;
                Ok((polls == 1).then(|| bytes::Bytes::copy_from_slice(&original)))
            })
            .await
            .unwrap();
            assert_eq!(polls, expected_polls);
            if let Some(chunk) = next {
                assert!(chunk.len() <= CHUNK_LIMIT);
                reconstructed.extend(chunk);
            } else {
                assert_eq!(reconstructed, original);
            }
        }
        assert!(pending.is_empty());
        let empty = bounded_frame_chunk(&mut pending, async {
            polls += 1;
            Ok(Some(bytes::Bytes::new()))
        })
        .await
        .unwrap();
        assert_eq!(empty, Some(Vec::new()));
        assert_eq!(
            polls, 3,
            "empty frames must not start an internal polling loop"
        );
    }

    #[tokio::test]
    async fn dropping_pending_stream_releases_its_frame() {
        let storage = bytes::Bytes::from(vec![7; CHUNK_LIMIT * 2]);
        let mut stream = MediaStream {
            status: 200,
            headers: header::HeaderMap::new(),
            response: None,
            pending: storage.clone(),
            idle_timeout: Duration::from_secs(1),
        };
        assert!(!storage.is_unique());
        let chunk = bounded_frame_chunk(&mut stream.pending, async { unreachable!() })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunk.len(), CHUNK_LIMIT);
        drop(stream);
        assert!(storage.is_unique());
    }
}
