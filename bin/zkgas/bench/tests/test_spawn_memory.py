import importlib.util
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'cases' / 'spawn_memory.py'
SPEC = importlib.util.spec_from_file_location('spawn_memory_cases', SOURCE)
cases = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cases)
sys.path.insert(0, str(SOURCE.parent))
from spawn_input import make_input_case


class SpawnMemoryTests(unittest.TestCase):
    def test_completion_gates_cover_both_spawn_levels_and_empty_leaf(self):
        for kind, opcode in enumerate(cases.OPCODES):
            row = cases.make_case('00', kind, 65536, 64)
            expected = row['request']['program']['expected_route_counts']
            self.assertEqual(expected['opcode:0x00'], 64)
            self.assertEqual(expected['opcode:0xf3'], 65)
            self.assertEqual(expected['opcode:0x3e'], 0)
            self.assertEqual(expected[f'opcode:0x{opcode:02x}'], 128 if kind == 0 else 64)
            self.assertEqual(row['expected_spawn_count'], 128)

    def test_matched_envelope_changes_only_length_word(self):
        left = cases.make_case('600000', 0, 0, 64)['request']['program']
        right = cases.make_case('600000', 0, 65536, 64)['request']['program']
        self.assertEqual(left['code_hex'], right['code_hex'])
        a, b = (bytes.fromhex(p['input_hex'][2:]) for p in (left, right))
        self.assertEqual(len(a), len(b))
        self.assertEqual(a[:64] + a[96:], b[:64] + b[96:])
        self.assertEqual(left['expected_route_counts'], right['expected_route_counts'])

    def test_invalid_boundaries_fail_before_emitting_requests(self):
        for kind, length, count in ((4, 0, 1), (0, 1, 1), (0, 1048608, 1),
                                    (0, 0, 0), (0, 0, 1025), (False, 0, 1)):
            with self.assertRaises(ValueError):
                cases.make_case('00', kind, length, count)

    def test_input_counterpart_preserves_gates_and_separates_identity(self):
        output = cases.make_case('00', 0, 65536, 64)
        inputs = make_input_case('00', 0, 65536, 64)
        self.assertNotEqual(output['sample_id'], inputs['sample_id'])
        self.assertEqual(output['request'], inputs['request'])
        self.assertEqual(inputs['parameters']['inner_calldata_length'], 65664)
        self.assertNotIn('output_length', inputs['parameters'])

    def test_finite_catalog_has_control_for_each_pair_and_no_clobber(self):
        compilation = {'source_sha256': 'a' * 64}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'catalog'
            with patch.object(cases, 'compile_source', return_value=('00', compilation)):
                self.assertEqual(cases.prepare(output, 'unused'), 44)
                import json
                catalog = json.loads((output / 'catalog.json').read_text())
                groups = {}
                for row in catalog['cases']:
                    groups.setdefault(row['pair_group'], set()).add(row['role'])
                    self.assertEqual(cases.digest((output / row['request_path']).read_bytes()),
                                     row['request_sha256'])
                self.assertTrue(all('control' in roles for roles in groups.values()))
                with self.assertRaises(FileExistsError):
                    cases.prepare(output, 'unused')


if __name__ == '__main__':
    unittest.main()
