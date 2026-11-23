// Generated domain records; field names match Rust and the gRPC contracts.
export type PlaybackStartPolicy = "unspecified" | "resume" | "beginning" | "position";
export type PlaybackState = "unspecified" | "preparing" | "ready" | "playing" | "paused" | "stopping" | "stopped" | "expired" | "failed" | "uncertain";
export type PlaybackReportOutcome = "unspecified" | "not_sent" | "pending" | "confirmed" | "uncertain" | "failed";
export type PlaybackEventKind = "unspecified" | "started" | "progress" | "paused" | "resumed" | "seeked" | "heartbeat";
export type PlaybackMethod = "unspecified" | "direct_play" | "direct_stream" | "transcode";
export type PlaybackTrackKind = "unspecified" | "audio" | "subtitle";
export type PlaybackSubtitleMode = "unspecified" | "webvtt" | "burn_in" | "unavailable";
export type PlaybackErrorCode = "unspecified" | "profile_required" | "profile_changed" | "source_unavailable" | "item_unavailable" | "unsupported_media" | "transcoding_unavailable" | "playback_authorization_unavailable" | "playback_conflict" | "session_expired" | "operation_conflict" | "capacity_exhausted" | "upstream_unavailable";
export interface Rights {
  label: string;
  note: string;
  source: string;
  license_url: string;
  attribution: string;
}
export interface Artwork {
  id: string;
  url: string;
  label: string;
  creator: string;
  license: string;
  source: string;
  license_url: string;
  kind: string;
}
export interface Credit {
  id: string;
  name: string;
  role: string;
  department: string;
  artwork: Artwork | null;
}
export interface CatalogNotice {
  id: string;
  section: string;
  title: string;
  text: string;
  link_label: string;
  link_url: string;
}
export interface CatalogInfo {
  notices: Array<CatalogNotice>;
  retrieved_at: string;
  branding: Array<Artwork>;
}
export interface Rating {
  provider: string;
  value: number;
  scale: number;
  votes: number | null;
  retrieved_at: string;
}
export interface Runtime {
  seconds: number | null;
  minimum_seconds: number | null;
  maximum_seconds: number | null;
  note: string;
  display: string;
}
export interface ProviderReference {
  provider: string;
  status: string;
  url: string;
}
export interface Item {
  id: string;
  title: string;
  kind: string;
  parent_id: string | null;
  year: number;
  subtitle: string;
  overview: string;
  tags: Array<string>;
  colors: Array<string>;
  release_date: string;
  added_order: number;
  title_rights: Rights | null;
  metadata_rights: Rights | null;
  artwork: Artwork | null;
  rating: Rating | null;
  runtime: Runtime | null;
  provider_references: Array<ProviderReference>;
  progress_percent: number | null;
  provenance_notes: Array<string>;
  metadata_retrieved_at: string;
  watched: boolean | null;
}
export interface Page {
  size: number;
  token: string;
}
export interface MediaLibrary {
  id: string;
  title: string;
}
export interface HomeRow {
  id: string;
  title: string;
  items: Array<Item>;
  read_error: HomeRowReadError | null;
}
export interface HomeRowReadError {
  code: string;
  message: string;
}
export interface SearchRequest {
  query: string;
  library_id: string;
  types: SearchTypes | null;
  methods: SearchMethods | null;
  source: string;
  page: Page | null;
  source_id: string;
}
export interface SearchTypes {
  movie: boolean;
  series: boolean;
  episode: boolean;
}
export interface SearchMethods {
  title_only: boolean;
  text: boolean;
  fuzzy: boolean;
  semantic: boolean;
}
export interface SourceLibraryScope {
  source_id: string;
  library_id: string;
}
export interface User {
  id: string;
  display_name: string;
}
export interface JellyfinSource {
  id: string;
  display_name: string;
}
export interface JellyfinProfile {
  id: string;
  display_name: string;
}
export interface JellyfinLink {
  source_id: string;
  jellyfin_user_id: string;
}
export interface PlaybackClientCapabilities {
  mp4: boolean;
  h264: boolean;
  aac: boolean;
  native_hls: boolean;
  mse_hls: boolean;
  webvtt: boolean;
}
export interface PlaybackMediaSource {
  id: string;
  display_name: string;
}
export interface PlaybackTrack {
  id: string;
  kind: PlaybackTrackKind;
  display_name: string;
  language: string | null;
  selected: boolean;
  available: boolean;
  subtitle_mode: PlaybackSubtitleMode | null;
  media_url: string | null;
}
export interface PlaybackTrackChoices {
  audio_track_id: string | null;
  subtitle_track_id: string | null;
  subtitles_off: boolean;
}
export interface PlaybackTransport {
  method: PlaybackMethod;
  mime_type: string;
  media_url: string;
  revision: number;
  duration_ms: string | null;
  start_position_ms: string;
  timeline_offset_ms: string;
}
export interface PlaybackOperation {
  request_id: string | null;
  sequence: number | null;
  outcome: PlaybackReportOutcome;
}
export interface PlaybackSession {
  session_id: string;
  control_capability: string;
  state: PlaybackState;
  selected_source: PlaybackMediaSource | null;
  tracks: Array<PlaybackTrack>;
  transport: PlaybackTransport | null;
  transport_revision: number;
  last_accepted_sequence: number;
  report_outcome: PlaybackReportOutcome;
  lease_expires_at: string;
  expires_at: string;
  pending_operation: PlaybackOperation | null;
  cleanup_pending: boolean;
  failure_code: PlaybackErrorCode | null;
}
export interface CreatePlaybackSessionRequest {
  request_id: string;
  item_id: string;
  scope: SourceLibraryScope | null;
  client_capabilities: PlaybackClientCapabilities | null;
  start_policy: PlaybackStartPolicy;
  position_ms: string | null;
}
export interface CreatePlaybackSessionResponse {
  session: PlaybackSession | null;
}
export interface GetPlaybackSessionRequest {
  session_id: string;
  control_capability: string;
}
export interface GetPlaybackSessionResponse {
  session: PlaybackSession | null;
}
export interface UpdatePlaybackSessionRequest {
  session_id: string;
  control_capability: string;
  request_id: string;
  expected_transport_revision: number;
  position_ms: string;
  track_choices: PlaybackTrackChoices | null;
  compatible_transcode_fallback: boolean | null;
}
export interface UpdatePlaybackSessionResponse {
  session: PlaybackSession | null;
}
export interface ReportPlaybackEventRequest {
  session_id: string;
  control_capability: string;
  transport_revision: number;
  sequence: number;
  kind: PlaybackEventKind;
  position_ms: string;
  paused: boolean;
  muted: boolean;
  volume: number;
}
export interface ReportPlaybackEventResponse {
  accepted_sequence: number;
  report_outcome: PlaybackReportOutcome;
  session: PlaybackSession | null;
}
export interface StopPlaybackSessionRequest {
  session_id: string;
  control_capability: string;
  request_id: string;
  final_sequence: number;
  transport_revision: number;
  position_ms: string;
  stop_reason: string;
}
export interface StopPlaybackSessionResponse {
  session: PlaybackSession | null;
}
export interface ItemPage {
  items: Array<Item>;
  next_page_token: string;
}
export interface JellyfinSourcePage {
  sources: Array<JellyfinSource>;
  next_page_token: string;
}
export interface JellyfinProfilePage {
  profiles: Array<JellyfinProfile>;
  next_page_token: string;
}
export interface CreditPage {
  credits: Array<Credit>;
  next_page_token: string;
}
export interface SearchCapabilities {
  methods: Array<string>;
  types: Array<string>;
  sources: Array<string>;
  semantic_description: string;
  maximum_query_length: number;
}
export interface LibraryPage {
  libraries: Array<MediaLibrary>;
  next_page_token: string;
}
export interface ArtworkPage {
  artwork: Array<Artwork>;
  next_page_token: string;
}
export interface SearchPage {
  items: Array<Item>;
  next_page_token: string;
  effective_methods: SearchMethods | null;
  degradation_reason: string;
}
export interface ClientConfig {
  endpoint: string;
  user_id: string;
  timeout_ms: number;
}
export interface ClientSettings {
  scope: SourceLibraryScope | null;
  source: string;
  types: SearchTypes | null;
  methods: SearchMethods | null;
}
