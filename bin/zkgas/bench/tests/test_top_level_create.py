"""Top-level creation matrix, canonical controls and native all-user gates."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class TopLevelCreateTests(unittest.TestCase):
    def load(self):
        source = CASES/'top_level_create.py'
        self.assertTrue(source.is_file(), 'top-level creation producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('top_level_create_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_fixed_matrix_unique_requests_and_equal_block_references(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'
            self.assertEqual(cases.prepare(root), 44)
            catalog = json.loads((root/'catalog.json').read_text())
            rows = catalog['cases']; by_id = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by_id), 44)
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'top_level_create.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual({r['parameters']['transaction_kind'] for r in rows}, {'call', 'create'})
            self.assertEqual({r['parameters']['count_per_block'] for r in rows}, {1, 16, 128, 512})
            self.assertEqual({r['parameters']['block_count'] for r in rows}, {1, 3})
            requests = set()
            for row in rows:
                encoded = (root/row['request_path']).read_bytes(); requests.add(encoded)
                self.assertEqual(row['request_sha256'], cases.digest(encoded))
                request = json.loads(encoded); program = request['program']; params = row['parameters']
                baseline = by_id[row['baseline_sample_id']]
                self.assertEqual(baseline['parameters']['block_count'], params['block_count'])
                self.assertEqual(program['transactions']['kind'], params['transaction_kind'])
                self.assertEqual(program['transactions']['count_per_block'], params['count_per_block'])
                self.assertEqual(program['transactions']['gas_limit'], params['transaction_gas_limit'])
                self.assertEqual(program['input_hex'], '0x')
                self.assertEqual(program['block_count'], params['block_count'])
                self.assertEqual(request['expected_user_success'], params['expected_user_success'])
                self.assertFalse({'storage_state', 'authorizations', 'access_list'} & program.keys())
                self.assertEqual(program['expected_route_counts']['opcode:0xf0'], 0)
                self.assertEqual(program['expected_route_counts']['opcode:0xf5'], 0)
            self.assertEqual(len(requests), 44)
            before = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): cases.prepare(root)
            self.assertEqual(before, {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_unreachable_tails_change_analysis_data_without_extra_execution(self):
        cases = self.load()
        for length in (32, 4096, 16384, 49152):
            row = cases.make_case('create', 'stop_zero', length=length, gas_limit=3_000_000)
            program = row['request']['program']; code = bytes.fromhex(program['code_hex'][2:])
            self.assertEqual(code, bytes(length))
            self.assertEqual(program['expected_route_counts']['opcode:0x00'], 1)
            self.assertEqual(program['transactions']['expected_runtime_hex'], '0x')
        for shape, chunk in [('stop_jumpdest', b'\x5b'), ('stop_push32', b'\x7f'+b'\x5b'*32)]:
            row = cases.make_case('create', shape, length=49152, gas_limit=3_000_000)
            code = bytes.fromhex(row['request']['program']['code_hex'][2:])
            self.assertEqual(len(code), 49152)
            self.assertEqual(code[0], 0)
            self.assertEqual(code[1:1+len(chunk)], chunk)
            self.assertEqual(row['request']['program']['expected_route_counts']['opcode:0x00'], 1)
            self.assertEqual(row['request']['program']['expected_route_counts']['opcode:0x5b'], 0)
        empty = cases.make_case('create', 'empty')
        self.assertEqual(empty['request']['program']['code_hex'], '0x')
        self.assertEqual(empty['request']['program']['expected_route_counts']['opcode:0x00'], 1)
        for size in (0, 32, 256, 24576, 24577):
            row = cases.make_case('create', 'return_runtime', runtime_bytes=size, gas_limit=8_000_000)
            program = row['request']['program']
            self.assertEqual(program['code_hex'], '0x61'+format(size, '04x')+'6000f3')
            self.assertEqual(row['request']['expected_user_success'], size <= 24576)
            if size <= 24576:
                self.assertEqual(program['transactions']['expected_runtime_hex'], '0x'+'00'*size)
            else:
                self.assertNotIn('expected_runtime_hex', program['transactions'])
        for shape, length in [('revert_zero', 5), ('invalid_zero', 1)]:
            row = cases.make_case('create', shape, length=length, gas_limit=3_000_000)
            self.assertFalse(row['request']['expected_user_success'])
            self.assertNotIn('expected_runtime_hex', row['request']['program']['transactions'])

    def test_typed_bounds_reject_unknown_or_misleading_cases(self):
        cases = self.load()
        bad = [dict(kind='unknown', shape='empty'), dict(kind=True, shape='empty'),
               dict(kind='create', shape=[]), dict(kind='create', shape=True),
               dict(kind='create', shape='empty', count=0),
               dict(kind='create', shape='empty', count=True),
               dict(kind='create', shape='empty', count=2049),
               dict(kind='create', shape='empty', block_count=True),
               dict(kind='create', shape='empty', block_count=33),
               dict(kind='create', shape='empty', gas_limit=20_999),
               dict(kind='create', shape='empty', gas_limit=True),
               dict(kind='create', shape='empty', gas_limit=15_000_001),
               dict(kind='create', shape='stop_zero', length=0),
               dict(kind='create', shape='stop_zero', length=49153),
               dict(kind='create', shape='stop_zero', length=True),
               dict(kind='create', shape='return_zero', length=4),
               dict(kind='call', shape='empty'),
               dict(kind='create', shape='return_runtime', runtime_bytes=True),
               dict(kind='create', shape='return_runtime', runtime_bytes=65536)]
        for kwargs in bad:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError): cases.make_case(**kwargs)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set frozen native for creation qualification')
    def test_all_real_native_cases_have_every_user_and_aggregate_correct_gas(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); cases.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                folder = root/row['sample_id']; folder.mkdir()
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(root/'cases'/row['request_path']),
                    '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'),
                    '--report', str(folder/'native.json'), '--execution-profile', 'evm-gas-only'],
                    capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, f"{row['sample_id']}: {result.stderr.decode()}")
                report = json.loads((folder/'native.json').read_text()); params = row['parameters']
                count = params['count_per_block']; blocks = params['block_count']
                self.assertEqual(report['transaction_count'], blocks*(count+1))
                self.assertEqual(report['transactions']['kind'], params['transaction_kind'])
                self.assertEqual(report['transactions']['count_per_block'], count)
                all_user_gas = 0
                for index, receipts in enumerate(report['receipts_by_block']):
                    self.assertEqual(len(receipts), count+1)
                    self.assertTrue(receipts[0]['success'])
                    for receipt in receipts[1:]:
                        self.assertEqual(receipt['success'], params['expected_user_success'])
                        self.assertLessEqual(receipt['gas_used'], params['transaction_gas_limit'])
                    user_gas = sum(r['gas_used'] for r in receipts[1:]); all_user_gas += user_gas
                    self.assertEqual(report['blocks'][index]['user_transaction_gas_used'], user_gas)
                    self.assertEqual(report['blocks'][index]['block_gas_used'], user_gas+receipts[0]['gas_used'])
                self.assertEqual(report['user_transaction_gas_used_total'], all_user_gas)
                self.assertEqual(sum(b['exceptional_instructions'] for b in report['observations_by_block']),
                                 params['expected_exceptional_instructions'])
                users = [u for b in report['transactions']['blocks'] for u in b['users']]
                self.assertEqual(len(users), count*blocks)
                self.assertEqual([u['nonce'] for u in users], list(range(count*blocks)))
                if params['transaction_kind'] == 'create':
                    self.assertTrue(all(u['to'] is None and u['create_address'] is not None for u in users))
                    self.assertTrue(all((u['poststate'] is not None) == params['expected_created_account_present']
                                        for u in users))
                    if params['expected_created_account_present']:
                        self.assertTrue(all(u['poststate']['nonce'] == 1 for u in users))
                else:
                    self.assertTrue(all(u['create_address'] is None for u in users))
                for opcode, expected in json.loads((root/'cases'/row['request_path']).read_text())['program']['expected_route_counts'].items():
                    self.assertEqual(report['observed_route_counts'].get(opcode, 0), expected)


if __name__ == '__main__': unittest.main()
