export const categories = ['happy-path', 'retry', 'regression', 'all'];

export function selectedJourney() {
  const journey = process.env.SCREENROOM_E2E_JOURNEY ?? 'browse';
  if (!['browse', 'playback'].includes(journey)) throw new Error('Invalid SCREENROOM_E2E_JOURNEY. Choose browse, playback.');
  return journey;
}

export function selectedCategory() {
  const journey = selectedJourney();
  const category = process.env.E2E_CATEGORY ?? (journey === 'playback' ? 'happy-path' : 'all');
  if (!categories.includes(category)) {
    throw new Error(`Invalid E2E_CATEGORY ${JSON.stringify(category)}. Choose ${categories.join(', ')}.`);
  }
  if (journey === 'playback' && category !== 'happy-path') throw new Error('Playback journey supports happy-path only.');
  return category;
}
