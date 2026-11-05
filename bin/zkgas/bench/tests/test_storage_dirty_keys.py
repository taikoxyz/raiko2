"""Independent literal trajectory oracle for finite many-key dirty SSTOREs."""
from collections import Counter
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
MODES = ('stop', 'revert')
KEYS = (1, 16, 256, 1024)
LEVELS = ('zero', 'full')
GRID = tuple(itertools.product(MODES, KEYS, LEVELS))
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
SOURCES = ('storage_fanout.py', 'storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def events(test, actual, expected, label):
    """Avoid unittest's quadratic whole-list difference path for large streams."""
    test.assertEqual(len(actual), len(expected), label + ' length')
    for i, (a, e) in enumerate(zip(actual, expected)):
        if a != e: test.fail(f'{label}[{i}]: {a!r} != {e!r}')


def trajectory(test, result, state, *, keys, rounds, mode, account=0):
    start = (account + 1) * 65536 + 1
    initial = {0: start} | dict.fromkeys(range(start, start + keys), 1)
    reads = [(0, start, True)]; writes = []
    for key in range(start, start + keys):
        reads.extend(((key, 1, True), (key, 5, False)))
        writes.append((key, 1, 5, 1, False))
    for key in range(start, start + keys):
        reads.append((key, 5, False))
        for _ in range(rounds):
            writes.extend(((key, 5, 7, 1, False), (key, 7, 5, 1, False)))
        reads.append((key, 5, False))
    for key in range(start, start + keys):
        reads.extend(((key, 5, False), (key, 1, False)))
        writes.append((key, 5, 1, 1, False))
    reads.append((0, start, False))
    events(test, result['reads'], reads, 'ordered reads/raw values/warmth')
    events(test, result['writes'], writes, 'ordered writes/raw originals/warmth')
    test.assertEqual(state, initial, 'complete final contract map')
    test.assertEqual(result['terminal'], 0 if mode == 'stop' else 0xfd)
    test.assertEqual(result['stack'], [])
    test.assertEqual(len(result['memory']), 192)
    test.assertEqual([int.from_bytes(result['memory'][i:i + 32], 'big') for i in (0, 64, 96, 128, 160)],
                     [keys, start, start + keys - 1, rounds, keys * rounds])
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3, 0x54: 0,
             0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)])
                   for op, n in result['counts'].items())
    loads = 2200 + 2600 * keys; stores = keys * (3000 + 200 * rounds)
    gross = 21000 + ordinary + 18 + loads + stores
    before = 2800 * keys; refund = before if mode == 'stop' else 0; applied = min(refund, gross // 5)
    test.assertEqual((result['gas'], result['refund'], result['applied'], result['charged']),
                     (gross, refund, applied, gross - applied))
    if mode == 'revert': test.assertEqual(result['refund_before_revert'], before)
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=18,
        storage_read_gas=loads, storage_write_gas=stores, gross_gas=gross,
        refund_counter_before_terminal=before, refund_counter=refund, refund_applied=applied, charged_gas=gross - applied)


