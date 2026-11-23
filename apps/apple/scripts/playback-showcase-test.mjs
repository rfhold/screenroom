import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { extractPlayback } from './playback-showcase-extract.mjs';

const apple = resolve(dirname(fileURLToPath(import.meta.url)), '..');
mkdirSync(join(apple, 'build/showcase-tooling'), { recursive: true });
const temporary = mkdtempSync(join(apple, 'build/showcase-tooling/fixtures-'));
const env = { SCREENROOM_BACKEND_INSTANCE_ID: 'fixture-instance', SCREENROOM_SHOWCASE_ITEM_ID: 'fixture-item', SCREENROOM_SHOWCASE_TITLE: 'Nosferatu', SCREENROOM_SHOWCASE_YEAR: '1922' };
const base = {
    run_id: 'current-run',
    manifest: {
        schema_version: 1, journey: 'playback', backend_instance_id: 'fixture-instance',
        item: { id: 'fixture-item', title: 'Nosferatu', year: 1922 },
        platforms: [{
            slug: 'ipados', runtime: 'Measured fixture runtime', outcome: 'blocked',
            states: [
                { id: 'error', label: 'Error', status: 'captured', image: 'error.png', viewport: 'unknown', observation: 'Visible disabled control and error notice.' },
                { id: 'playing', label: 'Playing', status: 'blocked', reason: 'Natural error.', viewport: 'unknown', observation: 'Not captured.' },
                { id: 'cleanup', label: 'Closed', status: 'captured', image: 'cleanup.png', viewport: 'unknown', observation: 'Returned normally to details; provider cleanup unconfirmed.' },
            ],
        }],
    },
};
let number = 0;
const uuids = ['0229F25F-8EC5-45B1-B083-7D09A90C28EE', '27BBA1A0-2D1E-402E-9DA3-4ED67F6B3475', '6C035F4C-C1EF-4488-85F9-165DBD904EAD'];
function decorated(name, counter = 0, uuid = uuids[0]) {
    return name.replace(/\.(json|png)$/, `_${counter}_${uuid}.$1`);
}
function fixture(mutate = () => {}, mode = 'canonical') {
    const directory = join(temporary, String(number++));
    mkdirSync(directory);
    const envelope = structuredClone(base);
    const index = [{
        testIdentifier: 'PlayerScreenshots/testNosferatuPlayerStates()',
        testIdentifierURL: 'test://com.apple.xcode/Screenroom/Screenroom-iOS-E2ETests/PlayerScreenshots/testNosferatuPlayerStates',
        attachments: ['playback-manifest-success.json', 'playback-error.png', 'playback-cleanup.png'].map((name, i) => ({
            suggestedHumanReadableName: name, exportedFileName: `${i}.bin`, isAssociatedWithFailure: false, timestamp: 101,
        })),
    }];
    if (mode !== 'canonical') {
        for (const [i, attachment] of index[0].attachments.entries()) {
            if (mode === 'mixed' && i === 2) continue;
            attachment.suggestedHumanReadableName = decorated(attachment.suggestedHumanReadableName, 0, uuids[i]);
        }
    }
    mutate(envelope, index);
    writeFileSync(join(directory, 'manifest.json'), JSON.stringify(index));
    writeFileSync(join(directory, '0.bin'), JSON.stringify(envelope));
    const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF1sAAAAASUVORK5CYII=', 'base64');
    writeFileSync(join(directory, '1.bin'), png);
    writeFileSync(join(directory, '2.bin'), png);
    return { directory, output: join(directory, 'playback') };
}
try {
    let positives = 0;
    let negatives = 0;
    function succeeds(mutate, mode) {
        const good = fixture(mutate, mode);
        extractPlayback(good.directory, good.output, 'ipad', 'iOS', 100, 'current-run', env);
        assert.deepEqual(JSON.parse(readFileSync(join(good.output, 'manifest.json'))), base.manifest);
        for (const [file, source] of [['error.png', '1.bin'], ['cleanup.png', '2.bin']]) {
            assert.deepEqual(readFileSync(join(good.output, file)), readFileSync(join(good.directory, source)));
        }
        positives++;
        return good;
    }
    function rejects(mutate, mode, label) {
        const { directory, output } = fixture(mutate, mode);
        assert.throws(() => extractPlayback(directory, output, 'ipad', 'iOS', 100, 'current-run', env), undefined, label);
        assert.equal(existsSync(output), false, `${label} wrote output before validation`);
        negatives++;
    }
    const good = succeeds();
    assert.throws(() => extractPlayback(good.directory, good.output, 'ipad', 'iOS', 100, 'current-run', env));
    const mutations = {
        duplicate: (_, index) => index[0].attachments.push(structuredClone(index[0].attachments[1])),
        failure: (_, index) => { index[0].attachments[1].isAssociatedWithFailure = true; },
        stale: (_, index) => { index[0].attachments[1].timestamp = 99; },
        staleManifest: envelope => { envelope.run_id = 'previous-run'; },
        unsafeAttachment: (_, index) => { index[0].attachments[1].exportedFileName = '../1.bin'; },
        unsafeImage: envelope => { envelope.manifest.platforms[0].states[0].image = '../error.png'; },
        foreignTest: (_, index) => { index[0].testIdentifier = 'HomeE2ETests/testHappyPathShowcase()'; },
        missing: (_, index) => { index[0].attachments.pop(); },
        foreignBackend: envelope => { envelope.manifest.backend_instance_id = 'foreign-instance'; },
        foreignItem: envelope => { envelope.manifest.item.id = 'foreign-item'; },
        foreignPlatform: envelope => { envelope.manifest.platforms[0].slug = 'ios'; },
        duplicateState: envelope => { envelope.manifest.platforms[0].states.push(envelope.manifest.platforms[0].states[0]); },
        noReason: envelope => { delete envelope.manifest.platforms[0].states[1].reason; },
        noObservation: envelope => { delete envelope.manifest.platforms[0].states[0].observation; },
        unexpectedMetadata: envelope => { envelope.manifest.platforms[0].states[0].privateLocator = 'must-not-export'; },
        extra: (_, index) => { index[0].attachments.push({ ...index[0].attachments[1], suggestedHumanReadableName: 'playback-foreign.png' }); },
    };
    for (const mode of ['canonical', 'decorated', 'mixed']) {
        if (mode !== 'canonical') succeeds(undefined, mode);
        for (const [name, mutate] of Object.entries(mutations)) rejects(mutate, mode, `${mode}/${name}`);
    }
    succeeds((_, index) => {
        for (const a of index[0].attachments) a.suggestedHumanReadableName = decorated(a.suggestedHumanReadableName, 12, uuids[0].toLowerCase());
    });
    const malformed = [
        name => decorated(name).replace('_0_', '_-1_'),
        name => decorated(name).replace('_0_', '_1.5_'),
        name => decorated(name).replace('_0_', '_counter_'),
        name => decorated(name).replace('_0_', '_０_'),
        name => decorated(name).replace(uuids[0], uuids[0].slice(1)),
        name => decorated(name).replace(uuids[0], uuids[0].replaceAll('-', '')),
        name => decorated(name).replace(uuids[0], 'Z' + uuids[0].slice(1)),
        name => name.replace(/\.(json|png)$/, '_0_partial.$1'),
        name => name.replace(/\.(json|png)$/, `_${uuids[0]}.$1`),
        name => name.replace(/\.(json|png)$/, '_0.$1'),
        name => decorated(decorated(name)),
        name => `prefix-${decorated(name)}`,
        name => `../${decorated(name)}`,
        name => `folder/${decorated(name)}`,
        name => `folder\\${decorated(name)}`,
        name => decorated(name) + '.exe',
        name => decorated(name).replace(/\.(json|png)$/, '.bin'),
        name => decorated(name).replace(/\.(json|png)$/, '.$1.exe'),
        name => decorated(name).replace(/\.(json|png)$/, extension => extension.toUpperCase()),
        name => decorated(name) + '\n',
        name => decorated(name) + ' ',
    ];
    for (const target of [0, 1]) {
        for (const [i, change] of malformed.entries()) {
            rejects((_, index) => {
                index[0].attachments[target].suggestedHumanReadableName = change(index[0].attachments[target].suggestedHumanReadableName);
            }, 'canonical', `malformed/${target}/${i}`);
        }
        for (const collision of ['plain-decorated', 'decorated-plain', 'decorated-decorated']) {
            rejects((_, index) => {
                const a = index[0].attachments[target];
                const name = a.suggestedHumanReadableName;
                if (collision !== 'plain-decorated') a.suggestedHumanReadableName = decorated(name, 0, uuids[0]);
                index[0].attachments.push({
                    ...a,
                    suggestedHumanReadableName: collision === 'decorated-plain' ? name : decorated(name, 1, uuids[1]),
                });
            }, 'canonical', `collision/${target}/${collision}`);
        }
        for (const kind of ['failure', 'stale', 'unsafeAttachment']) {
            rejects((_, index) => {
                const a = index[0].attachments[target];
                if (kind === 'failure') a.isAssociatedWithFailure = true;
                if (kind === 'stale') a.timestamp = 99;
                if (kind === 'unsafeAttachment') a.exportedFileName = '../0.bin';
            }, 'decorated', `original-attachment-check/${target}/${kind}`);
        }
    }
    rejects((_, index) => { index[0].testIdentifierURL = index[0].testIdentifierURL.replace('Screenroom-iOS-', 'Screenroom-macOS-'); }, 'decorated', 'foreign-test-url');
    console.log(`Playback extraction: ${positives} canonical/decorated/mixed positives and ${negatives} negative fixtures passed; exact images and all pre-write guards preserved`);
} finally {
    rmSync(temporary, { recursive: true });
}
