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
  artwork: Array<Artwork>;
  notices: Array<CatalogNotice>;
  retrieved_at: string;
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
export interface Preferences {
  search_types: SearchTypes | null;
  search_methods: SearchMethods | null;
  library_id: string;
  source: string;
  subtitle_preference: string;
}
export interface SearchRequest {
  query: string;
  library_id: string;
  types: SearchTypes | null;
  methods: SearchMethods | null;
  source: string;
  page: Page | null;
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
export interface ClientConfig {
  endpoint: string;
  mock_profile: string;
  timeout_ms: number;
}
