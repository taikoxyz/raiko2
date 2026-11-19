"""Concrete semantic and gas qualification for predeployed multi-account code."""
from collections import Counter
from contextlib import nullcontext
import copy
import hashlib
import importlib.util
import json
import os
import subprocess
import tempfile
from pathlib import Path
import sys
import unittest
from ethereum_hash import keccak256
from test_storage_access_growth import rlp_decode, trie_value

CASES = Path(__file__).resolve().parents[1] / 'cases'


class MultiaccountCodeTests(unittest.TestCase):
    def load(self):
        path = CASES / 'multiaccount_code.py'
        self.assertTrue(path.is_file(), 'multi-account producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('multiaccount_cases', path)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        return m

    def test_full_gas_and_own_zero(self):
        m = self.load()
        for targets in (16, 128, 512):
            full = m.make_case(targets, 32, 'unique', 'full')
            zero = m.make_case(targets, 32, 'unique', 'zero')
            self.assertEqual(full['baseline_sample_id'], zero['sample_id'])
            self.assertEqual(full['request']['program']['transactions'], zero['request']['program']['transactions'])
            self.assertGreaterEqual(full['parameters']['expected_block_charged_gas'], 29_700_000)
            self.assertLessEqual(full['parameters']['expected_block_charged_gas'], 30_000_000)
            self.assertGreater(full['parameters']['calls_per_user'], targets)




# Independent concrete EVM subset. No declared gas/count metadata is consumed.
BASE = 0x2000000000000000000000000000000000010000
PARENT = int('66' * 20, 16)
PREFIX = bytes.fromhex('3060005260206000f3')
COST = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x16: 3, 0x30: 2, 0x3d: 2,
        0x50: 2, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5a: 2,
        0x5b: 1, 0xf3: 0}


