"""BLS G1 MSM scalar/batch boundaries and native result qualification."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class BlsMsmValuesTests(unittest.TestCase):
    def load(self):
        source = CASES/'bls_msm_values.py'
        self.assertTrue(source.is_file(), 'BLS MSM value producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('bls_msm_value_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_curve_subgroup_and_independent_pointwise_expected_results(self):
        cases = self.load()
        p, r, g = cases.FIELD_MODULUS, cases.GROUP_ORDER, cases.GENERATOR
        self.assertEqual((g[1]*g[1]-g[0]**3-4) % p, 0)
        self.assertIsNone(cases.point_mul(r, g))
        self.assertEqual(cases.point_mul(r+1, g), g)
        self.assertEqual(cases.point_add(g, (g[0], -g[1] % p)), None)
        points = [cases.point_mul(i, g) for i in range(1, 17)]
        self.assertEqual(len(set(points)), 16)
        running = None
        for point in points:
            self.assertEqual((point[1]*point[1]-point[0]**3-4) % p, 0)
            self.assertIsNone(cases.point_mul(r, point))
            running = cases.point_add(running, g)
            self.assertEqual(running, point)
        for k in (1, 4, 16):
            for pattern in ('one', 'binary-dense', 'alternating', 'random'):
                scalar = cases.SCALARS[pattern]
                self.assertTrue(0 < scalar < r)
                result = None
                for point in points[:k]:
                    result = cases.point_add(result, cases.point_mul(scalar, point))
                expected = b''.join(value.to_bytes(64, 'big') for value in result)
                self.assertEqual(cases.expected_output(k, pattern), expected)
        result = None
        for i, point in enumerate(points, 1):
            scalar = (int.from_bytes(hashlib.sha256(f'bls-g1-msm-scalar-{i}'.encode()).digest(), 'big') &
                      ((1 << 255)-1)) % r
            self.assertEqual(cases.scalars_for(16, 'mixed-random')[i-1], scalar)
            result = cases.point_add(result, cases.point_mul(scalar, point))
        self.assertEqual(cases.expected_output(16, 'mixed-random'),
                         b''.join(value.to_bytes(64, 'big') for value in result))
        self.assertEqual(cases.SCALARS['random'],
                         int('70efa1463d7490ae8e43d134c3f832d1373383903e51edc8af34e73642855062', 16))
        self.assertEqual(cases.SCALARS['binary-dense'], (7 << 252)-1)
        self.assertEqual(cases.SCALARS['alternating'], int.from_bytes(b'\x55'*32, 'big'))

    def test_exact_input_encoding_and_terminal_gates(self):
        cases = self.load()
        for k in (1, 4, 16):
            row = cases.make_case('00', k, 'random', 4)
            raw = bytes.fromhex(row['request']['program']['input_hex'][2:])
            self.assertEqual(len(raw), 192+160*k)
            self.assertEqual(int.from_bytes(raw[:32], 'big'), 4)
            self.assertEqual(int.from_bytes(raw[32:64], 'big'), k)
            for i in range(k):
                pair = raw[64+160*i:64+160*(i+1)]
                x, y = cases.point_mul(i+1, cases.GENERATOR)
                self.assertEqual(pair[:64], x.to_bytes(64, 'big'))
                self.assertEqual(pair[64:128], y.to_bytes(64, 'big'))
                self.assertEqual(pair[128:], cases.SCALARS['random'].to_bytes(32, 'big'))
                self.assertEqual(pair[:16], b'\0'*16)
                self.assertEqual(pair[64:80], b'\0'*16)
            self.assertEqual(raw[-128:], cases.expected_output(k, 'random'))
            counts = row['request']['program']['expected_route_counts']
            self.assertEqual(counts['opcode:0xfa'], 4)
            self.assertEqual(counts['opcode:0xf3'], 1)
            for op in ('0xfd', '0x00', '0xf1', '0xf2', '0xf4', '0xf0', '0xf5', '0x3e'):
                self.assertEqual(counts[f'opcode:{op}'], 0)
            self.assertEqual(row['parameters']['source_derived_native_gas_per_call'],
                             {1: 12000, 4: 38256, 16: 129984}[k])
            self.assertTrue(row['value_boundary_probe_only'])
        self.assertEqual(cases.expected_output(1, 'one'),
                         b''.join(v.to_bytes(64, 'big') for v in cases.GENERATOR))
        self.assertEqual(cases.encode_point(None), b'\0'*128)

    def test_typed_bounds_and_legal_transaction_count_coverage(self):
        cases = self.load()
        self.assertEqual(cases.MAX_COUNT, 2048)
        self.assertGreater(cases.MAX_COUNT*12000, 16_777_216)
        self.assertEqual(cases.make_case('00', 1, 'one', 2048)['parameters']['count'], 2048)
        for count in (-1, 2049, True, '1', 1.0):
            with self.assertRaises(ValueError):
                cases.make_case('00', 1, 'one', count)
        for k in (0, 2, 17, True, '1', 1.0):
            with self.assertRaises(ValueError):
                cases.make_case('00', k, 'one', 1)
        for pattern in ('unknown', None, True):
            with self.assertRaises(ValueError):
                cases.make_case('00', 1, pattern, 1)
        for runtime in ('', '00'*24577, 'zz'):
            with self.assertRaises(ValueError):
                cases.make_case(runtime, 1, 'one', 1)

    @unittest.skipUnless(os.environ.get('SOLC_BIN'), 'set SOLC_BIN for real compilation')
    def test_18_plan_catalog_compiler_hashes_and_no_overwrite(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(cases.prepare(output, Path(os.environ['SOLC_BIN'])), 18)
            catalog = json.loads((output/'catalog.json').read_text())
            rows = catalog['cases']
            self.assertEqual(len({row['sample_id'] for row in rows}), 18)
            keys = {(r['parameters']['k'], r['parameters']['scalar_pattern'], r['parameters']['count'])
                    for r in rows}
            expected = {(k, p, 1) for k in (1, 4, 16)
                        for p in ('one', 'binary-dense', 'alternating', 'random')}
            expected |= {(k, 'random', 0) for k in (1, 4, 16)}
            expected |= {(1, 'binary-dense', 4), (1, 'random', 4), (16, 'mixed-random', 1)}
            self.assertEqual(keys, expected)
            self.assertEqual(catalog['source_sha256'], cases.digest(cases.SOURCE.read_bytes()))
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'bls_msm_values.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual(catalog['compiler_artifact_sha256'], cases.digest((output/'compiler.json').read_bytes()))
            runtime = (output/'runtime.hex').read_text().strip()
            self.assertEqual(catalog['runtime_sha256'], cases.digest(bytes.fromhex(runtime)))
            self.assertLessEqual(len(bytes.fromhex(runtime)), 24576)
            for row in rows:
                data = (output/row['request_path']).read_bytes()
                self.assertEqual(row['request_sha256'], cases.digest(data))
                self.assertEqual(json.loads(data)['program']['code_hex'], '0x'+runtime)
            before = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):
                cases.prepare(output, Path(os.environ['SOLC_BIN']))
            self.assertEqual(before, {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def run_native(self, root, name, request):
        directory = root/name
        directory.mkdir()
        path = directory/'request.json'
        path.write_text(json.dumps(request))
        report = directory/'native.json'
        result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(path),
            '--execution-profile', 'evm-gas-only', '--output', str(directory/'guest-input.json'),
            '--report', str(report)], capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        native = json.loads(report.read_text())
        self.assertEqual(native['execution_profile'], 'evm-gas-only')
        self.assertEqual(native['receipts_by_block'][0][1]['success'], request['expected_user_success'])
        for route, count in request['program']['expected_route_counts'].items():
            self.assertEqual(native['observed_route_counts'].get(route, 0), count, (name, route))
        return native

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set compiler/native binary for real BLS output qualification')
    def test_native_scalar_and_batch_axes_zero_and_repeated_outputs(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE, 'BlsMsmValues')
        selections = [(k, p, 1) for k in (1, 4, 16) for p in ('one', 'random')]
        selections += [(1, 'binary-dense', 1), (1, 'alternating', 1), (16, 'mixed-random', 1)]
        selections += [(k, 'random', 0) for k in (1, 4, 16)]
        selections += [(1, 'random', 4), (1, 'binary-dense', 4)]
        with tempfile.TemporaryDirectory() as temporary:
            root, reports = Path(temporary), {}
            for k, pattern, count in selections:
                row = cases.make_case(runtime, k, pattern, count)
                reports[k, pattern, count] = self.run_native(root, f'{k}-{pattern}-{count}', row['request'])
            for count in (1, 4):
                stores = reports[1, 'random', count]['observed_route_counts']['opcode:0x52']
                control = reports[1, 'random', 0]['observed_route_counts']['opcode:0x52']
                self.assertEqual(stores-control, 4*count)
            for k in (1, 4, 16):
                self.assertEqual(reports[k, 'one', 1]['observed_route_counts'],
                                 reports[k, 'random', 1]['observed_route_counts'])

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set compiler/native binary for malformed BLS input qualification')
    def test_native_rejects_invalid_outputs_count_k_size_and_input_point(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE, 'BlsMsmValues')
        valid = cases.make_case(runtime, 1, 'random', 1)['request']
        original = bytes.fromhex(valid['program']['input_hex'][2:])
        invalid = {}
        for name, offset in (('first-output', len(original)-128), ('last-output', len(original)-1),
                             ('input-point', 64+127)):
            raw = bytearray(original)
            raw[offset] ^= 1
            invalid[name] = bytes(raw)
        invalid.update({'count': (2049).to_bytes(32, 'big')+original[32:],
                        'k': original[:32]+(2).to_bytes(32, 'big')+original[64:],
                        'short': original[:-1], 'long': original+b'\0', 'empty': b''})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name, raw in invalid.items():
                request = json.loads(json.dumps(valid))
                request['expected_user_success'] = False
                request['program']['input_hex'] = '0x'+raw.hex()
                request['program']['expected_route_counts'].update({
                    'opcode:0xfa': 1 if name in ('first-output', 'last-output', 'input-point') else 0,
                    'opcode:0xf3': 0, 'opcode:0xfd': 1})
                self.run_native(root, name, request)


if __name__ == '__main__':
    unittest.main()
