"""Literal byte and transient-lifecycle checks for the bounded eight-input panel."""
from collections import Counter
from contextlib import nullcontext
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

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('stop', 'revert'), (16, 4096), ('zero', 'full')))
OWNER = '0x3000000000000000000000000000000000010000'
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'transient_storage.py'
    assert path.is_file(), 'transient producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('transient_storage_cases', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    assert Path(module.__file__).resolve() == path.resolve()
    return module


def oracle():
    from transient_storage_vm import decode, execute, run_users
    return decode, execute, run_users


def events(test, actual, expected, label):
    test.assertEqual(len(actual), len(expected), label + ' length')
    for index, (observed, wanted) in enumerate(zip(actual, expected)):
        if observed != wanted: test.fail(f'{label}[{index}]: {observed!r} != {wanted!r}')


def trajectory(test, result, *, keys, rounds, mode, owner=OWNER):
    reads = [(0, 0)]; writes = [(0, 0, 1)]
    for key in range(1, keys + 1):
        reads.extend(((key, 0), (key, 5))); writes.append((key, 0, 5))
    for key in range(1, keys + 1):
        reads.append((key, 5))
        for _ in range(rounds): writes.extend(((key, 5, 7), (key, 7, 5)))
        reads.append((key, 5))
    for key in range(1, keys + 1):
        reads.extend(((key, 5), (key, 0))); writes.append((key, 5, 0))
    reads.append((0, 1))
    events(test, result['reads'], reads, 'attempted TLOAD values')
    events(test, result['writes'], writes, 'attempted TSTORE old/new values')
    events(test, result['journal'], [(owner, key, old, new) for key, old, new in writes], 'owner-qualified journal')
    test.assertEqual(result['terminal'], 0 if mode == 'stop' else 0xfd)
    test.assertEqual(result['stack'], [])
    test.assertEqual(len(result['memory']), 192)
    test.assertEqual([int.from_bytes(result['memory'][i:i + 32], 'big') for i in (0, 64, 96, 128, 160)],
                     [keys, 1, keys, rounds, keys * rounds])
    test.assertEqual(result['initial_transient'], {})
    test.assertEqual(result['frame_transient'], {(owner, 0): 1} if mode == 'stop' else {})
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3,
             0x5c: 0, 0x5d: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)])
                   for op, n in result['counts'].items())
    loads = 100 * (2 + 6 * keys); stores = 100 * (1 + keys * (2 + 2 * rounds))
    gross = 21000 + ordinary + 18 + loads + stores
    test.assertEqual((result['gas'], result['refund'], result['applied'], result['charged']), (gross, 0, 0, gross))
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=18,
        transient_read_gas=loads, transient_write_gas=stores, gross_gas=gross,
        refund_counter_before_terminal=0, refund_counter=0, refund_applied=0, charged_gas=gross)