def instructions(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        data = code[pc:pc + width]; pc += width
        assert len(data) == width
        yield at, op, data


def run(code, contracts, targets, calls, *, current_address=0, child=False, return_override=None):
    """Bounded execution with per-call assertions; no enormous event-list diffs."""
    pc = 0; stack = []; memory = bytearray(); counts = Counter(); gas = 0
    warm = set(); call_count = 0; cold = 0; returned = b''; ending = None
    def extend(end):
        nonlocal gas
        words = (end + 31) // 32; old = len(memory) // 32
        if words > old:
            gas += 3 * (words - old) + words * words // 512 - old * old // 512
            memory.extend(bytes((words - old) * 32))
    for _ in range(4_000_000):
        assert 0 <= pc < len(code), 'PC outside runtime'
        op = code[pc]; pc += 1; counts[op] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif 0x80 <= op <= 0x8f: stack.append(stack[-(op - 0x7f)]); gas += 3
        elif op in (0xf1, 0xf4):
            requested, target = stack.pop(), stack.pop()
            if op == 0xf1: assert stack.pop() == 0, 'CALL value must be zero'
            in_at, in_size, out_at, out_size = (stack.pop() for _ in range(4))
            assert (in_at, in_size, out_at, out_size) == (0, 0, 32, 32)
            assert requested == 10_000_000, 'GAS operand must come from GAS opcode'
            assert target == BASE + call_count % targets, f'target order at call{call_count}'
            assert stack == [call_count], f'body counter at call{call_count}'
            assert target in contracts, 'missing target'
            was_cold = target not in warm; cold += was_cold; warm.add(target)
            gas += 2600 if was_cold else 100; extend(out_at + out_size)
            context = target if op == 0xf1 else current_address
            result = run(contracts[target], {}, 0, 0, current_address=context, child=True)
            assert result['terminal'] == 0xf3 and result['output'] == context.to_bytes(32, 'big')
            assert result['counts'] == Counter({0x30: 1, 0x60: 3, 0x52: 1, 0xf3: 1})
            assert result['gas'] == 17 and result['end_pc'] == 9
            counts.update(result['counts']); gas += result['gas']
            returned = result['output'] if return_override is None else return_override
            memory[out_at:out_at + min(out_size, len(returned))] = returned[:out_size]
            stack.append(1); call_count += 1
        else:
            assert op in COST, f'unsupported or exceptional opcode{op:02x}'
            gas += COST[op]
            if op == 1: stack.append((stack.pop() + stack.pop()) % (1 << 256))
            elif op in (0x14, 0x16):
                a, b = stack.pop(), stack.pop(); stack.append(int(a == b) if op == 0x14 else a & b)
            elif op == 0x15: stack.append(int(stack.pop() == 0))
            elif op == 0x30: stack.append(current_address)
            elif op == 0x3d: stack.append(len(returned))
            elif op == 0x50: ending = stack.pop()
            elif op in (0x51, 0x52):
                at = stack.pop(); assert at in (0, 32, 64); extend(at + 32)
                if op == 0x51: stack.append(int.from_bytes(memory[at:at + 32], 'big'))
                else: memory[at:at + 32] = stack.pop().to_bytes(32, 'big')
            elif op in (0x56, 0x57):
                at = stack.pop(); take = op == 0x56 or stack.pop() != 0
                if take: assert code[at] == 0x5b; pc = at
            elif op == 0x5a: stack.append(10_000_000)
            elif op == 0x5b: pass
            elif op in (0, 0xf3):
                output = b''
                if op == 0xf3:
                    at, size = stack.pop(), stack.pop(); extend(at + size); output = bytes(memory[at:at + size])
                assert stack == [], 'terminal stack'
                if not child:
                    assert ending == calls and call_count == calls, 'ending counter/call bound'
                    assert cold == min(calls, targets), 'cold first visits'
                return {'terminal': op, 'output': output, 'counts': counts, 'gas': gas,
                        'calls': call_count, 'cold': cold, 'warm': call_count - cold,
                        'end_pc': pc, 'ending_counter': ending}
    raise AssertionError('concrete step bound exceeded')


def concrete(row, *, return_override=None):
    p = row['parameters']; program = row['request']['program']
    contracts = {int(c['address'], 16): bytes.fromhex(c['code_hex'][2:]) for c in program['prestate_contracts']}
    assert len(contracts) == p['target_count'], 'configured target count'
    for i, (at, code) in enumerate(contracts.items()):
        assert at == BASE + i, 'configured target order'
        assert len(code) == p['runtime_bytes_per_target'], 'configured runtime length'
        assert code[:9] == PREFIX and not any(code[9:-4]), 'executed prefix/unreachable tail'
        assert code[-4:] == (i + 1 if p['code_variant'] == 'unique' else 0).to_bytes(4, 'big'), 'unreachable salt identity'
    return run(bytes.fromhex(program['code_hex'][2:]), contracts, p['target_count'], p['calls_per_user'], current_address=PARENT, return_override=return_override)


def order_mutant(row):
    bad = copy.deepcopy(row); code = bytearray.fromhex(bad['request']['program']['code_hex'][2:]); code[3] = 1
    for pc, op, data in instructions(code):
        if op == 0x62 and int.from_bytes(data, 'big') == row['parameters']['calls_per_user']:
            code[pc + 1:pc + 4] = (row['parameters']['calls_per_user'] + 1).to_bytes(3, 'big')
    bad['request']['program']['code_hex'] = '0x' + code.hex()
    return bad


def salt_mutant(row):
    bad = copy.deepcopy(row); contracts = bad['request']['program']['prestate_contracts']
    for c in contracts: c['code_hex'] = contracts[0]['code_hex']
    return bad


def short_return_mutant(row):
    bad = copy.deepcopy(row)
    for c in bad['request']['program']['prestate_contracts']:
        code = bytearray.fromhex(c['code_hex'][2:]); code[5] = 31; c['code_hex'] = '0x' + code.hex()
    return bad


class MultiaccountQualificationTests(MultiaccountCodeTests):
    def test_catalog_matrix_bytes_own_controls_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'cases'; self.assertEqual(m.prepare(root), 36)
            catalog = json.loads((root / 'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 36); self.assertEqual(len({r['request_sha256'] for r in rows}), 36)
            self.assertEqual(catalog['frozen_model_id'], 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef')
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'multiaccount_code.py').read_bytes()).hexdigest())
            matrix = set(); controls = 0
            for row in rows:
                p = row['parameters']; matrix.add((p['target_count'], p['runtime_bytes_per_target'], p['code_variant'], p['workload_level']))
                raw = (root / row['request_path']).read_bytes(); req = json.loads(raw)
                self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                program = req['program']; contracts = program['prestate_contracts']
                self.assertEqual((program['block_count'], program['input_hex']), (1, '0x'))
                self.assertEqual(program['transactions'], {'kind': 'call', 'count_per_block': 4, 'gas_limit': p['transaction_gas_limit']})
                self.assertTrue(req['expected_user_success']); self.assertTrue(row['holdout_only'])
                self.assertEqual(len(contracts), p['target_count']); self.assertEqual(len({c['address'] for c in contracts}), p['target_count'])
                hashes = set()
                for i, c in enumerate(contracts):
                    self.assertEqual(c['address'], f'0x{BASE + i:040x}')
                    code = bytes.fromhex(c['code_hex'][2:]); self.assertEqual(len(code), p['runtime_bytes_per_target'])
                    self.assertEqual(code[:9], PREFIX); self.assertFalse(any(code[9:-4]))
                    self.assertEqual(code[-4:], (i + 1 if p['code_variant'] == 'unique' else 0).to_bytes(4, 'big'))
                    hashes.add(hashlib.sha256(code).digest())
                self.assertEqual(len(hashes), p['target_count'] if p['code_variant'] == 'unique' else 1)
                baseline = by[row['baseline_sample_id']]; self.assertEqual(baseline['parameters']['calls_per_user'], 0)
                self.assertEqual(baseline['parameters']['transaction_gas_limit'], p['transaction_gas_limit'])
                self.assertEqual(baseline['parameters']['expected_witnessed_target_code_count'], 0)
                if p['workload_level'] == 'zero': controls += 1
            self.assertEqual(controls, 18)
            self.assertEqual(matrix, {(a, l, v, n) for a in (16, 128, 512) for l in (32, 4096, 24576) for v in ('shared', 'unique') for n in ('zero', 'full')})
            before = {f.name: hashlib.sha256(f.read_bytes()).digest() for f in (root / 'requests').iterdir()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(before, {f.name: hashlib.sha256(f.read_bytes()).digest() for f in (root / 'requests').iterdir()})

    def test_actual_bytes_targets_counts_gas_and_four_warmth_resets(self):
        m = self.load()
        for targets in (16, 128, 512):
            for level in ('zero', 'full'):
                row = m.make_case(targets, 32, 'unique', level); p = row['parameters']; result = concrete(row)
                self.assertEqual(result['terminal'], 0); self.assertEqual(result['calls'], p['calls_per_user'])
                self.assertEqual(result['gas'] + 21000, p['expected_user_charged_gas'])
                self.assertEqual(167355 + 4 * (result['gas'] + 21000), p['expected_block_charged_gas'])
                expected = {f'opcode:0x{k:02x}': v * 4 for k, v in result['counts'].items()}
                self.assertEqual(expected, {k: v for k, v in row['request']['program']['expected_route_counts'].items() if v})
                if level == 'full':
                    self.assertTrue(29_700_000 <= p['expected_block_charged_gas'] <= 30_000_000)
                    self.assertGreater(p['expected_block_charged_gas'] + 4 * 272, 30_000_000)
                for remaining in p['expected_remaining_header_gas_before_users']:
                    self.assertGreaterEqual(remaining, p['transaction_gas_limit'])
                self.assertEqual(p['expected_remaining_header_gas_before_users'], [31_000_000 - 167355 - i * p['expected_user_charged_gas'] for i in range(4)])
                self.assertEqual(p['expected_cold_calls_per_user'], result['cold']); self.assertEqual(p['expected_warm_calls_per_user'], result['warm'])
                # All four signed users run identical fresh transaction warmth.
                # This short control executes four times rather than assuming a
                # block-global warm set; full maps/gas above are then multiplied.
            control = m.make_case(1, 32, 'unique', calls=2)
            for _ in range(4):
                result = concrete(control); self.assertEqual((result['cold'], result['warm']), (1, 1))
            for variant in ('shared', 'unique'):
                full = m.make_case(targets, 32, variant, 'full'); zero = m.make_case(targets, 32, variant, 'zero')
                a = bytes.fromhex(full['request']['program']['code_hex'][2:]); b = bytes.fromhex(zero['request']['program']['code_hex'][2:])
                self.assertEqual(len(a), len(b)); changes = [i for i, (x, y) in enumerate(zip(a, b)) if x != y]
                bounds = [pc for pc, op, _ in instructions(b) if op == 0x62][1:]
                self.assertTrue(changes); self.assertTrue(all(any(pc < i <= pc + 3 for pc in bounds) for i in changes))
                self.assertEqual(full['request']['program']['expected_route_counts'], m.make_case(targets, 24576, 'shared' if variant == 'unique' else 'unique', 'full')['request']['program']['expected_route_counts'])

    def test_failure_guards_and_semantic_native_pass_mutants(self):
        m = self.load(); row = m.make_case(1, 32, 'unique', calls=2)
        for returned in (b'', bytes(31), bytes(32), bytes(33)):
            with self.subTest(length=len(returned)), self.assertRaises(AssertionError): concrete(row, return_override=returned)
        bad = copy.deepcopy(row); bad['request']['program']['prestate_contracts'][0]['code_hex'] = '0x' + bytes.fromhex('3060015260206000f3').ljust(32, b'\x00').hex()
        with self.assertRaises(AssertionError): concrete(bad)
        # Rotate the complete visited order while preserving CALL count, receipt
        # status and instruction histogram; only a concrete order oracle catches it.
        row = m.make_case(16, 32, 'unique', 'full'); bad = order_mutant(row)
        with self.assertRaisesRegex(AssertionError, 'target order|body counter'): concrete(bad)
        # Unique -> shared unreachable salt mutation leaves EVM gas/maps/status
        # unchanged. Byte identity/unique-code checks must independently reject it.
        contracts = row['request']['program']['prestate_contracts']; mutated = [c | {'code_hex': contracts[0]['code_hex']} for c in contracts]
        self.assertEqual(len({c['code_hex'] for c in mutated}), 1)
        self.assertNotEqual(len({c['code_hex'] for c in mutated}), row['parameters']['expected_configured_unique_code_count'])
        with self.assertRaisesRegex(AssertionError, 'salt identity'): concrete(salt_mutant(row))

    def test_typed_bounds_and_model_validation(self):
        m = self.load()
        for args in ((True, 32, 'shared', 'full'), (2, 32, 'shared', 'full'), (16, True, 'shared', 'full'),
                     (16, 31, 'shared', 'full'), (16, 33, 'shared', 'full'), (16, 24577, 'shared', 'full'),
                     (16, 32, True, 'full'), (16, 32, 'other', 'full'), (16, 32, 'shared', True)):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for kwargs in ({'calls': True}, {'calls': -1}, {'calls': 65536}, {'calls': 1}):
            with self.assertRaises(ValueError): m.make_case(16, **kwargs)
        with tempfile.TemporaryDirectory() as temporary:
            for value in ('', None, True, 'A' * 64, 'g' * 64):
                path = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(path, model_id=value)
                self.assertFalse(path.exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set frozen native for small cache controls, full gas and semantic negatives')
    def test_native_cache_controls_full_gas_and_negative_receipts(self):
        for kind in ('call', 'delegatecall'):
            with self.subTest(child_call_kind=kind): self.native_cache_controls(kind)

    def native_cache_controls(self, kind):
        m = self.load()
        retained = os.environ.get('MULTIACCOUNT_NATIVE_ARTIFACT_DIR')
        context = nullcontext(str(Path(retained) / kind)) if retained else tempfile.TemporaryDirectory()
        with context as temporary:
            root = Path(temporary)
            if retained: root.mkdir(parents=True, exist_ok=False)
            cases = [('n0', m.make_case(1, 32, 'unique', 'zero', child_call_kind=kind)),
                     ('first', m.make_case(1, 32, 'unique', calls=1, child_call_kind=kind)),
                     ('repeat', m.make_case(1, 4096, 'unique', calls=2, child_call_kind=kind)),
                     ('full', m.make_case(16, 32, 'unique', 'full', child_call_kind=kind))]
            for name, row in cases:
                folder = root / name; folder.mkdir(); source = folder / 'request.json'; source.write_text(json.dumps(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(folder / 'input.json'), '--output-bincode', str(folder / 'input.bin'), '--report', str(folder / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                (folder / 'native-process.json').write_text(json.dumps({'exit_code': result.returncode, 'stdout': result.stdout.decode(), 'stderr': result.stderr.decode()}))
                self.assertEqual(result.returncode, 0, result.stderr.decode()); report = json.loads((folder / 'native.json').read_text()); p = row['parameters']
                self.assertEqual([r['gas_used'] for r in report['receipts_by_block'][0]], [167355] + [p['expected_user_charged_gas']] * 4)
                self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
                self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'], 0)
                self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in row['request']['program']['expected_route_counts'].items() if v})
                prestate = report['prestate_contracts']; self.assertEqual(prestate['configured_account_count'], p['target_count'])
                self.assertEqual(prestate['witnessed_account_count'], p['expected_witnessed_target_accounts'])
                self.assertEqual(prestate['witnessed_unique_code_count'], p['expected_witnessed_target_code_count'])
                self.assertEqual(prestate['witnessed_code_bytes'], p['expected_witnessed_target_code_bytes'])
                # Authenticate code-target identity independently of ADDRESS,
                # which returns the parent context under DELEGATECALL.
                guest = json.loads((folder / 'input.json').read_text())
                witness = guest['witnesses'][0]['witness']
                actual_codes = [bytes.fromhex(code[2:]) for code in witness['codes']]
                hashes = [keccak256(code) for code in actual_codes]
                self.assertEqual(len(hashes), len(set(hashes)), 'witness code must be hash deduplicated')
                configured = row['request']['program']['prestate_contracts']
                expected_codes = {bytes.fromhex(c['code_hex'][2:]) for c in configured} if p['calls_per_user'] else set()
                target_codes = {code for code in actual_codes if code[:9] == PREFIX}
                self.assertEqual(target_codes, expected_codes)
                self.assertEqual(len(target_codes), p['expected_witnessed_target_code_count'])
                self.assertEqual(sum(map(len, target_codes)), p['expected_witnessed_target_code_bytes'])
                self.assertEqual(len(actual_codes), prestate['total_witness_unique_code_count'])
                self.assertEqual(sum(map(len, actual_codes)), prestate['total_witness_code_bytes'])
                nodes = {keccak256(bytes.fromhex(raw[2:])): bytes.fromhex(raw[2:]) for raw in guest['proposal_state_nodes']}
                state_root = bytes.fromhex(guest['proposal_ancestor_headers'][-1]['header']['stateRoot'][2:])
                if p['calls_per_user']:
                    for c in configured:
                        key = keccak256(bytes.fromhex(c['address'][2:]))
                        account = rlp_decode(trie_value(state_root, key, nodes))
                        self.assertEqual(account[:2], [b'\x01', b''])
                        self.assertEqual(account[2].hex(), '56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421')
                        self.assertEqual(account[3], keccak256(bytes.fromhex(c['code_hex'][2:])))
                users = report['transactions']['blocks'][0]['users']; self.assertEqual([u['nonce'] for u in users], list(range(4)))
                self.assertEqual({u['to'] for u in users}, {'0x6666666666666666666666666666666666666666'})
                self.assertEqual({u['gas_limit'] for u in users}, {p['transaction_gas_limit']})
                if os.environ.get('ZKGAS_QUOTE_BIN'):
                    quoted = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-bincode', str(folder / 'input.bin'), '--output', str(folder / 'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', 'multiaccount-code-author', '--sample-id', row['sample_id']], capture_output=True, timeout=180)
                    self.assertEqual(quoted.returncode, 0, quoted.stderr.decode()); quote = json.loads((folder / 'quote.jsonl').read_text())['quote']
                    self.assertEqual(quote['model_id'], m.MODEL); self.assertEqual(quote['public_output'], report['public_output']); self.assertEqual(quote['guest_input_sha256'], report['guest_input_sha256'])

            full = cases[-1][1]
            negatives = [('rotated-order-native-pass', order_mutant(full), True),
                         ('unique-to-shared-native-pass', salt_mutant(full), True),
                         ('short-return-native-reject', short_return_mutant(full), False)]
            if kind == 'delegatecall':
                for name, opcode in (('wrong-context-native-reject', 0x33), ('child-failure-native-reject', 0xfe)):
                    bad = copy.deepcopy(cases[1][1])
                    for c in bad['request']['program']['prestate_contracts']:
                        code = bytearray.fromhex(c['code_hex'][2:]); code[0 if opcode == 0x33 else 8] = opcode
                        c['code_hex'] = '0x' + code.hex()
                    negatives.append((name, bad, False))
            for name, row, succeeds in negatives:
                folder = root / name; folder.mkdir(); source = folder / 'request.json'; source.write_text(json.dumps(row['request']))
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(source), '--output', str(folder / 'input.json'), '--output-bincode', str(folder / 'input.bin'), '--report', str(folder / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                (folder / 'native-process.json').write_text(json.dumps({'exit_code': result.returncode, 'stdout': result.stdout.decode(), 'stderr': result.stderr.decode()}))
                self.assertEqual(result.returncode == 0, succeeds, result.stderr.decode())
                if succeeds:
                    report = json.loads((folder / 'native.json').read_text())
                    self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
                    self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in full['request']['program']['expected_route_counts'].items() if v})
                with self.assertRaises(AssertionError): concrete(row)
            if kind == 'delegatecall' and os.environ.get('ZKGAS_QUOTE_BIN'):
                original = json.loads((root / 'first' / 'input.json').read_text())
                self.quote_missing_witness_negatives(original, root)

    @unittest.skipUnless(os.environ.get('MULTIACCOUNT_REPLAY_INPUT') and os.environ.get('ZKGAS_QUOTE_BIN'),
                         'set retained native input and frozen quote for focused missing-witness replay')
    def test_quote_missing_witness_reaches_proposal_replay(self):
        original = json.loads(Path(os.environ['MULTIACCOUNT_REPLAY_INPUT']).read_text())
        retained = os.environ.get('MULTIACCOUNT_REPLAY_ARTIFACT_DIR')
        context = nullcontext(retained) if retained else tempfile.TemporaryDirectory()
        with context as temporary:
            root = Path(temporary)
            if retained: root.mkdir(parents=True, exist_ok=False)
            self.quote_missing_witness_negatives(original, root)

    def quote_missing_witness_negatives(self, original, root):
        corpus_id = 'multiaccount-delegate-witness-negatives'
        target_code_hashes = {keccak256(bytes.fromhex(code[2:])).hex()
                              for code in original['witnesses'][0]['witness']['codes']
                              if bytes.fromhex(code[2:]).startswith(PREFIX)}
        self.assertTrue(target_code_hashes, 'positive source must contain target code')
        for name in ('missing-code', 'missing-proof'):
            bad = copy.deepcopy(original)
            if name == 'missing-code':
                codes = bad['witnesses'][0]['witness']['codes']
                codes[:] = [code for code in codes if not bytes.fromhex(code[2:]).startswith(PREFIX)]
            else:
                bad['proposal_state_nodes'] = []
                bad['witnesses'][0]['witness']['state_indices'] = []
            folder = root / name; folder.mkdir(); source = folder / 'input.json'; source.write_text(json.dumps(bad))
            result = subprocess.run([os.environ['ZKGAS_QUOTE_BIN'], '--input-json', str(source), '--output', str(folder / 'quote.jsonl'), '--execution-profile', 'evm-gas-only', '--corpus-id', corpus_id, '--sample-id', name], capture_output=True, timeout=180)
            (folder / 'quote-process.json').write_text(json.dumps({'exit_code': result.returncode, 'stdout': result.stdout.decode(), 'stderr': result.stderr.decode()}))
            self.assertTrue((folder / 'quote.jsonl').is_file(), 'a failed replay row must be published; CLI errors are not replay evidence')
            rows = [json.loads(line) for line in (folder / 'quote.jsonl').read_text().splitlines()]
            self.assertEqual(len(rows), 1); failed = rows[0]
            self.assertEqual(result.returncode, 2, result.stderr.decode())
            self.assertEqual(failed['status'], 'failed'); self.assertEqual(failed['execution_profile'], 'evm-gas-only')
            self.assertEqual(failed['failure']['code'], 'proposal_quote')
            self.assertEqual(failed['corpus_id'], corpus_id); self.assertEqual(failed['sample_id'], name)
            self.assertEqual(failed['observed_guest_input_sha256'], hashlib.sha256(source.read_bytes()).hexdigest())
            self.assertEqual(failed['observed_guest_input_byte_length'], source.stat().st_size)
            self.assertEqual(failed['verified_key']['corpus_id'], corpus_id)
            self.assertEqual(failed['verified_key']['sample_id'], name)
            self.assertRegex(failed['verified_key']['guest_input_sha256'], '^[0-9a-f]{64}$')
            self.assertNotIn('quote', failed)
            message = failed['failure']['message']
            if name == 'missing-code':
                self.assertIn('bytecode for', message); self.assertIn('not found', message)
                self.assertTrue(any(digest in message for digest in target_code_hashes), message)
            else: self.assertIn('state trie unresolved while loading account', message)


class DelegatecallTests(MultiaccountCodeTests):
    def test_delegatecall_operand_context_gas_and_fresh_warmth(self):
        m = self.load()
        for targets in (16, 128, 512):
            for level in ('zero', 'full'):
                row = m.make_case(targets, 32, 'unique', level, child_call_kind='delegatecall')
                p = row['parameters']; result = concrete(row); n = p['calls_per_user']
                # Independently derived: parent54+148N; child17N; cold2600,
                # warm100; parent memory9 only for nonzero work, intrinsic21000.
                expected = 21054 + 265 * n + 2500 * min(n, targets) + (9 if n else 0)
                self.assertEqual(result['gas'] + 21000, expected)
                self.assertEqual(p['expected_user_charged_gas'], expected)
                self.assertEqual(result['counts'][0xf4], n); self.assertEqual(result['counts'][0xf1], 0)
                self.assertEqual(result['counts'][0x30], 2 * n)
                self.assertEqual(row['request']['program']['expected_route_counts'],
                                 {f'opcode:0x{k:02x}': v * 4 for k, v in result['counts'].items()} |
                                 {k: v for k, v in row['request']['program']['expected_route_counts'].items() if v == 0})
                self.assertEqual(p['child_call_kind'], 'delegatecall')
                self.assertEqual(p['child_address_context'], 'parent')
                self.assertEqual(p['expected_child_address_return'], '0x' + '66' * 20)
                self.assertIn('code-target identity', row['completion_gate'])
                self.assertEqual(p['expected_remaining_header_gas_before_users'],
                                 [31_000_000 - 167355 - i * expected for i in range(4)])
                for remaining in p['expected_remaining_header_gas_before_users']:
                    self.assertGreaterEqual(remaining, p['transaction_gas_limit'])
                if n:
                    self.assertTrue(29_700_000 <= 167355 + 4 * expected <= 30_000_000)
                    self.assertGreater(167355 + 4 * (expected + 265), 30_000_000)
            control = m.make_case(1, 32, 'unique', calls=2, child_call_kind='delegatecall')
            for _ in range(4):
                result = concrete(control)
                self.assertEqual((result['calls'], result['cold'], result['warm']), (2, 1, 1))
                self.assertEqual(result['gas'] + 21000, 24093)

    def test_delegatecall_catalog_distinct_pairs_and_cli(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'delegate'; self.assertEqual(m.prepare(root, child_call_kind='delegatecall'), 36)
            catalog = json.loads((root / 'catalog.json').read_text()); rows = catalog['cases']
            by = {r['sample_id']: r for r in rows}; self.assertEqual(len(by), 36)
            self.assertEqual(catalog['child_call_kind'], 'delegatecall')
            self.assertEqual(len({r['request_sha256'] for r in rows}), 36)
            self.assertEqual({(r['parameters']['target_count'], r['parameters']['runtime_bytes_per_target'], r['parameters']['code_variant'], r['parameters']['workload_level']) for r in rows},
                             {(a, l, v, n) for a in (16, 128, 512) for l in (32, 4096, 24576) for v in ('shared', 'unique') for n in ('zero', 'full')})
            self.assertEqual(sum(r['parameters']['workload_level'] == 'zero' for r in rows), 18)
            for row in rows:
                p = row['parameters']; self.assertTrue(row['sample_id'].startswith('multiaccount-delegatecall-code-'))
                self.assertNotEqual(row['sample_id'], m.sample_id(p['target_count'], p['runtime_bytes_per_target'], p['code_variant'], p['workload_level']))
                baseline = by[row['baseline_sample_id']]
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                self.assertEqual(baseline['parameters']['calls_per_user'], 0)
                raw = (root / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                req = json.loads(raw)
                self.assertEqual(req['program']['transactions'], {'kind': 'call', 'count_per_block': 4, 'gas_limit': p['transaction_gas_limit']})
                self.assertNotIn('child_call_kind', req['program'])
                self.assertEqual(len(req['program']['prestate_contracts']), p['target_count'])
                for i, c in enumerate(req['program']['prestate_contracts']):
                    code = bytes.fromhex(c['code_hex'][2:])
                    self.assertEqual(c['address'], f'0x{BASE + i:040x}')
                    self.assertEqual(len(code), p['runtime_bytes_per_target']); self.assertEqual(code[:9], PREFIX)
                    self.assertFalse(any(code[9:-4]))
                    self.assertEqual(code[-4:], (i + 1 if p['code_variant'] == 'unique' else 0).to_bytes(4, 'big'))
                paired = json.loads((root / baseline['request_path']).read_text())
                self.assertEqual(req['program']['transactions'], paired['program']['transactions'])
                opposite_id = m.sample_id(p['target_count'], p['runtime_bytes_per_target'], 'shared' if p['code_variant'] == 'unique' else 'unique', p['workload_level'], child_call_kind='delegatecall')
                opposite = json.loads((root / by[opposite_id]['request_path']).read_text())
                self.assertEqual(req['program']['expected_route_counts'], opposite['program']['expected_route_counts'])
            cli = Path(temporary) / 'cli'
            completed = subprocess.run([sys.executable, '-S', str(CASES / 'multiaccount_code.py'), '--output-dir', str(cli), '--child-call-kind', 'delegatecall'], capture_output=True, timeout=30)
            self.assertEqual(completed.returncode, 0, completed.stderr.decode())
            self.assertEqual(json.loads((cli / 'catalog.json').read_text())['cases'], rows)

    def test_delegatecall_guards_reject_wrong_context_order_and_hashes(self):
        m = self.load()
        row = m.make_case(1, 32, 'unique', calls=2, child_call_kind='delegatecall')
        for returned in (b'', bytes(31), BASE.to_bytes(32, 'big'), bytes(33)):
            with self.subTest(length=len(returned)), self.assertRaises(AssertionError): concrete(row, return_override=returned)
        for mutant in (order_mutant, salt_mutant, short_return_mutant):
            row = m.make_case(16, 32, 'unique', child_call_kind='delegatecall')
            with self.subTest(mutant=mutant.__name__), self.assertRaises(AssertionError): concrete(mutant(row))
        for value in (None, True, '', 'staticcall', 'DELEGATECALL', [], {}):
            with self.subTest(kind=value), self.assertRaises(ValueError): m.make_case(child_call_kind=value)
            with tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(path, child_call_kind=value)
                self.assertFalse(path.exists())

    @unittest.skipUnless(os.environ.get('MULTIACCOUNT_CALL_BASELINE'), 'set immutable original CALL catalog directory')
    def test_default_call_exact_original_request_bytes_and_ids(self):
        m = self.load(); baseline = Path(os.environ['MULTIACCOUNT_CALL_BASELINE'])
        old = json.loads((baseline / 'catalog.json').read_text())['cases']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'cases'; m.prepare(root)
            rows = json.loads((root / 'catalog.json').read_text())['cases']
            self.assertEqual([r['sample_id'] for r in rows], [r['sample_id'] for r in old])
            for a, b in zip(rows, old):
                self.assertEqual(a['request_sha256'], b['request_sha256'])
                self.assertEqual((root / a['request_path']).read_bytes(), (baseline / b['request_path']).read_bytes())
                self.assertEqual(a['baseline_sample_id'], b['baseline_sample_id'])
                self.assertEqual(a['parameters'], b['parameters'])


if __name__ == '__main__': unittest.main()
