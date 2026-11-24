"""Bounded actual-byte oracle for heterogeneous restore and root rollback."""
from collections import Counter
import copy
import hashlib
import importlib.util
import inspect
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_storage_mutation import decode, execute
from test_storage_root_diversity import address, run_users

CASES = Path(__file__).resolve().parents[1] / 'cases'
MODES = ('restore-stop', 'update-revert', 'restore-revert')
ACCOUNTS = (8, 64)
SIZES = (8192, 16384)
LEVELS = ('zero', 'small', 'full')
GRID = tuple(itertools.product(MODES, ACCOUNTS, SIZES, LEVELS))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
SOURCES = ('storage_root_diversity.py', 'storage_fanout.py', 'storage_mutation.py',
           'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def qualify(test, row, execution=None):
    """Literal intended trajectories; no producer gas/runtime/map helper calls."""
    p = row['parameters']; mode = p['mode']; a = p['account_count']; size = p['aggregate_state_size']
    n = p['mutations_per_user']; restore = mode != 'update-revert'; stop = mode == 'restore-stop'
    test.assertIn((mode, a, size, p['workload_level']), GRID)
    test.assertEqual(n, {'zero': 0, 'small': 32, 'full': p['full_mutations_per_user']}[p['workload_level']])
    program = row['request']['program']; contracts = program['prestate_contracts']
    k = size // a; visits = 64 // a
    initial = {address(i): {0: (i + 1) * 65536 + 1} | dict.fromkeys(range((i + 1) * 65536 + 1, (i + 1) * 65536 + k), 1) for i in range(a)}
    bases = {address(i): (i + 1) * 65536 for i in range(a)}
    phases = {owner: state[0] for owner, state in initial.items()}
    final_phases = {owner: phase + (visits * n if stop else 0) for owner, phase in phases.items()}
    test.assertEqual(p['phase_base_by_account'], bases)
    test.assertEqual(p['initial_phase_by_account'], phases)
    test.assertEqual(p['expected_final_phase_by_account'], final_phases)
    for alias in ('initial_phase', 'expected_final_phase_per_account', 'final_phase_schedule', 'profile'):
        test.assertNotIn(alias, p)
    test.assertEqual([c['address'] for c in contracts], list(initial))
    for c in contracts: test.assertEqual({int(key, 16): int(value, 16) for key, value in c['initial_storage'].items()}, initial[c['address']])
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertIs(row['request']['expected_user_success'], stop)
    test.assertIs(p['expected_user_success'], stop)
    test.assertEqual(p['expected_terminal'], 'STOP' if stop else 'REVERT')
    schedule = [address(j % a) for j in range(64)]
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=64, gas_limit=p['transaction_gas_limit'], recipients=schedule))
    test.assertEqual(p['recipient_schedule'], schedule)
    test.assertEqual(len({c['code_hex'] for c in contracts}), 1)
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual(p['runtime_sha256'], hashlib.sha256(code).hexdigest())
    test.assertEqual(p['runtime_bytes'], len(code))
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(len(records), 64)
    expected = copy.deepcopy(initial); routes = Counter(); pairs = set(); charged = 167355
    starts = []; attempted = []
    # Actual-byte instruction costs, independently specified here.
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3, 0x54: 0, 0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0}
    for j, record in enumerate(records):
        owner = address(j % a); start = phases[owner] + ((j // a) * n if stop else 0); end = start + n
        starts.append(start); attempted.append(end)
        test.assertEqual((record['recipient'], record['owner']), (owner, owner))
        result = record['result']; reads = [(0, start, True)]; writes = []
        for key in range(start, end):
            reads.extend([(key, 1, True), (key, 2, False)])
            writes.append((key, 1, 2, 1, False))
            if restore:
                reads.append((key, 1, False)); writes.append((key, 2, 1, 1, False))
            pairs.add((owner, key))
        reads.append((0, end, False)); writes.append((0, start, end, start, False)); pairs.add((owner, 0))
        test.assertEqual(result['terminal'], 0 if stop else 0xfd)
        test.assertEqual(result['reads'], reads, 'ordered raw reads/values/warmth')
        test.assertEqual(result['writes'], writes, 'ordered raw writes/originals')
        test.assertEqual(result['stack'], [])
        test.assertEqual(len(result['memory']), 128 if n else 96)
        test.assertEqual([int.from_bytes(result['memory'][offset:offset + 32], 'big') for offset in (0, 32, 64)], [n, (3 if restore else 2) * n, start])
        if stop: expected[owner][0] = end
        test.assertEqual(record['state'], expected[owner], 'complete committed map')
        ordinary = sum(count * (3 if 0x60 <= int(route[-2:], 16) <= 0x7f else costs[int(route[-2:], 16)]) for route, count in result['counts'].items() if count)
        gross = 21000 + ordinary + (12 if n else 9) + 2200 + n * (2300 if restore else 2200) + (2900 if n else 100) + n * (3000 if restore else 2900)
        before = 2800 * n if restore else 0; refund = before if stop else 0; applied = min(refund, gross // 5)
        for actual, value in (('gas', gross), ('refund', refund), ('applied', applied), ('charged', gross - applied)):
            test.assertEqual(result[actual], value)
        if not stop: test.assertEqual(result['refund_before_revert'], before)
        for field, value in dict(gross_gas=gross, refund_counter=refund, refund_counter_before_terminal=before, refund_applied=applied, charged_gas=gross - applied).items():
            test.assertEqual(p['gas_accounting_per_user'][field], value)
            test.assertEqual(p['expected_user_' + field], value)
        test.assertGreaterEqual(31_000_000 - charged, p['transaction_gas_limit'])
        routes.update(result['counts']); charged += result['charged']
    test.assertEqual(states, expected)
    test.assertEqual({owner: state[0] for owner, state in states.items()}, final_phases)
    test.assertEqual(len({tuple(sorted(state.items())) for state in states.values()}), a)
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(p['expected_exceptional_instructions'], 0)
    for op in (0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xfa, 0xfe): test.assertEqual(program['expected_route_counts'][f'opcode:0x{op:02x}'], 0)
    distinct = 64 * n + a if stop else a * (n + 1)
    test.assertEqual(len(pairs), distinct); test.assertLessEqual(distinct, 8192)
    fields = dict(expected_distinct_account_slot_pairs=distinct, expected_distinct_storage_keys_per_account=(visits * n if stop else n) + 1,
        expected_storage_writes=64 * ((2 if restore else 1) * n + 1), expected_main_storage_writes=64 * (2 if restore else 1) * n,
        expected_phase_storage_writes=64, expected_storage_reads=64 * ((3 if restore else 2) * n + 2),
        expected_initial_storage_entry_count=size, expected_final_storage_entry_count=size,
        expected_initial_storage_entry_count_per_account=k, expected_final_storage_entry_count_per_account=k,
        expected_counter=n, expected_checksum=(3 if restore else 2) * n,
        expected_distinct_body_keys_per_account=(visits * n if stop else n),
        expected_body_key_visits_per_account=visits * n,
        expected_repeated_body_key_visits_per_account=0 if stop else (visits - 1) * n,
        expected_net_main_storage_changes=0)
    for field, value in fields.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(p['start_phase_schedule'], starts); test.assertEqual(p['attempted_final_phase_schedule'], attempted)
    test.assertEqual(p['expected_block_charged_gas'], charged); test.assertLessEqual(charged, 30_000_000)
    test.assertEqual(p['header_prefix_remaining_gas'], [31_000_000 - 167355 - j * records[j]['result']['charged'] for j in range(64)])
    test.assertEqual(p['near_full'], charged >= 29_700_000)
    return records, states


class StorageRootJournalTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_root_journal.py'
        self.assertTrue(path.is_file(), 'storage root journal producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_root_journal_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_bounded_revert_rolls_back_in_place_but_keeps_attempts_and_refund(self):
        # slot7: 1->2->1, new slot8: 0->2, slot9 removed, zero-range REVERT.
        code = bytes.fromhex('600260075560016007556002600855600060095560006000fd')
        state = {7: 1, 9: 1}; alias = state; original = state.copy()
        result = execute(code, state, 200000)
        self.assertIs(state, alias); self.assertEqual(alias, original)
        self.assertEqual(result['terminal'], 0xfd); self.assertEqual(len(result['writes']), 4)
        self.assertEqual(result['refund_before_revert'], 7600)
        self.assertEqual((result['refund'], result['applied'], result['charged']), (0, 0, result['gas']))
        self.assertEqual(result['counts']['opcode:0xfd'], 1)
        state[7] = 3; self.assertEqual(original[7], 1)

    def test_bounded_revert_rejects_nonzero_offset_or_length(self):
        for code in ('60006001fd', '60016000fd'):
            with self.subTest(code=code), self.assertRaisesRegex(AssertionError, 'unsupported.*REVERT'):
                execute(bytes.fromhex(code), {}, 100000)

    def test_bounded_sstore_sentry_uses_gas_left_before_write(self):
        code = bytes.fromhex('600160005500')
        with self.assertRaisesRegex(AssertionError, 'SSTORE sentry'):
            execute(code, {0: 1}, 21000 + 6 + 2300)
        self.assertEqual(execute(code, {0: 1}, 21000 + 6 + 2301)['terminal'], 0)

    def test_strict_inputs(self):
        m = self.load()
        for args in ((None,), (True,), ('restore',), ('restore-stop', True), ('restore-stop', 8.0), ('restore-stop', 1), ('restore-stop', 8, True), ('restore-stop', 8, 8192.0), ('restore-stop', 8, 8193), ('restore-stop', 8, 8192, True), ('restore-stop', 8, 8192, 'FULL')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for n in (True, -1, 1.0, 8193):
            with self.assertRaises(ValueError): m.gas_accounting('restore-stop', n)
        for mode in (None, True, 'stop'):
            with self.assertRaises(ValueError): m.gas_accounting(mode, 0)
        self.assertEqual(m.make_case(), m.make_case('restore-stop', 8, 8192, 'full'))

    def test_all_36_actual_byte_trajectories(self):
        m = self.load()
        for args in GRID:
            with self.subTest(args=args): qualify(self, m.make_case(*args))

    def test_full_candidate_plus_one_independently_executed(self):
        m = self.load()
        for mode, a, size in itertools.product(MODES, ACCOUNTS, SIZES):
            row = m.make_case(mode, a, size); p = row['parameters']; n = p['mutations_per_user']
            self.assertEqual(m.full_n(mode, a, size), n)
            self.assertEqual(p['transaction_gas_limit'], p['expected_user_gross_gas'] + 2301)
            code = bytearray.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:]); factor = 2 if mode == 'update-revert' else 3
            for at, op, value in decode(code):
                if op == 0x62 and value in (n, factor * n): code[at + 1:at + 4] = (value + (1 if value == n else factor)).to_bytes(3, 'big')
            state = {int(k, 16): int(v, 16) for k, v in row['request']['program']['prestate_contracts'][0]['initial_storage'].items()}
            result = execute(code, state, 15_000_000); self.assertEqual(result['terminal'], 0 if mode == 'restore-stop' else 0xfd)
            nn = n + 1; stop = mode == 'restore-stop'; envelope = result['gas'] + 2301
            violations = [(64 * nn + a if stop else a * (nn + 1)) > 8192, (64 // a) * nn > size // a - 1 if stop else nn > size // a - 1,
                envelope > 15_000_000, 167355 + 64 * result['charged'] > 30_000_000]
            violations.extend(31_000_000 - 167355 - j * result['charged'] < envelope for j in range(64))
            self.assertTrue(any(violations), (mode, a, size, n))

    def test_catalog_grid_own_controls_hashes_provenance_and_exclusive_output(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(target), 36)
            raw = (target / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-root-journal-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_root_journal.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in SOURCES})
            helpers = {'storage_root_diversity.make_case': m.diversity.make_case, '_Factory': m._Factory,
                'storage_mutation._require': m.mutation._require, 'storage_mutation._check': m.mutation._check,
                'storage_mutation._key': m.mutation._key, 'storage_root_journal._runtime': m._runtime,
                'storage_root_journal.gas_accounting': m.gas_accounting, 'storage_root_journal.full_n': m.full_n}
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in helpers.items()})
            by = {row['sample_id']: row for row in rows}; self.assertEqual(len(by), 36)
            self.assertEqual(len({row['request_sha256'] for row in rows}), 36)
            self.assertEqual(sum(row['role'] == 'control' for row in rows), 12)
            self.assertEqual({tuple(row['parameters'][key] for key in ('mode', 'account_count', 'aggregate_state_size', 'workload_level')) for row in rows}, set(GRID))
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]; request_raw = (target / row['request_path']).read_bytes(); request = json.loads(request_raw)
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertEqual(request_raw, (json.dumps(request, indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['sample_id'], f"storage-root-journal-{p['mode']}-a{p['account_count']}-s{p['aggregate_state_size']}-n{p['mutations_per_user']}")
                self.assertEqual(row['baseline_sample_id'], row['sample_id'].rsplit('-n', 1)[0] + '-n0')
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                base = json.loads((target / baseline['request_path']).read_text())['program']
                self.assertEqual([c['initial_storage'] for c in request['program']['prestate_contracts']], [c['initial_storage'] for c in base['prestate_contracts']])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertIn('all36', row['completion_gate'])
            with self.assertRaises(FileExistsError): m.prepare(target)
            self.assertEqual((target / 'catalog.json').read_bytes(), raw)
            for invalid in (True, None, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=invalid)
                self.assertFalse(target.exists())
            alternate = Path(temporary) / 'alternate'; m.prepare(alternate, model_id='a' * 64)
            self.assertEqual(json.loads((alternate / 'catalog.json').read_text())['frozen_model_id'], 'a' * 64)

    def test_cli(self):
        with tempfile.TemporaryDirectory() as temporary:
            command = [sys.executable, '-S', str(CASES / 'storage_root_journal.py'), '--output-dir', str(Path(temporary) / 'matrix'), '--model-id', 'b' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 36})
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    def test_rejects_wrong_semantics_even_when_terminal_matches(self):
        m = self.load()
        for mode in MODES:
            for mutation in ('owner', 'key', 'warmth', 'refund', 'terminal', 'state', 'phase', 'tail', 'value-checksum', 'recipient', 'signed-success'):
                row = m.make_case(mode, level='small'); execution = None; program = row['request']['program']
                if mutation in ('owner', 'key', 'warmth', 'refund', 'terminal', 'state'):
                    execution = run_users(row); record = execution[0][0]; result = record['result']
                    if mutation == 'owner': record['owner'] = address(1)
                    elif mutation == 'key': result['reads'][1] = (result['reads'][1][0] + 1,) + result['reads'][1][1:]
                    elif mutation == 'warmth': result['reads'][1] = result['reads'][1][:2] + (False,)
                    elif mutation == 'refund': result['refund'] += 1
                    elif mutation == 'terminal': result['terminal'] = 0xfd if mode == 'restore-stop' else 0
                    else: record['state'][65537] = 2
                elif mutation == 'phase': row['parameters']['attempted_final_phase_schedule'][0] += 1
                elif mutation == 'tail': program['prestate_contracts'][0]['initial_storage'][hex(65536 + 1023)] = '0x3'
                elif mutation == 'recipient': program['transactions']['recipients'] = program['transactions']['recipients'][1:] + program['transactions']['recipients'][:1]
                elif mutation == 'signed-success': row['request']['expected_user_success'] = mode != 'restore-stop'
                else:
                    n = row['parameters']['mutations_per_user']; factor = 2 if mode == 'update-revert' else 3
                    for c in program['prestate_contracts']:
                        code = bytearray.fromhex(c['code_hex'][2:])
                        for at, op, value in decode(code):
                            if op == 0x62 and value in (2, factor * n): code[at + 1:at + 4] = (3 if value == 2 else (factor + 1) * n).to_bytes(3, 'big')
                        c['code_hex'] = '0x' + code.hex()
                    row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
                if mutation in ('tail', 'recipient', 'value-checksum'):
                    checked = run_users(row)
                    self.assertTrue(all(record['result']['terminal'] == (0 if mode == 'restore-stop' else 0xfd) for record in checked[0]))
                with self.subTest(mode=mode, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution)

    def test_wrong_raw_domain_and_body_guard_reach_unreachable_invalid(self):
        m = self.load()
        for mode, mutation in itertools.product(MODES, ('raw-domain', 'body-guard')):
            row = m.make_case(mode, level='small')
            for contract in row['request']['program']['prestate_contracts']:
                code = bytearray.fromhex(contract['code_hex'][2:])
                if mutation == 'raw-domain': code[1:4] = (65536).to_bytes(3, 'big')
                else:
                    instructions = list(decode(code))
                    for index, (_, op, _) in enumerate(instructions[:-1]):
                        at, next_op, value = instructions[index + 1]
                        if op == 0x54 and next_op == 0x62 and value == 1:
                            code[at + 1:at + 4] = (2).to_bytes(3, 'big'); break
                    else: self.fail('body read guard is missing')
                contract['code_hex'] = '0x' + code.hex()
            row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            records, states = run_users(row)
            self.assertTrue(all(record['result']['terminal'] == 0xfe for record in records))
            with self.subTest(mode=mode, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, (records, states))

    @unittest.skipUnless(os.environ.get('STORAGE_ROOT_JOURNAL_NATIVE_BIN'), 'opt in bounded frozen native smoke')
    def test_frozen_native_smoke(self):
        binary = Path(os.environ['STORAGE_ROOT_JOURNAL_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        m = self.load()
        for mode, a, mutant in (('restore-stop', 8, False), ('restore-revert', 64, False),
                                ('restore-stop', 8, True), ('restore-revert', 64, True)):
            row = m.make_case(mode, a); p = row['parameters']
            if mutant:
                # Native accepts the matching guard/checksum, but the intended
                # workload requires 1->2->1 rather than this 1->3->1 trajectory.
                n = p['mutations_per_user']
                for contract in row['request']['program']['prestate_contracts']:
                    code = bytearray.fromhex(contract['code_hex'][2:])
                    for at, op, value in decode(code):
                        if op == 0x62 and value in (2, 3 * n):
                            code[at + 1:at + 4] = (3 if value == 2 else 4 * n).to_bytes(3, 'big')
                    contract['code_hex'] = '0x' + code.hex()
                p['runtime_sha256'] = hashlib.sha256(code).hexdigest()
                with self.assertRaises(AssertionError): qualify(self, row)
            with self.subTest(mode=mode, mutant=mutant), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); source = root / 'request.json'; source.write_text(json.dumps(row['request']))
                process = subprocess.run([str(binary), '--input', str(source), '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'), '--report', str(root / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads((root / 'native.json').read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), 65); self.assertTrue(receipts[0]['success']); self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['success'] for r in receipts[1:]], [mode == 'restore-stop'] * 64)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 64)
                diagnostics = report['prestate_contracts']['initialized_storage']
                self.assertEqual([d['address'] for d in diagnostics], [address(i) for i in range(a)])
                for d in diagnostics:
                    self.assertEqual((d['initial_storage_entry_count'], d['final_storage_entry_count']), (8192 // a, 8192 // a))
                    for key in ('initial_storage_root', 'final_storage_root'): self.assertRegex(d[key], r'^0x[0-9a-f]{64}$')
                    self.assertEqual(d['initial_storage_root'] == d['final_storage_root'], mode != 'restore-stop')
                for key in ('initial_storage_root', 'final_storage_root'): self.assertEqual(len({d[key] for d in diagnostics}), a)


if __name__ == '__main__': unittest.main()
