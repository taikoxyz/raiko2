"""Concrete stack/value oracles for the prospective normal-dispatch panel."""
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
from test_stack_density import SEEDS, instructions

MASK = (1 << 256) - 1
CONTEXT = {0x34: 0, 0x42: 1786021201, 0x43: 1166000}
PATTERNS = {'dup16-pop': (0x8f, 0x50), 'swap16-swap16': (0x9f, 0x9f),
            'xor': (0x81, 0x81, 0x18, 0x50), 'shl255': (0x80, 0x60, 0x1b, 0x50),
            'sar255': (0x80, 0x60, 0x1d, 0x50), 'callvalue-pop': (0x34, 0x50),
            'timestamp-pop': (0x42, 0x50), 'number-pop': (0x43, 0x50)}
BODY_BYTES = {'dup16-pop': bytes.fromhex('8f50'), 'swap16-swap16': bytes.fromhex('9f9f'),
              'xor': bytes.fromhex('81811850'), 'shl255': bytes.fromhex('8060ff1b50'),
              'sar255': bytes.fromhex('8060ff1d50'), 'callvalue-pop': bytes.fromhex('3450'),
              'timestamp-pop': bytes.fromhex('4250'), 'number-pop': bytes.fromhex('4350')}


def sites(code, pattern):
    image = BODY_BYTES[pattern] * 128
    start = code.index(image)
    assert code.find(image, start + 1) == -1
    return tuple(start + len(BODY_BYTES[pattern]) * i for i in range(128))


def execute(code, body_starts=(), context=None):
    context = dict(CONTEXT if context is None else context)
    pc = 0; stack = []; memory = bytearray(32); counts = Counter(); states = Counter(); binary = Counter(); reads = Counter(); stores = []
    starts = set(body_starts)
    for _ in range(3_000_000):
        at = pc; op = code[pc]; pc += 1; counts[op] += 1
        if at in starts: states[at, tuple(stack)] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width
        elif 0x80 <= op <= 0x8f: stack.append(stack[-(op - 0x7f)])
        elif 0x90 <= op <= 0x9f:
            depth = op - 0x8f; stack[-1], stack[-1-depth] = stack[-1-depth], stack[-1]
        elif op == 0x50: stack.pop()
        elif op == 0x51:
            offset = stack.pop(); assert offset == 0; stack.append(int.from_bytes(memory, 'big'))
        elif op == 0x52:
            offset, value = stack.pop(), stack.pop(); assert offset == 0; memory[:] = value.to_bytes(32, 'big'); stores.append(value)
        elif op in (1, 0x14, 0x18, 0x1b, 0x1c, 0x1d):
            a, b = stack.pop(), stack.pop(); binary[at, op, a, b] += 1
            if op == 1: value = (a + b) & MASK
            elif op == 0x14: value = int(a == b)
            elif op == 0x18: value = a ^ b
            elif op == 0x1b: value = (b << a) & MASK if a < 256 else 0
            elif op == 0x1c: value = b >> a if a < 256 else 0
            else:
                signed = b - (1 << 256) if b & (1 << 255) else b
                value = (signed >> a) & MASK
            stack.append(value)
        elif op == 0x15: stack.append(int(stack.pop() == 0))
        elif op in CONTEXT:
            reads[at, op, context[op]] += 1; stack.append(context[op])
        elif op in (0x56, 0x57):
            target = stack.pop(); take = op == 0x56 or stack.pop() != 0
            if take: assert code[target] == 0x5b; pc = target
        elif op == 0x5b: pass
        elif op in (0, 0xfe): return op, stack, bytes(memory), counts, states, binary, reads, stores
        else: raise AssertionError(f'unsupported concrete opcode {op:02x}')
    raise AssertionError('holdout failed to terminate')


def closed_counts(pattern, n):
    b = n // 128
    c = Counter({0x62: 4*b+4, 0x52: b+1, 0x7f: 34, 0x5b: b+2, 0x51: 2*b+1,
                 0x14: b+18, 0x61: 2*b+18, 0x57: b+18, 0x60: b, 1: b, 0x56: b, 0x15: 17, 0: 1})
    for op in PATTERNS[pattern]: c[op] += n
    if pattern not in ('dup16-pop', 'swap16-swap16'):
        for op in PATTERNS[pattern][:-1]: c[op] += 1
        for op in (0x7f, 0x14, 0x15, 0x61, 0x57): c[op] += 1
    return +c


