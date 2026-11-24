"""Literal map and actual-byte oracle for the 32 storage-root diversity inputs."""
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

CASES = Path(__file__).resolve().parents[1] / 'cases'
PROFILES = ('background-values', 'disjoint-keys')
ACCOUNTS = (8, 64)
SIZES = (8192, 16384)
MODES = ('update', 'delete')
LEVELS = ('zero', 'full')
GRID = tuple(itertools.product(PROFILES, ACCOUNTS, SIZES, MODES, LEVELS))
MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
SOURCES = ('storage_fanout.py', 'storage_mutation.py', 'initcode_preparation.py',
           'top_level_create.py', 'spawn_memory.py')


def address(i):
    return f'0x{0x3000000000000000000000000000000000010000 + i:040x}'


def initial_map(profile, a, size, mode, i):
    """Literal spec: full-N boundary also applies to own N0."""
    k = size // a; full = 84 if mode == 'update' else 106
    if profile == 'background-values':
        return {key: 1 if key <= (64 // a) * full else i + 2 for key in range(k)}
    base = (i + 1) * 65536
    return {0: base + 1} | dict.fromkeys(range(base + 1, base + k), 1)


def run_users(row, *, owners=None):
    program = row['request']['program']
    contracts = {c['address']: c for c in program['prestate_contracts']}
    states = {owner: {int(k, 16): int(v, 16) for k, v in c['initial_storage'].items()}
              for owner, c in contracts.items()}
    records = []
    for recipient in program['transactions']['recipients']:
        owner = recipient if owners is None else owners[recipient]
        result = execute(bytes.fromhex(contracts[recipient]['code_hex'][2:]), states[owner],
                         program['transactions']['gas_limit'])
        records.append(dict(recipient=recipient, owner=owner, result=result, state=states[owner].copy()))
    return records, states


def qualify(test, row, *, execution=None):
    """Check intended semantics without using producer map, phase or gas helpers."""
    p = row['parameters']; profile = p['profile']; a = p['account_count']
    size = p['aggregate_state_size']; mode = p['mode']; level = p['workload_level']
    test.assertIn((profile, a, size, mode, level), GRID)
    full = 84 if mode == 'update' else 106; n = 0 if level == 'zero' else full
    test.assertEqual(p['mutations_per_user'], n); test.assertEqual(p['full_mutations_per_user'], full)
    k = size // a; visits = 64 // a
    bases = {address(i): 0 if profile == 'background-values' else (i + 1) * 65536 for i in range(a)}
    initial_phases = {owner: base + 1 for owner, base in bases.items()}
    final_phases = {owner: phase + visits * n for owner, phase in initial_phases.items()}
    test.assertEqual(p['phase_base_by_account'], bases)
    test.assertEqual(p['initial_phase_by_account'], initial_phases)
    test.assertEqual(p['expected_final_phase_by_account'], final_phases)
    test.assertNotIn('initial_phase', p); test.assertNotIn('expected_final_phase_per_account', p)
    request = row['request']; program = request['program']; contracts = program['prestate_contracts']
    test.assertEqual(request['schema_version'], 1); test.assertIs(request['expected_user_success'], True)
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertEqual([c['address'] for c in contracts], [address(i) for i in range(a)])
    schedule = [address(j % a) for j in range(64)]
    envelope = 466927 if mode == 'update' else 582075
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=64,
        gas_limit=envelope, recipients=schedule))
    test.assertEqual(p['transaction_gas_limit'], envelope)
    expected = {address(i): initial_map(profile, a, size, mode, i) for i in range(a)}
    for c in contracts:
        test.assertEqual(set(c), {'address', 'code_hex', 'initial_storage'})
        test.assertTrue(c['initial_storage'] == {hex(key): hex(value) for key, value in expected[c['address']].items()}, 'complete declared initial map')
    test.assertEqual(len({tuple(sorted(state.items())) for state in expected.values()}), a)
    test.assertEqual(len({c['code_hex'] for c in contracts}), 1)
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual(p['runtime_bytes'], len(code))
    test.assertEqual(p['runtime_sha256'], hashlib.sha256(code).hexdigest())
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(len(records), 64)
    routes = Counter(); pairs = set(); charged = 167355; total_writes = 0
    for j, record in enumerate(records):
        owner = address(j % a); start = initial_phases[owner] + (j // a) * n; final = start + n
        test.assertEqual((record['recipient'], record['owner']), (owner, owner), 'exact recipient/owner/order')
        result = record['result']; reads = [(0, start, True)]; writes = []
        new = 2 if mode == 'update' else 0
        for key in range(start, final):
            reads.extend([(key, 1, True), (key, new, False)])
            writes.append((key, 1, new, 1, False)); pairs.add((owner, key))
            if new: expected[owner][key] = new
            else: expected[owner].pop(key)
        reads.append((0, final, False)); writes.append((0, start, final, start, False))
        expected[owner][0] = final; pairs.add((owner, 0))
        test.assertEqual(result['terminal'], 0)
        test.assertEqual(result['reads'], reads, 'raw reads, values and transaction warmth')
        test.assertEqual(result['writes'], writes, 'raw writes and transaction originals')
        test.assertEqual(result['stack'], [])
        test.assertEqual(len(result['memory']), 128 if n else 96)
        test.assertEqual([int.from_bytes(result['memory'][offset:offset + 32], 'big') for offset in (0, 32, 64)], [n, new * n, start])
        test.assertEqual(record['state'], expected[owner], 'whole committed owner map')
        gross = (21000 + 159 + 9 + 2200 + 100) if n == 0 else (465927 if mode == 'update' else 581075)
        # N0 follows the same runtime control path; phase SSTORE is a no-op.
        refund = 4800 * n if mode == 'delete' else 0
        test.assertEqual(result['gas'], gross)
        test.assertEqual(result['refund'], refund)
        test.assertEqual(result['applied'], min(refund, gross // 5))
        test.assertEqual(result['charged'], gross - min(refund, gross // 5))
        for actual, field in (('gas', 'gross_gas'), ('refund', 'refund_counter'), ('applied', 'refund_applied'), ('charged', 'charged_gas')):
            test.assertEqual(result[actual], p['gas_accounting_per_user'][field])
            test.assertEqual(result[actual], p['expected_user_' + field])
        test.assertGreaterEqual(31_000_000 - charged, envelope)
        charged += result['charged']; routes.update(result['counts']); total_writes += len(writes)
    test.assertEqual(states, expected)
    test.assertEqual(len({tuple(sorted(state.items())) for state in states.values()}), a)
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    for op in (0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xfa, 0xfe):
        test.assertEqual(program['expected_route_counts'][f'opcode:0x{op:02x}'], 0)
    counts = dict(state_size_per_account=k, visits_per_account=visits, user_transactions_per_block=64,
        all_user_transactions=64, block_count=1, all_transactions_including_anchor=65,
        expected_distinct_account_slot_pairs=64 * n + a, expected_storage_writes=total_writes,
        expected_main_storage_writes=64 * n, expected_phase_storage_writes=64,
        expected_main_storage_writes_per_account=visits * n, expected_storage_writes_per_account=visits * (n + 1),
        expected_initial_storage_entry_count=size, expected_final_storage_entry_count=size - (64 * n if mode == 'delete' else 0),
        expected_initial_storage_entry_count_per_account=k,
        expected_final_storage_entry_count_per_account=k - (visits * n if mode == 'delete' else 0),
        expected_distinct_storage_keys_per_account=visits * n + 1, expected_counter=n,
        expected_checksum=0 if mode == 'delete' else 2 * n, phase_slot=0)
    for field, value in counts.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(len(pairs), 64 * n + a)
    test.assertEqual(sum(map(len, states.values())), counts['expected_final_storage_entry_count'])
    test.assertEqual({owner: state[0] for owner, state in states.items()}, final_phases)
    test.assertEqual(p['expected_block_charged_gas'], charged); test.assertLessEqual(charged, 30_000_000)
    test.assertEqual(p['header_prefix_remaining_gas'], [31_000_000 - 167355 - j * records[j]['result']['charged'] for j in range(64)])
    test.assertEqual(p['recipient_schedule'], schedule)
    test.assertEqual(p['start_phase_schedule'], [initial_phases[address(j % a)] + (j // a) * n for j in range(64)])
    test.assertEqual(p['final_phase_schedule'], [initial_phases[address(j % a)] + (j // a + 1) * n for j in range(64)])
    return records, states


class StorageRootDiversityTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_root_diversity.py'
        self.assertTrue(path.is_file(), 'storage root diversity producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_root_diversity_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_strict_typed_inputs(self):
        m = self.load()
        for args in ((True,), (None,), ('background',), ([],), ('background-values', True),
                     ('background-values', 8.0), ('background-values', 1), ('background-values', 64, True),
                     ('background-values', 8, 8192.0), ('background-values', 8, 8193),
                     ('background-values', 8, 8192, True), ('background-values', 8, 8192, 'insert'),
                     ('background-values', 8, 8192, 'update', True), ('background-values', 8, 8192, 'update', 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        self.assertEqual(m.make_case(), m.make_case('background-values', 8, 8192, 'update', 'full'))

    def test_all_32_actual_byte_trajectories(self):
        m = self.load()
        for args in GRID:
            with self.subTest(args=args): qualify(self, m.make_case(*args))

    def test_own_zero_full_maps_and_homogeneous_program_contract(self):
        m = self.load()
        for profile, a, size, mode in itertools.product(PROFILES, ACCOUNTS, SIZES, MODES):
            rows = [m.make_case(profile, a, size, mode, level) for level in LEVELS]
            self.assertEqual([c['initial_storage'] for c in rows[0]['request']['program']['prestate_contracts']],
                             [c['initial_storage'] for c in rows[1]['request']['program']['prestate_contracts']])
            for level, row in zip(LEVELS, rows):
                homogeneous = m.fanout.make_case(a, size, mode, level)
                program = copy.deepcopy(row['request']['program']); reference = copy.deepcopy(homogeneous['request']['program'])
                for actual, old in zip(program.pop('prestate_contracts'), reference.pop('prestate_contracts')):
                    self.assertEqual({k: v for k, v in actual.items() if k != 'initial_storage'},
                                     {k: v for k, v in old.items() if k != 'initial_storage'})
                self.assertEqual(program, reference, 'signed payload declarations and routes unchanged')
                self.assertEqual({k: v for k, v in row['request'].items() if k != 'program'},
                                 {k: v for k, v in homogeneous['request'].items() if k != 'program'})
                for field in ('gas_accounting_per_user', 'transaction_gas_limit', 'runtime_sha256', 'runtime_bytes',
                              'expected_block_charged_gas', 'header_prefix_remaining_gas'):
                    self.assertEqual(row['parameters'][field], homogeneous['parameters'][field])
                p = row['parameters']
                self.assertIn(profile, p['initial_storage_rule']); self.assertIn(profile, p['range_rule'])
                self.assertNotIn('fanout_validation', row['data_role'])
                self.assertIn('all32', row['completion_gate'])

    def test_catalog_grid_identities_hashes_sources_model_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 32)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']
            self.assertEqual(catalog['schema_version'], 1)
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-root-diversity-20261103')
            self.assertEqual(catalog['frozen_model_id'], MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_root_diversity.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in SOURCES})
            helpers = {'storage_fanout.make_case': m.fanout.make_case, 'storage_fanout.full_n': m.fanout.full_n,
                       'storage_mutation._runtime': m.fanout.mutation._runtime, 'storage_mutation.gas_accounting': m.fanout.mutation.gas_accounting}
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name, helper in helpers.items()})
            self.assertEqual(catalog['profiles'], list(PROFILES)); self.assertEqual(catalog['account_counts'], list(ACCOUNTS))
            self.assertEqual(catalog['aggregate_state_sizes'], list(SIZES)); self.assertEqual(catalog['modes'], list(MODES)); self.assertEqual(catalog['levels'], list(LEVELS))
            by = {r['sample_id']: r for r in rows}; self.assertEqual(len(by), 32)
            self.assertEqual(len({r['request_sha256'] for r in rows}), 32)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 16)
            self.assertEqual({tuple(r['parameters'][k] for k in ('profile', 'account_count', 'aggregate_state_size', 'mode', 'workload_level')) for r in rows}, set(GRID))
            references = set()
            for row in rows:
                p = row['parameters']; prefix = f"storage-root-diversity-{p['profile']}-a{p['account_count']}-s{p['aggregate_state_size']}-{p['mode']}"
                self.assertEqual(row['sample_id'], f"{prefix}-n{p['mutations_per_user']}")
                self.assertEqual(row['baseline_sample_id'], f'{prefix}-n0'); self.assertIn(row['baseline_sample_id'], by)
                reference = f"storage-fanout-a{p['account_count']}-s{p['aggregate_state_size']}-{p['mode']}-n{p['mutations_per_user']}"
                self.assertEqual(row['homogeneous_reference_sample_id'], reference); references.add(reference)
                self.assertEqual(row['family'], 'storage-root-diversity'); self.assertEqual(row['corpus_id'], catalog['corpus_id'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertEqual(row['request_path'], f"requests/{row['sample_id']}.json")
                request_raw = (output / row['request_path']).read_bytes()
                self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                self.assertTrue(request_raw == (json.dumps(json.loads(request_raw), indent=2, sort_keys=True) + '\n').encode(), 'canonical request serialization')
            self.assertEqual(len(references), 16)
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for invalid in ('A' * 64, '0' * 63, True, None, 0):
                path = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(path, model_id=invalid)
                self.assertFalse(path.exists())
            alternate = Path(temporary) / 'alternate'; m.prepare(alternate, model_id='a' * 64)
            self.assertEqual(json.loads((alternate / 'catalog.json').read_text())['frozen_model_id'], 'a' * 64)

    def test_cli_output_and_model_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'
            command = [sys.executable, '-S', str(CASES / 'storage_root_diversity.py'), '--output-dir', str(output), '--model-id', 'b' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 32})
            before = (output / 'catalog.json').read_bytes()
            self.assertEqual(json.loads(before)['frozen_model_id'], 'b' * 64)
            repeated = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(repeated.returncode, 0); self.assertEqual((output / 'catalog.json').read_bytes(), before)

    def test_rejects_successful_wrong_profile_tail_range_phase_and_values(self):
        m = self.load()
        for profile in PROFILES:
            for mutation in ('collapsed-maps', 'wrong-profile', 'tail-boundary', 'phase-base', 'missing-slot', 'extra-slot'):
                row = m.make_case(profile); contracts = row['request']['program']['prestate_contracts']; p = row['parameters']
                if mutation == 'collapsed-maps': contracts[1]['initial_storage'] = contracts[0]['initial_storage'].copy()
                elif mutation == 'wrong-profile':
                    for i, c in enumerate(contracts): c['initial_storage'] = {hex(k): hex(v) for k, v in initial_map(PROFILES[1 - PROFILES.index(profile)], 8, 8192, 'update', i).items()}
                elif mutation == 'tail-boundary':
                    key = 8 * 84 + 1 if profile == 'background-values' else 65536 + 1023
                    contracts[0]['initial_storage'][hex(key)] = '0x63'
                elif mutation == 'phase-base': contracts[0]['initial_storage']['0x0'] = hex(int(contracts[0]['initial_storage']['0x0'], 16) + 1)
                elif mutation == 'missing-slot': contracts[0]['initial_storage'].pop(next(k for k in contracts[0]['initial_storage'] if k != '0x0'))
                else: contracts[0]['initial_storage']['0xffffff'] = '0x1'
                if mutation in ('collapsed-maps', 'wrong-profile', 'tail-boundary', 'extra-slot'):
                    self.assertTrue(all(r['result']['terminal'] == 0 for r in run_users(row)[0]))
                with self.subTest(profile=profile, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row)
            # Zero cannot choose its salt boundary using current N=0.
            row = m.make_case(profile, level='zero')
            if profile == 'background-values':
                for i, c in enumerate(row['request']['program']['prestate_contracts']):
                    c['initial_storage'] = {hex(k): hex(1 if k == 0 else i + 2) for k in range(1024)}
            else: row['parameters']['phase_base_by_account'][address(0)] += 1
            self.assertTrue(all(r['result']['terminal'] == 0 for r in run_users(row)[0]))
            with self.assertRaises(AssertionError): qualify(self, row)
            # Update 2->3 plus matching checksum still passes the bytecode guard.
            row = m.make_case(profile); n = row['parameters']['mutations_per_user']
            for c in row['request']['program']['prestate_contracts']:
                code = bytearray.fromhex(c['code_hex'][2:])
                for at, op, value in decode(code):
                    if op == 0x62 and value in (2, 2 * n): code[at + 1:at + 4] = (3 if value == 2 else 3 * n).to_bytes(3, 'big')
                c['code_hex'] = '0x' + code.hex()
            row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            self.assertTrue(all(r['result']['terminal'] == 0 for r in run_users(row)[0]))
            with self.assertRaises(AssertionError): qualify(self, row)

    def test_rejects_successful_rotated_collapsed_and_wrong_owner_schedules(self):
        m = self.load()
        for profile, level in itertools.product(PROFILES, LEVELS):
            for mutation in ('rotate', 'collapse', 'owner', 'metadata-phase'):
                row = m.make_case(profile, level=level); program = row['request']['program']; execution = None
                recipients = program['transactions']['recipients']
                if mutation == 'rotate': program['transactions']['recipients'] = recipients[1:] + recipients[:1]
                elif mutation == 'collapse': program['transactions']['recipients'] = [recipients[0]] * 64
                elif mutation == 'owner': execution = run_users(row, owners={address(i): address((i + 1) % 8) for i in range(8)})
                else: row['parameters']['start_phase_schedule'][0] += 1
                if mutation != 'collapse' or level == 'zero':
                    checked = run_users(row) if execution is None else execution
                    self.assertTrue(all(r['result']['terminal'] == 0 for r in checked[0]))
                with self.subTest(profile=profile, level=level, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row, execution=execution)

    @unittest.skipUnless(os.environ.get('STORAGE_ROOT_DIVERSITY_NATIVE_BIN'), 'opt in bounded frozen native smoke')
    def test_frozen_native_smoke(self):
        m = self.load()
        for profile, a, mode in (('background-values', 8, 'update'), ('disjoint-keys', 64, 'delete')):
            row = m.make_case(profile, a, 8192, mode); p = row['parameters']
            with self.subTest(profile=profile, accounts=a, mode=mode), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); source = root / 'request.json'; report_path = root / 'native.json'
                source.write_text(json.dumps(row['request']))
                process = subprocess.run([os.environ['STORAGE_ROOT_DIVERSITY_NATIVE_BIN'], '--input', str(source),
                    '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'),
                    '--report', str(report_path), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads(report_path.read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), 65); self.assertTrue(all(r['success'] for r in receipts))
                self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['expected_user_charged_gas']] * 64)
                diagnostic = report['prestate_contracts']['initialized_storage']
                self.assertEqual([d['address'] for d in diagnostic], [address(i) for i in range(a)])
                for d in diagnostic:
                    self.assertEqual(d['initial_storage_entry_count'], 8192 // a)
                    self.assertEqual(d['final_storage_entry_count'], p['expected_final_storage_entry_count_per_account'])
                    for key in ('initial_storage_root', 'final_storage_root'): self.assertRegex(d[key], r'^0x[0-9a-f]{64}$')
                for key in ('initial_storage_root', 'final_storage_root'): self.assertEqual(len({d[key] for d in diagnostic}), a)


if __name__ == '__main__': unittest.main()
