// Generated domain records; field names match Rust and the gRPC contracts.
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
export interface ItemPage {
  items: Array<Item>;
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
  subtitle_preference: string;
}
