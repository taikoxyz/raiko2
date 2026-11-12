"""Bounded actual-byte oracle for few long root-REVERT calls."""
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
MODES = ('update-revert', 'restore-revert')
ACCOUNTS = (2, 4)
SIZES = (8192, 16384)
LEVELS = ('zero', 'full')
GRID = tuple(itertools.product(MODES, ACCOUNTS, SIZES, LEVELS))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
SOURCES = ('storage_root_journal.py', 'storage_root_diversity.py', 'storage_fanout.py', 'storage_mutation.py',
           'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def qualify(test, row, execution=None):
    """Literal intended trajectories; no producer gas/runtime/map helper calls."""
    p = row['parameters']; mode = p['mode']; a = p['account_count']; u = p['user_transactions_per_block']; size = p['aggregate_state_size']
    n = p['mutations_per_user']; restore = mode == 'restore-revert'
    test.assertIn((mode, a, size, p['workload_level']), GRID)
    test.assertEqual(n, {'zero': 0, 'full': p['full_mutations_per_user']}[p['workload_level']])
    program = row['request']['program']; contracts = program['prestate_contracts']
    test.assertEqual(a, u)
    k = size // a
    initial = {address(i): {0: (i + 1) * 65536 + 1} | dict.fromkeys(range((i + 1) * 65536 + 1, (i + 1) * 65536 + k), 1) for i in range(a)}
    bases = {address(i): (i + 1) * 65536 for i in range(a)}
    phases = {owner: state[0] for owner, state in initial.items()}
    final_phases = {owner: phase for owner, phase in phases.items()}
    test.assertEqual(p['phase_base_by_account'], bases)
    test.assertEqual(p['initial_phase_by_account'], phases)
    test.assertEqual(p['expected_final_phase_by_account'], final_phases)
    for alias in ('initial_phase', 'expected_final_phase_per_account', 'final_phase_schedule', 'profile'):
        test.assertNotIn(alias, p)
    test.assertEqual([c['address'] for c in contracts], list(initial))
    for c in contracts: test.assertEqual({int(key, 16): int(value, 16) for key, value in c['initial_storage'].items()}, initial[c['address']])
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertIs(row['request']['expected_user_success'], False)
    test.assertIs(p['expected_user_success'], False)
    test.assertEqual(p['expected_terminal'], 'REVERT')
    schedule = [address(j % a) for j in range(u)]
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=u, gas_limit=p['transaction_gas_limit'], recipients=schedule))
    test.assertEqual(p['recipient_schedule'], schedule)
    test.assertEqual(len({c['code_hex'] for c in contracts}), 1)
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual(p['runtime_sha256'], hashlib.sha256(code).hexdigest())
    test.assertEqual(p['runtime_bytes'], len(code))
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(len(records), u)
    expected = copy.deepcopy(initial); routes = Counter(); pairs = set(); charged = 167355
    starts = []; attempted = []
    # Actual-byte instruction costs, independently specified here.
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3, 0x54: 0, 0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0}
    for j, record in enumerate(records):
        owner = address(j % a); start = phases[owner]; end = start + n
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
        test.assertEqual(result['terminal'], 0xfd)
        test.assertEqual(result['reads'], reads, 'ordered raw reads/values/warmth')
        test.assertEqual(result['writes'], writes, 'ordered raw writes/originals')
        test.assertEqual(result['stack'], [])
        test.assertEqual(len(result['memory']), 128 if n else 96)
        test.assertEqual([int.from_bytes(result['memory'][offset:offset + 32], 'big') for offset in (0, 32, 64)], [n, (3 if restore else 2) * n, start])
        test.assertEqual(record['state'], expected[owner], 'complete committed map')
        ordinary = sum(count * (3 if 0x60 <= int(route[-2:], 16) <= 0x7f else costs[int(route[-2:], 16)]) for route, count in result['counts'].items() if count)
        gross = 21000 + ordinary + (12 if n else 9) + 2200 + n * (2300 if restore else 2200) + (2900 if n else 100) + n * (3000 if restore else 2900)
        before = 2800 * n if restore else 0; refund = 0; applied = 0
        for actual, value in (('gas', gross), ('refund', refund), ('applied', applied), ('charged', gross - applied)):
            test.assertEqual(result[actual], value)
        test.assertEqual(result['refund_before_revert'], before)
        test.assertEqual(p['gas_accounting_per_user'], dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary,
            memory_expansion_gas=12 if n else 9, storage_read_gas=2200 + n * (2300 if restore else 2200),
            storage_write_gas=(2900 if n else 100) + n * (3000 if restore else 2900),
            gross_gas=gross, refund_counter_before_terminal=before, refund_counter=0, refund_applied=0, charged_gas=gross))
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
    distinct = u * (n + 1)
    test.assertEqual(len(pairs), distinct); test.assertLessEqual(distinct, 8192)
    fields = dict(expected_distinct_account_slot_pairs=distinct, expected_distinct_storage_keys_per_account=n + 1,
        expected_storage_writes=u * ((2 if restore else 1) * n + 1), expected_main_storage_writes=u * (2 if restore else 1) * n,
        expected_phase_storage_writes=u, expected_storage_reads=u * ((3 if restore else 2) * n + 2),
        expected_initial_storage_entry_count=size, expected_final_storage_entry_count=size,
        expected_initial_storage_entry_count_per_account=k, expected_final_storage_entry_count_per_account=k,
        expected_counter=n, expected_checksum=(3 if restore else 2) * n,
        expected_distinct_body_keys_per_account=n,
        expected_body_key_visits_per_account=n,
        expected_repeated_body_key_visits_per_account=0,
        expected_net_main_storage_changes=0)
    for field, value in fields.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(p['start_phase_schedule'], starts); test.assertEqual(p['attempted_final_phase_schedule'], attempted)
    test.assertEqual(p['expected_block_charged_gas'], charged); test.assertLessEqual(charged, 30_000_000)
    test.assertEqual(p['header_prefix_remaining_gas'], [31_000_000 - 167355 - j * records[j]['result']['charged'] for j in range(u)])
    test.assertEqual(p['near_full'], charged >= 29_700_000)
    return records, states


class StorageLongRevertTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_long_revert.py'
        self.assertTrue(path.is_file(), 'long root-REVERT producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_long_revert_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_all_16_actual_byte_trajectories(self):
        m = self.load()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); records, _ = qualify(self, row)
                p = row['parameters']; u = args[1]
                self.assertEqual(p['all_user_transactions'], u)
                self.assertEqual(p['all_transactions_including_anchor'], u + 1)
                self.assertEqual(p['visits_per_account'], 1)
                self.assertFalse(p['preload_created_accounts_or_code'])
                self.assertLessEqual(p['transaction_gas_limit'], 15_000_000)
                self.assertLessEqual(p['transaction_gas_limit'], 16_777_216)
                self.assertEqual(p['expected_main_storage_writes_per_account'], (2 if args[0] == 'restore-revert' else 1) * p['mutations_per_user'])
                self.assertEqual(p['expected_storage_writes_per_account'], len(records[0]['result']['writes']))
                self.assertEqual(p['expected_user_receipt_statuses'], [False] * u)
                self.assertTrue(p['expected_anchor_success'])
                self.assertEqual(p['transaction_gas_limit'], m.journal.gas_accounting(args[0], p['full_mutations_per_user'])['gross_gas'] + 2301)

    def test_strict_inputs(self):
        m = self.load()
        for args in ((None,), (True,), ('restore-stop',), ('update-revert', True), ('update-revert', 2.0), ('update-revert', 8), ('update-revert', 2, True), ('update-revert', 2, 8192.0), ('update-revert', 2, 8193), ('update-revert', 2, 8192, True), ('update-revert', 2, 8192, 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for n in (True, -1, 1.0, 8193):
            with self.assertRaises(ValueError): m.fits('update-revert', 2, 8192, n)
        for function in (m.full_n, lambda *args: m.fits(*args, 0)):
            for args in ((None, 2, 8192), ('update-revert', True, 8192), ('restore-revert', 2, None)):
                with self.assertRaises(ValueError): function(*args)
        self.assertEqual(m.make_case(), m.make_case('update-revert', 2, 8192, 'full'))

    def test_n_plus_one_actual_bytes_before_candidate_constraints(self):
        m = self.load()
        for mode, u, size in itertools.product(MODES, ACCOUNTS, SIZES):
            row = m.make_case(mode, u, size); p = row['parameters']; n = p['mutations_per_user']
            self.assertEqual(m.full_n(mode, u, size), n); self.assertTrue(m.fits(mode, u, size, n))
            code = bytearray.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:]); factor = 2 if mode == 'update-revert' else 3
            for at, op, value in decode(code):
                if op == 0x62 and value in (n, factor * n): code[at + 1:at + 4] = (value + (1 if value == n else factor)).to_bytes(3, 'big')
            results = []
            for contract in row['request']['program']['prestate_contracts']:
                state = {int(k, 16): int(v, 16) for k, v in contract['initial_storage'].items()}; original = state.copy()
                result = execute(code, state, 20_000_000)
                self.assertEqual(result['terminal'], 0xfd); self.assertEqual(state, original)
                self.assertEqual(len(result['writes']), (2 if mode == 'restore-revert' else 1) * (n + 1) + 1)
                results.append(result)
            gas = results[0]['gas']; nn = n + 1; envelope = gas + 2301
            self.assertTrue(any((u * (nn + 1) > 8192, nn > size // u - 1,
                envelope > 15_000_000, envelope > 16_777_216, 167355 + u * gas > 30_000_000,
                any(31_000_000 - 167355 - j * gas < envelope for j in range(u)))))
            self.assertFalse(m.fits(mode, u, size, nn))
            # The VM enforces the pre-write >2300 sentry, independent of solver arithmetic.
            with self.assertRaisesRegex(AssertionError, 'SSTORE sentry'):
                execute(bytes.fromhex('600160005500'), {0: 1}, 21000 + 6 + 2300)
            self.assertEqual(execute(bytes.fromhex('600160005500'), {0: 1}, 21000 + 6 + 2301)['terminal'], 0)

    def test_catalog_own_pairs_metadata_and_strict_output(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 16)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-long-revert-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_long_revert.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in SOURCES})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in m.helpers().items()})
            for name in ('storage_root_journal._runtime', 'storage_root_journal.gas_accounting', 'top_level_create.make_case', '_Factory', 'storage_mutation._key', 'storage_mutation._check', 'storage_mutation._require', 'top_level_create.integer'):
                self.assertIn(name, catalog['helper_source_sha256'])
            by = {row['sample_id']: row for row in rows}; self.assertEqual(len(by), 16)
            self.assertEqual(len({row['request_sha256'] for row in rows}), 16)
            self.assertEqual(sum(row['role'] == 'control' for row in rows), 8)
            self.assertEqual({tuple(row['parameters'][key] for key in ('mode', 'account_count', 'aggregate_state_size', 'workload_level')) for row in rows}, set(GRID))
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]
                request_raw = (output / row['request_path']).read_bytes(); request = json.loads(request_raw)
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertEqual(request_raw, (json.dumps(request, indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['sample_id'], f"storage-long-revert-{p['mode']}-u{p['account_count']}-s{p['aggregate_state_size']}-n{p['mutations_per_user']}")
                self.assertEqual(row['baseline_sample_id'], row['sample_id'].rsplit('-n', 1)[0] + '-n0')
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                base = json.loads((output / baseline['request_path']).read_text())['program']
                self.assertEqual([c['initial_storage'] for c in request['program']['prestate_contracts']], [c['initial_storage'] for c in base['prestate_contracts']])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertIn('all16', row['completion_gate'])
                self.assertEqual(row['data_role'], 'unfitted_storage_long_revert_validation')
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for invalid in (True, None, 'A' * 64, '0' * 63, 0):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=invalid)
                self.assertFalse(target.exists())
            for invalid in (None, 'matrix', True):
                with self.assertRaises(ValueError): m.prepare(invalid)
            alternate = Path(temporary) / 'alternate'; m.prepare(alternate, model_id='a' * 64)
            self.assertEqual(json.loads((alternate / 'catalog.json').read_text())['frozen_model_id'], 'a' * 64)

    def test_cli(self):
        with tempfile.TemporaryDirectory() as temporary:
            command = [sys.executable, '-S', str(CASES / 'storage_long_revert.py'), '--output-dir', str(Path(temporary) / 'matrix'), '--model-id', 'b' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 16})
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    def mutant(self, row):
        n = row['parameters']['mutations_per_user']; factor = 2 if row['parameters']['mode'] == 'update-revert' else 3
        for contract in row['request']['program']['prestate_contracts']:
            code = bytearray.fromhex(contract['code_hex'][2:])
            for at, op, value in decode(code):
                if op == 0x62 and value in (2, factor * n): code[at + 1:at + 4] = (3 if value == 2 else (factor + 1) * n).to_bytes(3, 'big')
            contract['code_hex'] = '0x' + code.hex()
        row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()

    def test_oracle_rejects_wrong_semantics_with_matching_terminal(self):
        m = self.load()
        for mode, mutation in itertools.product(MODES, ('owner', 'key', 'warmth', 'refund', 'terminal', 'state', 'phase', 'tail', 'value-checksum', 'recipient', 'signed-success')):
            row = m.make_case(mode); execution = None; program = row['request']['program']
            if mutation in ('owner', 'key', 'warmth', 'refund', 'terminal', 'state'):
                execution = run_users(row); record = execution[0][0]; result = record['result']
                if mutation == 'owner': record['owner'] = address(1)
                elif mutation == 'key': result['reads'][1] = (result['reads'][1][0] + 1,) + result['reads'][1][1:]
                elif mutation == 'warmth': result['reads'][1] = result['reads'][1][:2] + (False,)
                elif mutation == 'refund': result['refund'] += 1
                elif mutation == 'terminal': result['terminal'] = 0
                else: record['state'][65537] = 2
            elif mutation == 'phase': row['parameters']['attempted_final_phase_schedule'][0] += 1
            elif mutation == 'tail': program['prestate_contracts'][0]['initial_storage'][hex(65536 + 4095)] = '0x3'
            elif mutation == 'recipient': program['transactions']['recipients'].reverse()
            elif mutation == 'signed-success': row['request']['expected_user_success'] = True
            else: self.mutant(row)
            if mutation in ('tail', 'recipient', 'value-checksum'):
                self.assertTrue(all(record['result']['terminal'] == 0xfd for record in run_users(row)[0]))
            with self.subTest(mode=mode, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution)

    def test_wrong_domain_and_guard_reach_invalid(self):
        m = self.load()
        for mode, mutation in itertools.product(MODES, ('raw-domain', 'body-guard')):
            row = m.make_case(mode)
            for contract in row['request']['program']['prestate_contracts']:
                code = bytearray.fromhex(contract['code_hex'][2:])
                if mutation == 'raw-domain': code[1:4] = (65536).to_bytes(3, 'big')
                else:
                    instructions = list(decode(code))
                    for index, (_, op, _) in enumerate(instructions[:-1]):
                        at, next_op, value = instructions[index + 1]
                        if op == 0x54 and next_op == 0x62 and value == 1:
                            code[at + 1:at + 4] = (2).to_bytes(3, 'big'); break
                    else: self.fail('body guard missing')
                contract['code_hex'] = '0x' + code.hex()
            row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            execution = run_users(row)
            self.assertTrue(all(record['result']['terminal'] == 0xfe for record in execution[0]))
            with self.assertRaises(AssertionError): qualify(self, row, execution)

    @unittest.skipUnless(os.environ.get('STORAGE_LONG_REVERT_NATIVE_BIN'), 'opt in bounded frozen native smoke')
    def test_frozen_native_smoke(self):
        binary = Path(os.environ['STORAGE_LONG_REVERT_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        m = self.load()
        for mode, mutant in itertools.product(MODES, (False, True)):
            row = m.make_case(mode); p = row['parameters']
            if mutant:
                self.mutant(row)
                with self.assertRaises(AssertionError): qualify(self, row)
            with self.subTest(mode=mode, mutant=mutant), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); source = root / 'request.json'; source.write_text(json.dumps(row['request']))
                process = subprocess.run([str(binary), '--input', str(source), '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'), '--report', str(root / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads((root / 'native.json').read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), 3); self.assertTrue(receipts[0]['success']); self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['success'] for r in receipts[1:]], [False] * 2)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 2)
                diagnostics = report['prestate_contracts']['initialized_storage']
                self.assertEqual([d['address'] for d in diagnostics], [address(i) for i in range(2)])
                for d in diagnostics:
                    self.assertEqual((d['initial_storage_entry_count'], d['final_storage_entry_count']), (4096, 4096))
                    for key in ('initial_storage_root', 'final_storage_root'): self.assertRegex(d[key], r'^0x[0-9a-f]{64}$')
                    self.assertEqual(d['initial_storage_root'], d['final_storage_root'])
                for key in ('initial_storage_root', 'final_storage_root'): self.assertEqual(len({d[key] for d in diagnostics}), 2)


if __name__ == '__main__': unittest.main()