class NormalDispatchHoldoutTests(unittest.TestCase):
    def load(self):
        path = CASES / 'normal_dispatch_holdout.py'
        self.assertTrue(path.is_file(), 'prospective producer is not implemented')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('normal_holdout', path)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m

    def test_fresh_24_matrix_contract_hashes_and_nooverwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'cases'; self.assertEqual(m.prepare(root), 24)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 24)
            self.assertEqual(len({r['request_sha256'] for r in rows}), 24)
            self.assertEqual({(r['parameters']['pattern'], r['parameters']['bulk_operations']) for r in rows}, {(p, n) for p in PATTERNS for n in (0, 8192, 196608)})
            self.assertEqual(catalog['frozen_model_id'], 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef')
            self.assertEqual(catalog['generator_sha256'], m.digest((CASES/'normal_dispatch_holdout.py').read_bytes()))
            self.assertEqual(set(catalog['source_sha256']), {'stack_density.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py'})
            for row in rows:
                p = row['parameters']; raw = (root/row['request_path']).read_bytes(); req = json.loads(raw)
                self.assertEqual(m.digest(raw), row['request_sha256']); self.assertEqual(row['data_role'], 'prospective_validation_probe'); self.assertTrue(row['holdout_only'])
                self.assertEqual(req['program']['transactions'], {'kind': 'call', 'count_per_block': 1, 'gas_limit': 15_000_000})
                self.assertEqual(req['program']['input_hex'], '0x'); self.assertEqual(req['program']['block_count'], 1); self.assertTrue(req['expected_user_success'])
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['bulk_operations'], 0)
                if p['bulk_operations'] > 8192: self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['bulk_operations'], 8192)
                self.assertEqual(p['unroll_factor'], 128); self.assertFalse(p['counter_present_on_body_stack'])
            frozen = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(frozen, {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other = Path(temporary)/'metadata'; m.prepare(other, model_id='1'*64)
            self.assertEqual({p.name: p.read_bytes() for p in (root/'requests').iterdir()}, {p.name: p.read_bytes() for p in (other/'requests').iterdir()})

    def test_independent_actual_operands_signed_shifts_context_memory_and_counts(self):
        m = self.load()
        self.assertNotEqual(SEEDS[-1], SEEDS[-2]); self.assertEqual(SEEDS[-1] & 1, 1)
        for pattern in PATTERNS:
            codes = []
            for n in (0, 8192, 196608):
                row = m.make_case(pattern, n); req = row['request']; code = bytes.fromhex(req['program']['code_hex'][2:]); codes.append(code)
                points = sites(code, pattern); result = execute(code, points); terminal, stack, memory, counts, states, binary, reads, stores = result
                self.assertEqual((terminal, stack), (0, [])); self.assertEqual(memory, (n//128).to_bytes(32, 'big')); self.assertEqual(stores, list(range(n//128+1)))
                self.assertEqual(counts, closed_counts(pattern, n)); self.assertEqual({f'opcode:0x{k:02x}': v for k, v in counts.items()}, {k: v for k, v in req['program']['expected_route_counts'].items() if v})
                self.assertEqual(states, Counter({(pc, SEEDS): n//128 for pc in points}) if n else Counter())
                self.assertEqual(sum(counts.values()), row['parameters']['expected_completed_user_opcodes'])
                for pc in points:
                    if pattern == 'xor': self.assertEqual(binary[pc+2, 0x18, SEEDS[-1], SEEDS[-2]], n//128)
                    if pattern in ('shl255', 'sar255'): self.assertEqual(binary[pc+3, 0x1b if pattern == 'shl255' else 0x1d, 255, SEEDS[-1]], n//128)
                if pattern.endswith('-pop') and pattern in ('callvalue-pop', 'timestamp-pop', 'number-pop'):
                    op = PATTERNS[pattern][0]; self.assertEqual(sum(v for (_, o, value), v in reads.items() if o == op and value == CONTEXT[op]), n+1)
                if pattern == 'sar255': self.assertEqual(row['parameters']['warmup_expected_result_hex'], '0x'+'ff'*32)
                if pattern == 'shl255': self.assertEqual(row['parameters']['warmup_expected_result_hex'], '0x80'+'00'*31)
            self.assertEqual(len({len(c) for c in codes}), 1)
            self.assertLessEqual(sum(a != b for a, b in zip(codes[0], codes[-1])), 3)

    def test_semantic_oracle_counter_depth_and_context_mutations(self):
        m = self.load()
        row = m.make_case('sar255', 8192); code = bytearray.fromhex(row['request']['program']['code_hex'][2:])
        first_shift = next(pc for pc, op, _ in instructions(code) if op == 0x1d); code[first_shift] = 0x1c
        self.assertEqual(execute(bytes(code))[0], 0xfe, 'logical shift cannot satisfy negative SAR warmup')
        row = m.make_case('number-pop', 8192); code = bytes.fromhex(row['request']['program']['code_hex'][2:])
        self.assertEqual(execute(code, context=CONTEXT | {0x43: CONTEXT[0x43]+1})[0], 0xfe)
        row = m.make_case('timestamp-pop', 8192); code = bytes.fromhex(row['request']['program']['code_hex'][2:])
        self.assertEqual(execute(code, context=CONTEXT | {0x42: CONTEXT[0x42]-1})[0], 0xfe)
        row = m.make_case('xor', 8192); code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); pc = sites(code, 'xor')[0]; code[pc] = 0x80
        out = execute(bytes(code), {pc}); self.assertEqual(out[0:2], (0, []))
        self.assertGreater(out[5][pc+2, 0x18, SEEDS[-1], SEEDS[-1]], 0, 'balanced wrong depth still needs operand oracle')
        code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); bound = next(pc for pc, op, data in instructions(code) if op == 0x62 and int.from_bytes(data, 'big') == 64)
        code[bound+1:bound+4] = bytes(3); out = execute(bytes(code)); self.assertEqual(out[0:2], (0, [])); self.assertNotEqual(out[3], closed_counts('xor', 8192))
        code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); guard = [pc for pc, op, _ in instructions(code) if op == 0x7f][-1]; code[guard+32] ^= 1
        self.assertEqual(execute(bytes(code))[0], 0xfe)

    def test_typed_bounds_and_metadata_model_validation(self):
        m = self.load()
        for pattern, n in [(True, 0), (None, 0), ('unknown', 0), ('shr255', 128), ('xor', True), ('xor', -128), ('xor', 64), ('xor', 196736), ('xor', 1.0)]:
            with self.subTest(pattern=pattern, n=n), self.assertRaises(ValueError): m.make_case(pattern, n)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('', None, True, 'A'*64, 'g'*64):
                with self.assertRaises(ValueError): m.prepare(Path(temporary)/'invalid', model_id=model)
            self.assertFalse((Path(temporary)/'invalid').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'), 'tester opts in frozen native and candidate quote for24 holdout qualifications')
    def test_actual_24_native_and_candidate_quotes(self):
        m = self.load(); model = os.environ.get('ZKGAS_EXPECTED_MODEL_ID', m.MODEL)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); m.prepare(root/'cases', model_id=model)
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                folder = root/row['sample_id']; folder.mkdir(); source = root/'cases'/row['request_path']
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'), '--report', str(folder/'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, result.stderr.decode()); report = json.loads((folder/'native.json').read_text())
                self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in json.loads(source.read_text())['program']['expected_route_counts'].items() if v})
                self.assertEqual((report['blocks'][0]['block_number'], report['blocks'][0]['block_timestamp']), (CONTEXT[0x43], CONTEXT[0x42]))
                self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0])); self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'], 0)
                result = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-bincode', str(folder/'input.bin'), '--output', str(folder/'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', 'normal-dispatch-holdout-qualification', '--sample-id', row['sample_id']], capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, result.stderr.decode()); quote = json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(quote['model_id'], model); self.assertEqual(quote['public_output'], report['public_output']); self.assertEqual(quote['guest_input_sha256'], report['guest_input_sha256'])


if __name__ == '__main__': unittest.main()
