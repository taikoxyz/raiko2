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


def execute(code, state, envelope, *, initial_warm_keys=None):
    """Run actual bytes; originals and access warmth reset at each transaction."""
    original = state.copy(); warm = set() if initial_warm_keys is None else set(initial_warm_keys); stack = []; memory = bytearray()
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
        elif op == 0x80:
            assert stack, 'DUP1 stack underflow'
            stack.append(stack[-1]); gas += 3
        elif op == 1:
            stack.append((stack.pop() + stack.pop()) & MASK); gas += 3
        elif op == 0x1b:
            shift, value = stack.pop(), stack.pop()
            stack.append(0 if shift >= 256 else (value << shift) & MASK); gas += 3
        elif op == 0x17:
            stack.append(stack.pop() | stack.pop()); gas += 3
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


class SizedStorageUpdateTests(unittest.TestCase):
    load = StorageMutationTests.load

    def qualify_sized(self, row, users, n, layout):
        program = row['request']['program']; p = row['parameters']
        self.assertEqual(program['code_hex'], '0x00')
        self.assertEqual(program['input_hex'], '0x')
        self.assertNotIn('storage_state', program)
        self.assertEqual(len(program['prestate_contracts']), 1)
        contract = program['prestate_contracts'][0]
        self.assertEqual(program['transactions']['recipients'], [contract['address']] * users)
        self.assertEqual(program['transactions']['count_per_block'], users)
        self.assertEqual(program['transactions']['gas_limit'], p['transaction_gas_limit'])
        raw = lambda i: i if layout is None else (i << 48) | (layout['L1'] << 64) | (layout['L3'] << 192)
        body_keys = [raw(i) for i in range(1, users * n + 1)]
        expected_state = dict.fromkeys([0] + body_keys, 1)
        self.assertEqual(len(set(body_keys)), users * n)
        self.assertNotIn(0, body_keys)
        state = {int(k, 16): int(v, 16) for k, v in contract['initial_storage'].items()}
        self.assertEqual(state, expected_state, 'complete nonzero prestate, not a hinted subset')
        code = bytes.fromhex(contract['code_hex'][2:]); routes = Counter(); results = []
        self.assertEqual(len(code), p['runtime_bytes'])
        self.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'])
        for tx in range(users):
            start = 1 + tx * n; final = start + n
            keys = [raw(i) for i in range(start, final)]
            reads = [(0, start, True)]
            for key in keys: reads.extend([(key, 1, True), (key, 2, False)])
            reads.append((0, final, False))
            writes = [(key, 1, 2, 1, False) for key in keys] + [(0, start, final, start, False)]
            result = execute(code, state, p['transaction_gas_limit']); results.append(result)
            self.assertEqual(result['terminal'], 0)
            self.assertEqual(result['stack'], [])
            self.assertEqual(result['reads'], reads)
            self.assertEqual(result['writes'], writes)
            self.assertEqual(int.from_bytes(result['memory'][:32], 'big'), n)
            self.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), 2 * n)
            self.assertEqual(int.from_bytes(result['memory'][64:96], 'big'), start)
            if n: self.assertEqual(int.from_bytes(result['memory'][96:128], 'big'), final - 1)
            self.assertEqual(len(result['memory']), 128 if n else 96)
            expected_state.update(dict.fromkeys(keys, 2)); expected_state[0] = final
            self.assertEqual(state, expected_state)
            self.assertEqual(result['refund'], 0); self.assertEqual(result['applied'], 0)
            self.assertEqual(result['gas'], result['charged'])
            self.assertEqual(result['gas'], p['expected_user_gross_gas'])
            self.assertEqual(result['charged'], p['expected_user_charged_gas'])
            remaining = 31_000_000 - 167355 - sum(r['charged'] for r in results[:-1])
            self.assertEqual(remaining, p['header_prefix_remaining_gas'][tx])
            self.assertGreaterEqual(remaining, p['transaction_gas_limit'])
            routes.update(result['counts'])
        self.assertEqual(routes, +Counter(program['expected_route_counts']))
        self.assertEqual(state[0], 1 + users * n)
        self.assertEqual(len(state), 1 + users * n)
        self.assertEqual(sum(len(r['writes']) for r in results), users * (n + 1))
        self.assertEqual(p['expected_block_charged_gas'], 167355 + sum(r['charged'] for r in results))
        self.assertLessEqual(p['expected_block_charged_gas'], 30_000_000)
        return results

    def test_prefixed_update_reuses_storage_kernel_with_real_empty_first_block(self):
        m = self.load()
        for users, n in [(1, 1), (2, 2), (3, 0)]:
            layout = {'L1': 7, 'L3': 11}
            row = m.make_prefixed_update_case(users=users, mutations_per_user=n, raw_key_layout=layout)
            old = m.make_sized_update_case(users=users, mutations_per_user=n, raw_key_layout=layout)
            program = row['request']['program']; p = row['parameters']
            self.assertEqual(program['block_count'], 2)
            self.assertEqual(program['transactions']['counts_by_block'], [0, users])
            self.assertNotIn('count_per_block', program['transactions'])
            self.assertEqual(program['transactions']['recipients'], old['request']['program']['transactions']['recipients'])
            self.assertEqual(program['prestate_contracts'], old['request']['program']['prestate_contracts'])
            self.assertEqual(program['expected_route_counts'], old['request']['program']['expected_route_counts'])
            self.assertEqual(p['all_transactions_including_anchor'], users + 2)
            self.assertEqual(p['user_transactions_by_block'], [0, users])
            self.assertEqual(p['anchor_gas_limit_upper_bound'], 1_000_000)
            self.assertEqual(p['block_charged_gas_upper_bounds'], [1_000_000, 1_000_000 + users * p['expected_user_charged_gas']])
            self.assertNotIn('expected_anchor_gas', p)
            self.assertNotIn('expected_block_charged_gas', p)
            self.assertNotEqual(row['sample_id'], old['sample_id'])
            contract = program['prestate_contracts'][0]
            state = {int(k, 16): int(v, 16) for k, v in contract['initial_storage'].items()}
            code = bytes.fromhex(contract['code_hex'][2:]); routes = Counter()
            for j in range(users):
                result = execute(code, state, p['transaction_gas_limit'])
                self.assertEqual(result['terminal'], 0)
                self.assertEqual(result['reads'][0], (0, 1 + j * n, True))
                self.assertEqual(result['refund'], 0)
                routes.update(result['counts'])
            self.assertEqual(state[0], 1 + users * n)
            self.assertEqual(routes, +Counter(program['expected_route_counts']))

    def test_sized_update_strict_shape_and_compact_layout(self):
        m = self.load()
        for users, n in [(True, 1), (0, 1), (65, 1), (1.0, 1), (1, True), (1, -1), (1, 65), (1, 1.0)]:
            with self.subTest(users=users, n=n), self.assertRaises(ValueError):
                m.make_sized_update_case(users=users, mutations_per_user=n)
        for layout in [True, [], [0]*4096, {}, {'L1':0}, {'L1':0,'L3':0,'keys':[]},
                       {'L1':True,'L3':0}, {'L1':0,'L3':1.0}, {'L1':-1,'L3':0}, {'L1':0,'L3':1<<64}]:
            with self.subTest(layout=str(layout)[:80]), self.assertRaises(ValueError):
                m.make_sized_update_case(users=1, mutations_per_user=1, raw_key_layout=layout)
        row = m.make_sized_update_case(users=3, mutations_per_user=2, raw_key_layout={'L1':7,'L3':11})
        program = row['request']['program']
        self.assertNotIn('storage_state', program)
        self.assertEqual(program['code_hex'], '0x00')
        self.assertEqual(program['transactions']['count_per_block'], 3)
        self.assertEqual(len(set(program['transactions']['recipients'])), 1)
        self.assertEqual(len(program['prestate_contracts']), 1)
        storage = program['prestate_contracts'][0]['initial_storage']
        self.assertEqual(storage, {hex(k):'0x1' for k in [0]+[(i<<48)|(7<<64)|(11<<192) for i in range(1,7)]})

    def test_sized_actual_compact_bytecode_executes_exact_updates(self):
        m = self.load()
        row = m.make_sized_update_case(users=2, mutations_per_user=3, raw_key_layout={'L1':7,'L3':11})
        contract = row['request']['program']['prestate_contracts'][0]
        code = bytes.fromhex(contract['code_hex'][2:])
        state = {int(k,16):int(v,16) for k,v in contract['initial_storage'].items()}
        for tx in range(2):
            result = execute(code,state,row['parameters']['transaction_gas_limit'])
            self.assertEqual(result['terminal'],0)
            expected = [(i<<48)|(7<<64)|(11<<192) for i in range(1+3*tx,4+3*tx)]
            self.assertEqual([key for key,*_ in result['writes']],expected+[0])
            self.assertTrue(all(state[k]==2 for k in expected))
            self.assertEqual(state[0],1+3*(tx+1))
            self.assertEqual(result['refund'],0)
            self.assertEqual(result['stack'],[])

    def test_tiny_sequential_and_u64_layouts_match_independent_complete_trajectory(self):
        m = self.load()
        for users in [1, 2, 3]:
            for n in [0, 1, 2, 3]:
                for layout in [None, {'L1': 0, 'L3': 0}, {'L1': (1 << 64) - 1, 'L3': (1 << 64) - 1}]:
                    with self.subTest(users=users, n=n, layout=layout):
                        row = m.make_sized_update_case(users=users, mutations_per_user=n, raw_key_layout=layout)
                        self.qualify_sized(row, users, n, layout)

    def test_full_64_by_64_pure_vm_has_unique_keys_and_admission_prefixes(self):
        m = self.load(); layout = {'L1': 1, 'L3': 2}
        row = m.make_sized_update_case(users=64, mutations_per_user=64, raw_key_layout=layout)
        self.qualify_sized(row, 64, 64, layout)
        p = row['parameters']
        self.assertEqual(p['expected_user_gross_gas'], 363551)
        self.assertEqual(p['transaction_gas_limit'], 364551)
        self.assertEqual(p['expected_block_charged_gas'], 23434619)
        self.assertEqual(p['expected_final_phase'], 4097)
        self.assertEqual(p['expected_distinct_storage_keys'], 4097)
        plain = m.make_sized_update_case(users=64, mutations_per_user=64)
        self.assertEqual(p['expected_user_gross_gas'] - plain['parameters']['expected_user_gross_gas'], 36 * 64)

    def test_sized_identity_binds_shape_order_layout_and_envelope(self):
        m = self.load(); layout = {'L1': 1, 'L3': 2}
        row = m.make_sized_update_case(users=2, mutations_per_user=3, raw_key_layout=layout)
        again = m.make_sized_update_case(users=2, mutations_per_user=3, raw_key_layout={'L3': 2, 'L1': 1})
        self.assertEqual(row, again)
        payload = dict(parameters=row['parameters'], ordered_body_raw_keys=[f'0x{(i << 48) | (1 << 64) | (2 << 192):064x}' for i in range(1, 7)], request=row['request'])
        expected = hashlib.sha256((json.dumps(payload, indent=2, sort_keys=True) + '\n').encode()).hexdigest()
        self.assertEqual(expected, row['identity_sha256'])
        layout['L1'] = 99
        self.assertEqual(row['parameters']['raw_key_layout']['L1'], 1)
        variants = [m.make_sized_update_case(users=1, mutations_per_user=6, raw_key_layout={'L1':1,'L3':2}),
                    m.make_sized_update_case(users=2, mutations_per_user=3, raw_key_layout={'L1':2,'L3':1}),
                    m.make_sized_update_case(users=2, mutations_per_user=3)]
        self.assertTrue(all(v['identity_sha256'] != expected for v in variants))

    def test_sized_negative_phase_initial_value_transform_and_failure_jump(self):
        m = self.load(); layout = {'L1': 1, 'L3': 2}
        row = m.make_sized_update_case(users=2, mutations_per_user=2, raw_key_layout=layout)
        contract = row['request']['program']['prestate_contracts'][0]
        code = bytes.fromhex(contract['code_hex'][2:]); envelope = row['parameters']['transaction_gas_limit']
        initial = {int(k,16):int(v,16) for k,v in contract['initial_storage'].items()}
        for wrong_phase in [0, 5, 1 << 64]:
            state = dict(initial); state[0] = wrong_phase
            self.assertEqual(execute(code, state, envelope)['terminal'], 0xfe)
        wrong_initial = copy.deepcopy(row)
        first = hex((1 << 48) | (1 << 64) | (2 << 192))
        wrong_initial['request']['program']['prestate_contracts'][0]['initial_storage'][first] = '0x2'
        with self.assertRaises(AssertionError): self.qualify_sized(wrong_initial, 2, 2, layout)
        state = dict(initial); state[int(first,16)] = 2
        self.assertEqual(execute(code, state, envelope)['terminal'], 0xfe)
        # Every materialization must agree. Break only the second PUSH32 (the write),
        # leaving both body reads untouched; a different write key cannot qualify.
        pushes = [at for at, op, _ in decode(code) if op == 0x7f]
        self.assertEqual(len(pushes), 3)
        wrong = bytearray(code); wrong[pushes[1] + 1] ^= 1
        # The malformed write creates an absent slot and costs more; a generous
        # test-only envelope lets the independent checksum guard run to failure.
        self.assertEqual(execute(wrong, dict(initial), 15_000_000)['terminal'], 0xfe)
        # A wrong but initialized start can STOP on TX1; the literal specification
        # rejects the skipped cohort even though the local checksum remains correct.
        shifted = dict(initial); shifted[0] = 3
        result = execute(code, shifted, envelope)
        self.assertEqual(result['terminal'], 0)
        self.assertNotEqual(result['reads'][0], (0, 1, True))
        # Reusing the same cohort across TXs hits the original-value guard, while
        # ordinary execution leaves persistent2 and advances ordinal phase.
        reused = dict(initial); self.assertEqual(execute(code,reused,envelope)['terminal'],0)
        reused[0] = 1
        self.assertEqual(execute(code,reused,envelope)['terminal'],0xfe)
        broken_jump = bytearray(code)
        failure = next(at for at, op, _ in decode(code) if op == 0x5b and code[at+1] == 0xfe)
        at = next(at for at, op, value in decode(code) if op == 0x61 and value == failure)
        broken_jump[at+1:at+3] = b'\x00\x00'
        state = dict(initial); state[0] = 0
        with self.assertRaisesRegex(AssertionError, 'invalid jump destination'):
            execute(broken_jump,state,envelope)

    def test_legacy_complete_rows_and_requests_keep_parent_byte_identities(self):
        m = self.load()
        import storage_fanout as fanout
        payloads = []
        for mode in MODES:
            for size in SIZES:
                for level in LEVELS:
                    payloads.append(m.make_case(mode, size, level))
        for accounts in fanout.ACCOUNT_COUNTS:
            for size in fanout.STATE_SIZES:
                for mode in fanout.MODES:
                    for level in fanout.LEVELS:
                        payloads.append(fanout.make_case(accounts, size, mode, level))
        self.assertEqual(len(payloads), 48)
        digest = hashlib.sha256()
        for row in payloads:
            for value in [row, row['request']]:
                encoded = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
                digest.update(hashlib.sha256(encoded).digest())
        # Frozen from all actual parent mutation/fanout module outputs before
        # adding this opt-in. Complete rows include the unchanged request bytes.
        self.assertEqual(digest.hexdigest(), '8158c31cbe089b5b52c82e23d02348bb100b99a4f406d9b530a382817d44ff3d')




