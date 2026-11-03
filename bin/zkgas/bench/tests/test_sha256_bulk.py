"""Independent byte/gas/intent qualification for the finite SHA-256 panel."""
from collections import Counter
import copy
from datetime import datetime, timezone
import hashlib
import importlib.util
import inspect
import itertools
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('zero', 'a5'), (64, 4096, 1048576), ('zero', 'single', 'full')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'sha256_bulk.py'
    assert path.is_file(), 'SHA-256 bulk producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('sha256_bulk', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from sha256_bulk_vm import run_users
    p = row['parameters']; program = row['request']['program']
    test.assertIn((p['pattern'], p['input_bytes'], p['workload_level']), GRID)
    test.assertEqual(p['repetitions'], {'zero': 0, 'single': 1, 'full': p['full_repetitions']}[p['workload_level']])
    contracts = program['prestate_contracts']; owners = [c['address'] for c in contracts]
    test.assertEqual(len(set(owners)), 2)
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=2,
        gas_limit=p['transaction_gas_limit'], recipients=owners))
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertEqual(row['request']['schema_version'], 1); test.assertIs(row['request']['expected_user_success'], True)
    test.assertEqual(p['owner_tags'], [0, 1]); test.assertEqual(p['account_count'], 2)
    test.assertFalse(p['shared_runtime']); test.assertEqual(p['recipient_schedule'], owners)
    records = run_users(row); routes = Counter(); spent = 167355; prefixes = []; identities = set()
    for tag, (c, result) in enumerate(zip(contracts, records)):
        code = bytes.fromhex(c['code_hex'][2:]); test.assertEqual(c['initial_storage'], {})
        test.assertEqual(p['runtime_sha256'][tag], hashlib.sha256(code).hexdigest())
        test.assertEqual(p['runtime_bytes'], len(code)); test.assertLessEqual(len(code), 24576)
        test.assertEqual(result['terminal'], 0); test.assertEqual(result['stack'], [])
        test.assertEqual(result['attempted_calls'], p['repetitions']); test.assertEqual(result['completed_calls'], p['repetitions'])
        test.assertEqual(result['counter'], p['repetitions']); test.assertEqual(result['accumulator'], int(p['expected_xor_hex'][tag], 16))
        test.assertEqual(result['output'], 2 if not p['repetitions'] else int.from_bytes(result['calls'][-1]['digest'], 'big'))
        test.assertEqual(len(result['memory']), 32 * (3 + p['input_bytes'] // 32))
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['gas'], p['gas_accounting_per_user']['gross_gas'])
        test.assertEqual(result['mcopy_gas'], p['gas_accounting_per_user']['mcopy_gas'])
        test.assertGreater(result['counts']['opcode:0x5e'], 0)
        for i, call in enumerate(result['calls']):
            test.assertEqual((call['tag'], call['index'], call['sentinel']), (tag, i, 2))
            test.assertNotIn(call['input_sha256'], identities); identities.add(call['input_sha256'])
            test.assertEqual(call['native_gas'], 60 + 12 * ((p['input_bytes'] + 31) // 32))
            test.assertGreaterEqual(call['forwarded_gas'], call['native_gas'])
            test.assertEqual(call['forwarded_gas'], min(call['requested_gas'], call['available_gas'] - call['available_gas'] // 64))
            test.assertEqual(call['returned_gas'], call['forwarded_gas'] - call['native_gas'])
            test.assertGreater(call['gas_after_call'], 0)
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    for op in (0x54, 0x55, 0x5c, 0x5d, 0xf0, 0xf1, 0xf4, 0xfd, 0xfe): test.assertEqual(routes[f'opcode:0x{op:02x}'], 0)
    test.assertEqual((p['aggregate_state_size'], p['state_size_per_account']), (0, 0))
    test.assertEqual(p['expected_precompile_calls'], 2 * p['repetitions'])
    return records


class Sha256BulkTests(unittest.TestCase):
    def test_all_eighteen_actual_byte_trajectories(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                gas = m.gas_accounting(*args[:2], p['full_repetitions'])
                self.assertEqual(p['transaction_gas_limit'], gas['gross_gas'] + (gas['native_gas_per_call'] + 62) // 63 + 2301)

    def test_full_plus_one_and_gas_only_solver(self):
        m = producer(); from sha256_bulk_vm import execute
        # A binary-search candidate must never hash or build its expected digest XOR.
        with patch.object(m, 'expected_xor', side_effect=AssertionError('digest in gas-only search')):
            for pattern, length in itertools.product(m.PATTERNS, m.LENGTHS):
                n = m.full_repetitions(pattern, length)
                self.assertTrue(m.fits(pattern, length, n)); self.assertFalse(m.fits(pattern, length, n + 1))
                self.assertGreater(n, 0); self.assertLess(n, m.MAX_REPETITIONS)
        for pattern, length in itertools.product(m.PATTERNS, m.LENGTHS):
            n = m.full_repetitions(pattern, length)
            for r in (0, 1, n, n + 1):
                code, counts = m._runtime(pattern, length, r, 1)
                result = execute(code, 50_000_000, pattern=pattern, length=length, tag=1, repetitions=r)
                self.assertEqual(result['terminal'], 0); self.assertEqual(result['completed_calls'], r)
                self.assertEqual(result['counts'], +Counter(counts))
                gas = m.gas_accounting(pattern, length, r); self.assertEqual(result['gas'], gas['gross_gas'])
                words = 3 + length // 32
                self.assertEqual(gas['memory_expansion_gas'], 3 * words + words * words // 512)
                self.assertEqual(gas['mcopy_gas'], result['mcopy_gas'])
                self.assertEqual(gas['precompile_gas'], r * (100 + 60 + 12 * (length // 32)))
            gas = m.gas_accounting(pattern, length, n + 1); envelope = m._envelope(gas)
            self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas['gross_gas'] > 30_000_000 or
                any(31_000_000 - 167355 - j * gas['gross_gas'] < envelope for j in range(2)))

    def test_strict_domains_and_independent_limits(self):
        m = producer()
        for args in ((None,), (True,), ('A5',), ('a5', True), ('a5', 64.0), ('a5', 32), ('a5', 64, True), ('a5', 64, 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for fn in (m._runtime, m.gas_accounting, m.fits):
            for r in (True, -1, 1.0, m.MAX_REPETITIONS + 1, None):
                with self.assertRaises(ValueError): fn('zero', 64, r)
        for tag in (True, -1, 2, 0.0, None):
            with self.assertRaises(ValueError): m._runtime('zero', 64, 0, tag)
            with self.assertRaises(ValueError): m.expected_xor('zero', 64, 0, tag)
        for fn in (m.full_repetitions,):
            for args in (('bad', 64), ('zero', True), ('zero', 32)):
                with self.assertRaises(ValueError): fn(*args)
        for field, value in (('TRANSACTION_GAS_CAP', 21000), ('EIP7825_GAS_CAP', 21000), ('BLOCK_BUDGET', 167355), ('HEADER_GAS_LIMIT', 167355)):
            with self.subTest(field=field), patch.object(m, field, value): self.assertFalse(m.fits('zero', 64, 0))
        self.assertEqual(m.make_case(), m.make_case('a5', 64, 'full'))

    def test_fixed_width_shape_and_forwarding_boundary(self):
        m = producer(); from sha256_bulk_vm import decode, execute
        for length in m.LENGTHS:
            shapes = set(); sizes = set()
            for pattern, r, tag in itertools.product(m.PATTERNS, (0, 1, 123456), (0, 1)):
                code, _ = m._assemble(pattern, length, r, tag, (1 << 256) - 1 if tag else 0)
                shapes.add(tuple(op for _, op, _ in decode(code))); sizes.add(len(code))
            self.assertEqual(len(shapes), 1); self.assertEqual(len(sizes), 1)
            self.assertEqual(m.gas_accounting('zero', length, 1), m.gas_accounting('a5', length, 1))
        code, _ = m._runtime('a5', 1048576, 1)
        with self.assertRaisesRegex(AssertionError, 'forwarded gas'):
            execute(code, m.gas_accounting('a5', 1048576, 1)['gross_gas'], pattern='a5', length=1048576, tag=0, repetitions=1)

    def test_wrapper_guards_and_intent_mutations(self):
        m = producer(); from sha256_bulk_vm import execute, decode, run_users
        code, _ = m._runtime('a5', 64, 2, 0)
        for override in ({'status': 0}, {'return_size': 31}, {'return_size': 33}, {'write_size': 0}, {'write_size': 31}):
            with self.subTest(override=override):
                self.assertEqual(execute(code, 2_000_000, pattern='a5', length=64, tag=0, repetitions=2, override=override)['terminal'], 0xfe)
        # The same wrong bit on two digests cancels in XOR; per-call oracle rejects it.
        with self.assertRaisesRegex(AssertionError, 'per-call returned digest'):
            execute(code, 2_000_000, pattern='a5', length=64, tag=0, repetitions=2, override={'wrong_digest': True})
        with self.assertRaisesRegex(AssertionError, 'gas'):
            execute(code, 21000, pattern='a5', length=64, tag=0, repetitions=2)
        for label, selected in (('wrong-tag', dict(tag=1)), ('wrong-pattern', dict(pattern='zero'))):
            with self.subTest(label=label), self.assertRaises(AssertionError):
                execute(code, 2_000_000, **(dict(pattern='a5', length=64, tag=0, repetitions=2) | selected))
        ins = list(decode(code))
        # Call's output offset, MCOPY destination and counter increment are actual PUSH immediates.
        for label, at, value in (
            ('output-overlap', next(at for at, op, v in ins if op == 0x62 and v == 128 and code[at + 4] == 0x62), 64),
            ('copy-overlap', next(at for at, op, v in ins if op == 0x62 and v == 96 and code[at + 4] == 0x5e), 64),
        ):
            wrong = bytearray(code); wrong[at + 1:at + 4] = value.to_bytes(3, 'big')
            with self.subTest(label=label), self.assertRaises(AssertionError): execute(bytes(wrong), 2_000_000, pattern='a5', length=64, tag=0, repetitions=2)
        wrong = bytearray(code); at = code.index(bytes.fromhex('62000000516200000101')) + 5
        wrong[at + 1:at + 4] = (2).to_bytes(3, 'big')
        self.assertEqual(execute(bytes(wrong), 2_000_000, pattern='a5', length=64, tag=0, repetitions=2)['terminal'], 0xfe)
        row = m.make_case('a5', 64, 'single')
        for change in ('recipient', 'owner', 'storage', 'duplicate-code'):
            wrong = copy.deepcopy(row); program = wrong['request']['program']; contracts = program['prestate_contracts']
            if change == 'recipient': program['transactions']['recipients'].reverse()
            elif change == 'owner': contracts[1]['address'] = contracts[0]['address']
            elif change == 'storage': contracts[0]['initial_storage'] = {'0x00': '0x01'}
            else: contracts[1]['code_hex'] = contracts[0]['code_hex']
            with self.subTest(change=change), self.assertRaises(AssertionError): run_users(wrong)
        mutant = intent_mutant(m.make_case('a5', 1048576, 'single'))
        with self.assertRaisesRegex(AssertionError, 'input bytes'): run_users(mutant)

    def test_catalog_cli_source_and_request_bindings(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 18)
            catalog = json.loads((output / 'catalog.json').read_bytes()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 18); self.assertEqual(len({r['request_sha256'] for r in rows}), 18)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {n: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for n, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()})
            self.assertIn('_Factory', catalog['helper_source_sha256'])
            for row in rows:
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(raw, (json.dumps(json.loads(raw), indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['parameters']['transaction_gas_limit'], by[row['baseline_sample_id']]['parameters']['transaction_gas_limit'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for bad in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())
            for bad in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(bad)
            command = [sys.executable, '-S', str(CASES / 'sha256_bulk.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            process = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 18})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)

    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('SHA256_BULK_NATIVE_BIN'),
        'set SHA256_BULK_NATIVE_BIN for optional Linux frozen-native smoke')
    def test_frozen_native_three_specimens(self):
        m = producer(); binary = Path(os.environ['SHA256_BULK_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent))
        import process_guard
        from sha256_bulk_vm import run_users
        base = Path(os.environ['SHA256_BULK_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=256, output_mib=192, log_mib=16)
        for name, pattern, length, level, mutant in (
            ('a5-l1048576-single', 'a5', 1048576, 'single', False),
            ('zero-l64-full', 'zero', 64, 'full', False),
            ('a5-to-5a-l1048576-intent-mutant', 'a5', 1048576, 'single', True),
        ):
            row = m.make_case(pattern, length, level); qualify(self, row)
            original_request = m.encoded(row['request'])
            if mutant:
                row = intent_mutant(row)
                with self.assertRaisesRegex(AssertionError, 'input bytes'): run_users(row)
            p = row['parameters']; root = base / name; root.mkdir(); source = root / 'request.json'
            source.write_bytes(m.encoded(row['request'])); (root / 'requested-intent.json').write_bytes(original_request)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{f'cases/{n}': Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/sha256_bulk.py': Path(m.__file__), 'tests/test_sha256_bulk.py': Path(__file__),
                'tests/sha256_bulk_vm.py': Path(__file__).with_name('sha256_bulk_vm.py'),
                **{f'{n}.py': CASES.parent / f'{n}.py' for n in ('process_guard', 'artifacts', 'shared_memory')}}
            for relative, path in source_paths.items():
                target = sources / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(path.read_bytes())
            growing = [root / n for n in ('input.json', 'input.bin', 'native.json')]
            command = [str(binary), '--input', str(source), '--output', str(growing[0]), '--output-bincode', str(growing[1]),
                '--report', str(growing[2]), '--execution-profile', 'evm-gas-only']
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), limits=limits,
                native_sha256=NATIVE_SHA256, request_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                requested_intent_request_sha256=hashlib.sha256(original_request).hexdigest(),
                specimen_role='native-accepted-intent-mutant' if mutant else 'original-positive',
                mutation='a5->5a seed and matching real digest XOR guards' if mutant else None,
                streams='stdout and stderr combined in native.log by existing process_guard',
                source_sha256={n: hashlib.sha256(path.read_bytes()).hexdigest() for n, path in source_paths.items()},
                helper_source_sha256={n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()})
            # Require space for both bounded output and retained log beyond the free floor.
            self.assertGreaterEqual(shutil.disk_usage(root).free, (256 + 192 + 16) * 1024 * 1024)
            (root / 'process.json').write_bytes(m.encoded(manifest)); start = time.monotonic()
            try:
                outcome = process_guard.run(command, root, 'native', limits, growing=growing)
                manifest.update(exit_code=outcome['exit_code'], outcome='success')
            except BaseException as error:
                path = root / 'native-outcome.json'
                retained = json.loads(path.read_bytes()) if path.is_file() else {}
                manifest.update(exit_code=retained.get('exit_code'), outcome='failed', error=f'{type(error).__name__}: {error}')
                raise
            finally:
                manifest['elapsed_seconds'] = time.monotonic() - start
                manifest['files_sha256'] = {str(f.relative_to(root)): hashlib.sha256(f.read_bytes()).hexdigest()
                    for f in root.rglob('*') if f.is_file() and f.name != 'process.json'}
                (root / 'process.json').write_bytes(m.encoded(manifest))
            self.assertEqual(manifest['exit_code'], 0)
            report = json.loads(growing[2].read_bytes()); receipts = report['receipts_by_block'][0]
            self.assertEqual([r['success'] for r in receipts], [True] * 3)
            self.assertEqual([r['gas_used'] for r in receipts], [167355] + [p['expected_user_charged_gas']] * 2)
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            initialized = report['prestate_contracts']['initialized_storage']; self.assertEqual(len(initialized), 2)
            for entry in initialized:
                self.assertEqual((entry['initial_storage_entry_count'], entry['final_storage_entry_count']), (0, 0))
                self.assertEqual(entry['initial_storage_root'], entry['final_storage_root'])


def intent_mutant(row):
    """Native-valid a5→5a intent mutation with matching single-call XOR guards."""
    result = copy.deepcopy(row); p = result['parameters']; assert p['pattern'] == 'a5' and p['repetitions'] == 1
    seed = b'\x7f' + b'\xa5' * 32
    for tag, contract in enumerate(result['request']['program']['prestate_contracts']):
        code = bytes.fromhex(contract['code_hex'][2:]); assert code.count(seed) == 1
        old = hashlib.sha256((tag << 128).to_bytes(32, 'big') + b'\xa5' * (p['input_bytes'] - 32)).digest()
        new = hashlib.sha256((tag << 128).to_bytes(32, 'big') + b'\x5a' * (p['input_bytes'] - 32)).digest()
        assert code.count(b'\x7f' + old) == 1
        contract['code_hex'] = '0x' + code.replace(seed, b'\x7f' + b'\x5a' * 32).replace(b'\x7f' + old, b'\x7f' + new).hex()
    return result


if __name__ == '__main__': unittest.main()
