"""Bounded semantic and closed-form gas oracles for high-occupancy density inputs."""
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
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_stack_density import SEEDS, execute, instructions

PATTERNS = {'jumpdest': (0x5b,), 'push0-pop': (0x5f, 0x50),
            'dup16-pop': (0x8f, 0x50), 'swap16-swap16': (0x9f, 0x9f),
            'jumpdest-dup16-pop': (0x5b, 0x8f, 0x50)}
ANCHOR_GAS = 167355  # Frozen constructor fixture only; native receipts recheck this.
UNROLL = 2048


def closed_counts(pattern, batches):
    b = batches
    counts = Counter({0x62: 4*b+6, 0x52: b+1, 0x7f: 34, 0x5b: b+2,
                      0x51: 2*b+2, 0x14: b+19, 0x61: 2*b+19, 0x57: b+19,
                      0x60: b, 1: b, 0x56: b, 0x15: 18, 0: 1})
    for op in PATTERNS[pattern]: counts[op] += UNROLL*b
    return +counts


def static_cost(op):
    if op == 0: return 0
    if op == 0x5b: return 1
    if op in (0x50, 0x5f): return 2
    if op == 0x56: return 8
    if op == 0x57: return 10
    if op in (1, 0x14, 0x15, 0x51, 0x52) or 0x60 <= op <= 0x9f: return 3
    raise AssertionError(f'unsupported gas opcode {op:02x}')


def body_sites(code, pattern):
    decoded = instructions(code)
    jump = next(i for i, (_, op, _) in enumerate(decoded) if op == 0x57)
    body = decoded[jump+1:jump+1+UNROLL*len(PATTERNS[pattern])]
    assert tuple(op for _, op, _ in body) == PATTERNS[pattern]*UNROLL
    assert all(not data for _, _, data in body)
    return [pc for pc, _, _ in body]


def bounded_code(code, batches):
    """Change only the two fixed-width bounds, retaining the full frozen body."""
    code = bytearray(code); decoded = instructions(code)
    mloads = [i for i, (_, op, _) in enumerate(decoded) if op == 0x51]
    assert len(mloads) == 3
    for index in (mloads[0]+1, mloads[2]+1):
        pc, op, _ = decoded[index]; assert op == 0x62
        code[pc+1:pc+4] = batches.to_bytes(3, 'big')
    return bytes(code)


