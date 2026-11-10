import { error } from '@sveltejs/kit';
import { shells } from '../../../lib/showcase';
import type { EntryGenerator, PageServerLoad } from './$types';

export const entries: EntryGenerator = () => shells.map(({ slug }) => ({ shell: slug }));

export const load: PageServerLoad = ({ params }) => {
  const shell = shells.find(({ slug }) => slug === params.shell);
  if (!shell) error(404, 'Client shell not found');
  return { shell };
};
