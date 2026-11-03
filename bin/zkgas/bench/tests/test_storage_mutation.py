"""Concrete sequential EVM oracle for committed storage mutations (stdlib only)."""
from collections import Counter
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
MODES = ('update', 'delete', 'insert', 'mixed')
SIZES = (8192, 16384)
LEVELS = ('zero', 'small', 'full')
USERS = 8
BASE = 1048576
MASK = (1 << 256) - 1


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big')
        pc += width


def execute(code, state, envelope):
    """Run actual bytes; originals and access warmth reset at each transaction."""
    original = state.copy(); warm = set(); stack = []; memory = bytearray()
    counts = Counter(); reads = []; writes = []; gas = 21000; refund = 0; pc = 0
    boundaries = {at for at, op, _ in decode(code) if op == 0x5b}

    def expand(offset):
        nonlocal gas
        assert 0 <= offset < 1024, 'unexpected memory address'
        old = len(memory) // 32; new = max(old, (offset + 63) // 32)
        gas += 3 * new + new * new // 512 - 3 * old - old * old // 512
        memory.extend(bytes(32 * (new - old)))

    for _ in range(2_000_000):
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op == 1:
            stack.append((stack.pop() + stack.pop()) & MASK); gas += 3
        elif op == 0x1b:
            shift, value = stack.pop(), stack.pop()
            stack.append(0 if shift >= 256 else (value << shift) & MASK); gas += 3
        elif op == 0x14:
            stack.append(int(stack.pop() == stack.pop())); gas += 3
        elif op == 0x15:
            stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); expand(offset); gas += 3
            if op == 0x51: stack.append(int.from_bytes(memory[offset:offset + 32], 'big'))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x54:
            key = stack.pop(); cold = key not in warm; warm.add(key)
            value = state.get(key, 0); reads.append((key, value, cold)); stack.append(value)
            gas += 2100 if cold else 100
        elif op == 0x55:
            assert envelope - gas > 2300, 'SSTORE sentry'
            key, new = stack.pop(), stack.pop(); cold = key not in warm; warm.add(key)
            old = state.get(key, 0); initial = original.get(key, 0)
            writes.append((key, old, new, initial, cold))
            gas += 2100 if cold else 0
            if new == old: gas += 100
            elif initial == old:
                gas += 20000 if initial == 0 else 2900
                if initial and new == 0: refund += 4800
            else:
                gas += 100
                if initial:
                    if old == 0: refund -= 4800
                    if new == 0: refund += 4800
                if initial == new: refund += 19900 if initial == 0 else 2800
            if new: state[key] = new
            else: state.pop(key, None)
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0
            gas += 8 if op == 0x56 else 10
            if taken:
                assert target in boundaries, 'invalid jump destination'
                pc = target
        elif op == 0x5b: gas += 1
        elif op == 0xfd:
            offset, length = stack.pop(), stack.pop()
            assert offset == length == 0, 'unsupported nonzero REVERT range'
            assert gas <= envelope, 'out of gas'
            state.clear(); state.update(original)
            return dict(terminal=op, gas=gas, refund=0, applied=0, charged=gas,
                        refund_before_revert=refund, reads=reads, writes=writes,
                        counts=counts, memory=memory, stack=stack)
        elif op in (0, 0xfe):
            assert gas <= envelope, 'out of gas'
            return dict(terminal=op, gas=gas, refund=refund, applied=min(refund, gas // 5),
                        charged=gas - min(refund, gas // 5), reads=reads, writes=writes,
                        counts=counts, memory=memory, stack=stack)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound exceeded')


def qualify(test, row):
    """Expected trajectory follows the spec, never producer route or gas formulae."""
    p = row['parameters']; mode = p['mode']; size = p['state_size']; n = p['mutations_per_user']
    program = row['request']['program']; storage = program['storage_state']
    expected_state = dict.fromkeys(range(size), 1)
    state = {int(k, 16): int(v, 16) for k, v in storage['initial_storage'].items()}
    test.assertEqual(state, expected_state)
    test.assertEqual(program['transactions'], {'kind': 'call', 'count_per_block': 8, 'gas_limit': p['transaction_gas_limit']})
    test.assertEqual((program['block_count'], program['input_hex']), (1, '0x'))
    code = bytes.fromhex(program['code_hex'][2:]); counts = Counter(); proof = [0]; inserts = []; deletes = []; results = []
    for user in range(8):
        start = 1 + user * n; reads = [(0, start, True)]; writes = []; checksum = 0
        for i in range(n):
            present = start + i; absent = BASE + present
            operations = ([(present, 1, 2)] if mode == 'update' else [(present, 1, 0)] if mode == 'delete'
                          else [(absent, 0, 2)] if mode == 'insert' else [(present, 1, 0), (absent, 0, 2)])
            for key, old, new in operations:
                proof.append(key); reads.extend([(key, old, True), (key, new, False)])
                writes.append((key, old, new, old, False)); checksum += new
                if new: expected_state[key] = new
                else: expected_state.pop(key)
                if old == 0: inserts.append(key)
                if new == 0: deletes.append(key)
        final = start + n; reads.append((0, final, False))
        writes.append((0, start, final, start, False)); expected_state[0] = final
        result = execute(code, state, p['transaction_gas_limit']); results.append(result)
        test.assertEqual(result['terminal'], 0, 'runtime guard failed')
        test.assertEqual(result['stack'], [])
        test.assertEqual(result['reads'], reads, 'ordered read range/value/warmth')
        test.assertEqual(result['writes'], writes, 'ordered writes/originals')
        test.assertEqual(int.from_bytes(result['memory'][:32], 'big'), n, 'counter')
        test.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), checksum, 'checksum')
        test.assertEqual(len(result['memory']), 128 if n else 96)
        test.assertEqual(state, expected_state, 'committed transaction state')
        for name, field in (('gas', 'expected_user_gross_gas'), ('refund', 'expected_user_refund_counter'),
                            ('applied', 'expected_user_refund_applied'), ('charged', 'expected_user_charged_gas')):
            test.assertEqual(result[name], p[field], field)
        counts.update(result['counts'])
    test.assertEqual([int(k, 16) for k in storage['proof_slots']], proof)
    test.assertEqual([int(k, 16) for k in storage['insert_slots']], inserts)
    test.assertEqual([int(k, 16) for k in storage['delete_slots']], deletes)
    test.assertEqual(counts, +Counter(program['expected_route_counts']))
    test.assertEqual(len(proof), p['proof_slot_count'])
    test.assertEqual(len(set(proof)), len(proof)); test.assertLessEqual(len(proof), 8192)
    test.assertEqual(len(proof), p['expected_distinct_storage_keys'])
    test.assertEqual(sum(len(r['writes']) for r in results), p['expected_storage_writes'])
    test.assertEqual(state[0], p['expected_final_phase'])
    test.assertEqual(len(state), p['expected_final_storage_entry_count'])
    test.assertEqual(p['expected_block_charged_gas'], 167355 + sum(r['charged'] for r in results))
    test.assertLessEqual(p['expected_block_charged_gas'], 30_000_000)
    test.assertEqual(p['runtime_bytes'], len(code)); test.assertEqual(p['runtime_sha256'], hashlib.sha256(code).hexdigest())
    test.assertEqual(p['expected_counter'], n)
    test.assertEqual(p['expected_checksum'], 0 if mode == 'delete' else 2 * n)
    test.assertEqual(p['expected_main_storage_writes'], 8 * n * (2 if mode == 'mixed' else 1))
    test.assertEqual(p['expected_phase_storage_writes'], 8)
    return results


class StorageMutationTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_mutation.py'
        self.assertTrue(path.is_file(), 'storage mutation producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_mutation_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_strict_inputs_and_maximal_integer_full_bound(self):
        m = self.load()
        for args in ((None,), ('UPDATE',), (True,), ('update', True), ('update', 8192.0),
                     ('update', 8193), ('update', 8192, True), ('update', 8192, 'FULL')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for mode in MODES:
            for size in SIZES:
                full = m.make_case(mode, size, 'full'); p = full['parameters']; n = p['mutations_per_user']
                self.assertGreater(n, 64); self.assertTrue(m.fits(mode, size, n))
                self.assertFalse(m.fits(mode, size, n + 1), 'N+1 must violate its own candidate envelope/resource bounds')
                self.assertEqual(p['transaction_gas_limit'], p['expected_user_gross_gas'] + 1000)
                self.assertLessEqual(p['transaction_gas_limit'], 15_000_000)
                for j in range(8):
                    self.assertGreaterEqual(31_000_000 - 167355 - j * p['expected_user_charged_gas'], p['transaction_gas_limit'])
                with self.assertRaises(ValueError): m.fits(mode, size, True)
                with self.assertRaises(ValueError): m.fits(mode, size, -1)

    def test_maximal_full_bound_from_independently_executed_candidate(self):
        m = self.load()
        for mode in MODES:
            for size in SIZES:
                row = m.make_case(mode, size, 'full'); p = row['parameters']; n = p['mutations_per_user']
                code = bytearray.fromhex(row['request']['program']['code_hex'][2:])
                # Fixed-width loop/checksum/phase bounds are patched in concrete
                # bytes, without asking the producer's gas calculator or solver.
                for at, op, value in decode(code):
                    if op == 0x62 and value in (n, 2 * n):
                        code[at + 1:at + 4] = (value + (1 if value == n else 2)).to_bytes(3, 'big')
                result = execute(code, dict.fromkeys(range(size), 1), 15_000_000)
                self.assertEqual(result['terminal'], 0)
                envelope = result['gas'] + 1000; charged = result['charged']
                violations = [1 + 8 * (n + 1) * (2 if mode == 'mixed' else 1) > 8192,
                              mode != 'insert' and 8 * (n + 1) > size - 1,
                              envelope > 15_000_000, 167355 + 8 * charged > 30_000_000]
                violations.extend(31_000_000 - 167355 - j * charged < envelope for j in range(8))
                with self.subTest(mode=mode, size=size): self.assertTrue(any(violations), 'independent N+1 remains feasible')

    def test_catalog_24_identities_own_controls_provenance_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'matrix'
            self.assertEqual(m.prepare(root), 24)
            raw_catalog = (root / 'catalog.json').read_bytes(); catalog = json.loads(raw_catalog); rows = catalog['cases']
            self.assertEqual(catalog['frozen_model_id'], m.MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_mutation.py').read_bytes()).hexdigest())
            sources = ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in sources})
            by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 24); self.assertEqual(len({r['request_sha256'] for r in rows}), 24)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 8)
            self.assertEqual({(r['parameters']['mode'], r['parameters']['state_size'], r['parameters']['workload_level']) for r in rows},
                             {(mode, size, level) for mode in MODES for size in SIZES for level in LEVELS})
            for row in rows:
                p = row['parameters']; request_bytes = (root / row['request_path']).read_bytes()
                self.assertEqual(hashlib.sha256(request_bytes).hexdigest(), row['request_sha256'])
                request = json.loads(request_bytes); baseline = by[row['baseline_sample_id']]
                self.assertEqual(row['sample_id'], f"storage-mutation-{p['mode']}-s{p['state_size']}-n{p['mutations_per_user']}")
                self.assertEqual(baseline['parameters']['mutations_per_user'], 0)
                self.assertEqual((baseline['parameters']['mode'], baseline['parameters']['state_size']), (p['mode'], p['state_size']))
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                baseline_program = json.loads((root / baseline['request_path']).read_bytes())['program']
                background = baseline_program['storage_state']['initial_storage']
                self.assertEqual(request['program']['storage_state']['initial_storage'], background)
                code = bytes.fromhex(request['program']['code_hex'][2:])
                self.assertEqual(len(code), len(bytes.fromhex(baseline_program['code_hex'][2:])))
                self.assertTrue(all(op in (0x61, 0x62) for _, op, _ in decode(code) if 0x60 <= op <= 0x7f))
                self.assertEqual(p['expected_anchor_gas'], 167355)
                self.assertEqual((p['header_gas_limit'], p['charged_gas_budget']), (31_000_000, 30_000_000))
                if p['workload_level'] == 'zero':
                    storage = request['program']['storage_state']
                    self.assertEqual(storage['proof_slots'], ['0x0'])
                    self.assertEqual(storage['insert_slots'], []); self.assertEqual(storage['delete_slots'], [])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual((root / 'catalog.json').read_bytes(), raw_catalog)
            for bad in ('A' * 64, '0' * 63, True, None):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())

    def test_actual_bytes_all_24_sequential_users_maps_routes_and_exact_refunds(self):
        m = self.load()
        for mode in MODES:
            for size in SIZES:
                for level in LEVELS:
                    with self.subTest(mode=mode, size=size, level=level):
                        row = m.make_case(mode, size, level); results = qualify(self, row)
                        n = row['parameters']['mutations_per_user']
                        self.assertEqual(results[0]['refund'], n * 4800 if mode in ('delete', 'mixed') else 0)
                        self.assertEqual(results[0]['applied'], min(results[0]['refund'], results[0]['gas'] // 5))

    def test_semantic_negatives_include_successful_equal_value_wrong_ranges(self):
        m = self.load(); row = m.make_case('insert', 8192, 'small')
        program = row['request']['program']; code = bytes.fromhex(program['code_hex'][2:])
        offsets = [at for at, op, value in decode(code) if op == 0x62 and value == BASE]
        self.assertTrue(offsets)
        wrong = bytearray(code)
        for at in offsets: wrong[at + 1:at + 4] = (2 * BASE).to_bytes(3, 'big')
        state = dict.fromkeys(range(8192), 1)
        results = [execute(wrong, state, row['parameters']['transaction_gas_limit']) for _ in range(8)]
        self.assertTrue(all(r['terminal'] == 0 for r in results), 'negative must preserve receipts/checksums')
        mutated = copy.deepcopy(row); mutated['request']['program']['code_hex'] = '0x' + wrong.hex()
        with self.assertRaises(AssertionError): qualify(self, mutated)
        # Shift a present range into other all-one slots. Relocate real jump
        # immediates after inserting an ADD; guards/checksums still succeed.
        present_row = m.make_case('update', 8192, 'small')
        original = bytes.fromhex(present_row['request']['program']['code_hex'][2:])
        # PUSH3 values are big endian: locate phase+counter then key MSTORE.
        pattern = bytes.fromhex('62000040516200000051016200006052')
        location = original.index(pattern) + len(pattern) - 5
        extra = bytes.fromhex('6200040001')
        shifted = bytearray(original[:location] + extra + original[location:])
        for at, op, target in decode(original):
            if op == 0x61:
                new_at = at + (len(extra) if at >= location else 0)
                shifted[new_at + 1:new_at + 3] = (target + (len(extra) if target >= location else 0)).to_bytes(2, 'big')
        state = dict.fromkeys(range(8192), 1)
        self.assertTrue(all(execute(shifted, state, present_row['parameters']['transaction_gas_limit'])['terminal'] == 0 for _ in range(8)))
        present_row['request']['program']['code_hex'] = '0x' + shifted.hex()
        with self.assertRaises(AssertionError): qualify(self, present_row)
        for field in ('proof_slots', 'insert_slots'):
            mutated = copy.deepcopy(row); mutated['request']['program']['storage_state'][field].pop()
            with self.subTest(field=field), self.assertRaises(AssertionError): qualify(self, mutated)
        # Mutate every value2 body write to3: guards/checksum must reject it.
        wrong = bytearray(code)
        for at, op, value in decode(code):
            if op == 0x62 and value == 2: wrong[at + 1:at + 4] = (3).to_bytes(3, 'big')
        state = dict.fromkeys(range(8192), 1)
        self.assertEqual(execute(wrong, state, row['parameters']['transaction_gas_limit'])['terminal'], 0xfe)
        # A shortened loop and a wrong original-value guard must reject.
        for name in ('loop', 'old-value'):
            wrong = bytearray(code)
            if name == 'loop':
                at = code.index(bytes.fromhex('62000000516200004014')) + 5
                wrong[at + 1:at + 4] = (63).to_bytes(3, 'big')
            else:
                at = code.index(bytes.fromhex('546200000014')) + 1
                wrong[at + 1:at + 4] = (1).to_bytes(3, 'big')
            with self.subTest(guard=name):
                self.assertEqual(execute(wrong, dict.fromkeys(range(8192), 1), row['parameters']['transaction_gas_limit'])['terminal'], 0xfe)
        # Final checksum and phase guards must consume actual concrete values.
        for marker in (64, 128):
            wrong = bytearray(code)
            positions = [at for at, op, value in decode(code) if op == 0x62 and value == marker]
            self.assertTrue(positions)
            # N=64 appears in the loop and terminal/phase bound; the final one
            # is the post-write expected phase, while128 is expected checksum.
            at = positions[-1]; wrong[at + 1:at + 4] = (marker + 1).to_bytes(3, 'big')
            state = dict.fromkeys(range(8192), 1)
            with self.subTest(marker=marker):
                self.assertEqual(execute(wrong, state, row['parameters']['transaction_gas_limit'])['terminal'], 0xfe)

    @unittest.skipUnless(os.environ.get('STORAGE_MUTATION_NATIVE_BIN'), 'opt in one frozen native smoke')
    def test_frozen_native_small_insert_smoke(self):
        m = self.load(); row = m.make_case('insert', 8192, 'small'); p = row['parameters']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / 'request.json'; report_path = root / 'native.json'
            source.write_text(json.dumps(row['request']))
            process = subprocess.run([os.environ['STORAGE_MUTATION_NATIVE_BIN'], '--input', str(source),
                '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'),
                '--report', str(report_path), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
            self.assertEqual(process.returncode, 0, process.stderr.decode())
            report = json.loads(report_path.read_text()); receipts = report['receipts_by_block'][0]
            self.assertEqual(len(receipts), 9); self.assertTrue(all(r['success'] for r in receipts))
            self.assertEqual(receipts[0]['gas_used'], 167355)
            self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 8)
            self.assertEqual(report['storage_state']['final_storage_entry_count'], p['expected_final_storage_entry_count'])
            self.assertRegex(report['storage_state']['final_storage_root'], r'^0x[0-9a-f]{64}$')
            qualify(self, row)


if __name__ == '__main__': unittest.main()
