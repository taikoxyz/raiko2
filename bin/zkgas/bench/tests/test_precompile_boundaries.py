"""Finite compiler-free precompile vectors and native completion guards."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))


class PrecompileBoundaryTests(unittest.TestCase):
    def load(self):
        return importlib.import_module('precompile_boundaries')

    def test_finite_catalog_create_only_and_hashes(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'catalog'
            self.assertEqual(m.prepare(output), 36)
            catalog = json.loads((output / 'catalog.json').read_bytes())
            rows = catalog['cases']
            self.assertEqual(len({r['sample_id'] for r in rows}), 36)
            self.assertEqual({r['parameters']['repetitions'] for r in rows}, {0, 1, 8})
            self.assertEqual({r['parameters']['precompile_address'] for r in rows},
                {f'0x{x:040x}' for x in (1, 3, 6, 9, 10, 11, 13, 14, 15, 16, 17, 256)})
            by_id = {r['sample_id']: r for r in rows}
            for row in rows:
                raw = (output / row['request_path']).read_bytes()
                self.assertEqual(row['request_sha256'], hashlib.sha256(raw).hexdigest())
                self.assertEqual(by_id[row['baseline_sample_id']]['parameters']['repetitions'], 0)
                self.assertTrue(json.loads(raw)['expected_user_success'])
            before = {p: p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):
                m.prepare(output)
            self.assertEqual(before, {p: p.read_bytes() for p in output.rglob('*') if p.is_file()})
            for bad in (None, True, 'catalog'):
                with self.assertRaises(ValueError):
                    m.prepare(bad)

    def test_typed_bounds_and_gas_capacity(self):
        m = self.load()
        for bad in (None, True, [], 'unknown', 1):
            with self.subTest(vector=bad), self.assertRaises(ValueError):
                m.make_case(bad, 1)
        for bad in (None, True, -1, 1.0, '1', 4097):
            with self.subTest(count=bad), self.assertRaises(ValueError):
                m.make_case(m.BASE_VECTOR_IDS[0], bad)
        with self.assertRaises(ValueError):
            m.make_case('kzg-valid', 4096)

    def test_request_loop_gas_and_vector_structure(self):
        m = self.load()
        for vector_id in m.BASE_VECTOR_IDS:
            for n in (0, 1, 8):
                row = m.make_case(vector_id, n)
                p = row['parameters']; program = row['request']['program']; gas = m.gas_accounting(vector_id, n)
                vector = m.vectors()[vector_id]
                self.assertEqual(program['input_hex'], vector['input_hex'])
                self.assertEqual(p['expected_output_hex'], vector['expected_output_hex'])
                self.assertEqual(p['expected_precompile_calls'], n)
                counts = program['expected_route_counts']
                self.assertEqual(counts['opcode:0xfa'], n)
                self.assertEqual(counts['opcode:0x00'], 1)
                self.assertEqual(counts['opcode:0xfe'], 0)
                self.assertEqual(counts['opcode:0x3d'], n)
                self.assertEqual(counts['opcode:0x51'], (p['output_bytes']//32)*n+2)
                self.assertEqual(gas['charged_gas'], max(gas['gross_gas'], gas['eip7623_floor_gas']))
                self.assertEqual(gas['precompile_native_gas'], n*vector['native_gas'])
                self.assertEqual(gas['call_access_gas'], 100*n)
                self.assertGreaterEqual(p['transaction_envelope_reserve'], (vector['native_gas']+62)//63)
                self.assertEqual(program['transactions']['gas_limit'], gas['charged_gas']+p['transaction_envelope_reserve'])
                code = bytes.fromhex(program['code_hex'][2:])
                self.assertIn(b'\x73'+int(vector['address'], 16).to_bytes(20, 'big'), code)
                self.assertLessEqual(len(code), 24576)
                self.assertTrue(vector['provenance']['sources'])
                self.assertTrue(any(bytes.fromhex(vector['input_hex'][2:])))
        self.assertEqual(m.vectors()['bls-g2-msm-two']['input_hex'][-64:], f'{2:064x}')
        self.assertEqual(m.vectors()['bls-pairing-k1']['expected_output_hex'], '0x'+'00'*32)

    def test_independent_curve_outputs_and_standard_hash(self):
        m = self.load(); bank = m.vectors()
        p = int('1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab', 16)
        def words(raw, width):
            return tuple(int.from_bytes(raw[i:i+width], 'big') for i in range(0, len(raw), width))
        def add(a, b): return tuple((x+y)%p for x, y in zip(a, b))
        def neg(a): return tuple(-x%p for x in a)
        def mul(a, b): return ((a[0]*b[0]-a[1]*b[1])%p, (a[0]*b[1]+a[1]*b[0])%p)
        def inv(a):
            inverse = pow((a[0]*a[0]+a[1]*a[1])%p, -1, p)
            return (a[0]*inverse%p, -a[1]*inverse%p)
        def point_add(a, b):
            if a is None: return b
            if b is None: return a
            x,y = a; u,v = b
            if x == u and add(y,v) == (0,0): return None
            numerator = mul((3,0),mul(x,x)) if a == b else add(v,neg(y))
            denominator = mul((2,0),y) if a == b else add(u,neg(x))
            slope = mul(numerator,inv(denominator)); rx = add(add(mul(slope,slope),neg(x)),neg(u))
            return rx,add(mul(slope,add(x,neg(rx))),neg(y))
        for key in ('bls-g2-add-double', 'bls-g2-msm-two', 'bls-g2-msm-dense'):
            raw = bytes.fromhex(bank[key]['input_hex'][2:]); v = words(raw[:256],64)
            point = (v[:2],v[2:]); n = 2 if key == 'bls-g2-add-double' else int.from_bytes(raw[256:],'big')
            self.assertEqual(mul(point[1],point[1]), add(mul(mul(point[0],point[0]),point[0]),(4,4)))
            result = None; addend = point
            while n:
                if n & 1: result = point_add(result,addend)
                addend = point_add(addend,addend); n >>= 1
            expected = words(bytes.fromhex(bank[key]['expected_output_hex'][2:]),64)
            self.assertEqual((*result[0],*result[1]),expected)
            self.assertTrue(any(expected))
        for key, modulus, width in (('bn254-add-double', 21888242871839275222246405745257275088696311157297823662689037894645226208583,32),
                                    ('bls-g1-add-double',p,64)):
            raw = bytes.fromhex(bank[key]['input_hex'][2:]); x,y = words(raw[:2*width],width)
            self.assertEqual(y*y%modulus,(x*x*x+(3 if width == 32 else 4))%modulus)
            slope = 3*x*x*pow(2*y,-1,modulus)%modulus; u = (slope*slope-2*x)%modulus
            self.assertEqual(words(bytes.fromhex(bank[key]['expected_output_hex'][2:]),width),(u,(slope*(x-u)-y)%modulus))
        self.assertEqual(bank['blake2f-12']['expected_output_hex'],'0x'+hashlib.blake2b(b'abc').hexdigest())
        raw = bytes.fromhex(bank['kzg-valid']['input_hex'][2:])
        self.assertEqual(raw[:32],b'\x01'+hashlib.sha256(raw[96:144]).digest()[1:])
        for key in ('bls-map-fp','bls-map-fp2'):
            values = words(bytes.fromhex(bank[key]['expected_output_hex'][2:]),64)
            self.assertTrue(all(0 <= value < p for value in values))
            self.assertTrue(any(values))

    def test_each_output_word_is_poisoned_with_its_complement(self):
        m = self.load()
        for vector_id in m.vectors():
            row = m.make_case(vector_id,1); p = row['parameters']
            code = bytes.fromhex(row['request']['program']['code_hex'][2:])
            expected = bytes.fromhex(p['expected_output_hex'][2:])
            for index in range(0,len(expected),32):
                poison = bytes(byte^255 for byte in expected[index:index+32])
                self.assertIn(b'\x7f'+poison+b'\x62'+(p['output_memory_offset']+index).to_bytes(3,'big')+b'\x52',code)
        hashes = [source['sha256'] for vector in m.vectors().values() for source in vector['provenance']['sources']
                  if source.get('scenario_id')]
        self.assertTrue(hashes)
        self.assertEqual(set(hashes), {'90c0b5d593e579a8179ee831fc302ec5689918eed84097b4b187f9dacbecfb44'})

    def test_loader_and_selection_reject_invalid_typed_data_before_writes(self):
        m = self.load(); original = m.VECTOR_FILE
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for kwargs in ({'vector_ids': ['unknown']}, {'vector_ids': [True]}, {'vector_ids': ['p256-valid']*2},
                           {'repetitions': [1,8]}, {'repetitions': [0, True]}, {'repetitions': [0,1,1]},
                           {'repetitions': [0,4096], 'vector_ids': ['kzg-valid']}):
                with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                    m.prepare(root/'missing', **kwargs)
                self.assertFalse((root/'missing').exists())
            data = json.loads(original.read_bytes())
            for field, value in (('address','0x100'), ('address',True), ('input_hex','0x0'),
                                 ('input_hex','0xZZ'), ('native_gas',True), ('native_gas',0),
                                 ('expected_output_hex','0x00')):
                changed = json.loads(json.dumps(data)); changed['vectors'][0][field] = value
                path = root/'vectors.json'; path.write_text(json.dumps(changed)); m.VECTOR_FILE = path
                try:
                    with self.subTest(field=field,value=value), self.assertRaises(ValueError): m.vectors()
                finally: m.VECTOR_FILE = original

    @unittest.skipUnless(os.environ.get('PRECOMPILE_BOUNDARY_NATIVE_BIN'), 'set native binary for behavioral qualification')
    def test_native_all_base_counts_exact_gas_and_counts(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for vector_id in m.BASE_VECTOR_IDS:
                for n in (0, 1, 8):
                    row = m.make_case(vector_id, n); folder = root / row['sample_id']; folder.mkdir()
                    (folder / 'request.json').write_bytes(m.encoded(row['request']))
                    result = subprocess.run([os.environ['PRECOMPILE_BOUNDARY_NATIVE_BIN'], '--input', str(folder/'request.json'),
                        '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'), '--report', str(folder/'native.json'),
                        '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=60)
                    self.assertEqual(result.returncode, 0, f'{vector_id}/{n}: {result.stderr.decode()}')
                    report = json.loads((folder/'native.json').read_bytes())
                    self.assertEqual(report['user_transaction_gas_used_total'], row['parameters']['gas_accounting_per_user']['charged_gas'])
                    for opcode, expected in row['request']['program']['expected_route_counts'].items():
                        self.assertEqual(report['observed_route_counts'].get(opcode, 0), expected, f'{vector_id}/{n}/{opcode}')


    @unittest.skipUnless(os.environ.get('PRECOMPILE_BOUNDARY_NATIVE_BIN'), 'set native binary for guard mutants')
    def test_native_rejects_guard_mutants(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for mutant in ('missing-zero-copy','wrong-output','short-input','invalid-point','truncated-address','wrong-return-size','skipped-body'):
                key = 'bls-pairing-k1' if mutant == 'missing-zero-copy' else 'bls-g2-add-double' if mutant == 'invalid-point' else 'p256-valid'
                row = m.make_case(key,1); program = row['request']['program']; p = row['parameters']
                code = bytes.fromhex(program['code_hex'][2:])
                if mutant == 'missing-zero-copy':
                    operands = (b'\x62'+p['output_bytes'].to_bytes(3,'big')+b'\x62'+p['output_memory_offset'].to_bytes(3,'big')+
                        b'\x62'+p['input_bytes'].to_bytes(3,'big')+b'\x62\x00\x00\x00\x73'+int(p['precompile_address'],16).to_bytes(20,'big'))
                    self.assertEqual(code.count(operands),1)
                    code = code.replace(operands,b'\x62\x00\x00\x00'+operands[4:])
                elif mutant == 'wrong-output':
                    guard = b'\x7f'+bytes.fromhex(p['expected_output_hex'][2:])+b'\x14'
                    self.assertEqual(code.count(guard),1)
                    code = code.replace(guard,guard[:-2]+bytes([guard[-2]^1])+guard[-1:])
                elif mutant == 'wrong-return-size':
                    operand = b'\x3d\x62'+p['output_bytes'].to_bytes(3,'big'); self.assertEqual(code.count(operand),1)
                    code = code.replace(operand, b'\x3d\x62'+(p['output_bytes']+1).to_bytes(3,'big'))
                elif mutant == 'skipped-body':
                    operand = b'\x5b\x80\x62\x00\x00\x01\x14'; self.assertEqual(code.count(operand),1)
                    code = code.replace(operand,b'\x5b\x80\x62\x00\x00\x00\x14')
                elif mutant == 'short-input': program['input_hex'] = program['input_hex'][:-2]
                elif mutant == 'invalid-point': program['input_hex'] = '0x'+'ff'*64+program['input_hex'][130:]
                else:
                    operand = b'\x73'+(256).to_bytes(20,'big'); self.assertEqual(code.count(operand),1)
                    code = code.replace(operand,b'\x73'+bytes(20))
                program['code_hex'] = '0x'+code.hex()
                folder = root/mutant; folder.mkdir(); (folder/'request.json').write_bytes(m.encoded(row['request']))
                process = subprocess.run([os.environ['PRECOMPILE_BOUNDARY_NATIVE_BIN'], '--input',str(folder/'request.json'),
                    '--output',str(folder/'input.json'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],
                    capture_output=True,timeout=60)
                self.assertNotEqual(process.returncode,0,mutant)
                self.assertFalse((folder/'native.json').exists(),mutant)


if __name__ == '__main__':
    unittest.main()
