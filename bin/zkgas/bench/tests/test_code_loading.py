"""Large initial runtime with a single executed STOP; code loading stays separate from input."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class CodeLoadingTests(unittest.TestCase):
    def load(self):
        path = CASES/'code_loading.py'
        self.assertTrue(path.is_file(), 'code-loading producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('code_loading_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_exact_ten_matrix_data_locations_references_and_no_overwrite(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(cases.prepare(root), 10)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by_id = {r['sample_id']:r for r in rows}
            self.assertEqual(len(by_id), 10)
            self.assertEqual(catalog['previously_observed_reference_replay_count'], 1)
            self.assertEqual(catalog['new_model_development_case_count'], 9)
            self.assertEqual({(r['parameters']['runtime_bytes'],r['parameters']['pattern']) for r in rows}, {(1,'zero')} | {(n,p) for n in (32,4096,24576) for p in ('zero','jumpdest','push32')})
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'code_loading.py').read_bytes()))
            self.assertEqual(catalog['base_generator_sha256'], cases.digest((CASES/'top_level_create.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            requests = set()
            for row in rows:
                p = row['parameters']; raw = (root/row['request_path']).read_bytes(); request = json.loads(raw); program = request['program']; code = bytes.fromhex(program['code_hex'][2:]); requests.add(raw)
                self.assertEqual(cases.digest(raw), row['request_sha256']); self.assertEqual(cases.digest(code), p['runtime_sha256'])
                self.assertEqual(program['transactions'], {'kind':'call','count_per_block':1,'gas_limit':100000})
                self.assertEqual(program['block_count'], 1); self.assertEqual(program['input_hex'], '0x')
                self.assertEqual(len(code), p['runtime_bytes']); self.assertEqual(code[0], 0)
                self.assertEqual(p['bytes_location'], 'prestate_runtime'); self.assertEqual(p['input_calldata_bytes'], 0)
                self.assertEqual(p['transaction_kind'], 'call'); self.assertEqual(p['count_per_block'], 1)
                self.assertEqual(by_id[row['baseline_sample_id']]['parameters']['runtime_bytes'], 1)
                self.assertNotIn('expected_runtime_hex', program['transactions'])
                self.assertEqual(program['expected_route_counts']['opcode:0x00'], 1)
                self.assertTrue(all(v == (1 if op == 'opcode:0x00' else 0) for op,v in program['expected_route_counts'].items()))
                if p['pattern'] == 'jumpdest': self.assertEqual(code[1:], bytes([0x5b])*(len(code)-1))
                elif p['pattern'] == 'push32': self.assertEqual(code[1:], ((b'\x7f'+b'\x5b'*32)*((len(code)+31)//33))[:len(code)-1])
                else: self.assertEqual(code, bytes(len(code)))
                if p['pattern'] != 'zero':
                    reference = by_id[row['same_length_reference_sample_id']]['parameters']
                    self.assertEqual(reference['runtime_bytes'], p['runtime_bytes']); self.assertEqual(reference['pattern'], 'zero')
                else: self.assertNotIn('same_length_reference_sample_id', row)
            self.assertEqual(len(requests), 10)
            baseline = next(r for r in rows if r['parameters']['runtime_bytes'] == 1)
            original = cases.original.make_case('call', 'stop')
            self.assertEqual((root/baseline['request_path']).read_bytes(), cases.encoded(original['request']))
            before = {str(p.relative_to(root)):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): cases.prepare(root)
            self.assertEqual(before, {str(p.relative_to(root)):p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_first_opcode_stop_makes_every_suffix_unreachable(self):
        cases = self.load()
        for length in (1,32,4096,24576):
            for pattern in ('zero','jumpdest','push32'):
                row = cases.make_case(length,pattern)
                code = bytes.fromhex(row['request']['program']['code_hex'][2:])
                # Independent execution of the only reachable instruction: no
                # suffix byte can become an executed JUMPDEST/PUSH32/CREATE.
                pc = 0; op = code[pc]; pc += 1
                self.assertEqual(op, 0); self.assertEqual(pc, 1)
                self.assertEqual(row['request']['program']['expected_route_counts']['opcode:0x00'], 1)

    def test_typed_length_and_pattern_bounds(self):
        cases = self.load()
        for length,pattern in [(True,'zero'),('32','zero'),(0,'zero'),(24577,'zero'),(32,True),(32,None),(32,[]),(32,'unknown')]:
            with self.subTest(length=length,pattern=pattern), self.assertRaises(ValueError): cases.make_case(length,pattern)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'), 'set frozen native and quote for loading qualification')
    def test_all_ten_native_and_quote_code_byte_accounting(self):
        cases = self.load(); results = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); cases.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                p = row['parameters']; folder = root/row['sample_id']; folder.mkdir()
                native = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(root/'cases'/row['request_path']),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                self.assertEqual(native.returncode,0,native.stderr.decode())
                report = json.loads((folder/'native.json').read_text()); guest = json.loads((folder/'input.json').read_text())
                self.assertEqual(report['transaction_count'],2); self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
                self.assertEqual(report['observed_route_counts'], {'opcode:0x00':1}); self.assertEqual(report['attempted_route_counts'], {'opcode:0x00':1})
                self.assertEqual(report['user_transaction_gas_used_total'],21000)
                self.assertEqual(report['transactions']['kind'],'call'); self.assertIsNone(report['transactions']['blocks'][0]['users'][0]['create_address'])
                self.assertIn(json.loads((root/'cases'/row['request_path']).read_text())['program']['code_hex'],guest['witnesses'][0]['witness']['codes'])
                quote = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','code-loading-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(quote.returncode,0,quote.stderr.decode())
                q = json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(q['model_id'],'3794f020f947fbaff4b26e8128996fcf9dcffb350b4360cc1d3d4114a809605e')
                results.append((row,q))
            baseline = next(q for row,q in results if row['parameters']['runtime_bytes']==1)
            for row,q in results:
                resources = q['blocks'][0]['node']['pricing']['evm_local_resources']; reference = baseline['blocks'][0]['node']['pricing']['evm_local_resources']
                self.assertGreaterEqual(resources['unique_code_bytes'],reference['unique_code_bytes'])
                self.assertEqual(q['blocks'][0]['prove']['exact_resources'], resources)
                self.assertEqual(q['blocks'][0]['node']['pricing']['transaction_payload'],baseline['blocks'][0]['node']['pricing']['transaction_payload'])
                # Source-derived expectation is checked against actual fields;
                # checkpoint STOP shares the baseline program's code hash.
                length = row['parameters']['runtime_bytes']
                self.assertEqual(resources['unique_code_bytes']-reference['unique_code_bytes'],0 if length==1 else length)
                self.assertEqual(resources['unique_code_hash_count']-reference['unique_code_hash_count'],0 if length==1 else 1)
                if row['parameters']['pattern'] != 'zero':
                    zero = next(other for prior,other in results if prior['parameters']['runtime_bytes'] == length and prior['parameters']['pattern'] == 'zero')
                    self.assertEqual(resources, zero['blocks'][0]['node']['pricing']['evm_local_resources'])


if __name__ == '__main__': unittest.main()
