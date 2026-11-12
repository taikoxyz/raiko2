"""Independent literal trajectory oracle for finite high-limb dirty SSTOREs."""
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
KEYS = (16, 1024)
SHIFTS = (64, 128, 192)
LEVELS = ('zero', 'full')
GRID = tuple(itertools.product(MODES, KEYS, SHIFTS, LEVELS))
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
SOURCES = ('storage_dirty_keys.py', 'storage_fanout.py', 'storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def events(test, actual, expected, label):
    """Avoid unittest's quadratic whole-list difference path for large streams."""
    test.assertEqual(len(actual), len(expected), label + ' length')
    for i, (a, e) in enumerate(zip(actual, expected)):
        if a != e: test.fail(f'{label}[{i}]: {a!r} != {e!r}')


def trajectory(test, result, state, *, keys, rounds, mode, shift, account=0):
    start = (account + 1) * 65536 + 1
    initial = {0: start} | dict.fromkeys((start + (j << shift) for j in range(keys)), 1)
    reads = [(0, start, True)]; writes = []
    for key in (start + (j << shift) for j in range(keys)):
        reads.extend(((key, 1, True), (key, 5, False)))
        writes.append((key, 1, 5, 1, False))
    for key in (start + (j << shift) for j in range(keys)):
        reads.append((key, 5, False))
        for _ in range(rounds):
            writes.extend(((key, 5, 7, 1, False), (key, 7, 5, 1, False)))
        reads.append((key, 5, False))
    for key in (start + (j << shift) for j in range(keys)):
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
                     [keys, start, start + ((keys - 1) << shift), rounds, keys * rounds])
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x1b: 3, 0x51: 3, 0x52: 3, 0x54: 0,
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
    p = row['parameters']; mode = p['mode']; k = p['keys_per_account']; r = p['rounds_per_key']; shift = p['slot_shift']
    test.assertIn((mode, k, shift, p['workload_level']), GRID)
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
                         {0: start} | dict.fromkeys((start + (j << shift) for j in range(k)), 1))
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual((p['runtime_bytes'], p['runtime_sha256']), (len(code), hashlib.sha256(code).hexdigest()))
    test.assertLessEqual(len(code), 24576)
    test.assertEqual(row['request']['expected_user_success'], mode == 'stop')
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(len(records), 4); routes = Counter(); prefixes = []; spent = 167355
    pairs = set(); write_pairs = set()
    for i, record in enumerate(records):
        test.assertEqual((record['recipient'], record['owner']), (address(i), address(i)))
        result = record['result']; gas = trajectory(test, result, record['state'], keys=k, rounds=r, mode=mode, shift=shift, account=i)
        test.assertEqual(states[address(i)], record['state'])
        test.assertEqual(p['gas_accounting_per_user'], gas)
        for field in ('gross_gas', 'refund_counter_before_terminal', 'refund_counter', 'refund_applied', 'charged_gas'):
            test.assertEqual(p['expected_user_' + field], gas[field], field)
        routes.update(result['counts']); prefixes.append(31_000_000 - spent)
        test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit']); spent += result['charged']
        pairs.update((address(i), event[0]) for event in result['reads'] + result['writes'])
        write_pairs.update((address(i), event[0]) for event in result['writes'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    bases = {address(i): hex((i + 1) * 65536) for i in range(4)}
    phases = {owner: hex(int(base, 16) + 1) for owner, base in bases.items()}
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
        expected_counter=k, expected_round_counter=r, expected_marker=k * r, phase_slot='0x0',
        phase_base_by_account=bases, initial_phase_by_account=phases, expected_final_phase_by_account=phases,
        recipient_schedule=recipients, start_phase_schedule=list(phases.values()),
        final_phase_schedule=list(phases.values()), storage_phase_order=['seed', 'dirty', 'cleanup'],
        body_key_schedule_by_account={owner: [hex(int(start, 16) + (j << shift)) for j in range(k)] for owner, start in phases.items()},
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



# Immutable pre-extension request/runtime/core metadata hashes; never regenerated by tests.
GOLDENS = {'storage-dirty-keys-stop-k1-r0': ('651e27507ef347d491d5ecf67e8527773e0562f4d41a9205ee65049126a2d31c',
                                   '06d6161fd34471f5d5d0fa335bc20e9772a9b60cfde3dfa6b0fcb69d47c6c111',
                                   'eb35097b751d1f14fb08fb0f97b15daf61c30fa1d29cbe7b3bb1cdd855e9320b'),
 'storage-dirty-keys-stop-k1-r33955': ('8544d9542c87c2fb0ddbe307f9ca8373d8aafaab6951fab56a4a1003aeac9cda',
                                       'b60976e8e0e1df84a1d9f08b46cfef7fa3fa49d095ed683328d417e135df7fd3',
                                       '1716a19812c86ff32f2fb4d3e3e277b0bac0e066ab8c551d44e32a7556213641'),
 'storage-dirty-keys-stop-k16-r0': ('14d2bcdc7a26691cd8278bc366064eb1e69d7ba597829753e9206c6de4eb2fc3',
                                    'bcfbddd822dacc811a5eaecdfafb44876477e517ad741197eb091f1d5b1d1350',
                                    '8b07cb8fe452eb66e43aac10b8ce932799fd1928ca64a60571c42bca470670af'),
 'storage-dirty-keys-stop-k16-r2108': ('1618e14e2cbec2b7dbadad9e907b924dd798dbd7e3be068ade71e99e2dedc0d7',
                                       'f614ff5ac318dbed60e07afba009f66fca3470c9897b629bc6d322b35497d672',
                                       '85df1e1694404336393399f53c1887eb995b92ec819b4ba3caf9dce45773eda6'),
 'storage-dirty-keys-stop-k256-r0': ('a74899e324cc0f882ef30895785a5d4e1900309bd81bc5803acbdd15a65ee77f',
                                     '03307b9b1a7562164597a2697f7ddc263f8c02b1f0f41d02223ed097c8d34cae',
                                     '65a8b2a3f6d58f72dc0f227eeacbf73b672c13038aae106749e63b228b87e0d8'),
 'storage-dirty-keys-stop-k256-r117': ('76c10aa9fb55bd41c7c8430108aff2e4879e75c8adf6b62820b892cdebe5baa6',
                                       'ccc7aba785c6244b50a845696aa5259dabb47f7a301f798db0bed364dfc658fe',
                                       '12fd710ea4147d73107b727931d48fd107a6ed40bf7a086c5c1b5fcb4ad0dec6'),
 'storage-dirty-keys-stop-k1024-r0': ('bb75eb4d586b82c89ba157d4608e314fe6a7e2c3e66856c078177400c417949c',
                                      'eb2241f143b4b0c4964f4816183367cdd7478f3f5bc6e0ee432c0fa782b45938',
                                      '4d59b8831f2865b34e7513c619c2b0ed850b984d79063c24e973670aa253fe10'),
 'storage-dirty-keys-stop-k1024-r12': ('db07ca79e7bbdb09c082e91440c81ace5cb005922fe165a1a952a6c5f684a6ee',
                                       'f2fbbb69e06e1aedcac9b85f61cf2113793072182c41d9c6230dcb74afab54a9',
                                       'af6a1f8aacad86ffccee679e17f808cd0a1a3f22e59136d262ca8cb130bca65b'),
 'storage-dirty-keys-revert-k1-r0': ('6025cb1f648f24f081d9e5558a7c1bdbf2a33b198050b2b453d85ac67eeadb40',
                                     '6359c0ca0b06f162a950107d815d376c0a5cf16f7f9c348de137951718b6b4c1',
                                     '7c8f35dba3a7595fd03e86f00a64210c71b4b48a3cff64bc1d0acbaed3b578c3'),
 'storage-dirty-keys-revert-k1-r33942': ('d2614024221157529055d2291a36e62edae8366d6156b55d4397530344d5fe08',
                                         '4cb6af67a21ac0fda54cb3a300f54adf1cd7cafe9e18082b4d18bb7628bde98b',
                                         'd1dbb51bcb1eb10fc613e61f83add413f461c9d7bd54f81d458a6bec115801fe'),
 'storage-dirty-keys-revert-k16-r0': ('7542e2b2d582af5bd5fd6ec1231d73e17b312ac727c148433bece47412f0196c',
                                      'ba92381032d6b7db0f02321ce05c94b1aea913c286b78dfccdd41db68e275dc8',
                                      'fc573ef143ef5012c947237f5c655745dddbae4a8faf7afc6e8b23d3025a2bd9'),
 'storage-dirty-keys-revert-k16-r2095': ('f1f1bd24f8f33bcfef1e61d8371efd81753a49daf82ee15e54dd98557212e246',
                                         '3c2d23768471a0744ec6f25217b4839266d95e036c0d5d260c739675797f3b02',
                                         '20fa4a2b8329a820fceca7428c3ca0984fcf241e1661a5126a1c362d1ff41fe7'),
 'storage-dirty-keys-revert-k256-r0': ('9aa951643b54638df7965b91ac0d91860a6bb505ecf408b25cb73aeb94585635',
                                       '5780dc7a9dadfd7c3165dc8c851175abb84505cd24b1b0c3b33d4146557c9679',
                                       '4dde1eaa7b53720d2b0989d2f2699619afc9393e50b9784477caef8afc953951'),
 'storage-dirty-keys-revert-k256-r104': ('0bf14b43f351379144d384e33b0cedbd4641a35a250ca73ec14789185023a025',
                                         '1db9a5e970ecd4c1eeafb9fa575f4eb0af7831e3896050026c0836b6061b83fc',
                                         'f2bdc81260218bc79859192655b1d087c24b6b2e24bca721c0f3ae9d4f30092e'),
 'storage-dirty-keys-revert-k1024-r0': ('1d0d48bf5eee62d777569e1871c674603f0945de62cbc22f6e88b1e69c73b9c7',
                                        '7a0cb0b6c4c199e4fda10fe2a854fa3dc43110f7c7d7b60374f1e787a16eb01c',
                                        '57db43a411c5d75502f95b6bc8b6d76a1059c7fd2e9b2cd133c8d20726a049ff'),
 'storage-dirty-keys-revert-k1024-r5': ('79aa27ecd2ddc4b192fabec2cca62e4e9043f6218e5cd9180e4a707a6610dd87',
                                        'e19fb83a444c9ea6233ee4b0460ffec39a12d1ef271456a8456479395971c78b',
                                        'fac993abbfb72724df581d76ea5098c5177f7330d549352580f3e52cdcfa8fb9')}

class StorageKeyShiftTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_key_shift.py'
        self.assertTrue(path.is_file(), 'high-limb producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_key_shift_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def shared(self):
        sys.path.insert(0, str(CASES))
        import storage_dirty_keys
        return storage_dirty_keys

    def test_shl_operand_order_boundaries_overflow_and_gas(self):
        # Store a literal shifted result so stack operand reversal cannot pass.
        for value in (1, 3, (1 << 256) - 1, (1 << 255) + 3):
            for shift in (0, 1, 63, 64, 128, 192, 255, 256, 257, (1 << 256) - 1):
                code = b'\x7f' + value.to_bytes(32, 'big') + b'\x7f' + shift.to_bytes(32, 'big') + bytes.fromhex('1b60005200')
                result = execute(code, {}, 30000)
                expected = 0 if shift >= 256 else (value << shift) & ((1 << 256) - 1)
                with self.subTest(value=value, shift=shift):
                    self.assertEqual(int.from_bytes(result['memory'][:32], 'big'), expected)
                    self.assertEqual(result['gas'], 21018)

    def test_all_16_pre_extension_request_runtime_and_core_goldens(self):
        m = self.shared()
        for mode, k, level in itertools.product(MODES, (1, 16, 256, 1024), LEVELS):
            row = m.make_case(mode, k, level, slot_shift=0)
            self.assertEqual(row, m.make_case(mode, k, level))
            request = row.pop('request'); p = row['parameters']
            request_raw = (json.dumps(request, indent=2, sort_keys=True) + '\n').encode()
            metadata_raw = (json.dumps(row, indent=2, sort_keys=True) + '\n').encode()
            observed = (hashlib.sha256(request_raw).hexdigest(), p['runtime_sha256'], hashlib.sha256(metadata_raw).hexdigest())
            with self.subTest(sample=row['sample_id']): self.assertEqual(observed, GOLDENS[row['sample_id']])

    def test_all_24_literal_trajectories_and_shape(self):
        m = self.load(); shapes = {}
        for mode, k, shift, level in GRID:
            with self.subTest(mode=mode, keys=k, shift=shift, level=level):
                row = m.make_case(mode, k, shift, level); records, _ = qualify(self, row); p = row['parameters']
                counts = records[0]['result']['counts']; self.assertEqual(counts['opcode:0x1b'], 3 * k)
                key = mode, k, level
                shape = (p['rounds_per_key'], p['gas_accounting_per_user'], counts)
                if key in shapes: self.assertEqual(shape, shapes[key])
                shapes[key] = shape
                code = bytes.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])
                self.assertEqual(sum(op == 0x1b for _, op, _ in decode(code)), 3)
                instructions = list(decode(code))
                self.assertEqual([(instructions[i - 1][1], instructions[i - 1][2])
                    for i, (_, op, _) in enumerate(instructions) if op == 0x1b], [(0x62, shift)] * 3)

    def test_r_plus_one_actual_bytes_with_sufficient_gas_then_constraints(self):
        m = self.shared()
        for mode, k, shift in itertools.product(MODES, KEYS, SHIFTS):
            r = m.full_rounds(mode, k, slot_shift=shift)
            self.assertTrue(m.fits(mode, k, r, slot_shift=shift))
            code, _ = m._runtime(mode, k, r + 1, slot_shift=shift)
            state = {0: 65537} | dict.fromkeys((65537 + (j << shift) for j in range(k)), 1)
            result = execute(code, state, 20_000_000)
            gas = trajectory(self, result, state, keys=k, rounds=r + 1, mode=mode, shift=shift)
            self.assertEqual(gas, m.gas_accounting(mode, k, r + 1, slot_shift=shift))
            envelope = result['gas'] + 2301
            self.assertTrue(any((envelope > 15_000_000, envelope > 16_777_216,
                167355 + 4 * result['charged'] > 30_000_000,
                any(31_000_000 - 167355 - j * result['charged'] < envelope for j in range(4)))))
            self.assertFalse(m.fits(mode, k, r + 1, slot_shift=shift))

    def test_strict_apis(self):
        m = self.load(); shared = self.shared()
        for args in ((None,), (True,), ('STOP',), ('stop', True), ('stop', 1), ('stop', 16.0),
                     ('stop', 16, True), ('stop', 16, 0), ('stop', 16, 63), ('stop', 16, 64.0),
                     ('stop', 16, 64, True), ('stop', 16, 64, 'FULL')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        self.assertEqual(m.make_case(), m.make_case('stop', 16, 64, 'full'))
        functions = ((shared._runtime, ('stop', 16, 0)), (shared.gas_accounting, ('stop', 16, 0)),
                     (shared.fits, ('stop', 16, 0)), (shared.full_rounds, ('stop', 16)),
                     (shared.make_case, ('stop', 16, 'zero')))
        for function, args in functions:
            with self.assertRaises(TypeError): function(*args, 64)
            for shift in (True, False, None, -1, 1, 63, 256, 64.0, '64'):
                with self.subTest(function=function.__name__, shift=shift), self.assertRaises(ValueError): function(*args, slot_shift=shift)

    def test_catalog_24_new_8_exact_references_provenance_cli_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 24)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-key-shift-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_key_shift.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in SOURCES})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in m.helpers().items()})
            by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 24); self.assertEqual(len({r['request_sha256'] for r in rows}), 24)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 12)
            refs = catalog['unshifted_references']; self.assertEqual(len(refs), 8)
            expected_ids = {sample for sample in GOLDENS if '-k16-' in sample or '-k1024-' in sample}
            self.assertEqual({r['sample_id'] for r in refs}, expected_ids)
            for ref in refs:
                self.assertEqual((ref['request_sha256'], ref['runtime_sha256']), GOLDENS[ref['sample_id']][:2])
                self.assertFalse(ref['newly_sampled']); self.assertEqual(ref['slot_shift'], 0)
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]
                request_raw = (output / row['request_path']).read_bytes(); request = json.loads(request_raw)
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertEqual(row['sample_id'], f"storage-key-shift-{p['mode']}-k{p['keys_per_account']}-s{p['slot_shift']}-r{p['rounds_per_key']}")
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                base = json.loads((output / baseline['request_path']).read_bytes())['program']
                self.assertEqual([c['initial_storage'] for c in request['program']['prestate_contracts']], [c['initial_storage'] for c in base['prestate_contracts']])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertIn('all24', row['completion_gate']); self.assertEqual(row['data_role'], 'unfitted_storage_key_shift_validation')
                self.assertNotIn('B_i+1..B_i+K', p['initial_storage_rule'])
                self.assertIn('<<', p['initial_storage_rule'])
                for owner, schedule in p['body_key_schedule_by_account'].items():
                    self.assertEqual(len(schedule), p['keys_per_account']); self.assertTrue(all(type(key) is str and key.startswith('0x') for key in schedule))
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for bad in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())
            for bad in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(bad)
            command = [sys.executable, '-S', str(CASES / 'storage_key_shift.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 24})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    def mutate(self, row, kind):
        for contract in row['request']['program']['prestate_contracts']:
            code = bytearray.fromhex(contract['code_hex'][2:])
            instructions = list(decode(code))
            for index, (at, op, value) in enumerate(instructions):
                if kind == 'dirty-value' and op == 0x62 and value == 7:
                    code[at + 1:at + 4] = (9).to_bytes(3, 'big')
                elif kind == 'limb' and op == 0x62 and instructions[index + 1][1] == 0x1b:
                    code[at + 1:at + 4] = (128).to_bytes(3, 'big')
                elif kind == 'marker' and op == 0x63:
                    code[at + 1:at + 5] = (value + 1).to_bytes(4, 'big')
            contract['code_hex'] = '0x' + code.hex()
        row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()

    def test_semantic_negatives_wrong_limb_key_value_marker_phase(self):
        m = self.load()
        for mutation in ('limb', 'key', 'dirty-value', 'marker', 'phase-write', 'domain'):
            row = m.make_case('stop', 16, 64, 'full'); execution = None
            if mutation in ('key', 'phase-write'):
                execution = run_users(row); result = execution[0][0]['result']
                if mutation == 'key': result['writes'][1] = (65538,) + result['writes'][1][1:]
                else: result['writes'].append((0, 65537, 65537, 65537, False))
            elif mutation == 'domain': row['request']['program']['prestate_contracts'][0]['initial_storage']['0x10002'] = '0x1'
            else: self.mutate(row, mutation)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution)

    @unittest.skipUnless(os.environ.get('STORAGE_KEY_SHIFT_NATIVE_BIN'), 'opt in four frozen native specimens')
    def test_frozen_native_specimens(self):
        binary = Path(os.environ['STORAGE_KEY_SHIFT_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        m = self.load()
        for (mode, k, shift), mutant in itertools.product((('stop', 16, 64), ('revert', 1024, 192)), (False, True)):
            row = m.make_case(mode, k, shift); p = row['parameters']
            if mutant:
                self.mutate(row, 'dirty-value')
                with self.assertRaises(AssertionError): qualify(self, row)
            with self.subTest(mode=mode, keys=k, shift=shift, mutant=mutant), tempfile.TemporaryDirectory() as temporary:
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
