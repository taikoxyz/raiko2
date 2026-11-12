"""Compact EIP-7702 signed-entry holdouts and stale-list native qualification."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class AuthorizationHoldoutTests(unittest.TestCase):
    def load(self):
        source = CASES/'authorization_holdout.py'
        self.assertTrue(source.is_file(), 'compact authorization holdout producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('authorization_holdout_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_distinct_five_plan_holdout_retains_every_hash_and_never_overwrites(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'holdout'
            self.assertEqual(cases.prepare(output), 5)
            catalog = json.loads((output/'catalog.json').read_text())
            rows = catalog['cases']
            self.assertEqual(len({row['sample_id'] for row in rows}), 5)
            self.assertEqual({(row['parameters']['count'], row['parameters']['block_count']) for row in rows},
                             {(1,1),(16,1),(128,1),(512,1),(16,3)})
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'authorization_holdout.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual(catalog['runtime_sha256'], cases.digest(bytes.fromhex('60006000f3')))
            for row in rows:
                content = (output/row['request_path']).read_bytes()
                self.assertEqual(cases.digest(content), row['request_sha256'])
                request = json.loads(content)
                program = request['program']
                count, blocks = row['parameters']['count'], row['parameters']['block_count']
                self.assertEqual(program['authorizations'], {'pattern':'compact','count':count})
                self.assertEqual(program['code_hex'], '0x60006000f3')
                self.assertEqual(program['input_hex'], '0x')
                self.assertEqual(program['expected_route_counts']['opcode:0xf3'], blocks)
                self.assertEqual(program['expected_route_counts']['opcode:0xfa'], 0)
                self.assertEqual(row['parameters']['expected_signed_entries_total'], count*blocks)
                self.assertEqual(row['parameters']['expected_applied_delegations_by_block'], [1]+[0]*(blocks-1))
                self.assertEqual(row['parameters']['signature'], {'r':'0x1','s':'0x1','y_parity':0})
                self.assertEqual(row['parameters']['authorization_nonce'], 0)
                self.assertTrue(row['holdout_only'])
            saved = {p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):
                cases.prepare(output)
            self.assertEqual(saved,{p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_typed_count_and_block_bounds_are_enforced(self):
        cases = self.load()
        for count in (0,1025,True,1.0,'1',None):
            with self.assertRaises(ValueError):
                cases.make_case(count,1)
        for blocks in (0,33,True,1.0,'1',None):
            with self.assertRaises(ValueError):
                cases.make_case(16,blocks)
        self.assertEqual(cases.make_case(1024,32)['parameters']['expected_signed_entries_total'], 32768)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set compact-capable native fixture')
    def test_real_native_compact_entries_and_stale_blocks_keep_separate_counts(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for count,blocks in ((1,1),(512,1),(16,3)):
                row = cases.make_case(count,blocks)
                folder = root/row['sample_id']
                folder.mkdir()
                request = folder/'request.json'
                request.write_bytes(cases.encoded(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(request),
                    '--output',str(folder/'input.json'),'--report',str(folder/'native.json'),
                    '--execution-profile','evm-gas-only'], capture_output=True,timeout=90)
                self.assertEqual(result.returncode,0,result.stderr.decode())
                report = json.loads((folder/'native.json').read_text())
                self.assertEqual(report['authorizations']['pattern'],'compact')
                self.assertEqual(report['observed_route_counts']['opcode:0xf3'],blocks)
                self.assertEqual([b['signed_entries'] for b in report['authorizations']['blocks']], [count]*blocks)
                self.assertEqual([b['applied_delegations'] for b in report['authorizations']['blocks']], [1]+[0]*(blocks-1))
                authorities = [b['authorities'] for b in report['authorizations']['blocks']]
                self.assertTrue(all(len(items)==1 for items in authorities))
                self.assertEqual(len({items[0]['address'] for items in authorities}),1)
                self.assertTrue(all(items[0]['nonce']==1 for items in authorities))
                self.assertTrue(all(block[1]['success'] for block in report['receipts_by_block']))
                self.assertTrue(all(b['user_transaction_gas_used']<15_000_000 for b in report['blocks']))


if __name__ == '__main__':
    unittest.main()