class PrefixedStorageReadTests(unittest.TestCase):
    load = StorageMutationTests.load

    def test_literal_readonly_body_phase_only_writes_and_exact_gas(self):
        m = self.load()
        for users, n in [(1, 0), (1, 1), (2, 3), (2, 128)]:
            for layout in [None, {'L1': 0, 'L3': 0}, {'L1': 7, 'L3': (1 << 64) - 1}]:
                with self.subTest(users=users, n=n, layout=layout):
                    row = m.make_prefixed_read_case(users=users, reads_per_user=n, raw_key_layout=layout)
                    p = row['parameters']; program = row['request']['program']
                    self.assertEqual(program['block_count'], 2)
                    self.assertEqual(program['transactions']['counts_by_block'], [0, users])
                    self.assertNotIn('count_per_block', program['transactions'])
                    self.assertNotIn('storage_state', program)
                    self.assertEqual((program['code_hex'], program['input_hex']), ('0x00', '0x'))
                    contract, = program['prestate_contracts']
                    self.assertEqual(program['transactions']['recipients'], [contract['address']] * users)
                    raw = lambda i: i if layout is None else (i << 48) | (layout['L1'] << 64) | (layout['L3'] << 192)
                    keys = [raw(i) for i in range(1, users * n + 1)]
                    state = {int(k, 16): int(v, 16) for k, v in contract['initial_storage'].items()}
                    self.assertEqual(state, dict.fromkeys([0] + keys, 1))
                    code = bytes.fromhex(contract['code_hex'][2:]); counts = Counter()
                    self.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'])
                    self.assertEqual(len(code), p['runtime_bytes'])
                    for tx in range(users):
                        start = 1 + tx * n; final = start + n
                        result = execute(code, state, p['transaction_gas_limit'])
                        self.assertEqual((result['terminal'], result['stack']), (0, []))
                        expected_reads = [(0, start, True)] + [(raw(i), 1, True) for i in range(start, final)] + [(0, final, False)]
                        self.assertEqual(result['reads'], expected_reads)
                        self.assertEqual(result['writes'], [(0, start, final, start, False)])
                        self.assertEqual((result['refund'], result['applied']), (0, 0))
                        self.assertEqual(result['gas'], p['expected_user_gross_gas'])
                        self.assertEqual(result['charged'], p['expected_user_charged_gas'])
                        self.assertEqual(int.from_bytes(result['memory'][:32], 'big'), n)
                        self.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), n)
                        self.assertEqual(int.from_bytes(result['memory'][64:96], 'big'), start)
                        if n: self.assertEqual(int.from_bytes(result['memory'][96:128], 'big'), final - 1)
                        self.assertEqual(len(result['memory']), 128 if n else 96)
                        self.assertEqual(state, dict.fromkeys(keys, 1) | {0: final})
                        counts.update(result['counts'])
                    self.assertEqual(counts, +Counter(program['expected_route_counts']))
                    self.assertEqual(p['expected_storage_writes'], users)
                    self.assertEqual(p['expected_main_storage_writes'], 0)
                    self.assertEqual(p['expected_phase_storage_writes'], users)
                    self.assertEqual(p['expected_body_storage_reads'], users * n)
                    self.assertEqual(p['expected_final_phase'], 1 + users * n)
                    self.assertNotIn('expected_anchor_gas', p)
                    self.assertNotIn('expected_block_charged_gas', p)
                    self.assertEqual(p['block_charged_gas_upper_bounds'], [1_000_000, 1_000_000 + users * p['expected_user_charged_gas']])
                    for tx, remaining in enumerate(p['header_prefix_remaining_gas']):
                        self.assertEqual(remaining, 31_000_000 - 1_000_000 - tx * p['expected_user_charged_gas'])
                        self.assertGreaterEqual(remaining, p['transaction_gas_limit'])
                    self.assertEqual(p['evaluator_model_id'], '3e335de20582fca48f84b3df1b2ecd8d8d7281de02442ab35a134e4635431d73')

    def test_readonly_strict_limits_and_layout_are_not_legacy_mode_aliases(self):
        m = self.load()
        for users, n in [(True, 1), (0, 1), (65, 1), (1.0, 1), (1, True), (1, -1), (1, 129), (1, 1.0)]:
            with self.subTest(users=users, n=n), self.assertRaises(ValueError):
                m.make_prefixed_read_case(users=users, reads_per_user=n)
        for layout in [True, [], {}, {'L1': 0}, {'L1': 0, 'L3': 0, 'keys': []}, {'L1': True, 'L3': 0}, {'L1': 0, 'L3': 1.0}, {'L1': -1, 'L3': 0}, {'L1': 0, 'L3': 1 << 64}]:
            with self.subTest(layout=layout), self.assertRaises(ValueError):
                m.make_prefixed_read_case(users=1, reads_per_user=1, raw_key_layout=layout)
        self.assertEqual(m.MODES, ('update', 'delete', 'insert', 'mixed'))
        with self.assertRaises(ValueError): m.make_case('read', 8192, 'zero')
        with self.assertRaises(ValueError): m.make_sized_update_case(users=1, mutations_per_user=128)

    def test_readonly_bad_initial_slot_and_wrong_phase_fail_literal_guards(self):
        m = self.load(); row = m.make_prefixed_read_case(users=2, reads_per_user=2)
        contract, = row['request']['program']['prestate_contracts']
        code = bytes.fromhex(contract['code_hex'][2:]); cap = row['parameters']['transaction_gas_limit']
        state = {int(k, 16): int(v, 16) for k, v in contract['initial_storage'].items()}
        state[1] = 2
        self.assertEqual(execute(code, state, cap)['terminal'], 0xfe)
        state = dict.fromkeys(range(5), 1); state[0] = 2
        # A wrong but initialized start can pass once; the next range misses initial data.
        self.assertEqual(execute(code, state, cap)['terminal'], 0)
        self.assertEqual(execute(code, state, cap)['terminal'], 0xfe)
        with self.assertRaisesRegex(AssertionError, 'underflow'): execute(bytes([0x80, 0]), {}, 30000)

    def test_fixed_128_accounting_without_constructing_formal_64_user_input(self):
        m = self.load(); gas = m.gas_accounting('read', 128, raw_key_layout={'L1': 0, 'L3': 0})
        self.assertEqual(gas['storage_read_gas'], 2200 + 128 * 2100)
        self.assertEqual(gas['storage_write_gas'], 2900)
        self.assertEqual((gas['refund_counter'], gas['refund_applied']), (0, 0))
        self.assertLess(2_000_000 + 64 * gas['charged_gas'], 30_000_000)

    def test_all_78_legacy_rows_and_requests_remain_byte_identical(self):
        m = self.load()
        sys.path.insert(0, str(CASES))
        import storage_fanout as f
        rows = {}
        for mode in m.MODES:
            for size in m.STATE_SIZES:
                for level in m.LEVELS:
                    rows[f'mutation/{mode}/{size}/{level}'] = m.make_case(mode, size, level)
        for accounts in f.ACCOUNT_COUNTS:
            for size in f.STATE_SIZES:
                for mode in f.MODES:
                    for level in f.LEVELS:
                        rows[f'fanout/{accounts}/{size}/{mode}/{level}'] = f.make_case(accounts, size, mode, level)
        for users, n in [(1, 0), (1, 1), (2, 2), (3, 3), (2, 64)]:
            for layout in [None, {'L1': 0, 'L3': 0}, {'L1': 7, 'L3': (1 << 64) - 1}]:
                for prefix in [False, True]:
                    fn = m.make_prefixed_update_case if prefix else m.make_sized_update_case
                    label = f'{"prefixed" if prefix else "sized"}/{users}/{n}/{layout}'
                    rows[label] = fn(users=users, mutations_per_user=n, raw_key_layout=layout)
        frozen = {label: {'row_sha256': hashlib.sha256(m.encoded(row)).hexdigest(),
                          'request_sha256': hashlib.sha256(m.encoded(row['request'])).hexdigest()}
                  for label, row in rows.items()}
        self.assertEqual(len(frozen), 78)
        # Captured from the unmodified parent before adding the read mode.
        self.assertEqual(hashlib.sha256(m.encoded(frozen)).hexdigest(),
                         'ad47cc4048e7456e813ed49e436caadadfdd4fab6c0d6e3bf604583588249d90')


if __name__ == '__main__': unittest.main()
