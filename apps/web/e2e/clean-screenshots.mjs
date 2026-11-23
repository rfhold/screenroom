import { mkdir, rm } from 'node:fs/promises';
import { selectedCategory, selectedJourney } from './categories.mjs';

selectedCategory();

if (selectedJourney() === 'playback') {
  await rm(new URL('./build/screenshots/playback/', import.meta.url), { recursive: true, force: true });
  await mkdir(new URL('./build/screenshots/playback/', import.meta.url), { recursive: true });
} else for (const flow of ['home', 'sidebar', 'library', 'search', 'detail']) {
  for (const profile of ['desktop', 'mobile']) {
    await rm(new URL(`./build/screenshots/${flow}-${profile}.png`, import.meta.url), { force: true });
  }
}
