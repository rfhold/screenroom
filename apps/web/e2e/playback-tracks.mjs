/**
 * @param {{error: boolean, uncertain: boolean, mediaReady: boolean, controlsEnabled: boolean, selectorEnabled: boolean, requestedSelected: boolean, transportChanged: boolean, updateResponded: boolean}} evidence
 * @param {boolean} alreadySelected
 * @returns {'blocked' | 'waiting' | 'ready'}
 */
export function trackReadiness(evidence, alreadySelected) {
  if (evidence.error || evidence.uncertain) return 'blocked';
  if (!evidence.mediaReady || !evidence.controlsEnabled || !evidence.selectorEnabled || !evidence.requestedSelected) return 'waiting';
  if (!alreadySelected && (!evidence.transportChanged || !evidence.updateResponded)) return 'waiting';
  return 'ready';
}
