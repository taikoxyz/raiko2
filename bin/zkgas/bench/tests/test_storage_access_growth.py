"""Independent concrete key, cold-access and minimal target oracles for storage growth."""
from collections import Counter
from functools import lru_cache
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from ethereum_hash import keccak256

CASES = Path(__file__).resolve().parents[1]/'cases'
MASK = (1 << 256)-1


def rlp_encode(value):
    if isinstance(value, list):
        raw = b''.join(rlp_encode(item) for item in value); prefix = 0xc0
    else:
        raw = value; prefix = 0x80
        if len(raw) == 1 and raw[0] < 0x80: return raw
    if len(raw) < 56: return bytes([prefix+len(raw)])+raw
    length = len(raw).to_bytes((len(raw).bit_length()+7)//8, 'big')
    return bytes([prefix+55+len(length)])+length+raw


def rlp_decode(raw):
    def consume(at):
        prefix = raw[at]; at += 1
        if prefix < 0x80: return bytes([prefix]), at
        is_list = prefix >= 0xc0; base = 0xc0 if is_list else 0x80
        length = prefix-base
        if length > 55:
            width = length-55; length = int.from_bytes(raw[at:at+width], 'big'); at += width
        end = at+length; assert end <= len(raw)
        if not is_list: return raw[at:end], end
        values = []
        while at < end:
            value, at = consume(at); values.append(value)
        assert at == end; return values, end
    value, end = consume(0); assert end == len(raw); return value


def nibbles(raw):
    return tuple(n for byte in raw for n in (byte >> 4, byte & 15))


@lru_cache(maxsize=2)
def independent_storage_root(size):
    """Build an Ethereum MPT independently from raw integer preimages and value1."""
    leaves = [(nibbles(keccak256(key.to_bytes(32, 'big'))), b'\x01') for key in range(size)]
    def compact(path, leaf):
        fields = ((2*leaf+1,)+path) if len(path)%2 else ((2*leaf, 0)+path)
        return bytes((fields[i] << 4)|fields[i+1] for i in range(0, len(fields), 2))
    def node(items):
        if len(items) == 1: return [compact(items[0][0], True), items[0][1]]
        paths = [path for path, _ in items]; prefix = 0
        while prefix < min(map(len, paths)) and len({path[prefix] for path in paths}) == 1: prefix += 1
        def reference(child):
            raw = rlp_encode(child); return child if len(raw) < 32 else keccak256(raw)
        if prefix: return [compact(paths[0][:prefix], False), reference(node([(path[prefix:], value) for path, value in items]))]
        branches = [[] for _ in range(16)]; result = [b'']*17
        for path, value in items:
            if path: branches[path[0]].append((path[1:], value))
            else: result[16] = value
        for index, children in enumerate(branches):
            if children: result[index] = reference(node(children))
        return result
    return keccak256(rlp_encode(node(leaves)))


def trie_value(root, key, nodes):
    """Authenticate one account or slot through actual serialized witness nodes."""
    path = nibbles(key); reference = root
    while reference:
        node = reference if isinstance(reference, list) else rlp_decode(nodes[reference])
        if len(node) == 17:
            if not path: return node[16]
            reference = node[path[0]]; path = path[1:]; continue
        assert len(node) == 2
        fields = nibbles(node[0]); flag = fields[0]; segment = fields[1:] if flag & 1 else fields[2:]
        if path[:len(segment)] != segment: return b''
        path = path[len(segment):]
        if flag & 2: return node[1] if not path else b''
        reference = node[1]
    return b''


def decode(code):
    rows = []; pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op-0x5f if 0x60 <= op <= 0x7f else 0
        assert pc+width <= len(code); rows.append((at, op, code[pc:pc+width])); pc += width
    return rows


def execute(code, initial):
    """Interpret actual operands rather than reproducing producer count formulae."""
    state = {int(k, 16): int(v, 16) for k, v in initial.items()}
    before = state.copy(); stack = []; memory = bytearray(64); pc = 0; counts = Counter(); reads = []; warm = set()
    for _ in range(4_000_000):
        at = pc; op = code[pc]; pc += 1; counts[op] += 1
        if 0x60 <= op <= 0x7f:
            width = op-0x5f; stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width
        elif op in (1, 2, 0x14):
            a, b = stack.pop(), stack.pop(); stack.append((a+b)&MASK if op == 1 else (a*b)&MASK if op == 2 else int(a == b))
        elif op == 0x15: stack.append(int(stack.pop() == 0))
        elif op == 0x51:
            offset = stack.pop(); assert offset in (0, 32); stack.append(int.from_bytes(memory[offset:offset+32], 'big'))
        elif op == 0x52:
            offset, value = stack.pop(), stack.pop(); assert offset in (0, 32); memory[offset:offset+32] = value.to_bytes(32, 'big')
        elif op == 0x54:
            key = stack.pop(); value = state.get(key, 0); reads.append((at, key, value, key not in warm)); warm.add(key); stack.append(value)
        elif op in (0x56, 0x57):
            target = stack.pop(); take = op == 0x56 or stack.pop() != 0
            if take: assert code[target] == 0x5b; pc = target
        elif op == 0x5b: pass
        elif op in (0, 0xfe):
            return dict(terminal=op, counts=+counts, reads=reads, memory=bytes(memory), state=state, before=before, stack=stack)
        else: raise AssertionError(f'unsupported storage-growth opcode {op:02x}')
    raise AssertionError('storage-growth kernel did not terminate')


def assert_semantics(test, request, result, mode, state_size, reads):
    test.assertEqual(result['terminal'], 0); test.assertEqual(result['stack'], [])
    test.assertEqual(result['state'], result['before']); test.assertEqual(result['state'], dict.fromkeys(range(state_size), 1))
    keys = [0]+([0]*reads if mode == 'warm' else list(range(1, reads+1)) if mode == 'present' else list(range(1048577, 1048577+reads)))
    values = [1]+([0]*reads if mode == 'absent' else [1]*reads)
    test.assertEqual([(key, value) for _, key, value, _ in result['reads']], list(zip(keys, values)))
    test.assertEqual([cold for _, _, _, cold in result['reads']], [True]+([False]*reads if mode == 'warm' else [True]*reads))
    test.assertEqual(int.from_bytes(result['memory'][:32], 'big'), reads)
    test.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), 0 if mode == 'absent' else reads)
    targets = request['program']['storage_state']; test.assertEqual(targets['proof_slots'], [hex(key) for key in dict.fromkeys(keys)])
    test.assertEqual(targets['insert_slots'], []); test.assertEqual(targets['delete_slots'], [])
    test.assertEqual({f'opcode:0x{op:02x}': count for op, count in result['counts'].items()}, {op: count for op, count in request['program']['expected_route_counts'].items() if count})


