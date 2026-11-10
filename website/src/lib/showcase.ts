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

function captures(slug: string, name: string): Flow[] {
  return flows.map((flow) => ({
    ...flow,
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
