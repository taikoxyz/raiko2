"""Contracts and real native qualification for the finite BN254 capacity probe."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'


class PrecompileWorkloadTests(unittest.TestCase):
    def load(self):
        source = CASES / 'precompile_workload.py'
        self.assertTrue(source.is_file(), 'precompile workload generator is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('precompile_workload_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_expected_result_is_negative_double_generator_and_full_scalar_product(self):
        cases = self.load()
        field = cases.FIELD_MODULUS

        def add(left, right):
            if left is None:
                return right
            if right is None:
                return left
            x, y = left
            u, v = right
            if x == u and (y + v) % field == 0:
                return None
            slope = ((3*x*x * pow(2*y, -1, field)) if left == right else
                     ((v-y) * pow(u-x, -1, field))) % field
            next_x = (slope*slope-x-u) % field
            return next_x, (slope*(x-next_x)-y) % field

        expected = cases.expected_output('r-minus-two')
        doubled = add((1, 2), (1, 2))
        self.assertEqual(expected, (doubled[0], (-doubled[1]) % field))
        point, result, scalar = (1, 2), None, cases.GROUP_ORDER-2
        while scalar:
            if scalar & 1:
                result = add(result, point)
            point = add(point, point)
            scalar >>= 1
        self.assertEqual(expected, result)
        x, y = expected
        self.assertEqual((y*y-x*x*x-3) % field, 0)
        self.assertNotEqual(expected, (0, 0))
        for name, bits in (('dense253', 253), ('dense254', 254)):
            dense = cases.expected_output(name)
            x, y = dense
            self.assertEqual((y*y-x*x*x-3) % field, 0)
            self.assertNotEqual(dense, expected)
            self.assertEqual(cases.SCALARS[name].bit_length(), bits)
            self.assertEqual(cases.SCALARS[name].bit_count(), 253)
            self.assertLess(cases.SCALARS[name], cases.GROUP_ORDER)

    def test_count_word_is_only_input_change_and_exact_terminal_gates_remain(self):
        cases = self.load()
        rows = [cases.make_case('600000', count) for count in (0, 1, 4, 16, 32)]
        self.assertEqual(len({row['request']['program']['code_hex'] for row in rows}), 1)
        self.assertEqual([row['role'] for row in rows], ['control'] + ['candidate']*4)
        self.assertEqual(len({row['pair_group'] for row in rows}), 1)
        for row, count in zip(rows, (0, 1, 4, 16, 32)):
            program = row['request']['program']
            raw = bytes.fromhex(program['input_hex'][2:])
            self.assertEqual(len(raw), 128)
            self.assertEqual(int.from_bytes(raw[:32], 'big'), count)
            self.assertEqual(int.from_bytes(raw[32:64], 'big'), cases.GROUP_ORDER-2)
            self.assertEqual(program['expected_route_counts']['opcode:0xfa'], count)
            self.assertEqual(program['expected_route_counts']['opcode:0xf3'], 1)
            for route in ('opcode:0x00', 'opcode:0xfd', 'opcode:0xf1', 'opcode:0xf2',
                          'opcode:0xf4', 'opcode:0x3e'):
                self.assertEqual(program['expected_route_counts'][route], 0)
            self.assertEqual(row['expected_precompile_calls'], count)
            self.assertTrue(row['capacity_probe_only'])
        self.assertEqual(len({bytes.fromhex(row['request']['program']['input_hex'][2:])[32:]
                              for row in rows}), 1)
        for count in (0, 1, 4, 16, 32):
            a, b = (cases.make_case('600000', count, scalar_case)
                    for scalar_case in ('r-minus-two', 'dense254'))
            self.assertEqual(a['request']['program']['code_hex'], b['request']['program']['code_hex'])
            self.assertEqual(a['request']['program']['expected_route_counts'],
                             b['request']['program']['expected_route_counts'])
            self.assertEqual(len(bytes.fromhex(b['request']['program']['input_hex'][2:])), 128)

    def test_invalid_counts_and_runtime_bounds_emit_no_request(self):
        cases = self.load()
        for count in (-1, 4097, True, 1.0, '1'):
            with self.assertRaises(ValueError):
                cases.make_case('00', count)
        for runtime in ('', '00'*24577):
            with self.assertRaises(ValueError):
                cases.make_case(runtime, 1)
        self.assertEqual(cases.MAX_COUNT, 4096)
        self.assertGreater(cases.MAX_COUNT*6000, 16_777_216)
        for count in (2500, 4096):
            self.assertEqual(cases.make_case('00', count)['parameters']['count'], count)
        with self.assertRaises(ValueError):
            cases.make_case('00', 1, 'unknown-scalar')

    def test_finite_catalog_retains_every_case_hash_and_never_overwrites(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'probe'
            with patch.object(cases, 'compile_source', return_value=(
                    '00', {'source_sha256': 'a'*64})):
                self.assertEqual(cases.prepare(output, 'unused'), 11)
                catalog = json.loads((output / 'catalog.json').read_text())
                self.assertEqual(len({row['sample_id'] for row in catalog['cases']}), 11)
                self.assertEqual([row['parameters']['count'] for row in catalog['cases']],
                                 [0, 0, 1, 1, 1, 4, 4, 16, 16, 32, 32])
                self.assertEqual({row['parameters']['scalar_case'] for row in catalog['cases']},
                                 {'r-minus-two', 'dense253', 'dense254'})
                self.assertEqual([(row['parameters']['count'], row['parameters']['scalar_case'])
                                  for row in catalog['cases']
                                  if row['parameters']['scalar_case'] == 'dense253'],
                                 [(1, 'dense253')])
                for row in catalog['cases']:
                    self.assertEqual(cases.digest((output / row['request_path']).read_bytes()),
                                     row['request_sha256'])
                self.assertTrue(catalog['capacity_probe_only'])
                with self.assertRaises(FileExistsError):
                    cases.prepare(output, 'unused')

    @unittest.skipUnless(os.environ.get('SOLC_BIN'), 'set SOLC_BIN for explicit compiler qualification')
    def test_real_runtime_is_bounded_and_has_no_explicit_returndata_copy(self):
        cases = self.load()
        runtime, compilation = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE,
                                                     'PrecompileWorkload')
        code = bytes.fromhex(runtime)
        self.assertLessEqual(len(code), 24576)
        metadata_length = int.from_bytes(code[-2:], 'big')
        executable = code[:-metadata_length-2]
        opcodes, pc = [], 0
        while pc < len(executable):
            opcode = executable[pc]
            opcodes.append(opcode)
            pc += 1 + (opcode-0x5f if 0x60 <= opcode <= 0x7f else 0)
        self.assertIn(0xfa, opcodes)
        self.assertIn(0x3d, opcodes)
        self.assertNotIn(0x3e, opcodes)
        self.assertEqual(compilation['standard_input']['settings']['evmVersion'], 'shanghai')

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set compiler/native binary for real curve and count qualification')
    def test_native_zero_and_one_validate_actual_curve_result_and_complete_counts(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE,
                                         'PrecompileWorkload')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            counts = {}
            selected = [('r-minus-two', 0), ('r-minus-two', 1), ('dense254', 0),
                        ('dense254', 1), ('dense253', 1)]
            for scalar_case, count in selected:
                row = cases.make_case(runtime, count, scalar_case)
                folder = root / f'{scalar_case}-{count}'
                folder.mkdir()
                request, report = folder / 'request.json', folder / 'native.json'
                request.write_bytes(cases.encoded(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(request),
                    '--execution-profile', 'evm-gas-only', '--output', str(folder/'guest-input.json'),
                    '--report', str(report)], capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                native = json.loads(report.read_text())
                self.assertEqual(native['execution_profile'], 'evm-gas-only')
                for route, expected in row['request']['program']['expected_route_counts'].items():
                    self.assertEqual(native['observed_route_counts'].get(route, 0), expected)
                self.assertEqual(native['receipts_by_block'][0][1]['success'], True)
                counts[scalar_case, count] = native['observed_route_counts']
            # Input words preallocate the entire layout. Only the two per-iteration output
            # resets add stores; this also checks the control's setup is not skipped.
            for scalar_case in ('r-minus-two', 'dense254'):
                self.assertGreaterEqual(counts[scalar_case, 0]['opcode:0x52'], 4)
                self.assertEqual(counts[scalar_case, 1]['opcode:0x52']-
                                 counts[scalar_case, 0]['opcode:0x52'], 2)
            for count in (0, 1):
                self.assertEqual(counts['r-minus-two', count], counts['dense254', count])
            self.assertEqual(counts['r-minus-two', 1], counts['dense253', 1])

            # Direct ProgramRequests can bypass the Python constructor. The runtime itself
            # must reject an excessive count before entering the precompile loop.
            excessive = cases.make_case(runtime, 0)['request']
            excessive['expected_user_success'] = False
            original_input = bytes.fromhex(excessive['program']['input_hex'][2:])
            excessive['program']['input_hex'] = '0x' + ((4097).to_bytes(32, 'big')+
                                                       original_input[32:]).hex()
            excessive['program']['expected_route_counts'].update({
                'opcode:0xf3': 0, 'opcode:0xfd': 1})
            folder = root / 'excessive'
            folder.mkdir()
            request, report = folder/'request.json', folder/'native.json'
            request.write_bytes(cases.encoded(excessive))
            result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(request),
                '--execution-profile', 'evm-gas-only', '--output', str(folder/'guest-input.json'),
                '--report', str(report)], capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            native = json.loads(report.read_text())
            self.assertEqual(native['receipts_by_block'][0][1]['success'], False)
            self.assertEqual(native['observed_route_counts'].get('opcode:0xfa', 0), 0)
            self.assertEqual(native['observed_route_counts']['opcode:0xfd'], 1)


if __name__ == '__main__':
    unittest.main()
