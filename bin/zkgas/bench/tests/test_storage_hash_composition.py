"""Concrete storage, memory and child-journal oracles for finite composition probes."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from ethereum_hash import keccak256

CASES = Path(__file__).resolve().parents[1] / 'cases'
MASK = (1 << 256) - 1
ADDRESS = int.from_bytes(bytes([0x66]) * 20, 'big')
ZERO_HASH = bytes.fromhex('290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563')
TAG = int.from_bytes(b'storage-hash-child-v1'.ljust(32, b'\0'), 'big')


def decode(code):
    rows = []; pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op-0x5f if 0x60 <= op <= 0x7f else 0
        assert pc+width <= len(code); rows.append((at, op, code[pc:pc+width])); pc += width
    return rows


def execute(code, *, transactions=1, blocks=1):
    state = {}; counts = Counter(); writes = Counter(); reads = Counter(); read_history = []; hashes = Counter(); calls = []; frames = []; memory_stores = Counter()
    def frame(data, depth, original):
        stack = []; memory = bytearray(); returndata = b''; pc = 0; checkpoint = state.copy()
        def grow(end):
            if len(memory) < end: memory.extend(bytes(end-len(memory)))
        for _ in range(2_000_000):
            at = pc; op = code[pc]; pc += 1; counts[op] += 1
            if 0x60 <= op <= 0x7f:
                width = op-0x5f; stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width
            elif op in (1, 2, 0x14):
                a, b = stack.pop(), stack.pop(); stack.append((a+b)&MASK if op == 1 else (a*b)&MASK if op == 2 else int(a == b))
            elif op == 0x15: stack.append(int(stack.pop() == 0))
            elif op == 0x30: stack.append(ADDRESS)
            elif op == 0x35:
                offset = stack.pop(); stack.append(int.from_bytes(data[offset:offset+32].ljust(32, b'\0'), 'big'))
            elif op == 0x36: stack.append(len(data))
            elif op == 0x3d: stack.append(len(returndata))
            elif op == 0x50: stack.pop()
            elif op == 0x51:
                offset = stack.pop(); grow(offset+32); stack.append(int.from_bytes(memory[offset:offset+32], 'big'))
            elif op == 0x52:
                offset, value = stack.pop(), stack.pop(); grow(offset+32); memory[offset:offset+32] = value.to_bytes(32, 'big'); memory_stores[depth, at, offset, value] += 1
            elif op == 0x54:
                key = stack.pop(); value = state.get(key, 0); reads[depth, at, key, value] += 1
                read_history[-1].append((depth, key, value)); stack.append(value)
            elif op == 0x55:
                key, value = stack.pop(), stack.pop(); before = state.get(key, 0); writes[depth, at, key, before, value, original.get(key, 0)] += 1
                if value: state[key] = value
                else: state.pop(key, None)
            elif op == 0x20:
                offset, length = stack.pop(), stack.pop(); grow(offset+length); image = bytes(memory[offset:offset+length]); result = keccak256(image)
                hashes[depth, at, offset, length, image, result] += 1; stack.append(int.from_bytes(result, 'big'))
            elif op in (0x56, 0x57):
                target = stack.pop(); take = op == 0x56 or stack.pop() != 0
                if take: assert code[target] == 0x5b; pc = target
            elif op == 0x5b: pass
            elif op == 0xf4:
                gas, target, start, size, out, outsize = (stack.pop() for _ in range(6)); grow(max(start+size, out+outsize)); input_data = bytes(memory[start:start+size])
                assert depth == 0 and target == ADDRESS
                terminal, output = frame(input_data, depth+1, original); status = int(terminal == 0xf3); returndata = output
                memory[out:out+min(outsize, len(output))] = output[:outsize]
                calls.append((gas, target, start, size, out, outsize, input_data, status, output, state.copy()))
                stack.append(status)
            elif op in (0xf3, 0xfd):
                offset, length = stack.pop(), stack.pop(); grow(offset+length); output = bytes(memory[offset:offset+length]); before_rollback = state.copy()
                if op == 0xfd: state.clear(); state.update(checkpoint)
                frames.append((depth, op, data, bytes(memory), before_rollback, state.copy(), checkpoint)); return op, output
            elif op in (0, 0xfe):
                frames.append((depth, op, data, bytes(memory), state.copy(), state.copy(), checkpoint)); return op, b''
            else: raise AssertionError(f'unsupported concrete opcode {op:02x}')
        raise AssertionError('composition kernel did not terminate')
    terminals = []
    for _ in range(transactions*blocks):
        read_history.append([])
        original = state.copy(); terminal, _ = frame(b'', 0, original); terminals.append(terminal)
        if terminal != 0: break
    return dict(terminals=terminals, state=state, counts=+counts, writes=writes, reads=reads, read_history=read_history, hashes=hashes, calls=calls, frames=frames, memory_stores=memory_stores)


def assert_semantics(test, result, kernel, layout, rounds, users):
    test.assertEqual(result['terminals'], [0]*users); test.assertEqual(result['state'], {})
    target_depth = int(layout != 'direct')
    expected_reads = [(0, 2, 0), (target_depth, 2, 5), (target_depth, 2, 7), (0, 2, 5), (0, 2, 0)]
    test.assertEqual(result['read_history'], [expected_reads]*users)
    for (depth, pc, key, before, value, original), count in result['writes'].items():
        test.assertEqual(key, 2); test.assertEqual(original, 0)
        test.assertIn((before, value), {(0, 5), (5, 7), (7, 11), (11, 7), (7, 5), (5, 0)})
        if depth == target_depth and (before, value) in ((7, 11), (11, 7)):
            test.assertIn(kernel, ('dirty', 'mixed'))
    for (depth, pc, offset, length, image, digest), count in result['hashes'].items():
        test.assertEqual((depth, offset, length, image, digest), (target_depth, 0, 32, bytes(32), ZERO_HASH))
    expected_hashes = users*(1+(rounds if kernel in ('hash', 'mixed') else 0))
    test.assertEqual(sum(result['hashes'].values()), expected_hashes)
    test.assertEqual(result['counts'][0x55], users*(4+(2*rounds if kernel in ('dirty', 'mixed') else 0)))
    test.assertEqual(result['counts'][0x54], users*5)
    test.assertEqual(result['counts'][1], users*(rounds//64)); test.assertEqual(result['counts'][2], users)
    test.assertEqual(result['counts'][0x51], users*(2*(rounds//64)+3))
    test.assertEqual(result['counts'][0x52], users*(rounds//64+2+(2 if layout != 'direct' else 0)))
    test.assertEqual(len(result['calls']), users if layout != 'direct' else 0)
    for gas, target, start, size, out, outsize, data, status, marker, state in result['calls']:
        test.assertEqual((gas, target, start, size, out, outsize, data), (12_000_000, ADDRESS, 256, 32, 128, 32, TAG.to_bytes(32, 'big')))
        test.assertEqual(status, int(layout == 'return')); test.assertEqual(marker, rounds.to_bytes(32, 'big')); test.assertEqual(state, {2: 5})
    for depth, terminal, data, memory, before, after, checkpoint in result['frames']:
        test.assertEqual(memory[:32], bytes(32))
        if depth == target_depth:
            test.assertEqual(int.from_bytes(memory[64:96], 'big'), rounds//64)
            test.assertEqual(int.from_bytes(memory[128:160], 'big'), rounds)
        if depth == 0:
            test.assertEqual(after, {}); test.assertEqual(int.from_bytes(memory[128:160], 'big'), rounds)
        else:
            test.assertEqual(terminal, 0xf3 if layout == 'return' else 0xfd)
            test.assertEqual((before, after, checkpoint), ({2: 5}, {2: 5}, {2: 5}))


class StorageHashCompositionTests(unittest.TestCase):
    def load(self):
        path = CASES/'storage_hash_composition.py'; self.assertTrue(path.is_file(), 'storage/hash producer is not implemented')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('storage_hash_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_45_matrix_interfaces_baselines_source_hashes_and_nooverwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(m.prepare(root), 45); catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            expected = {(k, l, n, 1, 1) for k in ('dirty', 'hash', 'mixed') for l in ('direct', 'return', 'revert') for n in (0, 2048, 32768)}
            expected |= {('mixed', l, n, t, 1) for l in ('direct', 'return', 'revert') for t, ns in ((4, (0, 8192)), (16, (0, 2048))) for n in ns}
            expected |= {('mixed', l, n, 1, 4) for l in ('direct', 'return', 'revert') for n in (0, 32768)}
            self.assertEqual({(r['parameters']['kernel'], r['parameters']['layout'], r['parameters']['rounds_per_user'], r['parameters']['transactions_per_block'], r['parameters']['block_count']) for r in rows}, expected)
            self.assertEqual(len(by), 45); self.assertEqual(len({r['request_sha256'] for r in rows}), 45)
            self.assertEqual(catalog['generator_sha256'], m.digest((CASES/'storage_hash_composition.py').read_bytes()))
            self.assertEqual(set(catalog['source_sha256']), {'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py'})
            for row in rows:
                p = row['parameters']; raw = (root/row['request_path']).read_bytes(); request = json.loads(raw)
                self.assertEqual(m.digest(raw), row['request_sha256']); self.assertNotIn('storage_state', request['program']); self.assertNotIn('storage', request['program'])
                self.assertEqual(request['program']['transactions'], {'kind': 'call', 'count_per_block': p['transactions_per_block'], 'gas_limit': 15_000_000})
                self.assertEqual(request['program']['input_hex'], '0x'); self.assertTrue(request['expected_user_success']); self.assertEqual(p['all_user_transactions'], p['transactions_per_block']*p['block_count'])
                self.assertEqual(p['all_rounds_per_block'], p['rounds_per_user']*p['transactions_per_block']); self.assertTrue(row['holdout_only'])
                baseline = by[row['baseline_sample_id']]['parameters']; self.assertEqual((baseline['kernel'], baseline['layout'], baseline['transactions_per_block'], baseline['block_count'], baseline['rounds_per_user']), (p['kernel'], p['layout'], p['transactions_per_block'], p['block_count'], 0))
            before = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(before, {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other = Path(temporary)/'metadata'; m.prepare(other, model_id='1'*64)
            self.assertEqual({p.name: p.read_bytes() for p in (root/'requests').iterdir()}, {p.name: p.read_bytes() for p in (other/'requests').iterdir()})

    def test_concrete_storage_hash_rollback_all45_and_fixed_body(self):
        m = self.load(); self.assertEqual(keccak256(bytes(32)), ZERO_HASH)
        import hashlib
        self.assertNotEqual(hashlib.sha3_256(bytes(32)).digest(), ZERO_HASH)
        selections = [(k, l, n, 1, 1) for k in ('dirty', 'hash', 'mixed') for l in ('direct', 'return', 'revert') for n in (0, 2048, 32768)]
        selections += [('mixed', l, n, t, 1) for l in ('direct', 'return', 'revert') for t, ns in ((4, (0, 8192)), (16, (0, 2048))) for n in ns]
        selections += [('mixed', l, n, 1, 4) for l in ('direct', 'return', 'revert') for n in (0, 32768)]
        for k, l, n, t, b in selections:
            row = m.make_case(k, l, n, t, b); code = bytes.fromhex(row['request']['program']['code_hex'][2:]); result = execute(code, transactions=t, blocks=b)
            assert_semantics(self, result, k, l, n, t*b)
            self.assertEqual({f'opcode:0x{op:02x}': count for op, count in result['counts'].items()}, {op: count for op, count in row['request']['program']['expected_route_counts'].items() if count})
            self.assertEqual(result['counts'][0xf4], t*b if l != 'direct' else 0); self.assertEqual(result['counts'][0xf3], t*b if l == 'return' else 0); self.assertEqual(result['counts'][0xfd], t*b if l == 'revert' else 0)
            other = m.make_case(k, l, n, 1, 1); self.assertEqual(code.hex(), other['request']['program']['code_hex'][2:])
        for k in ('dirty', 'hash', 'mixed'):
            for l in ('direct', 'return', 'revert'):
                code = [bytes.fromhex(m.make_case(k, l, n)['request']['program']['code_hex'][2:]) for n in (0, 2048, 32768)]
                self.assertEqual(len({len(c) for c in code}), 1); self.assertLessEqual(sum(a != b for a, b in zip(code[0], code[-1])), 6)

    def test_wrong_slot_hash_and_marker_or_short_loop_are_detected(self):
        m = self.load(); row = m.make_case('mixed', 'revert', 128); code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); decoded = decode(code)
        store = next(i for i, (_, op, data) in enumerate(decoded) if op == 0x62 and int.from_bytes(data, 'big') == 11); slot_pc = decoded[store+1][0]; code[slot_pc+3] = 3
        result = execute(bytes(code)); self.assertEqual(result['terminals'], [0]); self.assertEqual(result['state'], {})
        self.assertEqual(result['counts'], execute(bytes.fromhex(row['request']['program']['code_hex'][2:]))['counts'])
        with self.assertRaises(AssertionError): assert_semantics(self, result, 'mixed', 'revert', 128, 1)
        code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); candidates = [(pc, op, data) for pc, op, data in decode(code) if op == 0x20]
        bulk_hash = candidates[1][0]; code[bulk_hash-1] = 64; result = execute(bytes(code)); self.assertEqual(result['terminals'], [0])
        with self.assertRaises(AssertionError): assert_semantics(self, result, 'mixed', 'revert', 128, 1)
        code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); bound = next(pc for pc, op, data in decode(code) if op == 0x62 and int.from_bytes(data, 'big') == 2 and code[pc+4] == 0x14)
        code[bound+1:bound+4] = bytes(3); self.assertEqual(execute(bytes(code))['terminals'], [0xfe])
        code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); marker = [i for i, (_, op, data) in enumerate(decode(code)) if op == 0x62 and int.from_bytes(data, 'big') == 128 and decode(code)[i+1][1] == 0x52][-1]
        marker_pc = decode(code)[marker][0]; code[marker_pc+3] = 160; self.assertEqual(execute(bytes(code))['terminals'], [0xfe])

    def test_wrong_initial_read_key_preserves_counts_but_fails_operand_oracle(self):
        m = self.load()
        for layout in ('direct', 'return', 'revert'):
            with self.subTest(layout=layout):
                row = m.make_case('mixed', layout, 128)
                original = bytes.fromhex(row['request']['program']['code_hex'][2:])
                decoded = decode(original)
                read = next(i for i, (_, op, _) in enumerate(decoded) if op == 0x54)
                pc, op, data = decoded[read-1]
                self.assertEqual((op, int.from_bytes(data, 'big')), (0x62, 2))
                code = bytearray(original); code[pc+3] = 3
                result = execute(bytes(code))
                self.assertEqual(result['terminals'], [0]); self.assertEqual(result['state'], {})
                self.assertEqual(result['counts'], execute(original)['counts'])
                with self.assertRaises(AssertionError):
                    assert_semantics(self, result, 'mixed', layout, 128, 1)

    def test_strict_domains_keep_gas_and_do_not_silently_prune_combinations(self):
        m = self.load()
        for args in [('wrong', 'direct', 64, 1, 1), ('dirty', 'bad', 64, 1, 1), ('dirty', 'direct', True, 1, 1), ('dirty', 'direct', 1, 1, 1), ('dirty', 'direct', 32832, 1, 1), ('dirty', 'direct', 64, 0, 1), ('dirty', 'direct', 64, 17, 1), ('dirty', 'direct', 64, 1, True), ('dirty', 'direct', 64, 1, 5)]:
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        self.assertEqual(m.make_case('mixed', 'return', 32768, 16, 4)['request']['program']['transactions']['gas_limit'], 15_000_000)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('', True, None, 'A'*64, 'g'*64):
                with self.assertRaises(ValueError): m.prepare(Path(temporary)/'bad', model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'), 'opt in frozen native/quote for focused positives and negatives only')
    def test_native_small_positive_reject_and_operand_oracle_limitation(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def native(request, name):
                out = root/name; out.mkdir(); source = out/'request.json'; source.write_bytes(m.encoded(request))
                run = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(out/'input.json'), '--output-bincode', str(out/'input.bin'), '--report', str(out/'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                return run, out
            for layout in ('direct', 'return', 'revert'):
                row = m.make_case('mixed', layout, 64); result, out = native(row['request'], layout); self.assertEqual(result.returncode, 0, result.stderr.decode())
                report = json.loads((out/'native.json').read_text()); self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0])); self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in row['request']['program']['expected_route_counts'].items() if v})
                quote = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-bincode', str(out/'input.bin'), '--output', str(out/'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', 'storage-hash-smoke', '--sample-id', row['sample_id']], capture_output=True, timeout=180)
                self.assertEqual(quote.returncode, 0, quote.stderr.decode()); quoted = json.loads((out/'quote.jsonl').read_text())['quote']; self.assertEqual(quoted['model_id'], m.MODEL); self.assertEqual(quoted['public_output'], report['public_output']); self.assertEqual(quoted['guest_input_sha256'], report['guest_input_sha256']); self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'], 0)
            row = m.make_case('mixed', 'revert', 64); code = bytearray.fromhex(row['request']['program']['code_hex'][2:]); decoded = decode(code); i = next(i for i, (_, op, data) in enumerate(decoded) if op == 0x62 and int.from_bytes(data, 'big') == 11); code[decoded[i+1][0]+3] = 3
            row['request']['program']['code_hex'] = '0x'+code.hex(); result, out = native(row['request'], 'wrong-slot-still-passes-native-counts'); self.assertEqual(result.returncode, 0, result.stderr.decode())
            with self.assertRaises(AssertionError): assert_semantics(self, execute(bytes(code)), 'mixed', 'revert', 64, 1)
            for kind in ('wrong-marker', 'child-oog'):
                row = m.make_case('mixed', 'return', 64); code = bytearray.fromhex(row['request']['program']['code_hex'][2:])
                if kind == 'child-oog':
                    pc = next(pc for pc, op, data in decode(code) if op == 0x62 and int.from_bytes(data, 'big') == 12_000_000); code[pc+1:pc+4] = bytes(3)
                else:
                    decoded = decode(code); i = [i for i, (_, op, data) in enumerate(decoded) if op == 0x62 and int.from_bytes(data, 'big') == 128 and decoded[i+1][1] == 0x52][-1]; code[decoded[i][0]+3] = 160
                row['request']['program']['code_hex'] = '0x'+code.hex(); result, out = native(row['request'], kind); self.assertNotEqual(result.returncode, 0); self.assertFalse((out/'input.bin').exists()); self.assertFalse((out/'native.json').exists())


if __name__ == '__main__': unittest.main()
