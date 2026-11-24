import json
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parent.parent


class ComposeTests(unittest.TestCase):
    def test_role_mounts_and_bootstrap_are_restricted(self):
        compose = yaml.safe_load((ROOT / 'compose.yaml').read_text())
        services = compose['services']
        self.assertFalse(services['catalog'].get('volumes'))
        for role in ('catalog', 'worker'):
            env = services[role]['environment']
            self.assertTrue(all(v is None for k, v in env.items() if k.startswith('SCREENROOM_EMBEDDING_')))
        for role in ('user', 'worker', 'index-operator', 'playback-gateway'):
            service = services[role]
            mount = next(v for v in service['volumes'] if ':/run/screenroom/credentials' in v)
            self.assertEqual(mount.endswith(':ro'), role != 'index-operator')
            self.assertEqual(service['environment']['SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY'], '/run/screenroom/credentials')
        operator = services['index-operator']
        self.assertNotIn('ports', operator)
        self.assertNotIn('profiles', operator)
        self.assertIn('operator_runtime:', operator['environment']['DATABASE_URL'])
        self.assertIn('SCREENROOM_MIGRATION_DATABASE_URL', operator['environment'])
        gateway = services['playback-gateway']
        self.assertEqual(gateway['networks'], ['private'])
        self.assertNotIn('ports', gateway)
        self.assertEqual(gateway['stop_grace_period'], '75s')
        self.assertEqual(gateway['command'], ['screenroom-playback-gateway'])
        self.assertIn('http://127.0.0.1:50054/ready', gateway['healthcheck']['test'])
        self.assertEqual(gateway['volumes'], [
            'backend-jellyfin-credentials:/run/screenroom/credentials:ro',
            'backend-playback-secrets:/run/screenroom/playback'])
        for role, service in services.items():
            if role != 'playback-gateway':
                self.assertFalse(any('backend-playback-secrets:' in v for v in service.get('volumes', [])))
        env = gateway['environment']
        self.assertIn('playback_runtime:', env['DATABASE_URL'])
        self.assertEqual(env['SCREENROOM_MIGRATION_DATABASE_URL'], services['user']['environment']['SCREENROOM_MIGRATION_DATABASE_URL'])
        self.assertNotEqual(env['DATABASE_URL'], env['SCREENROOM_MIGRATION_DATABASE_URL'])
        self.assertEqual(env['SCREENROOM_PLAYBACK_API_ORIGIN'], services['catalog']['environment']['SCREENROOM_CATALOG_PUBLIC_ORIGIN'])
        self.assertEqual(env['SCREENROOM_PLAYBACK_GATEWAY_LISTEN'], '0.0.0.0:50054')
        origins = env['SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS']
        self.assertEqual(origins, services['user']['environment']['SCREENROOM_USER_CORS_ORIGINS'])
        self.assertEqual(origins, services['catalog']['environment']['SCREENROOM_CATALOG_CORS_ORIGINS'])
        self.assertNotIn('*', origins)
        self.assertEqual(env['SCREENROOM_PLAYBACK_SECRET_DIRECTORY'], '/run/screenroom/playback')
        self.assertIn('backend-playback-secrets', compose['volumes'])
        sql = (ROOT / 'docker/backend/postgres.sql').read_text()
        self.assertIn("IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_playback') THEN CREATE ROLE screenroom_playback NOLOGIN NOINHERIT;", sql)
        self.assertIn("IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='playback_runtime') THEN CREATE ROLE playback_runtime LOGIN", sql)
        self.assertIn("IN ROLE screenroom_playback;", sql)
        self.assertNotIn('ALTER ROLE', sql)
        self.assertNotIn('DROP ', sql)
        self.assertNotIn('IN ROLE screenroom;', sql)

    def test_playback_routes_preserve_streaming_and_redact_queries(self):
        config = yaml.safe_load((ROOT / 'docker/backend/envoy.yaml').read_text())
        hcm = config['static_resources']['listeners'][0]['filter_chains'][0]['filters'][0]['typed_config']
        routes = hcm['route_config']['virtual_hosts'][0]['routes']
        prefixes = {r['match'].get('prefix'): r for r in routes}
        self.assertEqual(prefixes['/screenroom.playback_gateway.v1.PlaybackService/']['route'],
                         dict(cluster='playback', timeout='35s'))
        media = prefixes['/playback/v1/media/']
        self.assertEqual(media['route'], dict(cluster='playback', timeout='0s', idle_timeout='60s'))
        self.assertEqual(media['match']['headers'], [
            dict(name=':method', string_match=dict(safe_regex=dict(regex='GET|HEAD|OPTIONS')))])
        self.assertEqual(media['tracing']['overall_sampling'], dict(numerator=0))
        self.assertNotIn('tracing', hcm)
        self.assertIn('traceparent', media['request_headers_to_remove'])
        self.assertNotIn(':path', media['request_headers_to_remove'])
        self.assertNotIn('response_headers_to_remove', media)
        self.assertEqual(hcm['request_headers_timeout'], '10s')
        self.assertEqual(hcm['stream_idle_timeout'], '60s')
        log = hcm['access_log'][0]['typed_config']['log_format']['json_format']
        self.assertEqual(log['path'], '%PATH(NQ:PATH)%')
        self.assertNotIn('%REQ(:PATH)%', json.dumps(hcm))
        self.assertNotIn('%REQ(X-ENVOY-ORIGINAL-PATH', json.dumps(hcm))
        self.assertFalse(any(r['match'].get('path') == '/ready' for r in routes))
        for prefix in ('/screenroom.catalog.v1.', '/artwork/', '/screenroom.user.v1.UserService/'):
            self.assertIn(prefix, prefixes)
        cluster = next(c for c in config['static_resources']['clusters'] if c['name'] == 'playback')
        endpoint = cluster['load_assignment']['endpoints'][0]['lb_endpoints'][0]['endpoint']['address']['socket_address']
        self.assertEqual(endpoint, dict(address='playback-gateway', port_value=50054))
        edge = (ROOT / 'docker/backend/envoy.yaml').read_text()
        self.assertNotIn('envoy.filters.http.cors', edge)
        self.assertNotIn('access-control-allow-origin', edge)


if __name__ == '__main__':
    unittest.main()
