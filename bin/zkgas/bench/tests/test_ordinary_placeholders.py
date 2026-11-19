"""Independent execution/count/gas checks for the finite placeholder follow-up."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'


def execute(code, contracts, *, origin=1, caller=1):
    """Derive successful-path gas/counts; failure mutants test terminals/counts only."""
    counts = Counter()
    child_terminals = []
    call_operands = []
    warm = set()
    static = {0x00: 0, 0x01: 3, 0x14: 3, 0x15: 3, 0x32: 2, 0x33: 2,
              0x3d: 2, 0x50: 2, 0x56: 8, 0x57: 10, 0x5b: 1,
              0x80: 3, 0xf1: 0, 0xf3: 0, 0xfd: 0, 0xfe: 0}

    def frame(image, depth):
        stack, pc, gas, returndata = [], 0, 0, b''
        for _ in range(4_000_000):
            op = image[pc]
            pc += 1
            counts[f'opcode:0x{op:02x}'] += 1
            if 0x60 <= op <= 0x7f:
                width = op - 0x5f
                stack.append(int.from_bytes(image[pc:pc+width], 'big'))
                pc += width
                gas += 3
                continue
            gas += static[op]
            if op == 0x80:
                stack.append(stack[-1])
            elif op == 0x01:
                stack.append((stack.pop()+stack.pop()) % (1 << 256))
            elif op == 0x14:
                stack.append(int(stack.pop() == stack.pop()))
            elif op == 0x15:
                stack.append(int(stack.pop() == 0))
            elif op == 0x32:
                stack.append(origin)
            elif op == 0x33:
                stack.append(caller)
            elif op == 0x50:
                stack.pop()
            elif op in (0x56, 0x57):
                target = stack.pop()
                take = op == 0x56 or stack.pop() != 0
                if take:
                    assert image[target] == 0x5b
                    pc = target
            elif op == 0x5b:
                pass
            elif op == 0x3d:
                stack.append(len(returndata))
            elif op == 0xf1:
                assert depth == 0, 'the finite child must not call another frame'
                operands = tuple(stack.pop() for _ in range(7))
                forwarded, address, value, start, size, out, outsize = operands
                assert (forwarded, value, start, size, out, outsize) == (1000, 0, 0, 0, 0, 0)
                call_operands.append(operands)
                gas += 100 if address in warm else 2600
                warm.add(address)
                terminal, output, _, used = frame(contracts[address], depth+1)
                assert used <= forwarded
                child_terminals.append(terminal)
                gas += used
                returndata = output
                stack.append(int(terminal in (0x00, 0xf3)))
            elif op in (0xf3, 0xfd):
                offset, size = stack.pop(), stack.pop()
                # Normal probes return/revert zero bytes. A nonempty return is a mutation.
                words = (offset+size+31)//32 if size else 0
                gas += 3*words+words*words//512
                return op, bytes(size), stack, gas
            elif op in (0x00, 0xfe):
                return op, b'', stack, gas
            else:
                raise AssertionError(f'unexpected opcode {op:02x}')
        raise AssertionError('finite program did not terminate')

    terminal, _, stack, gas = frame(code, 0)
    return dict(terminal=terminal, stack=stack, gas=21000+gas, counts=counts,
                child_terminals=child_terminals, call_operands=call_operands)


class OrdinaryPlaceholderTests(unittest.TestCase):
    def load(self):
        source = CASES / 'ordinary_placeholders.py'
        self.assertTrue(source.is_file(), 'placeholder follow-up producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('ordinary_placeholder_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    @staticmethod
    def decode(row):
        program = row['request']['program']
        code = bytes.fromhex(program['code_hex'][2:])
        contracts = {int(c['address'], 16): bytes.fromhex(c['code_hex'][2:])
                     for c in program.get('prestate_contracts', [])}
        return code, contracts

    def test_real_instruction_results_frames_counts_and_gas(self):
        m = self.load()
        for operation in ('origin', 'call', 'caught-revert'):
            sizes = set()
            for n in (0, 1, 8, 720) if operation == 'origin' else (0, 1, 8):
                for lane in ('target', 'scaffold'):
                    row = m.make_case(operation, repetitions=n, lane=lane)
                    code, contracts = self.decode(row)
                    sizes.add(len(code))
                    actual = execute(code, contracts)
                    p = row['parameters']
                    self.assertEqual(actual['terminal'], 0)
                    self.assertEqual(actual['stack'], [])
                    self.assertEqual(actual['gas'], p['expected_user_charged_gas'])
                    for key, count in row['request']['program']['expected_route_counts'].items():
                        self.assertEqual(actual['counts'].get(key, 0), count, (operation, n, lane, key))
                    calls = 0 if operation == 'origin' else 1+(n if lane == 'target' else 0)
                    self.assertEqual(len(actual['call_operands']), calls)
                    self.assertEqual(p['expected_confirmed_call_events'], calls)
                    self.assertEqual(actual['counts'].get('opcode:0xfa', 0), 0)
                    if operation == 'origin':
                        self.assertEqual(actual['counts'].get('opcode:0x32', 0), n if lane == 'target' else 0)
                    else:
                        terminal = 0xfd if operation == 'caught-revert' else 0
                        self.assertEqual(actual['child_terminals'], [terminal]*calls)
                        self.assertEqual({len(c) for c in contracts.values()}, {32})
            self.assertEqual(sizes, {1024})

    def test_false_results_and_wrong_child_semantics_do_not_pass_gates(self):
        m = self.load()
        origin = m.make_case('origin', repetitions=8)
        code, contracts = self.decode(origin)
        self.assertEqual(execute(code, contracts, origin=2, caller=1)['terminal'], 0xfe)
        for operation, wrong_child in (
                ('call', bytes.fromhex('60006000fd')),
                ('caught-revert', b'\x00'),
                ('call', bytes.fromhex('60016000f3'))):
            row = m.make_case(operation, repetitions=8)
            code, contracts = self.decode(row)
            contracts[next(iter(contracts))] = wrong_child
            self.assertEqual(execute(code, contracts)['terminal'], 0xfe, operation)
        row = m.make_case('caught-revert', repetitions=8)
        code, contracts = self.decode(row)
        contracts[next(iter(contracts))] = b'\xfe'
        actual = execute(code, contracts)
        self.assertEqual(actual['terminal'], 0)
        self.assertNotEqual(actual['counts'].get('opcode:0xfd', 0), row['parameters']['expected_child_revert_events'])
        self.assertGreater(actual['counts'].get('opcode:0xfe', 0), 0)
        row = m.make_case('call', repetitions=8)
        code, contracts = self.decode(row)
        at = code.index(b'\xf1')
        mutated = code[:at]+b'\xfa'+code[at+1:]
        with self.assertRaises((AssertionError, KeyError)):
            execute(mutated, contracts)

    def test_ecrecover_requests_are_reused_without_another_generator(self):
        m = self.load()
        import precompile_boundaries as precompiles
        for n in (0, 1, 8):
            self.assertEqual(m.make_case('ecrecover', repetitions=n),
                             precompiles.make_case('ecrecover-valid', n))

    def test_typed_bounds_and_catalog_are_frozen_create_only(self):
        m = self.load()
        for kwargs in (dict(operation=True), dict(operation='staticcall'), dict(repetitions=True),
                       dict(repetitions=-1), dict(repetitions=65537), dict(lane=True),
                       dict(lane='other'), dict(operation='ecrecover', lane='scaffold')):
            args = dict(operation='origin', repetitions=1)
            args.update(kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                m.make_case(**args)
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'catalog'
            self.assertEqual(m.prepare(output), 23)
            catalog = json.loads((output/'catalog.json').read_bytes())
            by_id = {row['sample_id']: row for row in catalog['cases']}
            self.assertEqual(len(by_id), 23)
            for name, sha in catalog['source_sha256'].items():
                self.assertEqual(m.digest((CASES/name).read_bytes()), sha)
            for row in by_id.values():
                self.assertIn(row['baseline_sample_id'], by_id)
                self.assertEqual(m.digest((output/row['request_path']).read_bytes()), row['request_sha256'])
                self.assertEqual(row['request_sha256'], m.digest(m.encoded(json.loads((output/row['request_path']).read_bytes()))))
            before = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):
                m.prepare(output)
            self.assertEqual(before, {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()})
            bad = Path(temporary)/'bad'
            with self.assertRaises(ValueError):
                m.prepare(bad, model_id='invalid')
            self.assertFalse(bad.exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set frozen native constructor for real execution')
    def test_optional_native_receipts_routes_and_gas(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            for operation, n, lane in [('origin', 720, 'target'), ('origin', 8, 'scaffold'),
                                       ('call', 0, 'target'), ('call', 8, 'target'),
                                       ('call', 8, 'scaffold'), ('caught-revert', 0, 'target'),
                                       ('caught-revert', 8, 'target'), ('caught-revert', 8, 'scaffold')]:
                row = m.make_case(operation, repetitions=n, lane=lane)
                folder = Path(temporary)/row['sample_id']
                folder.mkdir()
                request = folder/'request.json'
                request.write_bytes(m.encoded(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(request),
                    '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'),
                    '--report', str(folder/'native.json'), '--execution-profile', 'evm-gas-only'],
                    capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                report = json.loads((folder/'native.json').read_bytes())
                self.assertEqual(report['transaction_count'], 2)
                self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
                self.assertEqual(report['user_transaction_gas_used_total'], row['parameters']['expected_user_charged_gas'])
                self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'], 0)
                for key, count in row['request']['program']['expected_route_counts'].items():
                    self.assertEqual(report['observed_route_counts'].get(key, 0), count)
                    self.assertEqual(report['attempted_route_counts'].get(key, 0), count)


if __name__ == '__main__':
    unittest.main()
