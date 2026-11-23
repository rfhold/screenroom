import playback from './showcase-playback.json';

export function playbackPlatforms(value: unknown): PlaybackPlatform[] {
  const fail = (): never => { throw new Error('Invalid showcase playback metadata'); };
  const record = (value: unknown): Record<string, unknown> =>
    value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : fail();
  const text = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0;
  const root = record(value);
  if (root.schema_version !== 1 || !Array.isArray(root.platforms)) fail();
  const slugs = new Set<string>();
  return (root.platforms as unknown[]).map((value) => {
    const platform = record(value);
    const slug = platform.slug;
    if (typeof slug !== 'string' || !['ios', 'ipados', 'macos', 'tvos', 'android', 'android-tv', 'linux', 'web'].includes(slug) || slugs.has(slug)) fail();
    slugs.add(slug as string);
    if (!text(platform.runtime) || platform.provenance !== 'Current-run E2E capture · Real Jellyfin-backed application'
      || !['observed', 'blocked'].includes(platform.outcome as string)) fail();
    const item = record(platform.item);
    if (!text(item.title) || !Number.isInteger(item.year) || (item.year as number) < 1) fail();
    if (!Array.isArray(platform.states) || !platform.states.length) fail();
    const ids = new Set<string>();
    const states = (platform.states as unknown[]).map((value) => {
      const state = record(value);
      if (typeof state.id !== 'string' || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(state.id) || ids.has(state.id)) fail();
      ids.add(state.id as string);
      if (!text(state.label) || !text(state.viewport)
        || !['captured', 'unavailable', 'blocked', 'not-observed'].includes(state.status as string)) fail();
      if (state.observation !== undefined && !text(state.observation)) fail();
      if (state.status === 'captured') {
        if (!text(state.observation)) fail();
        const prefix = `/showcase/${slug}/playback/`;
        if (typeof state.image !== 'string' || !state.image.startsWith(prefix)) fail();
        const relative = (state.image as string).slice(prefix.length);
        if (!/^[A-Za-z0-9_-]+(?:\/[A-Za-z0-9_.-]+)*\.png$/.test(relative)
          || relative.split('/').some((part) => part === '.' || part === '..')) fail();
      } else if (!text(state.reason) || state.image !== undefined) fail();
      return {
        id: state.id, label: state.label, status: state.status, viewport: state.viewport,
        ...(state.observation !== undefined ? { observation: state.observation } : {}),
        ...(state.status === 'captured' ? { image: state.image } : { reason: state.reason })
      } as PlaybackState;
    });
    return {
      slug, runtime: platform.runtime, outcome: platform.outcome,
      item: { title: item.title, year: item.year }, provenance: platform.provenance, states
    } as PlaybackPlatform;
  });
}

const currentPlayback = playbackPlatforms(playback);

export type Screenshot = {
  src: string;
  label: string;
  alt: string;
  viewport?: 'desktop' | 'mobile';
};

export type Flow = {
  slug: string;
  name: string;
  caption: string;
  screenshots: Screenshot[];
  provenance: string;
  playback?: PlaybackPlatform;
};

export type Shell = {
  slug: string;
  name: string;
  description: string;
  flows: Flow[];
};

const flows = [
  { slug: 'home', name: 'Home', caption: 'This historical fixture-backed capture shows titles and artwork from the original mock-server catalog.' },
  { slug: 'sidebar', name: 'Expanded sidebar', caption: 'The expanded sidebar exposes the shell’s navigation destinations.' },
  { slug: 'library', name: 'Movies library', caption: 'This historical fixture-backed capture shows the original mock-server movie collection.' },
  { slug: 'search', name: 'Search', caption: 'This historical fixture-backed capture shows a search of the original mock-server catalog.' },
  { slug: 'detail', name: 'Title details', caption: 'This historical fixture-backed capture shows title information from the original mock-server catalog.' }
];

export function captures(slug: string, name: string, platforms: PlaybackPlatform[] = currentPlayback): Flow[] {
  const browse: Flow[] = flows.map((flow) => ({
    ...flow,
    provenance: 'Historical E2E capture · Mock-server fixtures',
    screenshots: slug === 'web'
      ? (['desktop', 'mobile'] as const).map((viewport) => ({
          src: `/showcase/web/${viewport}${flow.slug === 'home' ? '' : `-${flow.slug}`}.png`,
          label: `${viewport === 'desktop' ? 'Desktop' : 'Mobile'} browser`,
          alt: `Screenroom Web ${flow.name.toLowerCase()} at a ${viewport} viewport. ${flow.caption}`,
          viewport
        }))
      : [{
          src: `/showcase/${slug}/${flow.slug}.png`,
          label: name,
          alt: `Screenroom ${name} ${flow.name.toLowerCase()}. ${flow.caption}`
        }]
  }));
  const evidence = platforms.find((platform) => platform.slug === slug);
  return [...browse, {
    slug: 'playback',
    name: 'Playback',
    caption: evidence
      ? `${evidence.item.title} (${evidence.item.year}) · Platform outcome: ${evidence.outcome}. Captures document observed states, not complete playback acceptance.`
      : 'No playback screenshots for this app.',
    provenance: evidence?.provenance ?? 'Not captured',
    screenshots: [],
    playback: evidence
  }];
}

export const shells: Shell[] = [
  { slug: 'ios', name: 'iOS', description: 'The native SwiftUI shell on iPhone.', flows: captures('ios', 'iOS') },
  { slug: 'ipados', name: 'iPadOS', description: 'The native SwiftUI shell on iPad.', flows: captures('ipados', 'iPadOS') },
  { slug: 'macos', name: 'macOS', description: 'The native SwiftUI desktop shell.', flows: captures('macos', 'macOS') },
  { slug: 'tvos', name: 'tvOS', description: 'The native SwiftUI shell for Apple TV.', flows: captures('tvos', 'tvOS') },
  { slug: 'android', name: 'Android', description: 'The native Compose mobile shell.', flows: captures('android', 'Android') },
  { slug: 'android-tv', name: 'Android TV', description: 'The native Compose TV shell.', flows: captures('android-tv', 'Android TV') },
  { slug: 'linux', name: 'Linux', description: 'The native GTK4 and libadwaita desktop shell.', flows: captures('linux', 'Linux') },
  {
    slug: 'web',
    name: 'Web',
    description: 'The Svelte web client, separate from this static website.',
    flows: captures('web', 'Web')
  }
];
export type PlaybackState = {
  id: string;
  label: string;
  status: 'captured' | 'unavailable' | 'blocked' | 'not-observed';
  viewport: string;
  image?: string;
  reason?: string;
  observation?: string;
};

export type PlaybackPlatform = {
  slug: string;
  runtime: string;
  outcome: 'observed' | 'blocked';
  item: { title: string; year: number };
  provenance: string;
  states: PlaybackState[];
};
