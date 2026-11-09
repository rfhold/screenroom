import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function verifyTestResults(tests, summary, selectors) {
    const expected = selectors.map(value => {
        assert.match(value, /^-only-testing:Screenroom-(iOS|tvOS|macOS)-E2ETests\/HomeE2ETests\/test[A-Za-z0-9]+$/);
        return value.slice('-only-testing:'.length);
    });
    assert.ok(expected.length > 0, 'No selected test methods');
    assert.equal(new Set(expected).size, expected.length, 'Duplicate selected methods');
    const passed = [];
    const runtimeWarnings = [];
    function visit(node, context = {}) {
        assert.equal(node.result, 'Passed', `Non-passed node: ${node.name}`);
        if (node.nodeType === 'Test Case') {
            for (const child of node.children ?? []) {
                assert.equal(child.nodeType, 'Runtime Warning', 'Unexpected child under a test method');
                assert.equal(typeof child.name, 'string');
                assert.equal(child.children, undefined);
                assert.equal(child.nodeIdentifier, undefined);
                assert.equal(child.nodeIdentifierURL, undefined);
                assert.equal(child.result, undefined);
                runtimeWarnings.push(child.name);
            }
            const url = new URL(node.nodeIdentifierURL);
            assert.equal(url.protocol, 'test:');
            assert.equal(url.hostname, 'com.apple.xcode');
            const parts = url.pathname.split('/').filter(Boolean);
            assert.equal(parts.length, 4, 'Unexpected test identity structure');
            assert.equal(parts[0], 'Screenroom');
            const [bundle, suite, method] = parts.slice(1);
            assert.equal(bundle, context.bundle, 'Test target differs from its container');
            assert.equal(suite, context.suite, 'Test class differs from its container');
            assert.equal(node.nodeIdentifier, `${suite}/${method}()`);
            assert.equal(node.name, `${method}()`);
            passed.push(`${bundle}/${suite}/${method}`);
        } else {
            assert.ok(['Test Plan', 'UI test bundle', 'Test Suite'].includes(node.nodeType), 'Unknown test node type');
            assert.ok(Array.isArray(node.children) && node.children.length > 0, 'Empty test container');
            let next = context;
            if (node.nodeType === 'UI test bundle') {
                assert.equal(new URL(node.nodeIdentifierURL).pathname, `/Screenroom/${node.name}`);
                next = { bundle: node.name };
            } else if (node.nodeType === 'Test Suite') {
                assert.equal(new URL(node.nodeIdentifierURL).pathname, `/Screenroom/${context.bundle}/${node.name}`);
                next = { ...context, suite: node.name };
            }
            node.children.forEach(child => visit(child, next));
        }
    }
    assert.ok(Array.isArray(tests.testNodes) && tests.testNodes.length > 0, 'No actual test nodes');
    tests.testNodes.forEach(node => visit(node));
    assert.equal(passed.length, expected.length, 'Actual test count differs from selection');
    assert.equal(new Set(passed).size, passed.length, 'Duplicate actual methods');
    assert.deepEqual([...passed].sort(), [...expected].sort(), 'Actual methods differ from selection');
    assert.equal(summary.result, 'Passed');
    assert.equal(summary.totalTestCount, expected.length);
    assert.equal(summary.passedTests, expected.length);
    for (const field of ['failedTests', 'skippedTests', 'expectedFailures']) assert.equal(summary[field], 0, field);
    assert.deepEqual(summary.testFailures, []);
    assert.ok(Array.isArray(summary.devicesAndConfigurations) && summary.devicesAndConfigurations.length === 1);
    const device = summary.devicesAndConfigurations[0];
    assert.equal(device.passedTests, expected.length);
    for (const field of ['failedTests', 'skippedTests', 'expectedFailures']) assert.equal(device[field], 0, field);
    return { verified: true, expected: [...expected].sort(), passed: [...passed].sort(), count: passed.length, runtimeWarnings };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    try {
        const [testsPath, summaryPath, ...selectors] = process.argv.slice(2);
        const evidence = verifyTestResults(JSON.parse(readFileSync(testsPath)), JSON.parse(readFileSync(summaryPath)), selectors);
        console.log(JSON.stringify(evidence, null, 2));
    } catch (error) {
        console.log(JSON.stringify({ verified: false, error: error.message }, null, 2));
        console.error(`Test-result verification failed: ${error.message}`);
        process.exitCode = 1;
    }
}
