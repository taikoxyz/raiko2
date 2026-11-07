"""Finite MODEXP value-axis inputs and real native completion qualification."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'


class ModexpValuesTests(unittest.TestCase):
    def load(self):
        source = CASES / 'modexp_values.py'
        self.assertTrue(source.is_file(), 'MODEXP value producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('modexp_value_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_pow_outputs_match_independent_closed_forms_at_every_width(self):
        cases = self.load()
        for width in (128, 256, 512):
            for shape in ('odd', 'power-two', 'even-crt'):
                modulus = cases.modulus_for(width, shape)
                for exponent in (0, 1, 3):
                    expected = (1 if exponent == 0 else
                                { 'odd': 188 if exponent == 1 else 6644672,
                                  'power-two': modulus-1,
                                  'even-crt': (1 << (8*width-2))-1}[shape])
                    self.assertEqual(int.from_bytes(cases.expected_output(width, shape, exponent), 'big'),
                                     expected)

    def test_payload_headers_lengths_and_every_terminal_gate(self):
        cases = self.load()
        for width in (128, 256, 512):
            row = cases.make_case('00', width, 'even-crt', 3, 4)
            request = row['request']
            raw = bytes.fromhex(request['program']['input_hex'][2:])
            self.assertEqual(len(raw), 64+96+3*width+1)
            self.assertEqual([int.from_bytes(raw[i:i+32], 'big') for i in range(0, 160, 32)],
                             [4, width, width, 1, width])
            self.assertEqual(raw[160:160+width], b'\xff'*width)
            self.assertEqual(raw[160+width], 3)
            self.assertEqual(int.from_bytes(raw[161+width:161+2*width], 'big'),
                             3 << (8*width-2))
            self.assertEqual(raw[-width:], cases.expected_output(width, 'even-crt', 3))
            counts = request['program']['expected_route_counts']
            self.assertEqual(counts['opcode:0xfa'], 4)
            self.assertEqual(counts['opcode:0xf3'], 1)
            for opcode in ('0xfd', '0x00', '0xf1', '0xf2', '0xf4', '0xf0', '0xf5', '0x3e'):
                self.assertEqual(counts[f'opcode:{opcode}'], 0)
            self.assertEqual(row['parameters']['source_derived_native_gas_per_call'],
                             {128: 512, 256: 2048, 512: 8192}[width])
            self.assertTrue(row['value_boundary_probe_only'])

    def test_generator_rejects_invalid_inputs_and_keeps_count_bound_above_legal_calls(self):
        cases = self.load()
        self.assertEqual(cases.MAX_COUNT, 65536)
        self.assertGreater(cases.MAX_COUNT*500, 16_777_216)
        self.assertEqual(cases.make_case('00', 128, 'odd', 0, 65536)['parameters']['count'], 65536)
        for width in (0, 32, 129, True, '128'):
            with self.assertRaises(ValueError):
                cases.make_case('00', width, 'odd', 1, 1)
        for count in (-1, 65537, True, 1.0, '1'):
            with self.assertRaises(ValueError):
                cases.make_case('00', 128, 'odd', 1, count)
        for shape in ('unknown', None, True):
            with self.assertRaises(ValueError):
                cases.make_case('00', 128, shape, 1, 1)
        for exponent in (-1, 2, 4, True, '3'):
            with self.assertRaises(ValueError):
                cases.make_case('00', 128, 'odd', exponent, 1)
        for runtime in ('', '00'*24577, 'zz'):
            with self.assertRaises(ValueError):
                cases.make_case(runtime, 128, 'odd', 1, 1)

    @unittest.skipUnless(os.environ.get('SOLC_BIN'), 'set SOLC_BIN for real compiler qualification')
    def test_frozen_32_plan_catalog_real_compiler_hashes_and_no_overwrite(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(cases.prepare(output, Path(os.environ['SOLC_BIN'])), 32)
            catalog = json.loads((output/'catalog.json').read_text())
            rows = catalog['cases']
            self.assertEqual(len({row['sample_id'] for row in rows}), 32)
            self.assertEqual({(row['parameters']['width'], row['parameters']['modulus_case'],
                               row['parameters']['exponent']) for row in rows
                              if row['parameters']['count'] == 1},
                             {(w, s, e) for w in (128, 256, 512)
                              for s in ('odd', 'power-two', 'even-crt') for e in (0, 1, 3)})
            self.assertEqual({(row['parameters']['width'], row['parameters']['modulus_case'],
                               row['parameters']['exponent'], row['parameters']['count']) for row in rows
                              if row['parameters']['count'] != 1},
                             {(w, 'even-crt', 3, 0) for w in (128, 256, 512)} |
                             {(512, 'even-crt', 3, n) for n in (4, 16)})
            self.assertEqual(catalog['source_sha256'], cases.digest(cases.SOURCE.read_bytes()))
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'modexp_values.py').read_bytes()))
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
        folder = root/name
        folder.mkdir()
        path = folder/'request.json'
        path.write_text(json.dumps(request))
        report = folder/'native.json'
        result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(path),
            '--execution-profile', 'evm-gas-only', '--output', str(folder/'guest-input.json'),
            '--report', str(report)], capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        native = json.loads(report.read_text())
        self.assertEqual(native['execution_profile'], 'evm-gas-only')
        self.assertEqual(native['receipts_by_block'][0][1]['success'], request['expected_user_success'])
        for route, count in request['program']['expected_route_counts'].items():
            self.assertEqual(native['observed_route_counts'].get(route, 0), count, (name, route))
        return native

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set compiler/native binary for real MODEXP qualification')
    def test_native_all_nine_128_value_shapes_and_512_crt_four_calls(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE, 'ModexpValues')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for shape in ('odd', 'power-two', 'even-crt'):
                for exponent in (0, 1, 3):
                    row = cases.make_case(runtime, 128, shape, exponent, 1)
                    self.run_native(root, f'{shape}-{exponent}', row['request'])
            observed = {}
            for count in (0, 1, 4):
                row = cases.make_case(runtime, 512, 'even-crt', 3, count)
                native = self.run_native(root, f'crt512-{count}', row['request'])
                observed[count] = native['observed_route_counts']
            # Complete output clearing on every iteration adds L/32 MSTOREs. The input
            # and full output allocation are shared with the zero-call control.
            self.assertEqual(observed[1]['opcode:0x52']-observed[0]['opcode:0x52'], 16)
            self.assertEqual(observed[4]['opcode:0x52']-observed[0]['opcode:0x52'], 64)

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set compiler/native binary for malformed-input qualification')
    def test_native_rejects_bad_first_last_output_and_headers_lengths_count_before_calls(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE, 'ModexpValues')
        valid = cases.make_case(runtime, 512, 'even-crt', 3, 1)['request']
        original = bytes.fromhex(valid['program']['input_hex'][2:])
        changes = {
            'first-output': (len(original)-512, 1), 'last-output': (len(original)-1, 1),
            'base-header': (64+31, 1), 'exp-header': (96+31, 1), 'mod-header': (128+31, 1),
            'unsupported-width': (32+31, 1),
        }
        invalid = {}
        for name, (offset, mask) in changes.items():
            changed = bytearray(original)
            changed[offset] ^= mask
            invalid[name] = bytes(changed)
        invalid.update({'short': original[:-1], 'long': original+b'\0',
                        'excessive': (65537).to_bytes(32, 'big')+original[32:]})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name, payload in invalid.items():
                request = json.loads(json.dumps(valid))
                request['expected_user_success'] = False
                request['program']['input_hex'] = '0x'+payload.hex()
                counts = request['program']['expected_route_counts']
                counts.update({'opcode:0xfa': 1 if name.endswith('output') else 0,
                               'opcode:0xf3': 0, 'opcode:0xfd': 1})
                self.run_native(root, name, request)


if __name__ == '__main__':
    unittest.main()
