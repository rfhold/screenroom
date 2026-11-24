"""Offline parity checks for the explicit deployment graphs."""

import json
from pathlib import Path
import re
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ARCHES = ['amd64', 'arm64']
USER = '10001:10001'


def pipeline(name):
    data = yaml.safe_load((ROOT / f'.tekton/screenroom-{name}.yaml').read_text())
    tasks = data['spec']['pipelineSpec']['tasks']
    result = {task['name']: task for task in tasks}
    assert len(result) == len(tasks), 'duplicate task name'
    return result


def params(task):
    return {entry['name']: entry['value'] for entry in task.get('params', [])}


def image_result(task):
    return f'$(tasks.{task}.results.image)'


class Packaging(unittest.TestCase):
    def test_explicit_images_match_across_preview_release_and_pulumi(self):
        preview = pipeline('preview')
        release = pipeline('release')
        promoted = json.loads(params(release['promote-release'])['images'])
        deployment = json.loads(params(preview['deploy-preview'])['image-config'])
        pulumi = (ROOT / 'infra/pulumi/index.ts').read_text()
        components = pulumi.split('const components = {', 1)[1].split('} as const;', 1)[0]
        roles = dict(re.findall(r'^\s*"?([\w-]+)"?: "(\w+Image)",$', components, re.M))
        self.assertEqual(set(roles), {'catalog', 'user', 'worker', 'playback-gateway', 'index-operator'})
        self.assertIn('config.require("webImage")', pulumi)
        roles['web'] = 'webImage'
        self.assertEqual(set(deployment), set(roles.values()))
        self.assertCountEqual((entry['key'] for entry in promoted), roles.values())
        self.assertCountEqual(preview['deploy-preview']['runAfter'], [r + '-manifest' for r in roles])
        self.assertEqual(release['deploy-production']['runAfter'], ['promote-release'])
        self.assertEqual(release['promote-release']['runAfter'], ['validate-release'])
        self.assertEqual(params(release['deploy-production'])['image-config'], '$(tasks.promote-release.results.images)')
        self.assertEqual(set(preview), {'fetch-repository', 'deploy-preview'}
                         | {f'{r}-{suffix}' for r in roles for suffix in [*ARCHES, 'manifest']})
        self.assertEqual(set(release), {'validate-release', 'promote-release', 'deploy-production'})
        for role, key in roles.items():
            with self.subTest(role=role):
                entry = next(entry for entry in promoted if entry['key'] == key)
                self.assertTrue(entry['repository'].endswith('/screenroom-' + role))
                self.assertCountEqual(entry['platforms'], ARCHES)
                self.assertEqual(entry['user'], USER)
                manifest = preview[role + '-manifest']
                self.assertEqual(manifest['taskRef']['name'], 'service-manifest-v1')
                self.assertEqual(deployment[key], image_result(role + '-manifest'))
                self.assertEqual(params(manifest)['expected-user'], USER)
                self.assertCountEqual(json.loads(params(manifest)['sources']), [
                    {'arch': arch, 'image': image_result(f'{role}-{arch}')} for arch in ARCHES])
                for arch in ARCHES:
                    task = preview[f'{role}-{arch}']
                    self.assertEqual(task['taskRef']['name'], 'service-build-v1')
                    build = params(task)
                    self.assertEqual(build['arch'], arch)
                    self.assertEqual(build['expected-user'], USER)
                    self.assertEqual(build['repository'], params(manifest)['repository'])
                    dockerfile = (ROOT / build['dockerfile']).read_text()
                    self.assertRegex(dockerfile, rf'(?m)^FROM .+ AS {re.escape(build["target"])}$')
                    if role != 'web':
                        self.assertEqual(build['target'], role)


if __name__ == '__main__':
    unittest.main()
