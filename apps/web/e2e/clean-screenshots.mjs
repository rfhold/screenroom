import { rm } from 'node:fs/promises';
import { selectedCategory } from './categories.mjs';

selectedCategory();

for (const flow of ['home', 'sidebar', 'library', 'search', 'detail']) {
  for (const profile of ['desktop', 'mobile']) {
    await rm(new URL(`./build/screenshots/${flow}-${profile}.png`, import.meta.url), { force: true });
  }
}
