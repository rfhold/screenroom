use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Milliseconds(u64);
impl Milliseconds {
    pub const ZERO: Self = Self(0);
    pub fn parse_decimal(value: &str) -> Result<Self> {
        if value.is_empty() || value.len() > 16 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(fail(Failure::InvalidInput));
        }
        let value = value
            .parse::<u64>()
            .map_err(|_| fail(Failure::InvalidInput))?;
        if value > (i64::MAX / 10_000) as u64 {
            return Err(fail(Failure::Bounds));
        }
        Ok(Self(value))
    }
    pub fn from_ticks(ticks: i64) -> Result<Self> {
        u64::try_from(ticks)
            .map(|n| Self(n / 10_000))
            .map_err(|_| fail(Failure::InvalidResponse))
    }
    pub fn decimal(self) -> String {
        self.0.to_string()
    }
    pub fn as_u64(self) -> u64 {
        self.0
    }
    pub fn ticks(self) -> i64 {
        (self.0 * 10_000) as i64
    }
    /// Browser time in a reset-timestamp transcode is relative to its single offset.
    pub fn provider_position(self, browser: Self) -> Result<Self> {
        let sum = self
            .0
            .checked_add(browser.0)
            .ok_or_else(|| fail(Failure::Bounds))?;
        Self::parse_decimal(&sum.to_string())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClientCapabilities {
    pub mp4: bool,
    pub h264: bool,
    pub aac: bool,
    pub native_hls: bool,
    pub mse_hls: bool,
    pub webvtt: bool,
}
impl ClientCapabilities {
    fn video(self) -> bool {
        self.mp4 && self.h264 && self.aac
    }
    fn hls(self) -> bool {
        self.video() && (self.native_hls || self.mse_hls)
    }
    fn device_profile(self) -> Value {
        let direct = if self.video() {
            vec![
                json!({"Container":"mp4,m4v","Type":"Video","VideoCodec":"h264","AudioCodec":"aac"}),
            ]
        } else {
            vec![]
        };
        let transcode = if self.hls() {
            vec![
                json!({"Container":"ts","Type":"Video","VideoCodec":"h264","AudioCodec":"aac",
                "Protocol":"hls","Context":"Streaming","MaxAudioChannels":"2","MinSegments":2,
                "BreakOnNonKeyFrames":false,"CopyTimestamps":false}),
            ]
        } else {
            vec![]
        };
        let mut subtitles: Vec<_> = ["ass", "ssa", "pgssub", "dvdsub", "dvbsub"]
            .into_iter()
            .map(|format| json!({"Format":format,"Method":"Encode"}))
            .collect();
        if self.webvtt {
            subtitles.push(json!({"Format":"vtt","Method":"External"}));
        }
        json!({"Name":"Screenroom conservative browser","MaxStreamingBitrate":20_000_000,
            "DirectPlayProfiles":direct,"TranscodingProfiles":transcode,
            "CodecProfiles":[{"Type":"Video","Codec":"h264","Conditions":[
                {"Condition":"LessThanEqual","Property":"VideoBitDepth","Value":"8","IsRequired":true},
                {"Condition":"LessThanEqual","Property":"VideoLevel","Value":"41","IsRequired":true}
            ]}],
            "SubtitleProfiles":subtitles})
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMethod {
    DirectPlay,
    DirectStream,
    Transcode,
}
impl PlaybackMethod {
    pub(super) fn provider_name(self) -> &'static str {
        match self {
            Self::DirectPlay => "DirectPlay",
            Self::DirectStream => "DirectStream",
            Self::Transcode => "Transcode",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Subtitle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMode {
    WebVtt,
    BurnIn,
    Unavailable,
}

#[derive(Clone)]
pub struct Track {
    pub kind: TrackKind,
    pub display_name: String,
    pub language: Option<String>,
    pub selected: bool,
    pub available: bool,
    pub subtitle_mode: Option<SubtitleMode>,
    pub resource: Option<ResourceRef>,
    pub(super) index: i32,
    pub(super) scope: ResourceScope,
}
impl std::fmt::Debug for Track {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Track")
            .field("kind", &self.kind)
            .field("selected", &self.selected)
            .field("available", &self.available)
            .field("subtitle_mode", &self.subtitle_mode)
            .finish()
    }
}
#[derive(Debug, Clone)]
pub struct TrackSelection {
    scope: ResourceScope,
    pub(super) audio: Option<i32>,
    subtitle: Option<i32>,
    pub(super) burn_in: bool,
    audio_requires_remux: bool,
    original_audio_index: Option<i32>,
}
#[derive(Debug)]
pub struct NegotiationRequest<'a> {
    pub item_id: &'a str,
    pub capabilities: ClientCapabilities,
    pub position: Milliseconds,
    pub selection: Option<&'a TrackSelection>,
}
/// Persist `compatible_used` before invoking a forced negotiation. This budget has no
/// sequence/receipt policy; the durable gateway must not recreate it to bypass the limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct NegotiationBudget {
    compatible_used: bool,
}
impl NegotiationBudget {
    pub fn from_recorded(compatible_used: bool) -> Self {
        Self { compatible_used }
    }
    pub fn compatible_used(&self) -> bool {
        self.compatible_used
    }
    pub fn reserve_compatible(&mut self) -> Result<CompatibleTranscodePermit> {
        if self.compatible_used {
            return Err(fail(Failure::Bounds));
        }
        self.compatible_used = true;
        Ok(CompatibleTranscodePermit(()))
    }
}
#[derive(Debug)]
pub struct CompatibleTranscodePermit(());

#[derive(Debug)]
pub struct NegotiatedPlayback {
    pub method: PlaybackMethod,
    pub mime_type: &'static str,
    pub duration: Option<Milliseconds>,
    pub start_position: Milliseconds,
    pub timeline_offset: Milliseconds,
    pub resource: ResourceRef,
    pub tracks: Vec<Track>,
    pub source_display_name: String,
    pub(super) scope: ResourceScope,
    pub(super) audio_index: Option<i32>,
    pub(super) subtitle_index: Option<i32>,
    pub(super) default_audio_index: Option<i32>,
}
impl NegotiatedPlayback {
    pub fn encoding_identity(&self) -> EncodingIdentity {
        EncodingIdentity {
            device: self.scope.device.clone(),
            play_session: self.scope.play_session.clone(),
        }
    }
    pub fn private_source_id(&self) -> &str {
        &self.scope.source
    }
    pub fn private_play_session_id(&self) -> &str {
        &self.scope.play_session
    }
    pub fn selection(
        &self,
        audio: Option<&Track>,
        subtitle: Option<&Track>,
    ) -> Result<TrackSelection> {
        for (track, kind) in [(audio, TrackKind::Audio), (subtitle, TrackKind::Subtitle)] {
            if track.is_some_and(|t| t.scope != self.scope || t.kind != kind || !t.available) {
                return Err(fail(Failure::InvalidInput));
            }
        }
        Ok(TrackSelection {
            scope: self.scope.clone(),
            audio: audio.map(|t| t.index).or(self.audio_index),
            subtitle: subtitle.map(|t| t.index),
            burn_in: subtitle.is_some_and(|t| t.subtitle_mode == Some(SubtitleMode::BurnIn)),
            audio_requires_remux: audio.map(|t| t.index).or(self.audio_index)
                != self.default_audio_index,
            original_audio_index: self.default_audio_index,
        })
    }
    pub(super) fn check_auth(&self, auth: &SessionAuth) -> Result<()> {
        self.scope.check_auth(auth)
    }
}
impl PlaybackAdapter {
    pub async fn negotiate(
        &self,
        auth: &SessionAuth,
        request: NegotiationRequest<'_>,
    ) -> Result<NegotiatedPlayback> {
        self.negotiate_inner(auth, request, false).await
    }
    pub async fn compatible_transcode(
        &self,
        auth: &SessionAuth,
        request: NegotiationRequest<'_>,
        _permit: CompatibleTranscodePermit,
    ) -> Result<NegotiatedPlayback> {
        self.negotiate_inner(auth, request, true).await
    }
    async fn negotiate_inner(
        &self,
        auth: &SessionAuth,
        request: NegotiationRequest<'_>,
        forced: bool,
    ) -> Result<NegotiatedPlayback> {
        id(request.item_id)?;
        if !request.capabilities.video() || (forced && !request.capabilities.hls()) {
            return Err(fail(Failure::Unsupported));
        }
        if let Some(selection) = request.selection {
            selection.scope.check_auth(auth)?;
            if selection.scope.item != request.item_id {
                return Err(fail(Failure::InvalidInput));
            }
        }
        let body = self
            .user_control(
                auth,
                Method::GET,
                &[
                    "Users",
                    &auth.intent.personal_profile,
                    "Items",
                    request.item_id,
                ],
                &[("Fields", "MediaSources,MediaStreams")],
                None,
                false,
            )
            .await?;
        let item: Value = self.decode(&body, false)?;
        if item["Id"].as_str() != Some(request.item_id)
            || !matches!(item["Type"].as_str(), Some("Movie" | "Episode"))
            || item["PartCount"].as_u64().is_some_and(|n| n > 1)
            || item["IsLive"].as_bool() == Some(true)
            || item["IsEncrypted"].as_bool() == Some(true)
        {
            return Err(fail(Failure::Unsupported));
        }
        let burn = request.selection.is_some_and(|s| s.burn_in);
        let audio_choice = request.selection.is_some_and(|s| s.audio_requires_remux);
        if burn && !request.capabilities.hls() {
            return Err(fail(Failure::Unsupported));
        }
        // Source discovery is read-only. Jellyfin only applies explicit subtitle/audio
        // choices when MediaSourceId matches, including the initial subtitles-off choice.
        let source_id = if let Some(selection) = request.selection {
            selection.scope.source.clone()
        } else {
            let sources = item["MediaSources"]
                .as_array()
                .ok_or_else(|| fail(Failure::InvalidResponse))?;
            if sources.is_empty() || sources.len() > 16 {
                return Err(fail(Failure::Bounds));
            }
            let mut sources: Vec<SourceDto> = sources
                .iter()
                .filter_map(|value| serde_json::from_value(value.clone()).ok())
                .filter(|source: &SourceDto| source.validate().is_ok())
                .collect();
            sources.sort_by_key(|source| {
                if source.browser_direct()
                    && matches!(source.container.as_deref(), Some("mp4" | "m4v"))
                {
                    0
                } else if source.browser_video() {
                    1
                } else {
                    2
                }
            });
            sources
                .into_iter()
                .next()
                .ok_or_else(|| fail(Failure::Unsupported))?
                .id
        };
        let body = json!({
            "UserId":auth.intent.personal_profile,"DeviceProfile":request.capabilities.device_profile(),
            "StartTimeTicks":request.position.ticks(),
            "MediaSourceId":source_id,
            "AudioStreamIndex":request.selection.and_then(|s|s.audio),
            "SubtitleStreamIndex":request.selection.and_then(|s|s.subtitle).unwrap_or(-1),
            "AutoOpenLiveStream":false,"EnableDirectPlay":!forced && !burn && !audio_choice,
            "EnableDirectStream":!forced && !burn,"EnableTranscoding":request.capabilities.hls(),
            "AllowVideoStreamCopy":!forced && !burn,"AllowAudioStreamCopy":!forced,
            "AlwaysBurnInSubtitleWhenTranscoding":burn,"MaxStreamingBitrate":20_000_000,
            "MaxAudioChannels":2
        });
        let body = self
            .user_control(
                auth,
                Method::POST,
                &["Items", request.item_id, "PlaybackInfo"],
                &[],
                Some(body),
                true,
            )
            .await?;
        let info: InfoDto = self.decode(&body, true)?;
        if info.error_code.is_some() {
            return Err(fail(Failure::Unsupported));
        }
        id(&info.play_session_id).map_err(|_| fail(Failure::InvalidResponse))?;
        if info.media_sources.is_empty() || info.media_sources.len() > 16 {
            return Err(fail(Failure::Bounds));
        }
        let mut candidates = Vec::new();
        for source in info.media_sources {
            if source.validate().is_err() {
                continue;
            }
            if source.id != source_id {
                continue;
            }
            let method = if !forced
                && !burn
                && !audio_choice
                && source.supports_direct_play
                && source.browser_direct()
            {
                PlaybackMethod::DirectPlay
            } else if request.capabilities.hls()
                && !forced
                && !burn
                && source.browser_video()
                && (source.supports_transcoding || source.supports_direct_stream)
                && source.transcoding_url.as_ref().is_some_and(|url| {
                    url.split('?').nth(1).is_some_and(|query| {
                        query
                            .split('&')
                            .any(|p| p.eq_ignore_ascii_case("VideoCodec=copy"))
                    })
                })
            {
                PlaybackMethod::DirectStream
            } else if request.capabilities.hls()
                && source.supports_transcoding
                && source.transcoding_url.is_some()
            {
                PlaybackMethod::Transcode
            } else {
                continue;
            };
            candidates.push((method, source));
        }
        candidates.sort_by_key(|(method, _)| match method {
            PlaybackMethod::DirectPlay => 0,
            PlaybackMethod::DirectStream => 1,
            PlaybackMethod::Transcode => 2,
        });
        let (method, source) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| fail(Failure::Unsupported))?;
        let desired_subtitle = request.selection.and_then(|selection| selection.subtitle);
        let subtitle_matches = match (desired_subtitle, source.default_subtitle_stream_index) {
            (None, None | Some(-1)) => true,
            (Some(desired), Some(actual)) => desired == actual,
            _ => false,
        };
        if !subtitle_matches {
            return Err(fail(Failure::Unsupported));
        }
        if request
            .selection
            .and_then(|selection| selection.audio)
            .is_some_and(|audio| source.default_audio_stream_index != Some(audio))
        {
            return Err(fail(Failure::Unsupported));
        }
        let duration = source
            .run_time_ticks
            .map(Milliseconds::from_ticks)
            .transpose()?;
        if duration.is_some_and(|d| d < request.position) {
            return Err(fail(Failure::InvalidInput));
        }
        let scope = ResourceScope {
            device: auth.intent.device.clone(),
            server: auth.intent.server.clone(),
            profile: auth.intent.personal_profile.clone(),
            item: request.item_id.into(),
            source: source.id.clone(),
            play_session: info.play_session_id,
        };
        let resource = if method == PlaybackMethod::DirectPlay {
            if !matches!(source.container.as_deref(), Some("mp4" | "m4v")) {
                return Err(fail(Failure::Unsupported));
            }
            let mut url = self.url(&["Videos", request.item_id, "stream.mp4"])?;
            url.query_pairs_mut()
                .append_pair("static", "true")
                .append_pair("mediaSourceId", &scope.source)
                .append_pair("deviceId", &scope.device.0)
                .append_pair("playSessionId", &scope.play_session);
            self.resource(&scope, &url, url.as_str(), 0)?
        } else {
            let raw = source
                .transcoding_url
                .as_deref()
                .ok_or_else(|| fail(Failure::Unsupported))?;
            // Jellyfin v10.10.7 emits an API-relative "/videos/..." path without its
            // deployment basepath. Only negotiation may anchor that form under our base.
            let raw = if raw.starts_with('/')
                && !raw.starts_with("//")
                && !raw.starts_with(self.base_directory()?.path())
            {
                raw.trim_start_matches('/')
            } else {
                raw
            };
            let mut resource = self.resource(&scope, &self.base_directory()?, raw, 0)?;
            if resource.kind != ResourceKind::Playlist {
                return Err(fail(Failure::Unsupported));
            }
            if resource.url.query_pairs().any(|(k, v)| {
                k.eq_ignore_ascii_case("StartTimeTicks") && v != "0"
                    || k.eq_ignore_ascii_case("CopyTimestamps") && !v.eq_ignore_ascii_case("true")
            }) {
                return Err(fail(Failure::Unsupported));
            }
            let query: std::collections::BTreeMap<_, _> = resource
                .url
                .query_pairs()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
                .collect();
            let expected_video = if method == PlaybackMethod::DirectStream {
                "copy"
            } else {
                "h264"
            };
            if query.get("videocodec").map(String::as_str) != Some(expected_video)
                || query.get("audiocodec").is_some_and(|codec| codec != "aac")
                || (burn
                    && (query.get("subtitlemethod").map(String::as_str) != Some("Encode")
                        || query
                            .get("subtitlestreamindex")
                            .and_then(|n| n.parse::<i32>().ok())
                            != request.selection.and_then(|s| s.subtitle)))
                || (!burn
                    && (query
                        .get("subtitlestreamindex")
                        .is_some_and(|index| index != "-1")
                        || query.get("subtitlemethod").is_some_and(|method| {
                            method != "External"
                                && !(desired_subtitle.is_none() && method == "Encode")
                        })))
                || (audio_choice
                    && query
                        .get("audiostreamindex")
                        .and_then(|n| n.parse::<i32>().ok())
                        != request.selection.and_then(|s| s.audio))
            {
                return Err(fail(Failure::Unsupported));
            }
            if desired_subtitle.is_none() && !burn {
                // Jellyfin omits the -1 index but can retain Encode. Pin off before media IO.
                let mut pairs: Vec<_> = resource
                    .url
                    .query_pairs()
                    .filter(|(key, _)| !key.eq_ignore_ascii_case("SubtitleStreamIndex"))
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect();
                pairs.push(("SubtitleStreamIndex".into(), "-1".into()));
                resource.url.set_query(None);
                resource.url.query_pairs_mut().extend_pairs(pairs);
            }
            resource
        };
        let audio_index = request
            .selection
            .and_then(|s| s.audio)
            .or(source.default_audio_stream_index);
        let subtitle_index = request.selection.and_then(|s| s.subtitle);
        let mut tracks = Vec::new();
        for stream in source.media_streams {
            if stream.index < 0 || stream.index > 4096 {
                return Err(fail(Failure::InvalidResponse));
            }
            let kind = match stream.kind.as_str() {
                "Audio" => TrackKind::Audio,
                "Subtitle" => TrackKind::Subtitle,
                _ => continue,
            };
            let mode = if kind == TrackKind::Subtitle {
                Some(if stream.is_external_url {
                    SubtitleMode::Unavailable
                } else if request.capabilities.webvtt
                    && stream.supports_external_stream
                    && matches!(
                        stream.codec.as_deref(),
                        Some("srt" | "subrip" | "vtt" | "webvtt" | "mov_text" | "text")
                    )
                {
                    SubtitleMode::WebVtt
                } else if request.capabilities.hls() && source.supports_transcoding {
                    SubtitleMode::BurnIn
                } else {
                    SubtitleMode::Unavailable
                })
            } else {
                None
            };
            let subtitle_resource = if mode == Some(SubtitleMode::WebVtt) {
                let mut url = self.url(&[
                    "Videos",
                    request.item_id,
                    &scope.source,
                    "Subtitles",
                    &stream.index.to_string(),
                    "Stream.vtt",
                ])?;
                // Full-file WebVTT uses the provider's original timeline, never the HLS offset.
                url.query_pairs_mut()
                    .append_pair("copyTimestamps", "true")
                    .append_pair("addVttTimeMap", "false");
                Some(self.resource(&scope, &url, url.as_str(), 0)?)
            } else {
                None
            };
            let language = stream.language.as_deref().and_then(track_language);
            let display_name = stream.display_name(language.as_deref(), kind);
            if tracks.iter().any(|t: &Track| t.index == stream.index) {
                return Err(fail(Failure::InvalidResponse));
            }
            tracks.push(Track {
                kind,
                display_name,
                language,
                selected: if kind == TrackKind::Audio {
                    audio_index == Some(stream.index)
                } else {
                    subtitle_index == Some(stream.index)
                },
                available: if kind == TrackKind::Audio {
                    request.capabilities.hls() || audio_index == Some(stream.index)
                } else {
                    mode != Some(SubtitleMode::Unavailable)
                },
                subtitle_mode: mode,
                resource: subtitle_resource,
                index: stream.index,
                scope: scope.clone(),
            });
        }
        let labels: Vec<_> = tracks
            .iter()
            .map(|track| track.display_name.clone())
            .collect();
        for (position, track) in tracks.iter_mut().enumerate() {
            if labels
                .iter()
                .filter(|label| **label == track.display_name)
                .count()
                > 1
            {
                let ordinal = labels[..=position]
                    .iter()
                    .filter(|label| **label == track.display_name)
                    .count();
                track.display_name.push_str(&format!(" · Track {ordinal}"));
            }
        }
        for (index, kind) in [
            (audio_index, TrackKind::Audio),
            (subtitle_index, TrackKind::Subtitle),
        ] {
            if index.is_some_and(|i| {
                !tracks
                    .iter()
                    .any(|t| t.index == i && t.kind == kind && t.available)
            }) {
                return Err(fail(Failure::Unsupported));
            }
        }
        Ok(NegotiatedPlayback {
            method,
            mime_type: if method == PlaybackMethod::DirectPlay {
                "video/mp4"
            } else {
                "application/vnd.apple.mpegurl"
            },
            duration,
            start_position: request.position,
            // Full VOD HLS retains the original timeline (StreamInfo.BuildParams).
            // The player seeks to start_position; adding it again would double resume.
            timeline_offset: Milliseconds::ZERO,
            resource,
            tracks,
            source_display_name: "Media".into(),
            scope,
            audio_index,
            subtitle_index,
            // PlaybackInfo replaces its default with the requested selection, not the file default.
            default_audio_index: request
                .selection
                .map(|selection| selection.original_audio_index)
                .unwrap_or(source.default_audio_stream_index),
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InfoDto {
    media_sources: Vec<SourceDto>,
    play_session_id: String,
    error_code: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SourceDto {
    id: String,
    protocol: String,
    container: Option<String>,
    run_time_ticks: Option<i64>,
    #[serde(default)]
    supports_direct_play: bool,
    #[serde(default)]
    supports_direct_stream: bool,
    #[serde(default)]
    supports_transcoding: bool,
    transcoding_url: Option<String>,
    #[serde(default)]
    requires_opening: bool,
    #[serde(default)]
    is_infinite_stream: bool,
    #[serde(default)]
    is_remote: bool,
    #[serde(default)]
    is_encrypted: bool,
    live_stream_id: Option<String>,
    video_type: Option<String>,
    iso_type: Option<String>,
    path: Option<String>,
    #[serde(default)]
    required_http_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    media_streams: Vec<StreamDto>,
    default_audio_stream_index: Option<i32>,
    default_subtitle_stream_index: Option<i32>,
}
impl SourceDto {
    fn browser_video(&self) -> bool {
        let video: Vec<_> = self
            .media_streams
            .iter()
            .filter(|s| s.kind == "Video")
            .collect();
        video.len() == 1
            && video[0].codec.as_deref() == Some("h264")
            && video[0].bit_depth.is_none_or(|n| n <= 8)
            && video[0].level.is_none_or(|n| n <= 41.0)
    }
    fn browser_direct(&self) -> bool {
        self.browser_video()
            && self
                .media_streams
                .iter()
                .filter(|s| s.kind == "Audio")
                .all(|s| s.codec.as_deref() == Some("aac") && s.channels.is_none_or(|n| n <= 2))
    }
    fn validate(&self) -> Result<()> {
        id(&self.id)?;
        if self.protocol != "File"
            || self.requires_opening
            || self.is_infinite_stream
            || self.is_remote
            || self.is_encrypted
            || self.live_stream_id.is_some()
            || !self.required_http_headers.is_empty()
            || self.media_streams.len() > 128
            || self.video_type.as_deref().is_some_and(|t| t != "VideoFile")
            || self.iso_type.is_some()
            || self.path.as_ref().is_some_and(|p| p.contains("://"))
        {
            return Err(fail(Failure::Unsupported));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StreamDto {
    index: i32,
    #[serde(rename = "Type")]
    kind: String,
    codec: Option<String>,
    language: Option<String>,
    #[serde(default)]
    supports_external_stream: bool,
    #[serde(default)]
    is_external_url: bool,
    bit_depth: Option<u32>,
    level: Option<f64>,
    channels: Option<u32>,
    #[serde(default)]
    is_forced: bool,
    #[serde(default)]
    is_hearing_impaired: bool,
    title: Option<String>,
}

// A bounded language/script/region subset; no private-use or arbitrary variant text.
fn track_language(value: &str) -> Option<String> {
    if value.len() > 12 {
        return None;
    }
    let mut parts = value.split('-');
    let language = parts.next()?;
    if !matches!(language.len(), 2 | 3) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut result = language.to_ascii_lowercase();
    let mut next = parts.next();
    if next.is_some_and(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic())) {
        result.push('-');
        result.push_str(&next?.to_ascii_lowercase());
        next = parts.next();
    }
    if let Some(region) = next {
        if !(region.len() == 2 && region.bytes().all(|b| b.is_ascii_alphabetic())
            || region.len() == 3 && region.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        result.push('-');
        result.push_str(&region.to_ascii_lowercase());
    }
    if parts.next().is_some() {
        return None;
    }
    Some(result)
}

impl StreamDto {
    fn display_name(&self, language: Option<&str>, kind: TrackKind) -> String {
        let mut label = match language {
            Some("en" | "eng") => "English".into(),
            Some("fr" | "fra" | "fre") => "French".into(),
            Some("de" | "deu" | "ger") => "German".into(),
            Some("es" | "spa") => "Spanish".into(),
            Some("ja" | "jpn") => "Japanese".into(),
            Some("it" | "ita") => "Italian".into(),
            Some("pt" | "por") => "Portuguese".into(),
            Some("zh" | "zho" | "chi") => "Chinese".into(),
            Some(value) => value.to_owned(),
            None => "Unknown language".into(),
        };
        let codec = match self
            .codec
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("aac") => "AAC",
            Some("ac3") => "AC-3",
            Some("eac3") => "E-AC-3",
            Some("dts") => "DTS",
            Some("truehd") => "TrueHD",
            Some("flac") => "FLAC",
            Some("opus") => "Opus",
            Some("vorbis") => "Vorbis",
            Some("mp3") => "MP3",
            Some("srt" | "subrip") => "SRT",
            Some("vtt" | "webvtt") => "WebVTT",
            Some("ass") => "ASS",
            Some("ssa") => "SSA",
            Some("pgssub") => "PGS",
            Some("dvdsub") => "DVD subtitles",
            Some("dvbsub") => "DVB subtitles",
            Some("mov_text" | "text") => "Text",
            _ => "Unknown codec",
        };
        label.push_str(&format!(" · {codec}"));
        if kind == TrackKind::Audio
            && let Some(channels @ 1..=32) = self.channels
        {
            label.push_str(&format!(" · {channels} channels"));
        }
        if self.is_forced {
            label.push_str(" · Forced");
        }
        if self.is_hearing_impaired {
            label.push_str(" · SDH");
        }
        // Only a complete categorical title is recognized; never echo provider text.
        if self.title.as_deref().is_some_and(|title| {
            title.eq_ignore_ascii_case("commentary")
                || title.eq_ignore_ascii_case("audio commentary")
        }) {
            label.push_str(" · Commentary");
        }
        label
    }
}
