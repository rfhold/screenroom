import { writeFile, rename } from 'node:fs/promises';

const target = new URL('../fixtures/catalog/tmdb.json', import.meta.url);
const token = process.env.TMDB_READ_ACCESS_TOKEN;
const selections = [
  ['nosferatu', 'Nosferatu', 1922], ['the-general', 'The General', 1926],
  ['his-girl-friday', 'His Girl Friday', 1940], ['night-of-the-living-dead', 'Night of the Living Dead', 1968],
  ['big-buck-bunny', 'Big Buck Bunny', 2008], ['sintel', 'Sintel', 2010],
  ['tears-of-steel', 'Tears of Steel', 2012], ['sprite-fright', 'Sprite Fright', 2021]
];
async function api(path) {
  const response = await fetch(`https://api.themoviedb.org/3/${path}`, { headers: { Authorization: `Bearer ${token}` }, redirect: 'error' });
  if (!response.ok) throw new Error(`TMDB HTTP ${response.status}`);
  return response.json();
}
const people = credits => ({
  cast: (credits.cast ?? []).filter((p, i, all) => all.findIndex(q => q.id === p.id && q.character === p.character) === i).slice(0, 12).map(p => ({ id: p.id, name: p.name, role: p.character || 'Cast' })),
  crew: (credits.crew ?? []).filter(p => ['Director', 'Writer', 'Screenplay', 'Story', 'Original Music Composer', 'Producer', 'Executive Producer', 'Creator', 'Art Direction', 'Sound Designer', 'Editor', 'Director of Photography'].includes(p.job)).filter((p, i, all) => all.findIndex(q => q.id === p.id && q.job === p.job) === i).map(p => ({ id: p.id, name: p.name, role: p.job }))
});
function snapshot(d, url, credits = d.credits, requireRating = true) {
  if (!Number.isInteger(d.id) || (requireRating && (!Number.isFinite(d.vote_average) || !Number.isInteger(d.vote_count)))) throw new Error('Invalid metadata');
  return { id: d.id, title: d.title ?? d.name, date: d.release_date ?? d.air_date ?? d.first_air_date, overview: d.overview, runtime: d.runtime, rating: d.vote_average ?? null, votes: d.vote_count ?? null, url, ...people(credits) };
}
try {
  if (!token) throw new Error('TMDB_READ_ACCESS_TOKEN is required');
  const data = { retrievedAt: new Date().toISOString(), items: {} };
  for (const [key, title, year] of selections) {
    // TMDB dates this verified 2021 Blender film to its later 2024 release.
    if (key === 'sprite-fright') {
      const d = await api('movie/891761?append_to_response=credits');
      const homepage = new URL(d.homepage);
      if (d.id !== 891761 || d.title !== title || !d.credits?.crew?.some(p => p.id === 1993493 && p.name === 'Matthew Luhn' && p.job === 'Director') || homepage.protocol !== 'https:' || homepage.hostname !== 'studio.blender.org' || homepage.pathname !== '/films/sprite-fright/') throw new Error('Sprite Fright identity mismatch');
      data.items[key] = { ...snapshot(d, `https://www.themoviedb.org/movie/${d.id}`), dateNote: 'Original Blender release: 2021. TMDB records 2024-04-06; its provider date is retained, not used to redefine the original release year.' };
      continue;
    }
    const results = await api(`search/movie?query=${encodeURIComponent(title)}&primary_release_year=${year}`);
    const match = results.results.find(d => d.title === title && d.release_date?.startsWith(`${year}-`));
    if (!match) throw new Error(`No exact title/year match: ${title}`);
    const d = await api(`movie/${match.id}?append_to_response=credits`);
    if (d.title !== title || !d.release_date.startsWith(`${year}-`)) throw new Error(`Identity mismatch: ${title}`);
    data.items[key] = snapshot(d, `https://www.themoviedb.org/movie/${d.id}`);
  }
  const results = await api('search/tv?query=The%20Beverly%20Hillbillies&first_air_date_year=1962');
  const match = results.results.find(d => d.name === 'The Beverly Hillbillies' && d.first_air_date?.startsWith('1962-'));
  if (!match) throw new Error('No exact series match');
  const show = await api(`tv/${match.id}?append_to_response=credits`);
  const root = `https://www.themoviedb.org/tv/${show.id}`;
  data.items['beverly-hillbillies'] = { ...snapshot(show, root), creators: show.created_by.map(p => ({ id: p.id, name: p.name, role: 'Creator' })) };
  const season = await api(`tv/${show.id}/season/1?append_to_response=credits`);
  if (show.name !== 'The Beverly Hillbillies' || !show.first_air_date.startsWith('1962-') || season.season_number !== 1) throw new Error('Series identity mismatch');
  data.items['beverly-hillbillies-s1'] = snapshot(season, `${root}/season/1`, season.credits, false);
  for (const [number, title, date] of [[1, 'The Clampetts Strike Oil', '1962-09-26'], [2, 'Getting Settled', '1962-10-03']]) {
    const d = await api(`tv/${show.id}/season/1/episode/${number}?append_to_response=credits`);
    if (d.name !== title || d.air_date !== date || d.episode_number !== number || d.season_number !== 1) throw new Error('Episode identity mismatch');
    data.items[`beverly-hillbillies-s1-e${number}`] = snapshot(d, `${root}/season/1/episode/${number}`, { cast: [...(d.credits.cast ?? []), ...(d.guest_stars ?? d.credits.guest_stars ?? [])], crew: d.credits.crew });
  }
  const serialized = `${JSON.stringify(data, null, 2)}\n`;
  if (serialized.includes(token)) throw new Error('Secret in metadata');
  await writeFile(new URL(`${target}.tmp`), serialized);
  await rename(new URL(`${target}.tmp`), target);
  console.log(`Imported ${Object.keys(data.items).length} verified TMDB records (${data.retrievedAt}).`);
} catch (error) {
  // Never print fetch errors, headers, response bodies, or credentials.
  console.error(error.message.startsWith('No exact') || error.message === 'TMDB_READ_ACCESS_TOKEN is required' || /^TMDB HTTP \d+$/.test(error.message) ? error.message : 'TMDB import failed; existing fixture preserved.');
  process.exitCode = 1;
}
