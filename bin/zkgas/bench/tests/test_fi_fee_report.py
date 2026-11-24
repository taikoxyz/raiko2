"""Exact FI fee sensitivities; these fixtures are hypothetical, never observations."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BENCH = Path(__file__).resolve().parents[1]
TOOL = BENCH / 'fi_fee_report.py'


def load():
    assert TOOL.is_file(), 'FI fee sensitivity calculator is missing'
    spec = importlib.util.spec_from_file_location('fi_fee_test_subject', TOOL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def config():
    return {'schema_version': 1, 'evidence_kind': 'sensitivity',
            'base_fee_wei': '10000000000000000', 'doubling_thresholds': [50, 200, 1000],
            'initial_pending': 0, 'enqueue_count': 900,
            'pending_samples': [0, 25, 49, 50, 100, 200, 300, 400, 900],
            'sp1_usd_per_billion_pgu': '0.5', 'eth_usd': '2000',
            'reserve_multiplier': '1.5', 'fi_extra_pgu_scenarios': [
                {'name': 'explicit zero control', 'pgu': '0'},
                {'name': 'hypothetical 3.6B', 'pgu': '3600000000'}]}


class FeeTests(unittest.TestCase):
    def test_floor_and_doubling_boundaries(self):
        m = load()
        for model, pending, want in [('linear', 0, 3), ('linear', 1, 4),
                                     ('linear', 4, 9), ('doubling_interpolated', 1, 4),
                                     ('doubling_interpolated', 2, 6),
                                     ('doubling_interpolated', 4, 12)]:
            with self.subTest(model=model, pending=pending):
                self.assertEqual(m.fee_wei(model, 3, 2, pending), want)
        self.assertEqual(m.fee_wei('doubling_interpolated', 3, 5, 6), 7)
        self.assertEqual(m.fee_wei('doubling_interpolated', 1, 1, 255), 1 << 255)
        with self.assertRaises(OverflowError):
            m.fee_wei('doubling_interpolated', 1, 1, 256)

    def test_known_batch_and_sample_keep_scope_separate(self):
        m = load()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'; path.write_text(json.dumps(config()))
            report = m.build_report(path)
        batch = report['batches'][0]
        self.assertEqual((batch['model'], batch['threshold']), ('linear', 50))
        self.assertEqual(batch['total_fee_wei'], '89910000000000000000')
        self.assertEqual(batch['total_fee_eth'], '89.91')
        self.assertEqual((batch['status'], batch['priced_count'], batch['final_pending']),
                         ('complete', 900, 900))
        sample = next(row for row in report['samples'] if row['model'] == 'doubling_interpolated'
                      and row['threshold'] == 50 and row['pending'] == 900)
        self.assertEqual(sample['fee_eth'], '2621.44')
        self.assertTrue(report['sensitivity_only'])
        self.assertEqual(report['config'], config())

    def test_exact_costs_zero_control_and_ceiling_to_wei(self):
        m = load(); cfg = config()
        cfg.update(base_fee_wei='1', doubling_thresholds=[2], initial_pending=0,
                   enqueue_count=2, pending_samples=[0, 1],
                   sp1_usd_per_billion_pgu='0.000000001', eth_usd='3',
                   reserve_multiplier='1.5', fi_extra_pgu_scenarios=[
                       {'name': 'zero', 'pgu': '0'}, {'name': 'one PGU', 'pgu': '1'},
                       {'name': 'three PGU', 'pgu': '3'}])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'; path.write_text(json.dumps(cfg))
            report = m.build_report(path)
        costs = report['cost_scenarios']
        self.assertEqual([row['required_fee_wei'] for row in costs], ['0', '1', '2'])
        self.assertEqual(costs[1]['incremental_sp1_usd'],
                         {'numerator': '1', 'denominator': '1000000000000000000'})
        self.assertEqual(costs[1]['reserve_adjusted_sp1_usd'],
                         {'numerator': '3', 'denominator': '2000000000000000000'})
        self.assertEqual(report['samples'][0]['covers_reserve_adjusted_sp1_cost'],
                         {'zero': True, 'one PGU': True, 'three PGU': False})
        self.assertEqual(report['batches'][0]['covers_reserve_adjusted_sp1_cost'],
                         {'zero': True, 'one PGU': True, 'three PGU': False})

    def test_fee_overflow_is_bounded_and_partial_batch_has_no_full_total(self):
        m = load(); maximum = (1 << 256) - 1
        self.assertEqual(m.fee_wei('linear', maximum, 2, 0), maximum)
        for model, base, threshold, pending in [
                ('linear', maximum, 2, 1),
                ('doubling_interpolated', 1, 1, (1 << 64) - 1),
                ('doubling_interpolated', maximum, 1, 1)]:
            with self.subTest(model=model), self.assertRaises(OverflowError):
                m.fee_wei(model, base, threshold, pending)
        cfg = config(); cfg.update(base_fee_wei=str(1 << 254), doubling_thresholds=[1],
                                   pending_samples=[0, 2], enqueue_count=4)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'; path.write_text(json.dumps(cfg))
            report = m.build_report(path)
        batch = report['batches'][1]
        self.assertEqual(batch['status'], 'fee_overflow')
        self.assertEqual((batch['priced_count'], batch['first_unavailable_pending']), (2, 2))
        self.assertIsNone(batch['total_fee_wei'])
        self.assertEqual(batch['priced_prefix_fee_wei'], str(3 * (1 << 254)))
        self.assertIsNone(batch['covers_reserve_adjusted_sp1_cost'])
        self.assertEqual(report['samples'][3]['status'], 'fee_overflow')
        self.assertIsNone(report['samples'][3]['fee_wei'])

    def test_aggregate_sum_above_uint256_is_valid_accounting(self):
        m = load(); cfg = config()
        cfg.update(base_fee_wei=str(1 << 254), doubling_thresholds=[100],
                   initial_pending=0, enqueue_count=5, pending_samples=[0])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'; path.write_text(json.dumps(cfg))
            report = m.build_report(path)
        self.assertEqual(report['status'], 'complete')
        self.assertTrue(all(row['status'] == 'complete' for row in report['batches']))
        self.assertGreater(int(report['batches'][0]['total_fee_wei']), (1 << 256) - 1)

    def test_zero_denominator_and_invalid_direct_fee_inputs_reject(self):
        m = load()
        for model, base, threshold, pending in [('linear', 1, 0, 0),
                ('linear', 0, 1, 0), ('linear', 1, 1, -1),
                ('linear', 1, 1, True), ('unknown', 1, 1, 0)]:
            with self.subTest(model=model, threshold=threshold), self.assertRaises(ValueError):
                m.fee_wei(model, base, threshold, pending)

    def test_config_rejects_invalid_types_missing_unknown_and_bounds(self):
        m = load()
        changes = [('schema_version', True), ('schema_version', 2),
                   ('evidence_kind', 'measured'), ('base_fee_wei', '0'),
                   ('base_fee_wei', '-1'), ('base_fee_wei', str(1 << 256)),
                   ('base_fee_wei', '01'), ('doubling_thresholds', [0]),
                   ('doubling_thresholds', [1, 1]), ('doubling_thresholds', [True]),
                   ('doubling_thresholds', list(range(1, 18))),
                   ('doubling_thresholds', [1 << 64]), ('initial_pending', -1),
                   ('initial_pending', True), ('enqueue_count', 10001),
                   ('enqueue_count', -1), ('enqueue_count', 1.0),
                   ('pending_samples', [0, 0]), ('pending_samples', list(range(33))),
                   ('pending_samples', [-1]), ('pending_samples', [1 << 64]),
                   ('sp1_usd_per_billion_pgu', 0.5), ('eth_usd', '0'),
                   ('eth_usd', '1e3'), ('eth_usd', '2000.0'), ('eth_usd', '9' * 39),
                   ('eth_usd', '0.' + '0' * 18 + '1'), ('reserve_multiplier', '0.9'),
                   ('fi_extra_pgu_scenarios', []),
                   ('fi_extra_pgu_scenarios', [{'name': 'x', 'pgu': str(1 << 128)}]),
                   ('fi_extra_pgu_scenarios', [{'name': ' ', 'pgu': '1'}]),
                   ('fi_extra_pgu_scenarios', [{'name': 'x' * 65, 'pgu': '1'}]),
                   ('fi_extra_pgu_scenarios', [{'name': 'x', 'pgu': '1', 'extra': 0}]),
                   ('fi_extra_pgu_scenarios', [{'name': 'x', 'pgu': '1'}] * 2),
                   ('fi_extra_pgu_scenarios', [{'name': str(i), 'pgu': '1'} for i in range(33)])]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'
            for key, value in changes:
                cfg = config(); cfg[key] = value; path.write_text(json.dumps(cfg))
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    m.build_report(path)
            for key in config():
                cfg = config(); del cfg[key]; path.write_text(json.dumps(cfg))
                with self.subTest(missing=key), self.assertRaises(ValueError):
                    m.build_report(path)
            cfg = config(); cfg['unexpected'] = 0; path.write_text(json.dumps(cfg))
            with self.assertRaises(ValueError): m.build_report(path)

    def test_strict_json_and_span_boundaries(self):
        m = load()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'config.json'
            for raw in ['{"schema_version":1,"schema_version":1}',
                        json.dumps(config()).replace('"pgu": "0"', '"pgu": "0", "pgu": "1"'),
                        json.dumps(config()).replace('"eth_usd": "2000"', '"eth_usd": NaN'),
                        json.dumps(config()).replace('"eth_usd": "2000"', '"eth_usd": Infinity'),
                        '{' * 1000, '[]', ' ' * 65537]:
                path.write_text(raw)
                with self.subTest(raw=raw[:60]), self.assertRaises(ValueError):
                    m.build_report(path)
            cfg = config(); cfg.update(initial_pending=(1 << 64) - 2, enqueue_count=1)
            path.write_text(json.dumps(cfg)); report = m.build_report(path)
            self.assertEqual(report['batches'][0]['final_pending'], (1 << 64) - 1)
            cfg['enqueue_count'] = 2; path.write_text(json.dumps(cfg))
            with self.assertRaises(ValueError): m.build_report(path)
            cfg.update(initial_pending=0, enqueue_count=0)
            path.write_text(json.dumps(cfg)); report = m.build_report(path)
            self.assertEqual(report['batches'][0]['total_fee_wei'], '0')
            self.assertEqual(report['batches'][0]['priced_count'], 0)

    def test_cli_deterministic_exclusive_and_invalid_leaves_no_artifacts(self):
        load()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); path = root / 'config.json'; raw = json.dumps(config()).encode()
            path.write_bytes(raw)
            def run(output):
                return subprocess.run([sys.executable, str(TOOL), '--config', str(path),
                                       '--output', str(output)], capture_output=True, timeout=10)
            first, second = root / 'one.json', root / 'two.json'
            self.assertEqual(run(first).returncode, 0)
            self.assertEqual(run(second).returncode, 0)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            report = json.loads(first.read_bytes())
            self.assertEqual(report['config_sha256'], hashlib.sha256(raw).hexdigest())
            self.assertEqual(report['calculator_sha256'], hashlib.sha256(TOOL.read_bytes()).hexdigest())
            first.write_text('preserve existing output')
            self.assertEqual(run(first).returncode, 1)
            self.assertEqual(first.read_text(), 'preserve existing output')
            second.unlink(); first.unlink(); path.write_text('{"schema_version":1}')
            self.assertEqual(run(first).returncode, 1)
            self.assertEqual(sorted(p.name for p in root.iterdir()), ['config.json'])
            cfg = config(); cfg.update(doubling_thresholds=[1], pending_samples=[1000])
            path.write_text(json.dumps(cfg))
            self.assertEqual(run(first).returncode, 2)
            self.assertEqual(json.loads(first.read_bytes())['status'], 'partial')


if __name__ == '__main__':
    unittest.main()