def qualify(test, row, execution=None):
    p = row['parameters']; mode = p['mode']; k = p['keys_per_account']; r = p['rounds_per_key']
    test.assertIn((mode, k, p['workload_level']), GRID)
    test.assertEqual(r, 0 if p['workload_level'] == 'zero' else p['full_rounds_per_key'])
    program = row['request']['program']; contracts = program['prestate_contracts']
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=4,
        gas_limit=p['transaction_gas_limit'], recipients=[OWNER] * 4))
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertEqual(len(contracts), 1); test.assertEqual(contracts[0]['address'], OWNER)
    test.assertEqual(contracts[0]['initial_storage'], {})
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual((p['runtime_bytes'], p['runtime_sha256']), (len(code), hashlib.sha256(code).hexdigest()))
    test.assertLessEqual(len(code), 24576)
    test.assertEqual(row['request']['expected_user_success'], mode == 'stop')
    _, _, run_users = oracle(); records = run_users(row) if execution is None else execution
    test.assertEqual(len(records), 4); routes = Counter(); prefixes = []; spent = 167355; pairs = set()
    for record in records:
        test.assertEqual((record['recipient'], record['owner']), (OWNER, OWNER))
        result = record['result']; gas = trajectory(test, result, keys=k, rounds=r, mode=mode)
        test.assertEqual(record['after_finalize'], {}, 'per-transaction transient clear')
        test.assertEqual(p['gas_accounting_per_user'], gas)
        for field in ('gross_gas', 'refund_counter_before_terminal', 'refund_counter', 'refund_applied', 'charged_gas'):
            test.assertEqual(p['expected_user_' + field], gas[field], field)
        routes.update(result['counts']); prefixes.append(31_000_000 - spent)
        test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit']); spent += result['charged']
        attempted = {(record['owner'], key) for key, _ in result['reads']}
        test.assertEqual(attempted, {(OWNER, key) for key in range(k + 1)})
        test.assertEqual(record['attempted_pairs'], attempted); pairs.update(attempted)
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    fields = dict(account_count=1, user_transactions_per_block=4, all_user_transactions=4,
        all_transactions_including_anchor=5, visits_per_account=4, aggregate_state_size=0,
        state_size_per_account=0, peak_transient_state_entries_per_transaction=k + 1,
        expected_distinct_transient_owner_slot_pairs=k + 1, expected_transient_reads=4 * (2 + 6 * k),
        expected_transient_writes=4 * (1 + k * (2 + 2 * r)), expected_seed_transient_writes=4 * k,
        expected_dirty_transient_writes=8 * k * r, expected_cleanup_transient_writes=4 * k,
        expected_sentinel_transient_writes=4, expected_transient_writes_per_user=1 + k * (2 + 2 * r),
        expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0,
        expected_counter=k, expected_round_counter=r, expected_marker=k * r,
        recipient_schedule=[OWNER] * 4, transient_phase_order=['seed', 'dirty', 'cleanup'],
        body_key_schedule=list(range(1, k + 1)), expected_terminal='STOP' if mode == 'stop' else 'REVERT',
        expected_user_success=mode == 'stop', expected_user_receipt_statuses=[mode == 'stop'] * 4,
        expected_anchor_success=True, header_prefix_remaining_gas=prefixes, expected_block_charged_gas=spent,
        near_full=spent >= 29_700_000, counter_memory_offset=0, phase_memory_offset=64, key_memory_offset=96,
        round_counter_memory_offset=128, marker_memory_offset=160, dirty_batch_size=64,
        dirty_batches_per_key=r // 64, dirty_tail_pairs_per_key=r % 64,
        expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
        expected_exceptional_instructions=0, header_gas_limit=31_000_000, charged_gas_budget=30_000_000,
        expected_anchor_gas=167355, transaction_envelope_reserve=1000, shared_runtime=True,
        unused_primary_code_hex='0x00', execution_profile='evm-gas-only')
    for field, value in fields.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(len(pairs), k + 1); test.assertLessEqual(spent, 30_000_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    test.assertEqual(p['transaction_gas_limit'], program['transactions']['gas_limit'])
    for op in (0x54, 0x55, 0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xfa, 0xfe):
        test.assertEqual(program['expected_route_counts'].get(f'opcode:0x{op:02x}', 0), 0)
    return records


def dirty_nine(row):
    decode, _, _ = oracle(); contract = row['request']['program']['prestate_contracts'][0]
    code = bytearray.fromhex(contract['code_hex'][2:])
    for at, op, value in decode(code):
        if op == 0x62 and value == 7: code[at + 1:at + 4] = (9).to_bytes(3, 'big')
    contract['code_hex'] = '0x' + code.hex(); row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()


class TransientStorageTests(unittest.TestCase):
    def test_all_eight_literal_trajectories(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(args[0], args[1], p['full_rounds_per_key'])['gross_gas'] + 1000)

    def test_batch_and_full_plus_one_boundaries(self):
        m = producer(); _, execute, _ = oracle()
        for mode, r in itertools.product(('stop', 'revert'), (0, 1, 63, 64, 65)):
            code, counts = m._runtime(mode, 16, r); result = execute(code, 20_000_000, owner=OWNER)
            self.assertEqual(trajectory(self, result, keys=16, rounds=r, mode=mode), m.gas_accounting(mode, 16, r))
            self.assertEqual(result['counts'], +Counter(counts))
        for mode, k in itertools.product(('stop', 'revert'), (16, 4096)):
            self.assertTrue(m.fits(mode, k, 0)); r = m.full_rounds(mode, k)
            self.assertTrue(0 < r < 65536); self.assertTrue(m.fits(mode, k, r))
            code, counts = m._runtime(mode, k, r + 1); result = execute(code, 20_000_000, owner=OWNER)
            gas = trajectory(self, result, keys=k, rounds=r + 1, mode=mode)
            self.assertEqual(gas, m.gas_accounting(mode, k, r + 1)); self.assertEqual(result['counts'], +Counter(counts))
            envelope = gas['gross_gas'] + 1000
            self.assertTrue(any((envelope > 15_000_000, envelope > 16_777_216,
                167355 + 4 * gas['charged_gas'] > 30_000_000,
                any(31_000_000 - 167355 - j * gas['charged_gas'] < envelope for j in range(4)), len(code) > 24576)))
            self.assertFalse(m.fits(mode, k, r + 1))

    def test_tstore_cost_boundary_owner_domain_and_frame_rollback(self):
        _, execute, _ = oracle(); code = bytes.fromhex('600160005d00')
        with self.assertRaisesRegex(AssertionError, 'out of gas'): execute(code, 21105, owner=OWNER)
        state = {}; result = execute(code, 21106, owner=OWNER, transient=state)
        self.assertEqual(result['gas'], 21106); self.assertEqual(state, {(OWNER, 0): 1})
        other = OWNER[:-1] + '1'
        self.assertEqual(execute(bytes.fromhex('60005c00'), 22000, owner=other, transient=state)['reads'], [(0, 0)])
        before = state.copy(); result = execute(bytes.fromhex('600260005d60006000fd'), 22000, owner=OWNER, transient=state)
        self.assertEqual(state, before); self.assertEqual(result['writes'], [(0, 1, 2)])
        self.assertEqual(result['journal'], [(OWNER, 0, 1, 2)]); self.assertEqual(result['terminal'], 0xfd)

    def test_exceptional_frame_restores_state_and_invalid_consumes_envelope(self):
        _, execute, _ = oracle(); state = {(OWNER, 9): 3}; before = state.copy()
        result = execute(bytes.fromhex('600160005dfe'), 22000, owner=OWNER, transient=state)
        self.assertEqual(state, before); self.assertEqual(result['frame_transient'], before)
        self.assertEqual(result['gas'], 22000); self.assertEqual(result['terminal'], 0xfe)
        self.assertEqual(result['writes'], [(0, 0, 1)])
        with self.assertRaisesRegex(AssertionError, 'out of gas'):
            execute(bytes.fromhex('600160005d60015c00'), 21106, owner=OWNER, transient=state)
        self.assertEqual(state, before)

    def test_strict_domains(self):
        m = producer()
        for args in ((None,), (True,), ('STOP',), ('stop', True), ('stop', 16.0), ('stop', 1), ('stop', 16, True), ('stop', 16, 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for function in (m.gas_accounting, m.fits, m._runtime):
            for r in (True, -1, 1.0, 65537, None):
                with self.assertRaises(ValueError): function('stop', 16, r)
            for mode, k in ((None, 16), ('STOP', 16), ('stop', True), ('stop', 1)):
                with self.assertRaises(ValueError): function(mode, k, 0)
        for mode, k in ((None, 16), ('revert', True), ('stop', 1)):
            with self.assertRaises(ValueError): m.full_rounds(mode, k)
        self.assertEqual(m.make_case(), m.make_case('stop', 16, 'full'))
        self.assertIsInstance(m.fits('stop', 16, 65536), bool)

    def test_runtime_guards_reject_marker_seed_post_counter_key(self):
        m = producer(); decode, execute, _ = oracle()
        code, _ = m._runtime('stop', 16, 65)
        for name in ('entry', 'seed', 'post', 'counter', 'key', 'batch', 'tail', 'marker'):
            wrong = bytearray(code)
            if name == 'entry':
                at = code.index(bytes.fromhex('620000005c6200000014')) + 5
                wrong[at + 1:at + 4] = (1).to_bytes(3, 'big')
            elif name in ('seed', 'post'):
                pattern = bytes.fromhex('62000060515c62' + ('000000' if name == 'seed' else '000005') + '14')
                at = code.index(pattern) + 6; wrong[at + 1:at + 4] = (2).to_bytes(3, 'big')
            elif name == 'counter':
                at = code.index(bytes.fromhex('62000000516200000101')) + 5
                wrong[at + 1:at + 4] = (2).to_bytes(3, 'big')
            elif name == 'key':
                at = code.index(bytes.fromhex('6200004051620000005101'))
                wrong[at + 1:at + 4] = (32).to_bytes(3, 'big')
            elif name == 'batch':
                at = code.index(bytes.fromhex('62000080516200004014')) + 5
                wrong[at + 1:at + 4] = bytes(3)
            elif name == 'tail':
                at = code.index(bytes.fromhex('620000805162000001016200008052')) + 5
                wrong[at + 1:at + 4] = bytes(3)
            else:
                at = next(at for at, op, _ in decode(code) if op == 0x63)
                wrong[at + 1:at + 5] = (1041).to_bytes(4, 'big')
            with self.subTest(name=name): self.assertEqual(execute(wrong, 20_000_000, owner=OWNER)['terminal'], 0xfe)

    def test_semantic_negative_evidence(self):
        m = producer(); _, _, run_users = oracle()
        for mutation in ('owner', 'key', 'value', 'terminal', 'gas', 'reset', 'fd-retention', 'marker', 'counter', 'metadata', 'domain', 'recipient', 'dirty-nine'):
            mode = 'revert' if mutation == 'fd-retention' else 'stop'; row = m.make_case(mode, 16)
            execution = None
            if mutation in ('owner', 'key', 'value', 'terminal', 'gas', 'reset', 'fd-retention', 'marker', 'counter'):
                execution = run_users(row); record = execution[0]; result = record['result']
                if mutation == 'owner': record['owner'] = OWNER[:-1] + '1'
                elif mutation == 'key': result['writes'][1] = (17, 0, 5)
                elif mutation == 'value': result['writes'][1] = (1, 0, 6)
                elif mutation == 'terminal': result['terminal'] = 0xfd
                elif mutation == 'gas': result['gas'] += 1
                elif mutation == 'reset': record['after_finalize'][(OWNER, 0)] = 1
                elif mutation == 'fd-retention': result['journal'].clear()
                else:
                    offset = 160 if mutation == 'marker' else 0
                    result['memory'][offset:offset + 32] = (1).to_bytes(32, 'big')
            elif mutation == 'metadata': row['parameters']['expected_distinct_transient_owner_slot_pairs'] *= 4
            elif mutation == 'domain': row['request']['program']['prestate_contracts'][0]['initial_storage']['0x1'] = '0x5'
            elif mutation == 'recipient': row['request']['program']['transactions']['recipients'][1] = OWNER[:-1] + '1'
            else: dirty_nine(row)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution)
        code = bytes.fromhex(m.make_case('stop', 16, 'zero')['request']['program']['prestate_contracts'][0]['code_hex'][2:])
        _, execute, _ = oracle(); state = {}
        self.assertEqual(execute(code, 20_000_000, owner=OWNER, transient=state)['terminal'], 0)
        self.assertEqual(execute(code, 20_000_000, owner=OWNER, transient=state)['terminal'], 0xfe)

    def test_catalog_cli_source_and_helpers(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 8)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(catalog['corpus_id'], 'unzen-transient-storage-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL); self.assertEqual(catalog['users'], 4)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'transient_storage.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() for name, module in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in m.helpers().items()})
            for name in ('_Factory', 'storage_mutation._key', 'storage_mutation._check', 'storage_mutation._require', 'top_level_create.make_case', 'top_level_create.integer', 'transient_storage._runtime', 'transient_storage.gas_accounting', 'transient_storage.fits', 'transient_storage.full_rounds'):
                self.assertIn(name, catalog['helper_source_sha256'])
            self.assertEqual(len(by), 8); self.assertEqual(len({r['request_sha256'] for r in rows}), 8)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 4)
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]
                self.assertEqual(p['aggregate_state_size'], 0)
                self.assertEqual(p['state_size_per_account'], 0)
                self.assertEqual(p['peak_transient_state_entries_per_transaction'], p['keys_per_account'] + 1)
                request_raw = (output / row['request_path']).read_bytes(); request = json.loads(request_raw)
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertEqual(request_raw, (json.dumps(request, indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['sample_id'], f"transient-storage-{p['mode']}-k{p['keys_per_account']}-r{p['rounds_per_key']}")
                self.assertEqual(row['baseline_sample_id'], row['sample_id'].rsplit('-r', 1)[0] + '-r0')
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertIn('all8', row['completion_gate']); self.assertEqual(row['data_role'], 'unfitted_transient_storage_validation')
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for bad in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())
            for bad in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(bad)
            command = [sys.executable, '-S', str(CASES / 'transient_storage.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 8})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    @unittest.skipUnless(os.environ.get('TRANSIENT_STORAGE_NATIVE_BIN'), 'set TRANSIENT_STORAGE_NATIVE_BIN to run optional native smoke')
    def test_frozen_native_positive_and_dirty_nine(self):
        binary = Path(os.environ['TRANSIENT_STORAGE_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        retained = os.environ.get('TRANSIENT_STORAGE_NATIVE_OUTPUT_DIR')
        context = nullcontext(retained) if retained else tempfile.TemporaryDirectory()
        with context as temporary:
            base = Path(temporary)
            if retained: base.mkdir(parents=True, exist_ok=False)
            for mutant in (False, True):
                row = producer().make_case('stop', 16); p = row['parameters']
                if mutant:
                    dirty_nine(row)
                    with self.assertRaises(AssertionError): qualify(self, row)
                root = base / ('dirty-nine' if mutant else 'positive'); root.mkdir()
                source = root / 'request.json'; source.write_text(json.dumps(row['request']))
                command = [str(binary), '--input', str(source), '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'), '--report', str(root / 'native.json'), '--execution-profile', 'evm-gas-only']
                (root / 'command.json').write_text(json.dumps(command, indent=2) + '\n')
                try:
                    process = subprocess.run(command, capture_output=True, timeout=180)
                except subprocess.TimeoutExpired as error:
                    (root / 'stdout.log').write_bytes(error.stdout or b'')
                    (root / 'stderr.log').write_bytes(error.stderr or b'')
                    (root / 'process.json').write_text(json.dumps({'timeout_seconds': 180}) + '\n')
                    raise
                (root / 'stdout.log').write_bytes(process.stdout); (root / 'stderr.log').write_bytes(process.stderr)
                (root / 'process.json').write_text(json.dumps({'exit_code': process.returncode}) + '\n')
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads((root / 'native.json').read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(report['execution_profile'], 'evm-gas-only')
                expected = +Counter(row['request']['program']['expected_route_counts'])
                for field in ('observed_route_counts', 'attempted_route_counts'):
                    self.assertEqual(+Counter({key: value for key, value in report[field].items() if key.startswith('opcode:')}), expected)
                self.assertEqual(len(receipts), 5); self.assertTrue(receipts[0]['success']); self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['success'] for r in receipts[1:]], [True] * 4)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 4)
                diagnostics = report['prestate_contracts']['initialized_storage']
                self.assertEqual(len(diagnostics), 1); self.assertEqual(diagnostics[0]['address'], OWNER)
                self.assertEqual((diagnostics[0]['initial_storage_entry_count'], diagnostics[0]['final_storage_entry_count']), (0, 0))
                self.assertEqual(diagnostics[0]['initial_storage_root'], diagnostics[0]['final_storage_root'])


if __name__ == '__main__': unittest.main()
