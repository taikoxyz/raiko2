"""Finite EIP-7702 recovery/nonce-pattern catalog and native execution gates."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class AuthorizationValuesTests(unittest.TestCase):
    def load(self):
        source = CASES/'authorization_values.py'
        self.assertTrue(source.is_file(), 'authorization value producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('authorization_value_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_12_plan_matrix_one_shared_baseline_reference_hashes_and_no_overwrite(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(cases.prepare(output), 12)
            catalog = json.loads((output/'catalog.json').read_text())
            rows = catalog['cases']
            self.assertEqual(len({row['sample_id'] for row in rows}), 12)
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'authorization_values.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual(catalog['runtime_sha256'], cases.digest(bytes.fromhex('60006000f3')))
            requests = [(row, json.loads((output/row['request_path']).read_text())) for row in rows]
            expected = {(pattern, count) for pattern in ('sequential', 'repeated') for count in (4, 16, 64, 256)}
            expected |= {('distinct', 4), ('distinct', 16), ('sequential', 1)}
            self.assertEqual({(req['program']['authorizations']['pattern'], req['program']['authorizations']['count'])
                              for _, req in requests if 'authorizations' in req['program']}, expected)
            self.assertEqual(sum('authorizations' not in req['program'] for _, req in requests), 1)
            self.assertEqual(sum(row['parameters']['count'] == 1 for row in rows), 1)
            self.assertEqual(len({(output/row['request_path']).read_bytes() for row in rows}), 12)
            for row, req in requests:
                self.assertEqual(cases.digest((output/row['request_path']).read_bytes()), row['request_sha256'])
                self.assertEqual(req['program']['code_hex'], '0x60006000f3')
                self.assertEqual(req['program']['input_hex'], '0x')
                self.assertEqual(req['program']['expected_route_counts']['opcode:0xf3'], 1)
                self.assertEqual(req['program']['expected_route_counts']['opcode:0xfa'], 0)
            saved = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):
                cases.prepare(output)
            self.assertEqual(saved, {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_typed_bounds_never_emit_empty_type4_list(self):
        cases = self.load()
        for pattern, count in ((None, 0), ('unknown', 1), (True, 1), ('sequential', 0),
                               ('sequential', 1025), ('distinct', True), ('repeated', '1')):
            with self.assertRaises(ValueError):
                cases.make_case(pattern, count)
        reference = cases.make_case()
        self.assertNotIn('authorizations', reference['request']['program'])
        self.assertEqual(reference['parameters']['transaction_type'], 'eip1559')
        for pattern in ('sequential', 'repeated', 'distinct'):
            row = cases.make_case(pattern, 1024)
            self.assertEqual(row['parameters']['count'], 1024)
            self.assertEqual(row['parameters']['expected_applied_delegations'], 1 if pattern == 'repeated' else 1024)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set new native fixture for authorization qualification')
    def test_real_native_patterns_have_actual_nonce_and_delegation_gates(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for pattern, count in ((None, None), ('sequential', 1), ('sequential', 4),
                                   ('repeated', 4), ('distinct', 4)):
                row = cases.make_case(pattern, count)
                directory = root/row['sample_id']
                directory.mkdir()
                path = directory/'request.json'
                path.write_bytes(cases.encoded(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(path),
                    '--execution-profile', 'evm-gas-only', '--output', str(directory/'guest.json'),
                    '--report', str(directory/'native.json')], capture_output=True, timeout=45)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                report = json.loads((directory/'native.json').read_text())
                self.assertEqual(report['receipts_by_block'][0][1]['success'], True)
                self.assertEqual(report['observed_route_counts']['opcode:0xf3'], 1)
                if pattern is None:
                    self.assertNotIn('authorizations', report)
                    continue
                auth = report['authorizations']['blocks'][0]
                self.assertEqual(auth['signed_entries'], count)
                self.assertEqual(auth['applied_delegations'], row['parameters']['expected_applied_delegations'])
                self.assertEqual(len(auth['authorities']), count if pattern == 'distinct' else 1)
                for authority in auth['authorities']:
                    self.assertEqual(authority['nonce'], count if pattern == 'sequential' else 1)
                    self.assertEqual(authority['previous_nonce'], 0)


if __name__ == '__main__':
    unittest.main()
