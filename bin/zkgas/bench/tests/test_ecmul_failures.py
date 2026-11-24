"""Caught ECMUL failure requests must burn allowance and keep parent guards intact."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'
sys.path.insert(0,str(CASES))


class EcmulFailuresTests(unittest.TestCase):
    def load(self):return importlib.import_module('ecmul_failures')

    def test_six_cases_create_only_hashes_and_own_zero_baselines(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(m.prepare(output),6)
            catalog = json.loads((output/'catalog.json').read_bytes()); rows = catalog['cases']
            self.assertEqual({(r['parameters']['allowance'],r['parameters']['repetitions']) for r in rows},
                {(allowance,n) for allowance in (6000,200000) for n in (0,1,8)})
            self.assertEqual(len({r['sample_id'] for r in rows}),6)
            by_id = {r['sample_id']:r for r in rows}
            for row in rows:
                raw = (output/row['request_path']).read_bytes()
                self.assertEqual(row['request_sha256'],hashlib.sha256(raw).hexdigest())
                baseline = by_id[row['baseline_sample_id']]['parameters']
                self.assertEqual(baseline['repetitions'],0);self.assertEqual(baseline['allowance'],row['parameters']['allowance'])
                self.assertTrue(json.loads(raw)['expected_user_success'])
                self.assertFalse(row['parameters']['expected_subcall_success'])
            before = {p:p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(output)
            self.assertEqual(before,{p:p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_failed_subcalls_real_input_output_poison_counts_and_gas(self):
        m = self.load()
        for allowance in (1,5999,6000,152218,152219,200000,1000000):
            for n in (0,1,8):
                row = m.make_case(allowance,n); p = row['parameters']; program = row['request']['program']; gas = m.gas_accounting(allowance,n)
                raw = bytes.fromhex(program['input_hex'][2:])
                self.assertEqual(len(raw),96)
                self.assertEqual([int.from_bytes(raw[i:i+32],'big') for i in (0,32,64)],[1,1,0])
                self.assertEqual(p['payload_sha256'],hashlib.sha256(raw).hexdigest())
                self.assertEqual(p['expected_precompile_failed_calls'],n)
                self.assertEqual(p['expected_precompile_successful_calls'],0)
                self.assertFalse(p['expected_subcall_success']);self.assertTrue(p['expected_user_success'])
                self.assertEqual(p['expected_returndata_bytes'],0)
                self.assertEqual(p['expected_loop_counter'],n)
                self.assertEqual(program['expected_route_counts']['opcode:0xfa'],n)
                self.assertEqual(program['expected_route_counts']['opcode:0x3d'],n)
                self.assertEqual(program['expected_route_counts']['opcode:0x00'],1)
                self.assertEqual(program['expected_route_counts']['opcode:0xfe'],0)
                self.assertEqual(gas['ordinary_opcode_gas'],233+178*n)
                self.assertEqual(gas['gross_gas'],21671+n*(allowance+278))
                self.assertEqual(gas['precompile_failure_gas'],n*allowance)
                self.assertEqual(gas['call_access_gas'],100*n)
                self.assertEqual(gas['charged_gas'],max(gas['gross_gas'],gas['eip7623_floor_gas']))
                self.assertEqual(gas['transaction_envelope_reserve'],(allowance+62)//63+m.TAIL_GUARD_GAS)
                self.assertEqual(program['transactions']['gas_limit'],gas['charged_gas']+gas['transaction_envelope_reserve'])
                code = bytes.fromhex(program['code_hex'][2:])
                self.assertIn(b'\x73'+(7).to_bytes(20,'big')+b'\x62'+allowance.to_bytes(3,'big')+b'\xfa',code)
                for index,value in enumerate(m.OUTPUT_POISONS):
                    self.assertIn(b'\x7f'+value.to_bytes(32,'big')+b'\x62'+(128+32*index).to_bytes(3,'big')+b'\x52',code)

    def test_invalid_typed_bounds_and_budget(self):
        m = self.load()
        for allowance,n in ((True,1),(None,1),('6000',1),(6000.0,1),(0,1),(1000001,1),(6000,True),(6000,-1),(6000,4097),(6000,'1'),(200000,4096)):
            with self.subTest(allowance=allowance,n=n),self.assertRaises(ValueError):m.make_case(allowance,n)
        for output in (None,True,'catalog'):
            with self.assertRaises(ValueError):m.prepare(output)

    def test_stdlib_only_cli(self):
        self.load()
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run([sys.executable,'-S',str(CASES/'ecmul_failures.py'),'--output-dir',str(Path(temporary)/'cli')],capture_output=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr.decode());self.assertEqual(json.loads(result.stdout),{'prepared':6})

    @unittest.skipUnless(os.environ.get('ECMUL_FAILURES_NATIVE_BIN'),'set native constructor for failure qualification')
    def test_native_six_cases_and_four_allowance_edges(self):
        m = self.load()
        selections = [(a,n) for a in (6000,200000) for n in (0,1,8)]
        selections += [(a,1) for a in (5999,152218,152219)]  #6000 already qualified above; no actuals for these edges.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for allowance,n in selections:
                row = m.make_case(allowance,n);p = row['parameters'];folder = root/row['sample_id'];folder.mkdir()
                (folder/'request.json').write_bytes(m.encoded(row['request']))
                result = subprocess.run([os.environ['ECMUL_FAILURES_NATIVE_BIN'],'--input',str(folder/'request.json'),
                    '--output',str(folder/'input.json'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=60)
                self.assertEqual(result.returncode,0,f'{allowance}/{n}: {result.stderr.decode()}')
                report = json.loads((folder/'native.json').read_bytes())
                self.assertTrue(report['receipts_by_block'][0][1]['success'])
                self.assertEqual(report['user_transaction_gas_used_total'],p['gas_accounting_per_user']['charged_gas'])
                expected = row['request']['program']['expected_route_counts']
                self.assertEqual({k:v for k,v in report['observed_route_counts'].items() if v},{k:v for k,v in expected.items() if v})
                tx = json.loads((folder/'input.json').read_bytes())['witnesses'][0]['block']['body']['transactions'][1]['transaction']['Eip1559']
                self.assertEqual(tx['input'],row['request']['program']['input_hex'])


if __name__ == '__main__':unittest.main()
