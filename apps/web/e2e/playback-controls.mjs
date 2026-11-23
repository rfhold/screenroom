/** @param {{media: {muted: boolean} | null, muteControl: string | null}} state @param {boolean} muted */
export function muteSettled(state, muted) {
  return state.media?.muted === muted && state.muteControl === (muted ? 'Unmute' : 'Mute');
}

/** @param {{media: {volume: number, muted: boolean} | null, volumeValues: number[]}} state @param {number} volume */
export function volumeSettled(state, volume) {
  return state.media?.volume === volume && state.media.muted === false && state.volumeValues.includes(volume);
}
