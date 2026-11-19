"""Independent rolling-account oracle for the finite initialized-storage fanout."""
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

CASES = Path(__file__).resolve().parents[1] / 'cases'
ACCOUNTS = (1, 8, 64)
SIZES = (8192, 16384)
MODES = ('update', 'delete')
LEVELS = ('zero', 'full')
BASE = 0x3000000000000000000000000000000000010000


def address(index):
    return f'0x{BASE + index:040x}'


def run_users(row, *, owners=None):
    """Execute declared recipients; an optional owner permutation models misrouting."""
    program = row['request']['program']
    contracts = {c['address']: c for c in program['prestate_contracts']}
    states = {a: {int(k, 16): int(v, 16) for k, v in c['initial_storage'].items()}
              for a, c in contracts.items()}
    results = []
    for recipient in program['transactions']['recipients']:
        owner = recipient if owners is None else owners[recipient]
        code = bytes.fromhex(contracts[recipient]['code_hex'][2:])
        result = execute(code, states[owner], program['transactions']['gas_limit'])
        results.append(dict(recipient=recipient, owner=owner, result=result,
                            state=states[owner].copy()))
    return results, states


def qualify(test, row, *, execution=None):
    """Check intended raw-slot semantics independently of runtime/gas helpers."""
    p = row['parameters']; a = p['account_count']; size = p['aggregate_state_size']; n = p['mutations_per_user']; mode = p['mode']
    test.assertIn(a, ACCOUNTS); test.assertIn(size, SIZES); test.assertIn(mode, MODES)
    request = row['request']; program = request['program']; contracts = program['prestate_contracts']
    test.assertEqual(request['schema_version'], 1); test.assertIs(request['expected_user_success'], True)
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertNotIn('storage_state', program)
    test.assertEqual(len(contracts), a)
    test.assertEqual([c['address'] for c in contracts], [address(i) for i in range(a)])
    schedule = [address(j % a) for j in range(64)]
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=64,
        gas_limit=p['transaction_gas_limit'], recipients=schedule), 'exact signed recipient schedule')
    expected = {address(i): dict.fromkeys(range(size // a), 1) for i in range(a)}
    for c in contracts:
        test.assertEqual(set(c), {'address', 'code_hex', 'initial_storage'})
        test.assertEqual(c['initial_storage'], {hex(k): '0x1' for k in range(size // a)})
    codes = {c['code_hex'] for c in contracts}; test.assertEqual(len(codes), 1)
    code = bytes.fromhex(contracts[0]['code_hex'][2:])
    test.assertEqual(p['runtime_bytes'], len(code))
    test.assertEqual(p['runtime_sha256'], hashlib.sha256(code).hexdigest())
    results, states = run_users(row) if execution is None else execution
    test.assertEqual(len(results), 64)
    counts = Counter(); pairs = set(); total_writes = 0; charged = 167355
    for j, record in enumerate(results):
        target = address(j % a); start = 1 + (j // a) * n; final = start + n
        test.assertEqual((record['recipient'], record['owner']), (target, target), 'storage owner/order')
        result = record['result']; reads = [(0, start, True)]; writes = []
        new = 2 if mode == 'update' else 0
        for key in range(start, final):
            reads.extend([(key, 1, True), (key, new, False)])
            writes.append((key, 1, new, 1, False)); pairs.add((target, key))
            if new: expected[target][key] = new
            else: expected[target].pop(key)
        reads.append((0, final, False)); writes.append((0, start, final, start, False))
        expected[target][0] = final; pairs.add((target, 0))
        test.assertEqual(result['terminal'], 0, 'runtime guard')
        test.assertEqual(result['reads'], reads, 'ordered raw reads/values/warmth')
        test.assertEqual(result['writes'], writes, 'ordered raw writes/transaction originals')
        test.assertEqual(result['stack'], [])
        test.assertEqual(int.from_bytes(result['memory'][:32], 'big'), n)
        test.assertEqual(int.from_bytes(result['memory'][32:64], 'big'), new * n)
        test.assertEqual(int.from_bytes(result['memory'][64:96], 'big'), start)
        test.assertEqual(record['state'], expected[target], 'committed owner state')
        refund = 4800 * n if mode == 'delete' else 0
        test.assertEqual(result['refund'], refund)
        test.assertEqual(result['applied'], min(refund, result['gas'] // 5))
        for actual, field in (('gas', 'gross_gas'), ('refund', 'refund_counter'),
                              ('applied', 'refund_applied'), ('charged', 'charged_gas')):
            test.assertEqual(result[actual], p['gas_accounting_per_user'][field])
            test.assertEqual(result[actual], p['expected_user_' + field])
        test.assertGreaterEqual(31_000_000 - charged, p['transaction_gas_limit'])
        charged += result['charged']; total_writes += len(writes); counts.update(result['counts'])
    test.assertEqual(states, expected)
    test.assertEqual(counts, +Counter(program['expected_route_counts']))
    for op in (0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xfa, 0xfe):
        test.assertEqual(program['expected_route_counts'][f'opcode:0x{op:02x}'], 0)
    test.assertEqual(len(pairs), 64 * n + a)
    test.assertEqual(p['expected_distinct_account_slot_pairs'], len(pairs))
    test.assertEqual(p['expected_storage_writes'], total_writes)
    test.assertEqual(p['expected_main_storage_writes'], 64 * n)
    test.assertEqual(p['expected_phase_storage_writes'], 64)
    test.assertEqual(p['expected_initial_storage_entry_count'], size)
    test.assertEqual(p['expected_final_storage_entry_count'], sum(map(len, states.values())))
    test.assertEqual(p['expected_final_storage_entry_count'], size - (64 * n if mode == 'delete' else 0))
    test.assertEqual(p['expected_final_phase_per_account'], 1 + (64 // a) * n)
    test.assertEqual(p['expected_initial_storage_entry_count_per_account'], size // a)
    test.assertEqual(p['expected_final_storage_entry_count_per_account'], size // a - ((64 // a) * n if mode == 'delete' else 0))
    test.assertEqual(p['state_size_per_account'], size // a)
    test.assertEqual(p['visits_per_account'], 64 // a)
    test.assertEqual(p['user_transactions_per_block'], 64)
    test.assertEqual(p['expected_storage_writes_per_account'], (64 // a) * (n + 1))
    test.assertEqual(p['expected_main_storage_writes_per_account'], (64 // a) * n)
    test.assertEqual(p['expected_distinct_storage_keys_per_account'], (64 // a) * n + 1)
    test.assertEqual(p['expected_block_charged_gas'], charged); test.assertLessEqual(charged, 30_000_000)
    test.assertEqual(p['header_prefix_remaining_gas'], [31_000_000 - 167355 - j * results[j]['result']['charged'] for j in range(64)])
    test.assertEqual(p['recipient_schedule'], schedule)
    test.assertEqual(p['start_phase_schedule'], [1 + (j // a) * n for j in range(64)])
    test.assertEqual(p['final_phase_schedule'], [1 + (j // a + 1) * n for j in range(64)])
    return results


class StorageFanoutTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_fanout.py'
        self.assertTrue(path.is_file(), 'storage fanout producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_fanout_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_strict_inputs_and_maximal_full_bounds(self):
        m = self.load()
        for args in ((True,), (1.0,), (0,), (2,), (None,), (1, True), (1, 8192.0),
                     (1, 8193), (1, 8192, True), (1, 8192, 'insert'),
                     (1, 8192, 'UPDATE'), (1, 8192, 'update', True), (1, 8192, 'update', 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for a, size, mode in itertools.product(ACCOUNTS, SIZES, MODES):
            row = m.make_case(a, size, mode); p = row['parameters']; n = p['mutations_per_user']
            self.assertEqual(n, 84 if mode == 'update' else 106)
            self.assertEqual(p['transaction_gas_limit'], 466927 if mode == 'update' else 582075)
            self.assertEqual(p['expected_block_charged_gas'], 29986683 if mode == 'update' else 29918395)
            self.assertTrue(m.fits(a, size, mode, n)); self.assertFalse(m.fits(a, size, mode, n + 1))
            for invalid in (True, -1, 1.0, 8193):
                with self.assertRaises(ValueError): m.fits(a, size, mode, invalid)
            # Patch actual fixed-width loop/checksum bounds to execute N+1;
            # no call to the producer's solver or gas accounting for this oracle.
            code = bytearray.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])
            for at, op, value in decode(code):
                if op == 0x62 and value in (n, 2 * n):
                    code[at + 1:at + 4] = (value + (1 if value == n else 2)).to_bytes(3, 'big')
            result = execute(code, dict.fromkeys(range(size // a), 1), 15_000_000)
            self.assertEqual(result['terminal'], 0)
            envelope = result['gas'] + 1000; charged = result['charged']
            violations = [64 * (n + 1) + a > 8192, (64 // a) * (n + 1) > size // a - 1,
                envelope > 15_000_000, envelope > 16_777_216, 167355 + 64 * charged > 30_000_000]
            violations.extend(31_000_000 - 167355 - j * charged < envelope for j in range(64))
            self.assertTrue(any(violations), 'independently executed N+1 is feasible')

    def test_all_24_actual_byte_trajectories(self):
        m = self.load()
        for a, size, mode, level in itertools.product(ACCOUNTS, SIZES, MODES, LEVELS):
            with self.subTest(accounts=a, size=size, mode=mode, level=level):
                qualify(self, m.make_case(a, size, mode, level))

    def test_catalog_grid_identities_hashes_provenance_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 24)
            raw = (output / 'catalog.json').read_bytes(); catalog = json.loads(raw); rows = catalog['cases']
            self.assertEqual(catalog['corpus_id'], 'unzen-storage-fanout-20261102')
            self.assertEqual(catalog['frozen_model_id'], m.MODEL)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'storage_fanout.py').read_bytes()).hexdigest())
            sources = ('storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in sources})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(getattr(m.mutation, name)).encode()).hexdigest() for name in ('_runtime', 'gas_accounting')})
            by = {r['sample_id']: r for r in rows}; self.assertEqual(len(by), 24)
            self.assertEqual(len({r['request_sha256'] for r in rows}), 24)
            self.assertEqual(sum(r['role'] == 'control' for r in rows), 12)
            self.assertEqual({tuple(r['parameters'][k] for k in ('account_count', 'aggregate_state_size', 'mode', 'workload_level')) for r in rows}, set(itertools.product(ACCOUNTS, SIZES, MODES, LEVELS)))
            for row in rows:
                p = row['parameters']; baseline = by[row['baseline_sample_id']]
                self.assertEqual(row['sample_id'], f"storage-fanout-a{p['account_count']}-s{p['aggregate_state_size']}-{p['mode']}-n{p['mutations_per_user']}")
                self.assertEqual(baseline['parameters']['mutations_per_user'], 0)
                self.assertEqual(p['transaction_gas_limit'], baseline['parameters']['transaction_gas_limit'])
                request = (output / row['request_path']).read_bytes()
                self.assertEqual(hashlib.sha256(request).hexdigest(), row['request_sha256'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertEqual(row['corpus_id'], catalog['corpus_id'])
                self.assertNotIn('shape', p); self.assertNotIn('requested_tail_length', p)
            with self.assertRaises(FileExistsError): m.prepare(output)
            self.assertEqual((output / 'catalog.json').read_bytes(), raw)
            for invalid in ('A' * 64, '0' * 63, True, None):
                invalid_path = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(invalid_path, model_id=invalid)
                self.assertFalse(invalid_path.exists())
            alternate = Path(temporary) / 'alternate'; m.prepare(alternate, model_id='a' * 64)
            self.assertEqual(json.loads((alternate / 'catalog.json').read_text())['frozen_model_id'], 'a' * 64)

    def test_cli_output_and_model_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'
            command = [sys.executable, '-S', str(CASES / 'storage_fanout.py'),
                       '--output-dir', str(output), '--model-id', 'b' * 64]
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertEqual(json.loads(process.stdout), {'prepared': 24})
            catalog = json.loads((output / 'catalog.json').read_text())
            self.assertEqual(catalog['frozen_model_id'], 'b' * 64)
            before = (output / 'catalog.json').read_bytes()
            repeated = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(repeated.returncode, 0)
            self.assertEqual((output / 'catalog.json').read_bytes(), before)

    def test_semantic_negatives_reject_successful_wrong_recipient_owner_phase_and_value(self):
        m = self.load()
        # N0 is deliberately symmetric: final states/routes cannot prove order.
        for level in LEVELS:
            for mutation in ('rotate', 'collapse'):
                row = m.make_case(8, 8192, 'update', level); recipients = row['request']['program']['transactions']['recipients']
                row['request']['program']['transactions']['recipients'] = recipients[1:] + recipients[:1] if mutation == 'rotate' else [recipients[0]] * 64
                if mutation == 'rotate' or level == 'zero':
                    results, _ = run_users(row)
                    self.assertTrue(all(r['result']['terminal'] == 0 for r in results))
                with self.subTest(level=level, mutation=mutation), self.assertRaises(AssertionError): qualify(self, row)
        row = m.make_case(8)
        owners = {address(i): address((i + 1) % 8) for i in range(8)}
        execution = run_users(row, owners=owners)
        self.assertTrue(all(r['result']['terminal'] == 0 for r in execution[0]))
        with self.assertRaises(AssertionError): qualify(self, row, execution=execution)
        # Change update2 to3 AND checksum to3*N: runtime succeeds, semantics fail.
        row = m.make_case(8); n = row['parameters']['mutations_per_user']
        for contract in row['request']['program']['prestate_contracts']:
            code = bytearray.fromhex(contract['code_hex'][2:])
            for at, op, value in decode(code):
                if op == 0x62 and value in (2, 2 * n):
                    code[at + 1:at + 4] = (3 if value == 2 else 3 * n).to_bytes(3, 'big')
            contract['code_hex'] = '0x' + code.hex()
        row['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
        self.assertTrue(all(r['result']['terminal'] == 0 for r in run_users(row)[0]))
        with self.assertRaises(AssertionError): qualify(self, row)
        for field, value in (('0x0', '0x2'), ('0x1', '0x2')):
            row = m.make_case(8); row['request']['program']['prestate_contracts'][0]['initial_storage'][field] = value
            with self.subTest(slot=field), self.assertRaises(AssertionError): qualify(self, row)
        row = m.make_case(8); code = bytearray.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])
        at = [at for at, op, value in decode(code) if op == 0x62 and value == n][-1]
        code[at + 1:at + 4] = (n + 1).to_bytes(3, 'big')
        self.assertEqual(execute(code, dict.fromkeys(range(1024), 1), 466927)['terminal'], 0xfe)

    def test_malformed_declarations_fail_qualification(self):
        m = self.load()
        for mutation in ('duplicate', 'missing-slot', 'extra-slot', 'zero-value', 'unknown-recipient', 'count', 'primary', 'storage-state', 'nonuniform-code'):
            row = m.make_case(8); program = row['request']['program']; contracts = program['prestate_contracts']
            if mutation == 'duplicate': contracts[1]['address'] = contracts[0]['address']
            elif mutation == 'missing-slot': contracts[0]['initial_storage'].pop('0x1')
            elif mutation == 'extra-slot': contracts[0]['initial_storage']['0x10000'] = '0x1'
            elif mutation == 'zero-value': contracts[0]['initial_storage']['0x1'] = '0x0'
            elif mutation == 'unknown-recipient': program['transactions']['recipients'][0] = address(64)
            elif mutation == 'count': program['transactions']['count_per_block'] = 63
            elif mutation == 'primary': program['code_hex'] = '0xfe'
            elif mutation == 'storage-state': program['storage_state'] = {}
            else: contracts[0]['code_hex'] = '0x00'
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): qualify(self, row)

    @unittest.skipUnless(os.environ.get('STORAGE_FANOUT_NATIVE_BIN'), 'opt in bounded frozen native smoke')
    def test_frozen_native_smoke(self):
        m = self.load()
        for a, mode in ((8, 'update'), (64, 'delete')):
            row = m.make_case(a, 8192, mode); p = row['parameters']
            with self.subTest(accounts=a, mode=mode), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); source = root / 'request.json'; report_path = root / 'native.json'
                source.write_text(json.dumps(row['request']))
                process = subprocess.run([os.environ['STORAGE_FANOUT_NATIVE_BIN'], '--input', str(source),
                    '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'),
                    '--report', str(report_path), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(process.returncode, 0, process.stderr.decode())
                report = json.loads(report_path.read_text()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts), 65); self.assertTrue(all(r['success'] for r in receipts))
                self.assertEqual(receipts[0]['gas_used'], 167355)
                self.assertEqual([r['gas_used'] for r in receipts[1:]], [p['gas_accounting_per_user']['charged_gas']] * 64)
                diagnostic = report['prestate_contracts']['initialized_storage']
                self.assertEqual([d['address'] for d in diagnostic], [address(i) for i in range(a)])
                for d in diagnostic:
                    self.assertEqual(d['initial_storage_entry_count'], 8192 // a)
                    self.assertEqual(d['final_storage_entry_count'], p['expected_final_storage_entry_count_per_account'])
                    for key in ('initial_storage_root', 'final_storage_root'):
                        self.assertRegex(d[key], r'^0x[0-9a-f]{64}$')


if __name__ == '__main__': unittest.main()
