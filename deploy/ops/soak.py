#!/usr/bin/env python3
"""Collect wall-clock R1 evidence. Missing/failed samples count against availability."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import time
from urllib.request import Request, urlopen
from urllib.parse import urlsplit


def collect(args):
    url = urlsplit(args.url)
    if url.username or url.password or (url.scheme != 'https' and url.hostname not in ('localhost', '127.0.0.1', '::1')):
        raise ValueError('Use HTTPS, or loopback for local evidence; no URL credentials')
    if args.interval < 5 or args.seconds < args.interval:
        raise ValueError('Interval must be >=5 seconds; duration must include a sample')
    args.out.mkdir(parents=True, exist_ok=False)
    meta = {'start': time.time(), 'seconds': args.seconds, 'interval': args.interval,
            'endpoint': f'{url.scheme}://{url.netloc}{url.path}', 'schema_version': 1}
    (args.out / 'manifest.json').write_text(json.dumps(meta, indent=2) + '\n')
    origin = time.monotonic()
    with (args.out / 'samples.ndjson').open('w') as out:
        for slot in range(math.ceil(args.seconds / args.interval)):
            time.sleep(max(0, origin + slot * args.interval - time.monotonic()))
            row = {'slot': slot, 'observed_at': time.time(), 'available': False, 'ready': False}
            try:
                headers = {}
                if token := os.environ.get('SENTINEL_STATUS_AUTHORIZATION'):
                    headers['Authorization'] = token
                with urlopen(Request(args.url, headers=headers), timeout=4) as response:
                    payload = response.read(65537)
                    if len(payload) > 65536:
                        raise ValueError('Oversized status')
                    status = json.loads(payload)
                    row.update(available=True, status=status)
                    ops = status.get('operations') or {}
                    recent = all(isinstance(ops.get(k), (int, float)) and 0 <= row['observed_at'] - ops[k] <= 90
                                 for k in ('observed_at', 'scheduler_heartbeat', 'observer_heartbeat', 'delivery_heartbeat'))
                    row['ready'] = bool(status.get('ready') and status.get('coverage_complete')
                                        and status.get('observation_freshness') == 'fresh'
                                        and recent and ops.get('database_ok') is True and ops.get('observer_ok') is True)
            except Exception as error:  # Record the failed slot, never silently skip it.
                row['error_type'] = type(error).__name__  # Avoid URLs/credentials in exception text.
            out.write(json.dumps(row, separators=(',', ':')) + '\n')
            out.flush()
            os.fsync(out.fileno())
        time.sleep(max(0, origin + args.seconds - time.monotonic()))
    print(json.dumps(assess(args.out), indent=2))


def assess(directory, now=None):
    now = time.time() if now is None else now
    meta = json.loads((directory / 'manifest.json').read_text())
    interval, duration, start = meta['interval'], meta['seconds'], meta['start']
    if interval < 5 or duration < interval:
        raise ValueError('Invalid evidence schedule')
    elapsed = max(0, min(now - start, duration))
    expected = max(1, math.ceil(elapsed / interval))
    rows = {}
    for line in (directory / 'samples.ndjson').read_text().splitlines():
        row = json.loads(line)
        slot = row['slot']
        if not isinstance(slot, int) or slot < 0 or slot in rows:
            raise ValueError('Duplicate/invalid evidence slot')
        # Late samples cannot cover earlier downtime; future timestamps are invalid.
        due = start + slot * interval
        timely = due <= row['observed_at'] <= min(now, due + interval)
        rows[slot] = row if timely else {'available': False, 'ready': False}
    available = sum(bool(rows.get(i, {}).get('available')) for i in range(expected))
    ready = sum(bool(rows.get(i, {}).get('ready')) for i in range(expected))
    statuses = [r['status'] for r in rows.values() if r.get('status')]
    revisions = sorted({s.get('build', {}).get('revision', 'unrecorded') for s in statuses})
    pinned = bool(revisions) and all(len(r) == 40 and all(c in '0123456789abcdef' for c in r) for r in revisions)
    complete = elapsed >= duration and duration >= 7 * 86400
    ratios = (available / expected, ready / expected)
    verdict = 'PASS_SOAK_ONLY' if complete and pinned and min(ratios) >= .999 else ('FAIL' if complete else 'INCOMPLETE')
    return {'verdict': verdict, 'elapsed_seconds': elapsed, 'required_seconds': 7 * 86400,
            'expected_slots': expected, 'available_slots': available, 'ready_slots': ready,
            'missing_slots': sum(i not in rows for i in range(expected)),
            'query_availability': ratios[0], 'fresh_complete_availability': ratios[1],
            'build_revisions': revisions, 'build_pinned': pinned,
            'samples_sha256': hashlib.sha256((directory / 'samples.ndjson').read_bytes()).hexdigest(),
            'limitation': 'Sampled intervals, not exact continuous uptime. R1 additionally requires recovery, delivery, security and operator UX gates.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    sample = commands.add_parser('collect')
    sample.add_argument('--url', required=True)
    sample.add_argument('--out', type=Path, required=True)
    sample.add_argument('--seconds', type=int, default=7 * 86400)
    sample.add_argument('--interval', type=int, default=15)
    report = commands.add_parser('assess')
    report.add_argument('directory', type=Path)
    args = parser.parse_args()
    if args.command == 'collect':
        collect(args)
    else:
        result = assess(args.directory)
        print(json.dumps(result, indent=2))
        raise SystemExit(0 if result['verdict'] == 'PASS_SOAK_ONLY' else 2)


if __name__ == '__main__':
    main()
