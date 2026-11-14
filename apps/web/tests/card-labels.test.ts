import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cardLabels } from '../src/lib/core/card-labels.ts';
import type { Item } from '../src/lib/core/models.ts';

const episode = (meta: string, title = 'Episode title'): Item => ({ kind: 'Episode', title, meta } as Item);

test('episode labels use catalog subtitle numbering, with the episode title underneath in both contexts', () => {
  const item = episode('Series name · S2 E5 · ~25m');
  assert.deepEqual(cardLabels(item), { heading: 'Series name · S02:E05', primary: 'Series name', number: 'S02:E05', subtitle: 'Episode title' });
  assert.deepEqual(cardLabels(item, true), { heading: 'E05', primary: '', number: 'E05', subtitle: 'Episode title' });
  assert.deepEqual(cardLabels(episode('Series name · S02E05')), { heading: 'Series name · S02:E05', primary: 'Series name', number: 'S02:E05', subtitle: 'Episode title' });
  assert.equal(cardLabels(episode('Series name · S02E105')).heading, 'Series name · S02:E105');
  assert.equal(cardLabels(episode('Series · Name · S0 E1')).heading, 'Series · Name · S00:E01');
  assert.equal(cardLabels(episode('Series: Subtitle · S05E09')).primary, 'Series: Subtitle');
});

test('missing or unrecognized metadata never invents numbering or a series name', () => {
  for (const meta of ['', 'Unknown series', 'Series · S? E5', 'Series · S2 E?', 'Series · S-1 E5', 'Series · S2 E5 extra']) {
    const item = episode(meta, 'S2 E5 is part of this title');
    for (const contextual of [false, true]) assert.deepEqual(cardLabels(item, contextual), { heading: 'Episode', primary: 'Episode', subtitle: item.title });
  }
  assert.equal(cardLabels(episode('S2 E5')).heading, 'S02:E05');
  assert.equal(cardLabels(episode('S2 E5'), true).heading, 'E05');
});

test('movie, series, and season labels remain unchanged even with the season flag', () => {
  for (const kind of ['Movie', 'Series', 'Season'] as const) {
    const item = { kind, title: 'Title', year: 2001, meta: 'Metadata' } as Item;
    for (const contextual of [false, true]) assert.deepEqual(cardLabels(item, contextual), { heading: 'Title', primary: 'Title', subtitle: kind === 'Movie' ? '2001' : 'Metadata' });
  }
});
