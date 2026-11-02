import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
spec = importlib.util.spec_from_file_location('soak', Path(__file__).resolve().parents[1] / 'deploy/ops/soak.py')
soak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(soak)


class Evidence(unittest.TestCase):
    def test_missing_and_late_samples_fail_and_short_runs_cannot_pass_r1(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            (path / 'manifest.json').write_text(json.dumps({'start': 100, 'seconds': 30, 'interval': 10}))
            status = {'build': {'revision': 'a' * 40}}
            rows = [{'slot': 0, 'observed_at': 100, 'available': True, 'ready': True, 'status': status},
                    {'slot': 2, 'observed_at': 140, 'available': True, 'ready': True, 'status': status}]
            (path / 'samples.ndjson').write_text('\n'.join(map(json.dumps, rows)))
            result = soak.assess(path, now=140)
            self.assertEqual(result['verdict'], 'INCOMPLETE')
            self.assertEqual(result['available_slots'], 1)
            self.assertEqual(result['missing_slots'], 1)
            self.assertEqual(result['expected_slots'], 3)

    def test_duplicate_slots_cannot_inflate_availability(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            (path / 'manifest.json').write_text(json.dumps({'start': 0, 'seconds': 30, 'interval': 10}))
            (path / 'samples.ndjson').write_text('{"slot":0,"observed_at":0}\n' * 2)
            with self.assertRaises(ValueError):
                soak.assess(path, now=30)


if __name__ == '__main__':
    unittest.main()
