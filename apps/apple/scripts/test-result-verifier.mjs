import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { verifyTestResults } from './verify-test-results.mjs';

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const fixtures = process.argv[2];
let positive = 0;
let negative = 0;
for (const profile of ['ios', 'ipad', 'tvos']) {
    const scheme = profile === 'tvos' ? 'tvOS' : 'iOS';
    const target = `Screenroom-${scheme}-E2ETests`;
    function selectors(category) {
        const result = spawnSync('/bin/bash', ['-c', 'source "$1/test-category.sh"; configure_apple_test_category; printf "%s\\n" "${apple_test_methods[@]}"', 'verifier-test', scriptDirectory], {
            env: { ...process.env, E2E_CATEGORY: category }, encoding: 'utf8',
        });
        assert.equal(result.status, 0, result.stderr);
        const methods = result.stdout.trimEnd().split('\n');
        if (profile === 'tvos' && ['regression', 'all'].includes(category)) methods.push('testSidebarRemoteControls');
        return methods.map(method => `-only-testing:${target}/HomeE2ETests/${method}`);
    }
    function load(category, report) {
        return JSON.parse(readFileSync(join(fixtures, profile, category, `test-results-${report}.json`)));
    }
    function leaves(tests) { return tests.testNodes[0].children[0].children[0].children; }
    const categories = ['happy-path', 'retry', 'regression'];
    for (const category of categories) {
        verifyTestResults(load(category, 'tests'), load(category, 'summary'), selectors(category));
        positive++;
    }
    const allTests = load('regression', 'tests');
    const allSummary = load('regression', 'summary');
    allTests.testNodes[0].children[0].children[0].children = categories.flatMap(category => leaves(load(category, 'tests')));
    const allCount = profile === 'tvos' ? 6 : 5;
    assert.equal(selectors('all').length, allCount);
    allSummary.totalTestCount = allSummary.passedTests = allCount;
    allSummary.devicesAndConfigurations[0].passedTests = allCount;
    verifyTestResults(allTests, allSummary, selectors('all'));
    positive++;

    const mutations = {
        zero: tests => { tests.testNodes = []; },
        partial: tests => { leaves(tests).pop(); },
        skipped: tests => { leaves(tests)[0].result = 'Skipped'; },
        failed: tests => { leaves(tests)[0].result = 'Failed'; },
        unexpected: tests => {
            const leaf = leaves(tests)[0];
            leaf.nodeIdentifierURL += 'Unexpected';
            leaf.nodeIdentifier = leaf.nodeIdentifier.replace('()', 'Unexpected()');
            leaf.name = leaf.name.replace('()', 'Unexpected()');
        },
        wrongClass: tests => {
            const suite = tests.testNodes[0].children[0].children[0];
            suite.name = 'OtherTests';
            suite.nodeIdentifierURL = suite.nodeIdentifierURL.replace('HomeE2ETests', 'OtherTests');
            for (const leaf of suite.children) {
                leaf.nodeIdentifierURL = leaf.nodeIdentifierURL.replace('HomeE2ETests', 'OtherTests');
                leaf.nodeIdentifier = leaf.nodeIdentifier.replace('HomeE2ETests', 'OtherTests');
            }
        },
        wrongTarget: tests => {
            const bundle = tests.testNodes[0].children[0];
            bundle.name = 'Screenroom-Other-E2ETests';
            function change(node) {
                if (node.nodeIdentifierURL) node.nodeIdentifierURL = node.nodeIdentifierURL.replace(target, bundle.name);
                node.children?.forEach(change);
            }
            change(bundle);
        },
        duplicate: tests => { const cases = leaves(tests); cases[1] = structuredClone(cases[0]); },
        wrongContainer: tests => { tests.testNodes[0].children[0].name = 'Screenroom-Other-E2ETests'; },
        zeroSummary: (_, summary) => { summary.totalTestCount = summary.passedTests = 0; },
        skippedSummary: (_, summary) => { summary.skippedTests = 1; },
        failedSummary: (_, summary) => { summary.failedTests = 1; },
    };
    for (const [name, mutate] of Object.entries(mutations)) {
        const tests = structuredClone(allTests);
        const summary = structuredClone(allSummary);
        mutate(tests, summary);
        assert.throws(() => verifyTestResults(tests, summary, selectors('all')), undefined, `${profile}/${name} accepted`);
        const temporary = mkdtempSync(join(fixtures, 'negative-'));
        try {
            const testsPath = join(temporary, 'tests.json');
            const summaryPath = join(temporary, 'summary.json');
            writeFileSync(testsPath, JSON.stringify(tests));
            writeFileSync(summaryPath, JSON.stringify(summary));
            const result = spawnSync(process.execPath, [join(scriptDirectory, 'verify-test-results.mjs'), testsPath, summaryPath, ...selectors('all')], { encoding: 'utf8' });
            assert.equal(result.status, 1, `${profile}/${name} CLI accepted`);
            assert.equal(JSON.parse(result.stdout).verified, false);
        } finally {
            rmSync(temporary, { recursive: true });
        }
        negative++;
    }
}
console.log(`Actual-schema verifier checks passed: ${positive} positive categories, ${negative} negative mutations`);