class GasBudgetDensityTests(unittest.TestCase):
    def load(self):
        path = CASES/'gas_budget_density.py'
        self.assertTrue(path.is_file(), 'gas-budget density producer is not implemented')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('gas_budget_density', path)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m

    def test_twenty_matched_cases_deterministic_metadata_and_nooverwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'; self.assertEqual(m.prepare(output), 20)
            catalog = json.loads((output/'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 20); self.assertEqual(len({r['request_sha256'] for r in rows}), 20)
            self.assertEqual({(r['parameters']['pattern'], r['parameters']['count_per_block'], r['parameters']['level']) for r in rows},
                             {(p, count, level) for p in PATTERNS for count in (4, 16) for level in ('zero', 'full')})
            self.assertFalse(catalog['actuals_used_for_fitting']); self.assertEqual(len(catalog['required_comparison_quantities']), 3)
            self.assertEqual(catalog['frozen_model_id'], 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef')
            self.assertEqual(catalog['generator_sha256'], m.digest((CASES/'gas_budget_density.py').read_bytes()))
            for name, digest in catalog['source_sha256'].items(): self.assertEqual(digest, m.digest((CASES/name).read_bytes()))
            for row in rows:
                raw = (output/row['request_path']).read_bytes(); self.assertEqual(m.digest(raw), row['request_sha256'])
                p = row['parameters']; req = json.loads(raw); n = p['count_per_block']
                self.assertEqual(req['program']['transactions'], {'kind': 'call', 'count_per_block': n, 'gas_limit': p['transaction_gas_limit']})
                self.assertEqual(req['program']['block_count'], 1); self.assertEqual(req['program']['input_hex'], '0x'); self.assertTrue(req['expected_user_success'])
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['level'], 'zero')
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['pattern'], p['pattern'])
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['count_per_block'], n)
            snapshot = {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual(snapshot, {p.relative_to(output): p.read_bytes() for p in output.rglob('*') if p.is_file()})
            other = Path(temporary)/'other'; m.prepare(other, model_id='1'*64)
            self.assertEqual({p.name: p.read_bytes() for p in (output/'requests').iterdir()}, {p.name: p.read_bytes() for p in (other/'requests').iterdir()})

    def test_full_gas_solution_counts_envelopes_and_fixed_body(self):
        m = self.load()
        for pattern in PATTERNS:
            for packing in (4, 16):
                zero = m.make_case(pattern, packing, 'zero'); full = m.make_case(pattern, packing, 'full')
                codes = [bytes.fromhex(row['request']['program']['code_hex'][2:]) for row in (zero, full)]
                self.assertEqual(len(codes[0]), len(codes[1])); self.assertEqual(bounded_code(codes[1], 0), codes[0])
                self.assertEqual(zero['parameters']['transaction_gas_limit'], full['parameters']['transaction_gas_limit'])
                for row, code in zip((zero, full), codes):
                    p = row['parameters']; b = p['batch_count']; counts = closed_counts(pattern, b)
                    expected = {f'opcode:0x{op:02x}': n*packing for op, n in counts.items()}
                    self.assertEqual({k: v for k, v in row['request']['program']['expected_route_counts'].items() if v}, expected)
                    self.assertEqual(p['expected_user_route_counts'], {f'opcode:0x{op:02x}': n for op, n in counts.items()})
                    self.assertLessEqual(max(expected.values()), 31_000_000)
                    gas = 21000+3+sum(static_cost(op)*n for op, n in counts.items())
                    self.assertEqual(p['expected_user_gas_used'], gas); self.assertEqual(p['expected_block_gas_used'], ANCHOR_GAS+packing*gas)
                    self.assertEqual(p['expected_refund_gas'], 0); self.assertEqual(p['expected_memory_expansion_gas'], 3)
                    self.assertEqual(p['expected_completed_user_opcodes'], sum(counts.values()))
                    self.assertEqual(p['expected_completed_all_user_opcodes'], sum(counts.values())*packing)
                    self.assertEqual(p['bulk_operations_per_user'], UNROLL*b); self.assertEqual(p['bulk_operations_all_users'], UNROLL*b*packing)
                    self.assertEqual(len(body_sites(code, pattern)), UNROLL*len(PATTERNS[pattern]))
                    self.assertGreaterEqual(p['transaction_gas_limit'], gas+100_000); self.assertLessEqual(p['transaction_gas_limit'], 15_000_000)
                    self.assertLessEqual(p['transaction_gas_limit'], 31_000_000-ANCHOR_GAS-(packing-1)*gas)
                    if p['level'] == 'full':
                        self.assertTrue(29_700_000 <= p['expected_block_gas_used'] <= 29_990_000)
                        next_gas = 21000+3+sum(static_cost(op)*n for op, n in closed_counts(pattern, b+1).items())
                        self.assertGreater(ANCHOR_GAS+packing*next_gas, 29_990_000)

    def test_bounded_actual_ordered_operands_counter_guards_and_counts(self):
        m = self.load()
        for pattern, body in PATTERNS.items():
            source = bytes.fromhex(m.make_case(pattern)['request']['program']['code_hex'][2:])
            for batches in (0, 1, 3):
                code = bounded_code(source, batches); sites = body_sites(code, pattern)
                terminal, stack, memory, counts, states, loads, stores = execute(code, set(sites))
                self.assertEqual((terminal, stack), (0, [])); self.assertEqual(memory, batches.to_bytes(32, 'big'))
                self.assertEqual({int(k[-2:], 16): v for k, v in counts.items()}, dict(closed_counts(pattern, batches)))
                for start in range(0, len(sites), len(body)):
                    expected_stack = list(SEEDS)
                    for pc, op in zip(sites[start:start+len(body)], body):
                        self.assertEqual(states.get((pc, op, tuple(expected_stack)), 0), batches)
                        if op == 0x5f: expected_stack.append(0)
                        elif op == 0x8f: expected_stack.append(expected_stack[-16])
                        elif op == 0x50: expected_stack.pop()
                        elif op == 0x9f: expected_stack[-1], expected_stack[-17] = expected_stack[-17], expected_stack[-1]
                    self.assertEqual(tuple(expected_stack), SEEDS)
                self.assertEqual(sum(stores.values()), batches+1); self.assertEqual(sum(loads.values()), 2*batches+2)

    def test_count_preserving_counter_stack_and_operand_mutations(self):
        m = self.load(); code = bounded_code(bytes.fromhex(m.make_case('dup16-pop')['request']['program']['code_hex'][2:]), 2)
        baseline = execute(code, set(body_sites(code, 'dup16-pop')))
        bad = bytearray(code); decoded = instructions(code); third_load = [i for i, (_, op, _) in enumerate(decoded) if op == 0x51][2]
        pc = decoded[third_load+1][0]; bad[pc+3] = 3
        self.assertEqual(execute(bytes(bad))[0], 0xfe, 'independent final counter guard must reject wrong final value')
        bad = bytearray(code); pc = body_sites(code, 'dup16-pop')[0]; bad[pc:pc+2] = bytes((0x50, 0x8f))
        self.assertEqual(execute(bytes(bad))[0], 0xfe, 'same body opcode multiplicities with reversed stack access fail ending guards')
        # Swap two distinct starting seeds and their matching ending guards.
        # The program still succeeds with identical counts; actual-operand evidence rejects it.
        bad = bytearray(code); constants = [pc for pc, op, _ in decoded if op == 0x7f]
        for a, b in ((constants[0], constants[1]), (constants[-1], constants[-2])):
            left, right = bytes(bad[a+1:a+33]), bytes(bad[b+1:b+33]); bad[a+1:a+33], bad[b+1:b+33] = right, left
        changed = execute(bytes(bad), set(body_sites(code, 'dup16-pop')))
        self.assertEqual(changed[0:2], (0, [])); self.assertEqual(changed[3], baseline[3]); self.assertNotEqual(changed[4], baseline[4])

    def test_strict_domains_before_filesystem_effects(self):
        m = self.load()
        for pattern, packing, level in [(True, 4, 'full'), ('bad', 4, 'full'), ('jumpdest', True, 'full'), ('jumpdest', 1, 'full'), ('jumpdest', 4.0, 'full'), ('jumpdest', 4, True), ('jumpdest', 4, 'low')]:
            with self.assertRaises(ValueError): m.make_case(pattern, packing, level)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('', None, True, 'A'*64, 'g'*64):
                with self.assertRaises(ValueError): m.prepare(Path(temporary)/'invalid', model_id=model)
            self.assertFalse((Path(temporary)/'invalid').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'), 'opt in frozen native/quote smoke; full panel is separately qualified once')
    def test_frozen_native_quote_spots_and_semantic_rejection(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index, (pattern, packing, level) in enumerate((('jumpdest', 4, 'full'), ('dup16-pop', 16, 'zero'), ('swap16-swap16', 16, 'full'))):
                row = m.make_case(pattern, packing, level); req = row['request']; folder = root/str(index); folder.mkdir(); source = folder/'request.json'; source.write_bytes(m.encoded(req))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'), '--report', str(folder/'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, result.stderr.decode()); report = json.loads((folder/'native.json').read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), packing+1); self.assertEqual(receipts[0]['gas_used'], ANCHOR_GAS); self.assertTrue(all(r['success'] for r in receipts))
                self.assertEqual([receipt['gas_used'] for receipt in receipts[1:]], [row['parameters']['expected_user_gas_used']]*packing)
                self.assertEqual(report['blocks'][0]['block_gas_used'], row['parameters']['expected_block_gas_used'])
                self.assertEqual(report['user_transaction_gas_used_total'], packing*row['parameters']['expected_user_gas_used'])
                self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in req['program']['expected_route_counts'].items() if v})
                result = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-bincode', str(folder/'input.bin'), '--output', str(folder/'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', 'gas-budget-density-smoke', '--sample-id', row['sample_id']], capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, result.stderr.decode()); quote = json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(quote['model_id'], m.MODEL); self.assertEqual(quote['public_output'], report['public_output']); self.assertEqual(quote['guest_input_sha256'], report['guest_input_sha256'])
            req = m.make_case('dup16-pop', 4, 'zero')['request']; code = bytearray.fromhex(req['program']['code_hex'][2:]); pc = [pc for pc, op, _ in instructions(code) if op == 0x7f][-1]; code[pc+32] ^= 1; req['program']['code_hex'] = '0x'+code.hex()
            source = root/'bad.json'; source.write_bytes(m.encoded(req)); result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(root/'bad-input.json'), '--output-bincode', str(root/'bad.bin'), '--report', str(root/'bad-report.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
            self.assertNotEqual(result.returncode, 0); self.assertNotIn(b'Usage:', result.stderr)
            self.assertFalse((root/'bad-input.json').exists()); self.assertFalse((root/'bad.bin').exists()); self.assertFalse((root/'bad-report.json').exists())


if __name__ == '__main__': unittest.main()
