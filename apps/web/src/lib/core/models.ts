export type Kind = 'Movie' | 'Series' | 'Season' | 'Episode';
export type Rights = { label: string; note: string; source: string; licenseUrl?: string; attribution?: string };
export type Artwork = { id: string; url: string; label: string; creator: string; license: string; source: string; licenseUrl: string; type: string };
export type PersonCredit = { id: string; name: string; role: string; artwork?: Artwork };
export type Rating = { provider: string; value: number; scale: number; count: number | null; retrievedAt: string };
export type ProviderPresence = { provider: string; status: string; url: string };
export type Item = { id: string; title: string; kind: Kind; year: number; meta: string; overview: string; tags: string; colors: string[]; parent?: string; progress?: number; watched?: boolean | null; releaseDate?: string; addedOrder: number; rights: Rights; metadataRights: Rights; provenanceNotes: string[]; metadataRetrievedAt: string; artwork?: Artwork; cast: PersonCredit[]; crew: PersonCredit[]; rating?: Rating; providers: ProviderPresence[] };
export type Library = { id: string; title: string };
