#!/usr/bin/env python3
"""Local structural/security checks; does not provision or validate an account."""
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parents[2]
c = tomllib.loads((root / 'deploy/fly.toml').read_text())
assert isinstance(c['app'], str) and c['app'].strip()
assert c['env']['SENTINEL_DEPLOYMENT_MODE'] == 'production'
assert c['env']['SENTINEL_AUTO_MIGRATE'] == 'false'
assert c['env']['SENTINEL_WEBHOOK_FORMAT'] == 'discord'
assert 'SUI_GRAPHQL_URL' not in c['env']  # Requires an explicitly configured provider.
assert c['http_service']['force_https']
assert c['http_service']['auto_stop_machines'] == 'off'
assert not c['http_service']['auto_start_machines']
assert c['http_service']['checks'][0]['path'] == '/livez'
mounts = c['mounts'] if isinstance(c['mounts'], list) else [c['mounts']]
assert len(mounts) == 1 and mounts[0]['destination'] == '/data'
assert mounts[0]['snapshot_retention'] >= 7
assert '--listen 0.0.0.0:9108' in c['processes']['app']
assert 'release_command' not in c['deploy']
print('Fly TOML and local invariants valid; app ownership, secrets, volume ownership and provider access remain external gates.')
