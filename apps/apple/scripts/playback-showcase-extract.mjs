import assert from 'node:assert/strict';
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const safe = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
function canonicalAttachmentName(name) {
    if (typeof name !== 'string') return name;
    return name.replace(/^(playback-[a-z0-9]+(?:-[a-z0-9]+)*)_[0-9]+_[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{12}\.(json|png)$/, '$1.$2');
}
function keys(value, expected) {
    assert.deepEqual(Object.keys(value).sort(), [...expected].sort(), 'Unexpected manifest fields');
}
export function extractPlayback(source, output, profile, scheme, started, runID, env = process.env) {
    assert.ok(!existsSync(output), 'Playback output must be fresh');
    const index = JSON.parse(readFileSync(join(source, 'manifest.json')));
    assert.equal(index.length, 1, 'Expected one selected test attachment group');
    const group = index[0];
    assert.equal(group.testIdentifier, 'PlayerScreenshots/testNosferatuPlayerStates()');
    assert.equal(group.testIdentifierURL, `test://com.apple.xcode/Screenroom/Screenroom-${scheme}-E2ETests/PlayerScreenshots/testNosferatuPlayerStates`);
    const names = group.attachments.map(a => canonicalAttachmentName(a.suggestedHumanReadableName));
    const playbackNames = names.filter(name => typeof name === 'string' && /^playback-[a-z0-9]+(?:-[a-z0-9]+)*\.(json|png)$/.test(name));
    assert.equal(new Set(playbackNames).size, playbackNames.length, 'Duplicate canonical playback attachment');
    function attachment(name) {
        const matches = group.attachments.filter((_, i) => names[i] === name);
        assert.equal(matches.length, 1, `Missing/duplicate attachment ${name}`);
        const a = matches[0];
        assert.equal(a.isAssociatedWithFailure, false, 'Failure-associated attachment');
        assert.ok(Number.isFinite(a.timestamp) && a.timestamp >= Number(started), 'Stale attachment');
        assert.equal(basename(a.exportedFileName), a.exportedFileName, 'Unsafe exported attachment path');
        const path = join(source, a.exportedFileName);
        assert.ok(lstatSync(path).isFile() && !lstatSync(path).isSymbolicLink());
        assert.equal(realpathSync(path), resolve(path));
        return path;
    }
    const envelope = JSON.parse(readFileSync(attachment('playback-manifest-success.json')));
    keys(envelope, ['run_id', 'manifest']);
    assert.equal(envelope.run_id, runID, 'Foreign run manifest');
    const manifest = envelope.manifest;
    keys(manifest, ['schema_version', 'journey', 'backend_instance_id', 'item', 'platforms']);
    keys(manifest.item, ['id', 'title', 'year']);
    assert.equal(manifest.schema_version, 1);
    assert.equal(manifest.journey, 'playback');
    assert.equal(manifest.backend_instance_id, env.SCREENROOM_BACKEND_INSTANCE_ID);
    assert.deepEqual(manifest.item, { id: env.SCREENROOM_SHOWCASE_ITEM_ID, title: env.SCREENROOM_SHOWCASE_TITLE, year: Number(env.SCREENROOM_SHOWCASE_YEAR) });
    assert.equal(manifest.platforms.length, 1);
    const platform = manifest.platforms[0];
    keys(platform, ['slug', 'runtime', 'outcome', 'states']);
    assert.equal(platform.slug, profile === 'ipad' ? 'ipados' : profile);
    assert.ok(['ios', 'ipados', 'tvos', 'macos'].includes(platform.slug));
    assert.ok(['observed', 'blocked'].includes(platform.outcome));
    assert.ok(typeof platform.runtime === 'string' && platform.runtime.length > 0);
    assert.ok(Array.isArray(platform.states) && platform.states.length > 0);
    const ids = new Set();
    const images = new Set();
    const copies = [];
    for (const state of platform.states) {
        assert.match(state.id, safe);
        assert.ok(!ids.has(state.id), 'Duplicate state'); ids.add(state.id);
        for (const key of ['label', 'viewport', 'observation']) assert.ok(typeof state[key] === 'string' && state[key].length > 0, key);
        assert.ok(['captured', 'unavailable', 'blocked', 'not-observed'].includes(state.status));
        keys(state, ['id', 'label', 'status', 'viewport', 'observation', state.status === 'captured' ? 'image' : 'reason']);
        if (state.status === 'captured') {
            assert.equal(state.image, `${state.id}.png`, 'Unsafe image path');
            assert.ok(!images.has(state.image)); images.add(state.image);
            const path = attachment(`playback-${state.id}.png`);
            const bytes = readFileSync(path);
            assert.ok(bytes.length > 24 && bytes.subarray(0, 8).equals(Buffer.from('89504e470d0a1a0a', 'hex')), 'Invalid PNG');
            assert.ok(bytes.readUInt32BE(16) > 0 && bytes.readUInt32BE(20) > 0, 'Empty PNG');
            copies.push([path, state.image]);
        } else {
            assert.equal(state.image, undefined);
            assert.ok(typeof state.reason === 'string' && state.reason.length > 0);
        }
    }
    assert.ok(ids.has('cleanup'), 'Missing cleanup report');
    const namedImages = group.attachments.filter(a => a.suggestedHumanReadableName?.startsWith('playback-') && a.suggestedHumanReadableName.endsWith('.png'));
    assert.equal(namedImages.length, images.size, 'Unexpected playback attachments');
    mkdirSync(output);
    for (const [path, name] of copies) copyFileSync(path, join(output, name));
    writeFileSync(join(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    return manifest;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    try { extractPlayback(...process.argv.slice(2)); }
    catch (error) { console.error(error.message); process.exitCode = 1; }
}