class StorageAccessGrowthTests(unittest.TestCase):
    def load(self):
        path = CASES/'storage_access_growth.py'; self.assertTrue(path.is_file(), 'storage-access producer is missing')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('storage_access_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_independent_ethereum_mpt_oracle_primitives(self):
        self.assertEqual(keccak256(rlp_encode(b'')).hex(), '56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421')
        self.assertEqual(keccak256(bytes(32)).hex(), '290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563')
        for value in (b'', b'a', bytes(100), [b'one', [bytes(70), b'two']]): self.assertEqual(rlp_decode(rlp_encode(value)), value)
        self.assertEqual(independent_storage_root(8192).hex(), '43eaf2855d59283c61cca65195451e0ee6900dc356bf9170221bb2d03c74c4c1')
        self.assertEqual(independent_storage_root(16384).hex(), '96709f08191abe793e35e20f54302be1962e307a094a835550b9a27ff2526bb7')

    def test_matrix_baselines_hash_bindings_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(m.prepare(root), 26)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            expected = {(mode, size, n) for mode in ('warm', 'present', 'absent') for size in (8192, 16384) for n in (0, 64, 1024, 4096)} | {('warm', size, 65536) for size in (8192, 16384)}
            self.assertEqual({(r['parameters']['mode'], r['parameters']['state_size'], r['parameters']['reads']) for r in rows}, expected)
            self.assertEqual(len(by), 26); self.assertEqual(len({r['request_sha256'] for r in rows}), 26); self.assertEqual(catalog['frozen_model_id'], m.MODEL)
            self.assertEqual(catalog['generator_sha256'], m.digest((CASES/'storage_access_growth.py').read_bytes()))
            for name, digest in catalog['source_sha256'].items(): self.assertEqual(m.digest((CASES/name).read_bytes()), digest)
            for row in rows:
                p = row['parameters']; raw = (root/row['request_path']).read_bytes(); request = json.loads(raw); program = request['program']; storage = program['storage_state']
                self.assertEqual(m.digest(raw), row['request_sha256']); self.assertNotIn('transactions', program)
                self.assertEqual(program['block_count'], 1); self.assertEqual(program['input_hex'], '0x'); self.assertTrue(request['expected_user_success'])
                self.assertEqual(storage['initial_storage'], {hex(k): '0x1' for k in range(p['state_size'])})
                self.assertEqual(p['transaction_gas_limit'], 15_000_000); self.assertIn('EIP-1559', p['transaction_envelope']); self.assertEqual(p['execution_profile'], 'evm-gas-only')
                self.assertEqual(p['all_reads_per_proposal'], p['reads']+1); self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                base = by[row['baseline_sample_id']]['parameters']; self.assertEqual((base['mode'], base['state_size'], base['reads']), (p['mode'], p['state_size'], 0))
                other = by[row['state_size_reference_sample_id']]['parameters']; self.assertEqual((other['mode'], other['reads'], other['state_size']), (p['mode'], p['reads'], 8192))
            before = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(before, {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other = Path(temporary)/'metadata'; m.prepare(other, model_id='1'*64)
            self.assertEqual({p.name: p.read_bytes() for p in (root/'requests').iterdir()}, {p.name: p.read_bytes() for p in (other/'requests').iterdir()})

    def test_all26_concrete_keys_values_order_coldness_counts_and_fixed_body(self):
        m = self.load(); images = []
        for mode in ('warm', 'present', 'absent'):
            for size in (8192, 16384):
                for n in (0, 64, 1024, 4096)+( (65536,) if mode == 'warm' else ()):
                    row = m.make_case(mode, size, n); request = row['request']; code = bytes.fromhex(request['program']['code_hex'][2:]); images.append(code)
                    result = execute(code, request['program']['storage_state']['initial_storage']); assert_semantics(self, request, result, mode, size, n)
                    self.assertEqual(sum(op == 0x54 for _, op, _ in decode(code)), 65); self.assertEqual(result['counts'][0x54], n+1)
                    self.assertEqual(sum(cold for _, _, _, cold in result['reads']), 1 if mode == 'warm' else n+1)
        layouts = [[(pc, op) for pc, op, _ in decode(code)] for code in images]
        self.assertTrue(all(layout == layouts[0] for layout in layouts)); self.assertEqual(len({len(code) for code in images}), 1)
        for code in images: self.assertFalse({0x55, 0xf1, 0xf2, 0xf4, 0xfa} & {op for _, op, _ in decode(code)})

    def test_checksum_preserving_wrong_base_stride_and_missing_target_are_caught(self):
        m = self.load(); row = m.make_case('present', 8192, 128); request = row['request']; original = bytes.fromhex(request['program']['code_hex'][2:])
        # base1→2 and stride1→2 keep every value1, every instruction and success.
        rows = decode(original)
        for operand in ('base', 'stride'):
            code = bytearray(original); changed = 0
            for i, (pc, op, data) in enumerate(rows):
                if op == 0x62 and int.from_bytes(data, 'big') == 1 and rows[i+1][1] == (1 if operand == 'base' else 2):
                    # The counter increment also adds1. Bulk base immediately follows MUL.
                    if operand == 'base' and rows[i-1][1] != 2: continue
                    code[pc+1:pc+4] = (2).to_bytes(3, 'big'); changed += 1
            self.assertEqual(changed, 64); result = execute(bytes(code), request['program']['storage_state']['initial_storage'])
            self.assertEqual(result['terminal'], 0); self.assertEqual(result['counts'], execute(original, request['program']['storage_state']['initial_storage'])['counts'])
            self.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), 128)
            with self.assertRaises(AssertionError): assert_semantics(self, request, result, 'present', 8192, 128)
        broken = copy.deepcopy(request); broken['program']['storage_state']['proof_slots'].pop()
        with self.assertRaises(AssertionError): assert_semantics(self, broken, execute(original, request['program']['storage_state']['initial_storage']), 'present', 8192, 128)
        code = bytearray(original); rows = decode(code); pc = next(pc for pc, op, data in rows if op == 0x62 and int.from_bytes(data, 'big') == 128)
        code[pc+1:pc+4] = (64).to_bytes(3, 'big'); self.assertEqual(execute(bytes(code), request['program']['storage_state']['initial_storage'])['terminal'], 0xfe)

    def test_strict_domains(self):
        m = self.load()
        for args in [('bad', 8192, 64), (True, 8192, 64), ('warm', True, 64), ('warm', 8193, 64), ('warm', 8192, True), ('warm', 8192, -64), ('warm', 8192, 1), ('warm', 8192, 65600), ('present', 8192, 4160), ('absent', 16384, 65536)]:
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('', True, None, 'A'*64, 'g'*64):
                with self.assertRaises(ValueError): m.prepare(Path(temporary)/'bad', model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'), 'opt in frozen native/quote smoke')
    def test_native_modes_missing_target_guard_and_checksum_preserving_wrong_key(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def native(request, name):
                out = root/name; out.mkdir(); source = out/'request.json'; source.write_bytes(m.encoded(request))
                run = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(out/'input.json'), '--output-bincode', str(out/'input.bin'), '--report', str(out/'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                return run, out
            for mode in ('warm', 'present', 'absent'):
                row = m.make_case(mode, 8192, 64); result, out = native(row['request'], mode); self.assertEqual(result.returncode, 0, result.stderr.decode())
                report = json.loads((out/'native.json').read_text()); self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0])); self.assertEqual(report['execution_profile'], 'evm-gas-only')
                for op, count in row['request']['program']['expected_route_counts'].items(): self.assertEqual(report['observed_route_counts'].get(op, 0), count)
                storage = report['storage_state']; self.assertEqual(storage['initial_storage_entry_count'], 8192); self.assertEqual(storage['proof_slot_count'], 1 if mode == 'warm' else 65); self.assertEqual(storage['insert_slot_count']+storage['delete_slot_count'], 0)
                self.assertLess(storage['minimized_witness_node_count'], storage['full_witness_node_count'])
                quote = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-bincode', str(out/'input.bin'), '--output', str(out/'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', 'storage-access-smoke', '--sample-id', row['sample_id']], capture_output=True, timeout=180)
                self.assertEqual(quote.returncode, 0, quote.stderr.decode()); quote = json.loads((out/'quote.jsonl').read_text())['quote']; self.assertEqual(quote['model_id'], m.MODEL)
                local = quote['blocks'][0]['node']['pricing']['evm_local_resources']
                self.assertEqual(local['unique_storage_slot_access_count'], 5+(0 if mode == 'warm' else 64))
                self.assertEqual(local['unique_storage_slot_write_count'], 4)  # Ordinary Anchor only.
                guest = json.loads((out/'input.json').read_text()); raw_nodes = [bytes.fromhex(node[2:]) for node in guest['proposal_state_nodes']]
                nodes = {keccak256(node): node for node in raw_nodes}
                parent = bytes.fromhex(guest['proposal_ancestor_headers'][-1]['header']['stateRoot'][2:])
                account = rlp_decode(trie_value(parent, keccak256(bytes([0x66])*20), nodes))
                self.assertEqual(account[2], independent_storage_root(8192))
                expected = [0]+(list(range(1, 65)) if mode == 'present' else list(range(1048577, 1048641)) if mode == 'absent' else [])
                for key in expected:
                    value = trie_value(account[2], keccak256(key.to_bytes(32, 'big')), nodes)
                    self.assertEqual(value, b'' if mode == 'absent' and key else b'\x01')
            row = m.make_case('present', 8192, 64); request = row['request']; original = bytes.fromhex(request['program']['code_hex'][2:]); code = bytearray(original); decoded = decode(original)
            for i, (pc, op, data) in enumerate(decoded):
                if op == 0x62 and int.from_bytes(data, 'big') == 1 and decoded[i-1][1] == 2 and decoded[i+1][1] == 1: code[pc+1:pc+4] = bytes(3)
            request['program']['code_hex'] = '0x'+code.hex(); result, out = native(request, 'wrong-base-still-passes'); self.assertEqual(result.returncode, 0, result.stderr.decode())
            with self.assertRaises(AssertionError): assert_semantics(self, request, execute(bytes(code), request['program']['storage_state']['initial_storage']), 'present', 8192, 64)
            for negative in ('missing-target', 'wrong-warmup'):
                request = m.make_case('present', 8192, 64)['request']
                if negative == 'missing-target': request['program']['storage_state']['proof_slots'].pop()
                else: request['program']['storage_state']['initial_storage']['0x0'] = '0x2'
                result, out = native(request, negative); self.assertNotEqual(result.returncode, 0); self.assertFalse((out/'input.bin').exists()); self.assertFalse((out/'native.json').exists())


if __name__ == '__main__': unittest.main()
