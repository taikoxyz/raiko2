"""Prospective code-preparation validation selected before new-model measurements."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class CodePreparationHoldoutTests(unittest.TestCase):
    def load(self):
        p = CASES/'code_preparation_holdout.py'
        self.assertTrue(p.is_file(), 'prospective code-preparation producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('code_preparation_holdout_cases', p)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_fixed_fourteen_unfitted_two_known_sources_and_preserved_requests(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(m.prepare(root), 16)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']
            self.assertEqual(catalog['prospective_unfitted_case_count'], 14)
            self.assertEqual(catalog['previously_observed_reference_replay_count'], 2)
            self.assertEqual(catalog['frozen_policy']['unique_code_byte_pgu'], 64)
            self.assertEqual(catalog['frozen_policy']['nonempty_preparation_pgu'], 8192)
            self.assertEqual(catalog['frozen_policy']['prepared_byte_pgu'], 32)
            self.assertEqual(catalog['frozen_policy']['revision'], '5fd6697f')
            self.assertEqual(len({r['sample_id'] for r in rows}), 16)
            self.assertEqual(sum(r['holdout_only'] for r in rows), 14)
            self.assertEqual(sum(r['parameters']['validation_family']=='factory' for r in rows), 2)
            self.assertEqual(sum(r['parameters']['validation_family']=='loading' for r in rows), 6)
            self.assertEqual(sum(r['parameters']['validation_family']=='creation' for r in rows), 6)
            for row in rows:
                request = (root/row['request_path']).read_bytes()
                self.assertEqual(m.digest(request), row['request_sha256'])
                self.assertIn(row['baseline_sample_id'], {r['sample_id'] for r in rows})
                self.assertEqual(row['data_role'], 'prospective_unfitted_candidate' if row['holdout_only'] else 'previously_observed_reference_replay')
                if not row['holdout_only']: self.assertIsNotNone(row['prior_observed_reference_sample_id'])
            before = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(before, {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_factories_check_child_create_success_and_keep_outer_revert_work(self):
        m = self.load()
        for outer_revert in (False, True):
            row = m.factory(outer_revert); program = row['request']['program']
            code = bytes.fromhex(program['code_hex'][2:])
            self.assertEqual(code[:7], bytes.fromhex('656160006000f3'))
            self.assertIn(bytes.fromhex('f0801561'), code)
            self.assertEqual(code[-2:], bytes.fromhex('5bfe'))
            self.assertEqual(program['expected_route_counts']['opcode:0xf0'], 1)
            self.assertEqual(program['expected_route_counts']['opcode:0xf3'], 1)
            self.assertEqual(program['expected_route_counts']['opcode:0xfd'], int(outer_revert))
            self.assertEqual(program['expected_route_counts']['opcode:0x00'], int(not outer_revert))
            self.assertEqual(row['request']['expected_user_success'], not outer_revert)
            self.assertEqual(row['parameters']['expected_code_preparation'], {'nonempty_events':1,'total_bytes':24576})
        with self.assertRaises(ValueError): m.factory(1)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set frozen constructor for all sixteen native gates')
    def test_all_sixteen_native_inputs_include_real_child_and_user_work(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); m.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                folder = root/row['sample_id']; folder.mkdir()
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(root/'cases'/row['request_path']),
                    '--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),
                    '--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                self.assertEqual(result.returncode,0,f"{row['sample_id']}: {result.stderr.decode()}")
                native = json.loads((folder/'native.json').read_text()); request = json.loads((root/'cases'/row['request_path']).read_text())
                for opcode,expected in request['program']['expected_route_counts'].items():
                    self.assertEqual(native['observed_route_counts'].get(opcode,0),expected)
                self.assertTrue(all(r[0]['success'] and all(t['success']==request['expected_user_success'] for t in r[1:]) for r in native['receipts_by_block']))
                if row['parameters']['validation_family']=='factory':
                    self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],int(request['expected_user_success']))


if __name__=='__main__': unittest.main()
