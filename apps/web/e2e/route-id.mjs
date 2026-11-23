/** @param {string} value @param {'libraries' | 'titles'} resource @param {string} base */
export function routeId(value, resource, base) {
  const parts = new URL(value, base).pathname.split('/');
  if (parts.length !== 3 || parts[1] !== resource || !parts[2]) throw new Error(`Expected a scoped ${resource} route`);
  return decodeURIComponent(parts[2]);
}
