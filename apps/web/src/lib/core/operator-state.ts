export const encode = (value: unknown): string => JSON.stringify(value, (_, entry) => typeof entry === 'bigint' ? entry.toString() : entry);
export function clone<T>(value: T): T {
  if (Array.isArray(value)) return value.map(entry => clone(entry)) as T;
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, clone(entry)])) as T;
  return value;
}

/** A single explicit action, retained only while this view is open. */
export class SessionAction<T> {
  private readonly original: T;
  busy = false;
  constructor(body: T) { this.original = clone(body); }
  async send<R>(send: (body: T) => Promise<R>): Promise<R | null> {
    if (this.busy) return null;
    this.busy = true;
    try { return await send(clone(this.original)); }
    finally { this.busy = false; }
  }
}

export function operatorError(cause: unknown): string {
  const code = (cause as {code?: string})?.code;
  const messages: Record<string,string> = {
    stale:'These settings changed elsewhere. Reload before trying again.', source_busy:'A refresh is already in progress for this source.',
    invalid_credentials:'Enter an administrator API key.', credentials_rejected:'Jellyfin rejected this API key. Check its administrator permissions.',
    invalid_connection:'Enter a valid HTTP or HTTPS server URL without credentials, a query, or a fragment.', invalid_discovery:'Jellyfin returned an unsupported server response.',
    invalid_server_identity:'Jellyfin did not report a valid server identity.', server_identity_mismatch:'The server identity changed. Connect again before saving.',
    invalid_selection:'Choose a profile and supported libraries.', profile_unavailable:'This profile is no longer available. Connect again.',
    library_unavailable:'A selected library is no longer available. Connect again.', unsupported_library:'This library type is not supported.',
    unsupported_library_scope:'Choose at most one collections library.', selection_capacity:'Choose between 1 and 32 libraries.',
    local_user_unavailable:'The configured local user is unavailable. Check this installation’s user selector.', request_conflict:'This request conflicts with another change. Reload the server list.',
    request_expired:'This request can no longer be retried. Reload the server list.', server_already_connected:'This Jellyfin server is already connected. Reload the server list.',
    connection_unavailable:'This connection is unavailable.', credential_store_unavailable:'The server could not securely store the key. Check the backend configuration.',
    discovery_unavailable:'Could not reach Jellyfin. Check the URL and network access from the backend.',
    invalid:'These settings or this source are not supported. Check Advanced settings.', forbidden:'This action is unavailable.',
    not_found:'This server record is no longer available. Reload to check its state.', cancelled:'The request was cancelled. Reload to check its state.',
    deadline:'The request timed out. Check the server state before trying again.', unavailable:'Could not reach the server. Please try again.',
    invalid_response:'The server response could not be verified.'
  };
  return messages[code || ''] || 'Could not complete the request. Please try again.';
}
