"""Behavioral contracts for preallocated automatic CALL returndata copy cases."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'


class ReturnCopyTests(unittest.TestCase):
    def load(self):
        source = CASES / 'return_copy.py'
        self.assertTrue(source.is_file(), 'automatic-return-copy generator is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('return_copy_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_exact_terminal_gates_distinguish_return_revert_and_identity(self):
        cases = self.load()
        for kind, opcode in enumerate((0xf4, 0xf1, 0xf2, 0xfa)):
            for target, reverts, returns, child_reverts in (
                    ('self', False, 17, 0), ('self', True, 1, 16), ('identity', False, 1, 0)):
                row = cases.make_case('00', kind, 33, 16, 'full', target, reverts)
                gates = row['request']['program']['expected_route_counts']
                self.assertEqual(gates['opcode:0xf3'], returns)
                self.assertEqual(gates['opcode:0xfd'], child_reverts)
                self.assertEqual(gates['opcode:0x00'], 0)
                self.assertEqual(gates['opcode:0x3e'], 0)
                self.assertEqual(gates[f'opcode:0x{opcode:02x}'], 16)
                for other in (0xf4, 0xf1, 0xf2, 0xfa):
                    self.assertEqual(gates[f'opcode:0x{other:02x}'], 16 if other == opcode else 0)

    def test_outcap_triplet_freezes_runtime_input_width_and_all_other_words(self):
        cases = self.load()
        rows = [cases.make_case('600000', 0, 33, 64, mode, 'self', True)
                for mode in ('zero', 'half', 'full')]
        programs = [row['request']['program'] for row in rows]
        inputs = [bytes.fromhex(p['input_hex'][2:]) for p in programs]
        self.assertEqual([len(data) for data in inputs], [256, 256, 256])
        self.assertEqual([int.from_bytes(data[96:128], 'big') for data in inputs], [0, 16, 33])
        self.assertEqual(len({data[:96] + data[128:] for data in inputs}), 1)
        self.assertEqual(len({p['code_hex'] for p in programs}), 1)
        self.assertEqual([p['expected_route_counts'] for p in programs],
                         [programs[0]['expected_route_counts']] * 3)
        self.assertEqual(len({row['pair_group'] for row in rows}), 1)
        self.assertEqual([row['role'] for row in rows], ['control', 'candidate', 'candidate'])

    def test_finite_grid_preserves_every_triplet_and_does_not_clobber(self):
        cases = self.load()
        compilation = {'source_sha256': 'a' * 64}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'cases'
            with patch.object(cases, 'compile_source', return_value=('00', compilation)):
                self.assertEqual(cases.prepare(output, 'unused'), 90)
                catalog = json.loads((output / 'catalog.json').read_text())
                self.assertEqual(len({r['sample_id'] for r in catalog['cases']}), 90)
                groups = {}
                for row in catalog['cases']:
                    groups.setdefault(row['pair_group'], []).append(row)
                    self.assertEqual(cases.digest((output / row['request_path']).read_bytes()),
                                     row['request_sha256'])
                self.assertEqual(len(groups), 30)
                for rows in groups.values():
                    self.assertEqual({r['parameters']['copy_mode'] for r in rows}, {'zero', 'half', 'full'})
                kinds = {r['parameters']['call_kind'] for r in catalog['cases']}
                self.assertEqual(kinds, {'delegatecall', 'call', 'callcode', 'staticcall'})
                with self.assertRaises(FileExistsError):
                    cases.prepare(output, 'unused')

    def test_invalid_or_contradictory_parameters_emit_no_request(self):
        cases = self.load()
        for args in ((4, 1, 1, 'zero', 'self', False), (0, -1, 1, 'zero', 'self', False),
                     (0, 1048577, 1, 'zero', 'self', False), (0, 1, 0, 'zero', 'self', False),
                     (0, 1, 1025, 'zero', 'self', False), (False, 1, 1, 'zero', 'self', False),
                     (0, 1, 1, 'other', 'self', False), (0, 1, 1, 'full', 'identity', True)):
            with self.assertRaises(ValueError):
                cases.make_case('00', *args)

    def test_repetition_profile_retains_all_triplets_and_rejects_unknown_profile(self):
        cases = self.load()
        compilation = {'source_sha256': 'a' * 64}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'repetition'
            with patch.object(cases, 'compile_source', return_value=('00', compilation)) as compiler:
                try:
                    count = cases.prepare(output, 'unused', profile='repetition')
                except TypeError as error:
                    self.fail(f'finite repetition profile is not implemented: {error}')
                self.assertEqual(count, 12)
                catalog = json.loads((output / 'catalog.json').read_text())
                groups = {}
                for row in catalog['cases']:
                    groups.setdefault(row['pair_group'], []).append(row)
                    self.assertEqual(row['parameters']['call_kind'], 'delegatecall')
                    self.assertEqual(row['parameters']['target_kind'], 'self')
                    self.assertEqual(row['parameters']['return_length'], 65536)
                self.assertEqual(len(groups), 4)
                self.assertEqual({(r['parameters']['count'], r['parameters']['child_revert'])
                                  for r in catalog['cases']}, {(256, False), (256, True),
                                                              (512, False), (512, True)})
                for rows in groups.values():
                    self.assertEqual({r['parameters']['copy_mode'] for r in rows}, {'zero', 'half', 'full'})
                compiler.reset_mock()
                with self.assertRaises(ValueError):
                    cases.prepare(Path(temporary) / 'unknown', 'unused', profile='unknown')
                compiler.assert_not_called()
                self.assertFalse((Path(temporary) / 'unknown').exists())

    @unittest.skipUnless(os.environ.get('SOLC_BIN'), 'set SOLC_BIN for explicit compiler qualification')
    def test_real_runtime_uses_automatic_copy_only_and_fits_fixture_limit(self):
        cases = self.load()
        runtime, compilation = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE,
                                                     'ReturnCopy')
        code = bytes.fromhex(runtime)
        self.assertLessEqual(len(code), 24576)
        metadata_length = int.from_bytes(code[-2:], 'big')
        executable = code[:-metadata_length-2]
        opcodes, pc = [], 0
        while pc < len(executable):
            opcode = executable[pc]
            opcodes.append(opcode)
            pc += 1 + (opcode - 0x5f if 0x60 <= opcode <= 0x7f else 0)
        self.assertNotIn(0x3e, opcodes, 'RETURNDATACOPY would confound automatic copy')
        for opcode in (0xf4, 0xf1, 0xf2, 0xfa, 0xf3, 0xfd, 0x3d):
            self.assertIn(opcode, opcodes)
        self.assertEqual(compilation['standard_input']['settings']['evmVersion'], 'shanghai')

    @unittest.skipUnless(os.environ.get('SOLC_BIN') and os.environ.get('NATIVE_FIXTURE_BIN'),
                         'set explicit compiler/native binary for opcode-path qualification')
    def test_native_zero_and_full_copy_execute_identical_opcode_paths(self):
        cases = self.load()
        runtime, _ = cases.compile_source(Path(os.environ['SOLC_BIN']), cases.SOURCE, 'ReturnCopy')
        reports = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for mode in ('zero', 'full'):
                row = cases.make_case(runtime, 0, 33, 2, mode)
                folder = root / mode
                folder.mkdir()
                request, report = folder / 'request.json', folder / 'native.json'
                request.write_bytes(cases.encoded(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(request),
                    '--output', str(folder / 'guest-input.json'), '--report', str(report)],
                    capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                reports.append(json.loads(report.read_text()))
        self.assertEqual(reports[0]['observed_route_counts'], reports[1]['observed_route_counts'])


if __name__ == '__main__':
    unittest.main()
