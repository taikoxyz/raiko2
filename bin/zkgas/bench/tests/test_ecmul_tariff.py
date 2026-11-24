"""Finite ECMUL scalar/point fit and reserved probes, with old-request parity."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'
sys.path.insert(0,str(CASES))


class EcmulTariffTests(unittest.TestCase):
    def load(self):
        return importlib.import_module('ecmul_tariff')

    def test_finite_fit_reserved_catalogs_same_input_baselines_and_create_only(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for selection, expected in (('fit',18),('reserved',4),('all',22)):
                output = root/selection
                self.assertEqual(m.prepare(output,selection=selection),expected)
                catalog = json.loads((output/'catalog.json').read_bytes()); rows = catalog['cases']
                self.assertEqual(len({row['sample_id'] for row in rows}),expected)
                by_id = {row['sample_id']:row for row in rows}
                fit = {(p,s,n) for p in (1,2) for s in ('zero','two','dense') for n in (0,1,8)}
                reserved = {(3,s,n) for s in ('two','dense') for n in (0,4)}
                self.assertEqual({(r['parameters']['point_multiple'],r['parameters']['scalar_case'],r['parameters']['repetitions']) for r in rows},
                    fit if selection == 'fit' else reserved if selection == 'reserved' else fit|reserved)
                for row in rows:
                    p = row['parameters']; raw = (output/row['request_path']).read_bytes()
                    self.assertEqual(row['request_sha256'],hashlib.sha256(raw).hexdigest())
                    baseline = by_id[row['baseline_sample_id']]
                    self.assertEqual(baseline['parameters']['repetitions'],0)
                    self.assertEqual(baseline['parameters']['input_hex'],p['input_hex'])
                    self.assertEqual(row['data_role'],'fit' if p['point_multiple'] in (1,2) else 'reserved')
                    self.assertNotIn('unopened',json.dumps(row).lower())
                    self.assertEqual(row['role'],'control' if p['repetitions'] == 0 else 'candidate')
                before = {p:p.read_bytes() for p in output.rglob('*') if p.is_file()}
                with self.assertRaises(FileExistsError):m.prepare(output,selection=selection)
                self.assertEqual(before,{p:p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_real_operands_output_weights_and_exact_loop_gas(self):
        m = self.load(); modulus = m.FIELD_MODULUS; order = m.GROUP_ORDER
        def add(left,right):
            if left is None:return right
            if right is None:return left
            x,y = left; u,v = right
            if x == u and (y+v)%modulus == 0:return None
            slope = ((3*x*x*pow(2*y,-1,modulus)) if left == right else ((v-y)*pow(u-x,-1,modulus)))%modulus
            rx = (slope*slope-x-u)%modulus
            return rx,(slope*(x-rx)-y)%modulus
        def multiply(point,n):
            answer = None
            while n:
                if n&1:answer = add(answer,point)
                point = add(point,point); n >>= 1
            return (0,0) if answer is None else answer
        for point_multiple in (1,2,3):
            expected_point = multiply((1,2),point_multiple)
            for scalar_case,scalar in (('zero',0),('two',2),('dense',(3<<252)-1)):
                row = m.make_case(point_multiple,scalar_case,8); p = row['parameters']; program = row['request']['program']
                raw = bytes.fromhex(program['input_hex'][2:]); words = tuple(int.from_bytes(raw[i:i+32],'big') for i in range(0,len(raw),32))
                self.assertEqual(words,(*expected_point,scalar)); self.assertEqual(len(raw),96)
                expected_output = multiply(expected_point,scalar)
                self.assertEqual(p['expected_output_hex'],'0x'+b''.join(v.to_bytes(32,'big') for v in expected_output).hex())
                self.assertEqual(p['input_point'],list(expected_point))
                self.assertEqual(p['raw_scalar_hex'],f'0x{scalar:064x}')
                reduced = scalar%order
                self.assertEqual(p['reduced_scalar_hex'],f'0x{reduced:064x}')
                self.assertEqual(p['scalar_work_units'],reduced.bit_length()+reduced.bit_count())
                self.assertEqual(p['output_generator_scalar_hex'],f'0x{(scalar*point_multiple)%order:064x}')
                self.assertEqual(p['precompile_address'],'0x'+'00'*19+'07')
                self.assertEqual(p['requested_gas_per_call'],6000)
                self.assertEqual(p['gas_accounting_per_user']['precompile_native_gas'],48000)
                self.assertEqual(p['gas_accounting_per_user']['call_access_gas'],800)
                self.assertGreaterEqual(p['transaction_envelope_reserve'],(6000+62)//63)
                self.assertEqual(program['expected_route_counts']['opcode:0xfa'],8)
                self.assertEqual(program['expected_route_counts']['opcode:0x3d'],8)
                self.assertEqual(program['expected_route_counts']['opcode:0x00'],1)
                code = bytes.fromhex(program['code_hex'][2:]); output = bytes.fromhex(p['expected_output_hex'][2:])
                for index in (0,32):
                    self.assertIn(b'\x7f'+bytes(byte^255 for byte in output[index:index+32])+b'\x62'+(p['output_memory_offset']+index).to_bytes(3,'big')+b'\x52',code)
        self.assertEqual(m.make_case(1,'zero',1)['parameters']['expected_output_hex'],'0x'+'00'*64)

    def test_invalid_types_bounds_and_selection_rejected_before_writes(self):
        m = self.load()
        for args in ((True,'two',1),(0,'two',1),(4,'two',1),('1','two',1),(1,None,1),(1,True,1),
                     (1,[] ,1),(1,'unknown',1),(1,'two',True),(1,'two',-1),(1,'two',4097),(1,'two',1.0),(1,'two','1'),(1,'two',4096)):
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'missing'
            for selection in (None,True,[],'unknown'):
                with self.assertRaises(ValueError):m.prepare(output,selection=selection)
                self.assertFalse(output.exists())
        for output in (None,True,'catalog'):
            with self.assertRaises(ValueError):m.prepare(output)

    def test_frozen_precompile_matrix_digest_without_git_history(self):
        current = importlib.import_module('precompile_boundaries')
        self.assertEqual(hashlib.sha256(current.VECTOR_FILE.read_bytes()).hexdigest(),
            '23db9a204bb0fa5a12f96dc797a4af0bd142e4a2f9b35979a6483a710e26dc2f')
        frozen = hashlib.sha256()
        for vector_id in current.vectors():
            for n in (0,1,8):
                frozen.update(current.encoded(current.make_case(vector_id,n)))
                frozen.update(current.encoded(current.gas_accounting(vector_id,n)))
        # Computed from the complete48 rows/gas reports at f15a379c before extraction.
        self.assertEqual(frozen.hexdigest(),'3c845bf8f6b3ecd516ed1a65e3ae16bf74b76cec557929e0e9c11f892c27ccb9')

    def test_default_precompile_requests_and_gas_match_f15_commit_exactly(self):
        current = importlib.import_module('precompile_boundaries')
        baseline = subprocess.run(['git','show','f15a379c:bin/zkgas/bench/cases/precompile_boundaries.py'],cwd=CASES,
            capture_output=True,timeout=15)
        if baseline.returncode:
            self.skipTest('f15a379c Git baseline unavailable; independent frozen-matrix digest is checked separately')
        source = baseline.stdout
        old = types.ModuleType('precompile_boundaries_f15'); old.__file__ = str(CASES/'precompile_boundaries.py')
        exec(compile(source,str(CASES/'precompile_boundaries.py'),'exec'),old.__dict__)
        bank = subprocess.run(['git','show','f15a379c:bin/zkgas/bench/cases/precompile_boundary_vectors.json'],cwd=CASES,
            capture_output=True,timeout=15,check=True).stdout
        self.assertEqual(current.VECTOR_FILE.read_bytes(),bank)
        for vector_id in old.vectors():
            for n in (0,1,8):
                self.assertEqual(current.encoded(current.make_case(vector_id,n)),old.encoded(old.make_case(vector_id,n)),(vector_id,n))
                self.assertEqual(current.gas_accounting(vector_id,n),old.gas_accounting(vector_id,n),(vector_id,n))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); self.assertEqual(old.prepare(root/'old'),36); self.assertEqual(current.prepare(root/'new'),36)
            for path in (root/'old'/'requests').iterdir():
                self.assertEqual(path.read_bytes(),(root/'new'/'requests'/path.name).read_bytes())

    def test_stdlib_cli_all_selection(self):
        self.load()
        with tempfile.TemporaryDirectory() as temporary:
            command = [sys.executable,'-S',str(CASES/'ecmul_tariff.py'),'--selection','all','--output-dir',str(Path(temporary)/'all')]
            result = subprocess.run(command,capture_output=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr.decode()); self.assertEqual(json.loads(result.stdout),{'prepared':22})

    @unittest.skipUnless(os.environ.get('ECMUL_TARIFF_NATIVE_BIN'),'set native binary for ECMUL qualification')
    def test_native_all22_exact_status_gas_and_opcode_counts(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); m.prepare(root/'cases',selection='all')
            for row in json.loads((root/'cases/catalog.json').read_bytes())['cases']:
                folder = root/row['sample_id']; folder.mkdir(); p = row['parameters']
                result = subprocess.run([os.environ['ECMUL_TARIFF_NATIVE_BIN'],'--input',str(root/'cases'/row['request_path']),
                    '--output',str(folder/'input.json'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=60)
                self.assertEqual(result.returncode,0,f"{row['sample_id']}: {result.stderr.decode()}")
                report = json.loads((folder/'native.json').read_bytes())
                self.assertTrue(report['receipts_by_block'][0][1]['success'])
                self.assertEqual(report['user_transaction_gas_used_total'],p['gas_accounting_per_user']['charged_gas'])
                expected = json.loads((root/'cases'/row['request_path']).read_bytes())['program']['expected_route_counts']
                self.assertEqual({k:v for k,v in report['observed_route_counts'].items() if v},{k:v for k,v in expected.items() if v},row['sample_id'])
                transaction = json.loads((folder/'input.json').read_bytes())['witnesses'][0]['block']['body']['transactions'][1]['transaction']['Eip1559']
                self.assertEqual(transaction['input'],p['input_hex'])


if __name__ == '__main__':unittest.main()