def qualify(test, row, execution=None):
    p = row['parameters']; mode = p['mode']; k = p['keys_per_account']; r = p['rounds_per_key']
    test.assertIn((mode, k, p['workload_level']), GRID)
    test.assertEqual(r, 0 if p['workload_level'] == 'zero' else p['full_rounds_per_key'])
    program = row['request']['program']; contracts = program['prestate_contracts']
    recipients = [address(i) for i in range(4)]
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=4, gas_limit=p['transaction_gas_limit'], recipients=recipients))
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertEqual([c['address'] for c in contracts], recipients)
    test.assertEqual(len({c['code_hex'] for c in contracts}), 1)
    for i, c in enumerate(contracts):
        start = (i + 1) * 65536 + 1
        test.assertEqual({int(key, 16): int(value, 16) for key, value in c['initial_storage'].items()},
                         {0: start} | dict.fromkeys(range(start, start + k), 1))
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual((p['runtime_bytes'], p['runtime_sha256']), (len(code), hashlib.sha256(code).hexdigest()))
    test.assertLessEqual(len(code), 24576)
    test.assertEqual(row['request']['expected_user_success'], mode == 'stop')
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(len(records), 4); routes = Counter(); prefixes = []; spent = 167355
    pairs = set(); write_pairs = set()
    for i, record in enumerate(records):
        test.assertEqual((record['recipient'], record['owner']), (address(i), address(i)))
        result = record['result']; gas = trajectory(test, result, record['state'], keys=k, rounds=r, mode=mode, account=i)
        test.assertEqual(states[address(i)], record['state'])
        test.assertEqual(p['gas_accounting_per_user'], gas)
        for field in ('gross_gas', 'refund_counter_before_terminal', 'refund_counter', 'refund_applied', 'charged_gas'):
            test.assertEqual(p['expected_user_' + field], gas[field], field)
        routes.update(result['counts']); prefixes.append(31_000_000 - spent)
        test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit']); spent += result['charged']
        pairs.update((address(i), event[0]) for event in result['reads'] + result['writes'])
        write_pairs.update((address(i), event[0]) for event in result['writes'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    bases = {address(i): (i + 1) * 65536 for i in range(4)}
    phases = {owner: base + 1 for owner, base in bases.items()}
    fields = dict(account_count=4, user_transactions_per_block=4, all_user_transactions=4,
        all_transactions_including_anchor=5, visits_per_account=1, aggregate_state_size=4 * (k + 1),
        expected_distinct_account_slot_pairs=4 * (k + 1), expected_distinct_write_pairs=4 * k,
        expected_distinct_storage_keys_per_account=k + 1, expected_distinct_body_keys_per_account=k,
        expected_storage_reads=4 * (2 + 6 * k), expected_storage_writes=4 * k * (2 + 2 * r),
        expected_seed_storage_writes=4 * k, expected_dirty_storage_writes=8 * k * r,
        expected_cleanup_storage_writes=4 * k, expected_phase_storage_writes=0,
        expected_main_storage_writes=4 * k * (2 + 2 * r), expected_storage_writes_per_account=k * (2 + 2 * r),
        expected_initial_storage_entry_count=4 * (k + 1), expected_final_storage_entry_count=4 * (k + 1),
        expected_initial_storage_entry_count_per_account=k + 1, expected_final_storage_entry_count_per_account=k + 1,
        expected_counter=k, expected_round_counter=r, expected_marker=k * r, phase_slot=0,
        phase_base_by_account=bases, initial_phase_by_account=phases, expected_final_phase_by_account=phases,
        recipient_schedule=recipients, start_phase_schedule=list(phases.values()),
        final_phase_schedule=list(phases.values()), storage_phase_order=['seed', 'dirty', 'cleanup'],
        body_key_schedule_by_account={owner: list(range(start, start + k)) for owner, start in phases.items()},
        expected_terminal='STOP' if mode == 'stop' else 'REVERT', expected_user_success=mode == 'stop',
        expected_user_receipt_statuses=[mode == 'stop'] * 4, expected_anchor_success=True,
        header_prefix_remaining_gas=prefixes, expected_block_charged_gas=spent, near_full=spent >= 29_700_000,
        counter_memory_offset=0, phase_memory_offset=64, key_memory_offset=96,
        round_counter_memory_offset=128, marker_memory_offset=160, dirty_batch_size=64,
        dirty_batches_per_key=r // 64, dirty_tail_pairs_per_key=r % 64,
        expected_net_main_storage_changes=0, expected_exceptional_instructions=0,
        header_gas_limit=31_000_000, charged_gas_budget=30_000_000, expected_anchor_gas=167355,
        transaction_envelope_reserve=2301, shared_runtime=True, unused_primary_code_hex='0x00')
    for field, value in fields.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(len(pairs), 4 * (k + 1)); test.assertEqual(len(write_pairs), 4 * k)
    test.assertLessEqual(len(pairs), 8192); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(p['transaction_gas_limit'], program['transactions']['gas_limit'])
    test.assertLessEqual(p['transaction_gas_limit'], 15_000_000)
    test.assertLessEqual(p['transaction_gas_limit'], 16_777_216)
    for op in (0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xfa, 0xfe): test.assertEqual(program['expected_route_counts'][f'opcode:0x{op:02x}'], 0)
    return records, states


class StorageDirtyKeysTests(unittest.TestCase):
    def test_capacity_explicit_keys_are_bounded_and_defaults_stay_finite(self):
        m = self.load()
        try: m._validate('stop', 2)
        except ValueError: self.fail('strict explicit keys1..1024 extension is missing')
        self.assertEqual(m.KEY_COUNTS, (1,16,256,1024))
        for k in (1,2,447,448,895,896,1024):
            code, counts = m._runtime('revert', k, 0)
            state = {0:65537} | dict.fromkeys(range(65537,65537+k), 1)
            result = execute(code, state, 15_000_000)
            self.assertEqual(trajectory(self,result,state,keys=k,rounds=0,mode='revert'), m.gas_accounting('revert',k,0))
            self.assertEqual(result['counts'], +Counter(counts))
        for k in (True,False,0,-1,1025,1.0,None,'447'):
            with self.subTest(k=k), self.assertRaises(ValueError): m._validate('stop',k)

    def test_frozen_default_whole_rows_and_requests(self):
        m = self.load()
        frozen = {
            "storage-dirty-keys-revert-k1-r0": {
                "request_sha256": "6025cb1f648f24f081d9e5558a7c1bdbf2a33b198050b2b453d85ac67eeadb40",
                "row_sha256": "25ae71e308a7a7bd045c3c3a8d21afc4f26d62daae253792e93dc354d5b6b8de"
            },
            "storage-dirty-keys-revert-k1-r33942": {
                "request_sha256": "d2614024221157529055d2291a36e62edae8366d6156b55d4397530344d5fe08",
                "row_sha256": "0d5068a3df81a4eb6b9214bc9cdd4c4ede587e8bd4c0e9abc962c17dafb4021a"
            },
            "storage-dirty-keys-revert-k1024-r0": {
                "request_sha256": "1d0d48bf5eee62d777569e1871c674603f0945de62cbc22f6e88b1e69c73b9c7",
                "row_sha256": "60e4b1bf4f3ecc3687df7e87c7e4083c8f75b3685ce7fa875788b7c99e5f2513"
            },
            "storage-dirty-keys-revert-k1024-r5": {
                "request_sha256": "79aa27ecd2ddc4b192fabec2cca62e4e9043f6218e5cd9180e4a707a6610dd87",
                "row_sha256": "4ab734b7fb150041cae1fefc87cc62ae9d51647da61035448003670b03838abf"
            },
            "storage-dirty-keys-revert-k16-r0": {
                "request_sha256": "7542e2b2d582af5bd5fd6ec1231d73e17b312ac727c148433bece47412f0196c",
                "row_sha256": "0f70c99f5d9e2630fa4dffc5099bea29755e35716b6af663a5792dffb2f6db61"
            },
            "storage-dirty-keys-revert-k16-r2095": {
                "request_sha256": "f1f1bd24f8f33bcfef1e61d8371efd81753a49daf82ee15e54dd98557212e246",
                "row_sha256": "935afd59f1a9a0a925fa67639409b9844f844b42d821bc7a011a88b42dc7a166"
            },
            "storage-dirty-keys-revert-k256-r0": {
                "request_sha256": "9aa951643b54638df7965b91ac0d91860a6bb505ecf408b25cb73aeb94585635",
                "row_sha256": "8695f37f4ad6423dd26caf831953942d991e2034bc03a820a23e22a341f91e51"
            },
            "storage-dirty-keys-revert-k256-r104": {
                "request_sha256": "0bf14b43f351379144d384e33b0cedbd4641a35a250ca73ec14789185023a025",
                "row_sha256": "822a17159e6e12159ea69ff07ae06799821cc5936dfafaef7eac37d5d391d9d7"
            },
            "storage-dirty-keys-stop-k1-r0": {
                "request_sha256": "651e27507ef347d491d5ecf67e8527773e0562f4d41a9205ee65049126a2d31c",
                "row_sha256": "d047127e57c5f1fb3f25c8f69a59db9e643955d4f0bf579b3f87eb9e045190d3"
            },
            "storage-dirty-keys-stop-k1-r33955": {
                "request_sha256": "8544d9542c87c2fb0ddbe307f9ca8373d8aafaab6951fab56a4a1003aeac9cda",
                "row_sha256": "09b4a3cc28bef4bd68d81a15d1afd674a1eb05d24eecf90c68ed412617eb8d6f"
            },
            "storage-dirty-keys-stop-k1024-r0": {
                "request_sha256": "bb75eb4d586b82c89ba157d4608e314fe6a7e2c3e66856c078177400c417949c",
                "row_sha256": "1f29f9a10f4cbe80002493993c29306f2bfc1a75947d3924c2ee41d293c1be4c"
            },
            "storage-dirty-keys-stop-k1024-r12": {
                "request_sha256": "db07ca79e7bbdb09c082e91440c81ace5cb005922fe165a1a952a6c5f684a6ee",
                "row_sha256": "4ae2fb43d18498c7b7ab1e158f628696bfd748acb45e117d160f4d6d68210290"
            },
            "storage-dirty-keys-stop-k16-r0": {
                "request_sha256": "14d2bcdc7a26691cd8278bc366064eb1e69d7ba597829753e9206c6de4eb2fc3",
                "row_sha256": "2885bb6d09c71180300be3a27858e822d0006eb8120fad05a6434fcd9e91c87f"
            },
            "storage-dirty-keys-stop-k16-r2108": {
                "request_sha256": "1618e14e2cbec2b7dbadad9e907b924dd798dbd7e3be068ade71e99e2dedc0d7",
                "row_sha256": "68d9945dee4f11bb52d34ec20eb4e1a726b548761cb1ae4d0cf81b21d74deb12"
            },
            "storage-dirty-keys-stop-k256-r0": {
                "request_sha256": "a74899e324cc0f882ef30895785a5d4e1900309bd81bc5803acbdd15a65ee77f",
                "row_sha256": "8d0faa382a9543622af7dbb0ea1622833edd41ac1cf1b6185f466861a2acd34b"
            },
            "storage-dirty-keys-stop-k256-r117": {
                "request_sha256": "76c10aa9fb55bd41c7c8430108aff2e4879e75c8adf6b62820b892cdebe5baa6",
                "row_sha256": "2b931510a713911724f89d3cc6dd3375dd681b5a9b4e77cd0dc2af72fec11efd"
            }
        }
        for args in GRID:
            row = m.make_case(*args); expected = frozen[row['sample_id']]
            self.assertEqual(m.digest(m.encoded(row)), expected['row_sha256'])
            self.assertEqual(m.digest(m.encoded(row['request'])), expected['request_sha256'])

    def load(self):
        path = CASES / 'storage_dirty_keys.py'
        self.assertTrue(path.is_file(), 'many-key dirty-storage producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_dirty_keys_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_all_16_actual_byte_trajectories(self):
        m = self.load()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(args[0], args[1], p['full_rounds_per_key'])['gross_gas'] + 2301)

    def test_batch_boundaries_and_actual_r_plus_one_maximality(self):
        m = self.load()
        for mode, r in itertools.product(MODES, (0, 1, 63, 64, 65)):
            code, counts = m._runtime(mode, 1, r); state = {0: 65537, 65537: 1}
            result = execute(code, state, 20_000_000)
            self.assertEqual(trajectory(self, result, state, keys=1, rounds=r, mode=mode), m.gas_accounting(mode, 1, r))
            self.assertEqual(result['counts'], +Counter(counts))
        for mode, k in itertools.product(MODES, KEYS):
            r = m.full_rounds(mode, k); self.assertTrue(0 < r < 65536); self.assertTrue(m.fits(mode, k, r))
            code, _ = m._runtime(mode, k, r + 1); state = {0: 65537} | dict.fromkeys(range(65537, 65537 + k), 1)
            result = execute(code, state, 20_000_000)
            gas = trajectory(self, result, state, keys=k, rounds=r + 1, mode=mode)
            self.assertEqual(gas, m.gas_accounting(mode, k, r + 1)); envelope = gas['gross_gas'] + 2301
            self.assertTrue(any((envelope > 15_000_000, envelope > 16_777_216,
                167355 + 4 * gas['charged_gas'] > 30_000_000,
                any(31_000_000 - 167355 - j * gas['charged_gas'] < envelope for j in range(4)))))
            self.assertFalse(m.fits(mode, k, r + 1))
        with self.assertRaisesRegex(AssertionError, 'SSTORE sentry'):
            execute(bytes.fromhex('600160005500'), {0: 1}, 21000 + 6 + 2300)
        self.assertEqual(execute(bytes.fromhex('600160005500'), {0: 1}, 21000 + 6 + 2301)['terminal'], 0)

    def test_actual_byte_guards_reject_skipped_batch_tail_and_raw_domain(self):
        m = self.load()
        for mode, mutation in itertools.product(MODES, ('batch', 'tail', 'domain')):
            code, _ = m._runtime(mode, 1, 65); wrong = bytearray(code)
            if mutation == 'batch':
                # Skip the64-pair body. The final round guard sees only the tail.
                at = code.index(bytes.fromhex('62000080516200004014')) + 5
                wrong[at + 1:at + 4] = bytes(3)
            elif mutation == 'tail':
                # Execute the tail but omit its actual counter increment.
                at = code.index(bytes.fromhex('620000805162000001016200008052')) + 5
                wrong[at + 1:at + 4] = bytes(3)
            else:
                # Read phase from an absent domain rather than raw slot0.
                wrong[1:4] = (65536).to_bytes(3, 'big')
            result = execute(wrong, {0: 65537, 65537: 1}, 20_000_000)
            with self.subTest(mode=mode, mutation=mutation): self.assertEqual(result['terminal'], 0xfe)

    def test_strict_inputs(self):
        m = self.load()
        for args in ((None,), (True,), ('STOP',), ('stop', True), ('stop', 1.0), ('stop', 1025), ('stop', 1, True), ('stop', 1, 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for function in (m.gas_accounting, m.fits, m._runtime):
            for r in (True, -1, 1.0, 65537, None):
                with self.assertRaises(ValueError): function('stop', 1, r)
            for mode, k in ((None, 1), ('STOP', 1), ('stop', True), ('stop', 1025)):
                with self.assertRaises(ValueError): function(mode, k, 0)
        for mode, k in ((None, 1), ('revert', True), ('stop', 1025)):
            with self.assertRaises(ValueError): m.full_rounds(mode, k)
        self.assertEqual(m.make_case(), m.make_case('stop', 1, 'full'))
        self.assertIsInstance(m.fits('stop', 1, 65536), bool)

    def test_catalog_pairs_provenance_and_cli(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 16)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']; by = {row['sample_id']: row for row in rows}
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-dirty-keys-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_dirty_keys.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in SOURCES})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in m.helpers().items()})
            for name in ('_Factory', 'storage_mutation._key', 'storage_mutation._check', 'storage_mutation._require', 'top_level_create.make_case', 'top_level_create.integer', 'storage_dirty_keys._runtime', 'storage_dirty_keys.gas_accounting', 'storage_dirty_keys.fits', 'storage_dirty_keys.full_rounds'):
                self.assertIn(name, catalog['helper_source_sha256'])
            self.assertEqual(len(by), 16); self.assertEqual(len({r['request_sha256'] for r in rows}), 16)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 8)
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]
                request_raw = (output / row['request_path']).read_bytes(); request = json.loads(request_raw)
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertEqual(request_raw, (json.dumps(request, indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['sample_id'], f"storage-dirty-keys-{p['mode']}-k{p['keys_per_account']}-r{p['rounds_per_key']}")
                self.assertEqual(row['baseline_sample_id'], row['sample_id'].rsplit('-r', 1)[0] + '-r0')
                base = json.loads((output / baseline['request_path']).read_bytes())['program']
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                self.assertEqual([c['initial_storage'] for c in request['program']['prestate_contracts']], [c['initial_storage'] for c in base['prestate_contracts']])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertIn('all16', row['completion_gate'])
                self.assertEqual(row['data_role'], 'unfitted_storage_dirty_keys_validation')
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for bad in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())
            for bad in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(bad)
            command = [sys.executable, '-S', str(CASES / 'storage_dirty_keys.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 16})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    def mutant(self, row):
        for contract in row['request']['program']['prestate_contracts']:
            code = bytearray.fromhex(contract['code_hex'][2:])
            for at, op, value in decode(code):
                if op == 0x62 and value == 7: code[at + 1:at + 4] = (9).to_bytes(3, 'big')
            contract['code_hex'] = '0x' + code.hex()
        row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()

    def test_semantic_negatives(self):
        m = self.load()
        for mutation in ('owner', 'key', 'warmth', 'refund', 'terminal', 'state', 'phase-write', 'access-metadata', 'write-metadata', 'domain', 'recipient', 'marker', 'batch', 'tail', 'dirty-value'):
            row = m.make_case('stop', 1); execution = None
            if mutation in ('owner', 'key', 'warmth', 'refund', 'terminal', 'state', 'phase-write', 'batch', 'tail'):
                execution = run_users(row); record = execution[0][0]; result = record['result']
                if mutation == 'owner': record['owner'] = address(1)
                elif mutation == 'key': result['writes'][1] = (65538,) + result['writes'][1][1:]
                elif mutation == 'warmth': result['reads'][1] = result['reads'][1][:2] + (False,)
                elif mutation == 'refund': result['refund'] += 1
                elif mutation == 'terminal': result['terminal'] = 0xfd
                elif mutation == 'state': record['state'][65537] = 5
                elif mutation == 'phase-write': result['writes'].append((0, 65537, 65537, 65537, False))
                else: del result['writes'][1:129 if mutation == 'batch' else 3]
            elif mutation in ('access-metadata', 'write-metadata'): row['parameters']['expected_distinct_account_slot_pairs' if mutation == 'access-metadata' else 'expected_distinct_write_pairs'] += 1
            elif mutation == 'domain': row['request']['program']['prestate_contracts'][0]['initial_storage']['0x10002'] = '0x1'
            elif mutation == 'recipient': row['request']['program']['transactions']['recipients'].reverse()
            elif mutation == 'dirty-value': self.mutant(row)
            else:
                for contract in row['request']['program']['prestate_contracts']:
                    code = bytearray.fromhex(contract['code_hex'][2:])
                    for at, op, value in decode(code):
                        if op == 0x63: code[at + 1:at + 5] = (value + 1).to_bytes(4, 'big')
                    contract['code_hex'] = '0x' + code.hex()
                row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution)

    @unittest.skipUnless(os.environ.get('STORAGE_DIRTY_KEYS_NATIVE_BIN'), 'opt in four frozen native specimens')
    def test_frozen_native_specimens(self):
        binary = Path(os.environ['STORAGE_DIRTY_KEYS_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        m = self.load()
        for (mode, k), mutant in itertools.product((('stop', 1), ('revert', 1024)), (False, True)):
            row = m.make_case(mode, k); p = row['parameters']
            if mutant:
                self.mutant(row)
                with self.assertRaises(AssertionError): qualify(self, row)
            with self.subTest(mode=mode, keys=k, mutant=mutant), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); source = root / 'request.json'; source.write_text(json.dumps(row['request']))
                process = subprocess.run([str(binary), '--input', str(source), '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'), '--report', str(root / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads((root / 'native.json').read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), 5); self.assertTrue(receipts[0]['success']); self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['success'] for r in receipts[1:]], [mode == 'stop'] * 4)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 4)
                diagnostics = report['prestate_contracts']['initialized_storage']
                self.assertEqual([d['address'] for d in diagnostics], [address(i) for i in range(4)])
                for d in diagnostics:
                    self.assertEqual((d['initial_storage_entry_count'], d['final_storage_entry_count']), (k + 1, k + 1))
                    self.assertRegex(d['initial_storage_root'], r'^0x[0-9a-f]{64}$')
                    self.assertEqual(d['initial_storage_root'], d['final_storage_root'])
                self.assertEqual(len({d['initial_storage_root'] for d in diagnostics}), 4)


if __name__ == '__main__': unittest.main()
