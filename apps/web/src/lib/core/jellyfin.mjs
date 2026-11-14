/** @param {{ code?: string } | null | undefined} error */
export function progressError(error) {
  return error?.code === 'progress_unavailable'
    ? 'Continue Watching is unavailable. Jellyfin progress could not be read. Other catalog rows remain available.'
    : 'This row could not be loaded. Other catalog rows remain available.';
}

/**
 * @param {import('../../../../../target/client-core/wasm/screenroom_client_core').Client} client
 * @param {string} source
 * @param {string | null} profile
 * @param {import('./operation.mjs').MetadataOperation} operation
 */
export async function changeJellyfinLink(client, source, profile, operation) {
  return operation.read(async () => {
    let mutationError;
    let returned;
    try {
      returned = profile === null
        ? await client.clear_jellyfin_link(source)
        : await client.set_jellyfin_link(source, profile);
    } catch (cause) { mutationError = cause; }
    operation.check();
    client.invalidate_reads();
    const link = await client.get_jellyfin_link(source);
    operation.check();
    const confirmed = profile === null ? link === null : link?.source_id === source && link.jellyfin_user_id === profile;
    if (mutationError) return { link, confirmed, ambiguous: true };
    const validReturn = profile === null ? returned === source : typeof returned === 'object' && returned?.source_id === source && returned.jellyfin_user_id === profile;
    return { link, confirmed: confirmed && validReturn, ambiguous: false };
  });
}
