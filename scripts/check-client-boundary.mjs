import { readdir, readFile, lstat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const scope = [
  'Apple: Sources, Resources, project.yml',
  'Android: each module src/{production source sets}, module/root Gradle configuration',
  'Linux: src, data, build.rs, Cargo.toml',
  'Web SPA: src, static, Vite/Svelte configuration (not website)',
];

const excluded = /^(?:build|dist|target|node_modules|generated|\.svelte-kit|\.gradle|\.git|tests?|__tests__|fixtures|__fixtures__|e2e|androidTest|testFixtures|.*Tests|.*Test)$/i;
const source = /(?:\.(?:swift|kt|kts|java|rs|ts|tsx|js|jsx|mjs|svelte|html|css|xml|json|ya?ml|toml|gresource|desktop)|\/Makefile)$/i;
const media = /\.(?:png|jpe?g|svg|webp|gif|avif|ico|pdf|heic|mp4|webm)$/i;
const fixtureNames = /(?:Big Buck Bunny|Sintel|Tears of Steel|Sprite Fright|His Girl Friday|Night of the Living Dead|The Beverly Hillbillies|The Clampetts Strike Oil|Getting Settled|Matthew Luhn|891761)/i;
const fixtureIds = /["'`](?:big-buck-bunny|sintel|tears-of-steel|sprite-fright|his-girl-friday|night-of-the-living-dead|beverly-hillbillies|buster-keaton|duane-jones)["'`]/i;
const systemIcon = /^(?:home|search|menu|back|forward|close|chevron-left|chevron-right|arrow-left|arrow-right|refresh|settings|info|error|warning|loading|placeholder)(?:-symbolic)?\.svg$/;

function allowedMedia(file) {
  if (/^apps\/apple\/Resources\/(?:iOS|macOS)\/Assets\.xcassets\/AppIcon\.appiconset\//.test(file)) return true;
  if (file.startsWith('apps/apple/Resources/tvOS/Assets.xcassets/App Icon & Top Shelf Image.brandassets/')) return true;
  if (/^apps\/android\/[^/]+\/src\/[^/]+\/res\/(?:drawable|mipmap)(?:-[^/]+)?\/(?:ic_launcher(?:_round|_foreground|_monochrome)?|tv_banner)\.(?:png|xml)$/.test(file)) return true;
  if (/^apps\/linux\/data\/icons\/hicolor\/[^/]+\/apps\/tv\.screenroom\.(?:png|svg)$/.test(file)) return true;
  if (/^apps\/web\/static\/(?:favicon(?:-32)?\.(?:svg|png|ico)|icon-(?:192|512)\.png|apple-touch-icon\.png)$/.test(file)) return true;
  if (/^apps\/android\/[^/]+\/src\/[^/]+\/res\/drawable(?:-[^/]+)?\//.test(file)) {
    return systemIcon.test(path.posix.basename(file).replace(/\.xml$/, '.svg'));
  }
  return /\/(?:icons|glyphs)\//.test(file) && systemIcon.test(path.posix.basename(file));
}

function testFile(file) {
  return /(?:^|\/)[^/]+\.(?:test|spec)\.[^/]+$/.test(file) || /(?:Tests?|Test)\.(?:swift|kt|java|rs)$/.test(file) || /\/(?:test|tests)\.rs$/.test(file);
}

function allowedResourceReference(file, value) {
  const webRootPath = value.startsWith('/') && file.startsWith('apps/web/');
  const resolved = webRootPath
    ? path.posix.normalize(`apps/web/static${value}`)
    : path.posix.normalize(path.posix.join(path.posix.dirname(file), value));
  if (webRootPath) return resolved.startsWith('apps/web/static/') && allowedMedia(resolved);
  return !path.posix.isAbsolute(value) && resolved.startsWith('apps/') && allowedMedia(resolved);
}

// Keep line numbers stable while excluding comments from source rules.
function uncomment(text) {
  return text.replace(/<!--[^]*?-->|\/\*[^]*?\*\//g, value => value.replace(/[^\n]/g, ' '))
    .replace(/^\s*\/\/.*$/gm, value => ' '.repeat(value.length));
}

export function inspectSource(file, contents) {
  const text = uncomment(contents);
  const violations = [];
  const add = (rule, reason, index = 0) => violations.push({ path: file, line: text.slice(0, index).split('\n').length, rule, reason });
  const rules = [
    ['client-footer', /\b(?:ProviderAttribution(?:View)?|provider_attribution|loadAttribution|attributionError)\b|provider[.-]attribution|Data &amp; artwork credits|Data & artwork credits/g, 'global attribution and technical-credit footers are not client UI'],
    ['remote-preferences', /\b(?:PreferencesService|get_preferences|update_preferences|getPreferences|updatePreferences)\b/g, 'settings are client-owned; remote preferences RPCs are retired'],
    ['mock-profile', /\b(?:mock_profile|mockProfile)\b|x-screenroom-mock-profile/g, 'use an explicit trusted-local user UUID selector, not a mock profile'],
    ['private-api', /screenroom\.(?:catalog|user)\.(?:operator|internal)\.|\b(?:jellyfin_user_service_client|playback_state_service_client|ingestion_service_client)\b/g, 'operator and internal contracts must not be used by public clients'],
    ['local-artwork', /(?:["'`])(?:\.?\.?\/)*(?:artwork|Catalog)\//g, 'local artwork path; retrieve artwork through the client core'],
    ['local-artwork', /(?:BundledImage|Image\s*\(\s*(?:decorative\s*:\s*)?|UIImage\s*\(\s*named\s*:\s*|NSImage\s*\(\s*named\s*:\s*|poster::picture\s*\()\s*(?:file\s*:\s*)?["'`](?:tmdb|[^"'`]+\.(?:png|jpe?g|svg|webp))/gi, 'bundled image loader must not load media or provider branding'],
    ['local-artwork', /(?:include_bytes!|include_str!)\s*\(\s*["']([^"']*(?:artwork|catalog|tmdb|\.(?:png|jpe?g|svg|webp)))["']/gi, 'embedded catalog/artwork bytes bypass the mock server', true],
    ['local-artwork', /(?:assets\.open|AssetManager\.open|FileInputStream|from_file|from_filename|fileURLWithPath|contentsOfFile)\s*\(?\s*["'`]([^"'`]*(?:artwork[^"'`]*|tmdb[^"'`]*|\.(?:png|jpe?g|svg|webp)))["'`]/gi, 'local file/asset image loading bypasses the mock server', true],
    ['local-artwork', /(?:R\.drawable\.(?:tmdb|[^\s,;)]*(?:poster|portrait|logo))|data:image\/(?:png|jpeg|svg\+xml|webp))/gi, 'bundled drawable or inline image data is not server artwork'],
    ['fabricated-art', /\bArt\s*\(\s*["']tmdb["']/g, 'fabricated provider artwork record; use server records'],
    ['fixture-data', new RegExp(fixtureNames.source, 'gi'), 'fixture-specific title/name/provenance belongs to the mock server'],
    ['fixture-data', new RegExp(fixtureIds.source, 'gi'), 'fixture-specific identifier/branch belongs to the mock server'],
    ['fixture-provenance', /(?:This product uses the TMDB API|TMDB data and branding|Jellyfin is not connected|Title metadata, cast, crew|Public domain is jurisdiction|Files are unmodified downloads|simulated (?:offline fixture|mock server|mock) state)/gi, 'hardcoded catalog/provider provenance belongs to server CatalogInfo'],
    ['refresh-mock', /\brefreshCount\b/g, 'fabricated refresh counter/status; report actual read outcomes'],
    ['bundled-fixtures', /(?:fixtures\/catalog|Resources\/Catalog|data\/artwork|src\/main\/assets\/artwork)/g, 'packaging/source references server-owned fixture resources'],
  ];
  for (const [rule, regex, reason, resourceLoader] of rules) {
    for (const match of text.matchAll(regex)) {
      if (!resourceLoader || !allowedResourceReference(file, match[1])) add(rule, reason, match.index);
    }
  }
  const localImages = /(?:\bsrc\s*=\s*["']|\burl\(\s*["']?)([^"'\s)]+\.(?:png|jpe?g|svg|webp|gif|avif))(?:["')])/gi;
  for (const match of text.matchAll(localImages)) {
    const value = match[1];
    if (/^(?:https?:|blob:)/.test(value)) continue;
    if (!allowedResourceReference(file, value)) add('local-artwork', 'literal local image/CSS path is not server artwork', match.index);
  }
  if (/\bWelcome\b|["']welcome["']/.test(text) && /\bContinue\b|\bLog out\b|\blogout\b/.test(text)) {
    add('welcome-mock', 'fabricated Welcome/Continue/logout flow is not implemented authentication', text.search(/Welcome|["']welcome["']/));
  }
  return violations;
}

async function exists(file) {
  try { return await lstat(file); } catch (error) {
    if (error.code === 'ENOENT') return undefined;
    throw error;
  }
}

export async function checkClientBoundary(root) {
  root = path.resolve(root);
  const violations = [];
  const files = new Set();
  const links = new Set();
  if ((await exists(root))?.isSymbolicLink()) {
    return { scope, filesScanned: 0, violations: [{ path: '.', line: 1, rule: 'unverifiable-link', reason: 'audit root symlink requires explicit boundary review' }] };
  }
  async function productionStat(relative) {
    let current = '';
    for (const part of relative.split('/')) {
      current = current ? `${current}/${part}` : part;
      const stat = await exists(path.join(root, current));
      if (!stat) return;
      if (stat.isSymbolicLink()) {
        if (!links.has(current)) {
          links.add(current);
          violations.push({ path: current, line: 1, rule: 'unverifiable-link', reason: 'production resource/source symlink requires explicit boundary review' });
        }
        return;
      }
      if (current === relative) return stat;
      if (!stat.isDirectory()) return;
    }
  }
  async function walk(relative) {
    if (excluded.test(path.posix.basename(relative)) || testFile(relative)) return;
    const stat = await productionStat(relative);
    if (!stat) return;
    if (stat.isDirectory()) {
      if (/(?:^|\/)(?:artwork|Catalog)$/.test(relative) && !relative.includes('.xcassets/')) {
        violations.push({ path: relative, line: 1, rule: 'legacy-artwork-directory', reason: 'client-local catalog/artwork directory; load provider data through the shared core' });
      }
      for (const entry of (await readdir(path.join(root, relative))).sort()) await walk(`${relative}/${entry}`);
    } else if (!testFile(relative)) files.add(relative);
  }
  for (const item of ['apps/apple/Sources', 'apps/apple/Resources', 'apps/apple/project.yml',
    'apps/linux/src', 'apps/linux/data', 'apps/linux/build.rs', 'apps/linux/Cargo.toml',
    'apps/web/src', 'apps/web/static', 'apps/web/vite.config.ts', 'apps/web/svelte.config.js',
    'apps/android/build.gradle.kts', 'apps/android/settings.gradle.kts']) await walk(item);
  const android = await productionStat('apps/android');
  if (android?.isDirectory()) {
    for (const module of await readdir(path.join(root, 'apps/android'), { withFileTypes: true })) {
      if (excluded.test(module.name) || module.name.startsWith('.')) continue;
      if (module.isSymbolicLink()) {
        await productionStat(`apps/android/${module.name}`);
        continue;
      }
      if (!module.isDirectory()) continue;
      await walk(`apps/android/${module.name}/src`);
      await walk(`apps/android/${module.name}/build.gradle.kts`);
    }
  }
  for (const file of [...files].sort()) {
    const add = (rule, reason) => violations.push({ path: file, line: 1, rule, reason });
    if (/^apps\/web\/src\/lib\/demo\/(?:catalog|providers|search)\.ts$/.test(file)) {
      add('legacy-fixture-module', 'dead legacy web fixture module must be removed (navigation alone is allowed)');
    }
    if (media.test(file) && !allowedMedia(file)) add('bundled-media', 'client-bundled media/branding is forbidden; use server artwork');
    if (!source.test(file)) continue;
    const text = await readFile(path.join(root, file), 'utf8');
    if (/^apps\/android\/[^/]+\/src\/[^/]+\/res\/drawable[^/]*\/.*\.xml$/.test(file)
      && /<(?:vector|bitmap)\b/.test(text) && !allowedMedia(file)) {
      add('bundled-media', 'bundled vector/bitmap drawable is not an allowlisted app/system icon');
    }
    if (/\/(?:catalog|tmdb|artwork)\.json$/i.test(file) || (/\.json$/.test(file) && fixtureNames.test(text))) {
      add('fixture-json', 'client-local catalog/artwork/provenance JSON copy must be removed');
      continue;
    }
    // App-icon catalog metadata contains filenames, not catalog records.
    if (/\.xcassets\//.test(file) && /\/Contents\.json$/.test(file)) continue;
    violations.push(...inspectSource(file, text));
  }
  return { scope, filesScanned: files.size, violations };
}

async function main(args) {
  if (args.length && (args.length !== 2 || args[0] !== '--root')) throw new Error('Usage: node scripts/check-client-boundary.mjs [--root DIRECTORY]');
  const root = args[1] ?? fileURLToPath(new URL('../', import.meta.url));
  const result = await checkClientBoundary(root);
  console.log('Client boundary static source/resource audit:');
  for (const item of scope) console.log(`  ${item}`);
  console.log('Excluded: tests/fixtures, generated/build outputs, core crates, static website.');
  for (const item of result.violations) console.error(`${item.path}:${item.line} [${item.rule}] ${item.reason}`);
  console.log(`${result.filesScanned} files scanned; ${result.violations.length} violation(s). Static guard only, not runtime network proof.`);
  if (result.violations.length) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
