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

LINEAR_LENGTHS = (1269664, 1269665, 2097152)
LINEAR_GRID = tuple(itertools.product(('zero', 'a5'), LINEAR_LENGTHS, ('zero', 'single', 'full')))
# Frozen published default-panel canonical ProgramRequests; source identities may change.
DEFAULT_REQUEST_SHA256 = {
    'sha256-bulk-zero-l64-n0': 'f31c2c3bd56d0570b67084f85beb436ad97b55f5fa51810ca1bdf625a41e253b',
    'sha256-bulk-zero-l64-n1': '87f9cc5133ebfff9038a7f384d8b5e12e7abf86c393f19c74d7fafae785aede9',
    'sha256-bulk-zero-l64-n42557': '215a3f340383d299d24838b55397762effe1b4f069b2779e17380212f772f2be',
    'sha256-bulk-zero-l4096-n0': '9677d6336021affb98c9d08bf002556f2c7f86d5a518dce046d582a81a7f1e9e',
    'sha256-bulk-zero-l4096-n1': '468c90bdd01fb499bdd2bc324ab7faf2b4330092e30b19464917674559995270',
    'sha256-bulk-zero-l4096-n7999': 'e01d776b25e277b2d17acb7ad2bc8379efead15ac689be7d5fe9637e93284d36',
    'sha256-bulk-zero-l1048576-n0': '6a98319aadd1daf123d69080b8c7c187dd95fadd55cb5ce660126bdfd351047a',
    'sha256-bulk-zero-l1048576-n1': '7a2ea3bdda0832382181bf77ee36a36204d76c6563dc42cd41475d0c7e49c9ce',
    'sha256-bulk-zero-l1048576-n32': 'eb52d140aaa36c7f90e2283a21844cdc7bc0c1b7aa2cd775e2cc07b513639aad',
    'sha256-bulk-a5-l64-n0': 'fb26c255ccd9600045a0b0a4affe117ad57a72494fc6d83a7d77327b324cdd0b',
    'sha256-bulk-a5-l64-n1': '5a56326c73c49f970218b13409eeb3736f95b68613f17a7c82009e1cbf928e1e',
    'sha256-bulk-a5-l64-n42557': 'be115d1e2df2f6f2dec030dd2fb46600240e5bdd23fb5e3f0839b656ddec1614',
    'sha256-bulk-a5-l4096-n0': '44705922e2cd8468361d319dee38ec40d7bb6630fcf048f470f442320f19d747',
    'sha256-bulk-a5-l4096-n1': '8cdd4ff6f5fd2cbc2d3a336740f4a9fed8b73903cbc18a3cba5ee2a0a3468b15',
    'sha256-bulk-a5-l4096-n7999': 'd2d61f8f44f4f3bdab8cf1bc5e43bf7c08ecb2c939d073a75c946a1525de839e',
    'sha256-bulk-a5-l1048576-n0': '9383a1a6300309a0f2a1639876978d4170f8e63f09229878d38d1713f0637952',
    'sha256-bulk-a5-l1048576-n1': '55f36d9e884933f85d49b269e2765ca45dd03bc2cc1c64bb47d3bf500b0b3410',
    'sha256-bulk-a5-l1048576-n32': 'a277b552b350d1c6c41200ac43fe3f0a84ec56638c9c76f2d2f96b74919a5053',
}

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
    test.assertIn((p['pattern'], p['input_bytes'], p['workload_level']), GRID + LINEAR_GRID)
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
        test.assertEqual(len(result['memory']), 32 * (3 + (p['input_bytes'] + 31) // 32))
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
        self._native_three_specimens(linear_regime=False)

    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('SHA256_BULK_NATIVE_BIN'),
        'set SHA256_BULK_NATIVE_BIN for optional Linux frozen-native smoke')
    def test_frozen_native_linear_three_specimens(self):
        self._native_three_specimens(linear_regime=True)

    def _native_three_specimens(self, *, linear_regime):
        m = producer(); binary = Path(os.environ['SHA256_BULK_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent))
        import process_guard
        from sha256_bulk_vm import run_users
        base = Path(os.environ['SHA256_BULK_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=256, output_mib=192, log_mib=16)
        specimens = (
            ('zero-l1269664-single', 'zero', 1269664, 'single', False),
            ('a5-l1269665-full', 'a5', 1269665, 'full', False),
            ('zero-l2097152-single', 'zero', 2097152, 'single', False),
        ) if linear_regime else (
            ('a5-l1048576-single', 'a5', 1048576, 'single', False),
            ('zero-l64-full', 'zero', 64, 'full', False),
            ('a5-to-5a-l1048576-intent-mutant', 'a5', 1048576, 'single', True),
        )
        for name, pattern, length, level, mutant in specimens:
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
                specimen_role='native-accepted-intent-mutant' if mutant else 'linear-regime-positive' if linear_regime else 'original-positive',
                mutation='a5->5a seed and matching real digest XOR guards' if mutant else None,
                streams='stdout and stderr combined in native.log by existing process_guard',
                source_sha256={n: hashlib.sha256(path.read_bytes()).hexdigest() for n, path in source_paths.items()},
                helper_source_sha256={n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()})
            # Require space for both bounded output and retained log beyond the free floor.
            free = shutil.disk_usage(root).free
            required_free = (576 if linear_regime else 256 + 192 + 16) * 1024 * 1024
            manifest['disk_preflight'] = dict(free_bytes=free, required_free_bytes=required_free)
            self.assertGreaterEqual(free, required_free)
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


class Sha256LinearRegimeTests(unittest.TestCase):
    def test_linear_selector_and_original_request_hashes(self):
        m = producer()
        self.assertIn('linear_regime', inspect.signature(m.prepare).parameters)
        self.assertEqual(inspect.signature(m.prepare).parameters['linear_regime'].kind, inspect.Parameter.KEYWORD_ONLY)
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary); all_requests = set()
            for mode, lengths, corpus in ((False, (64, 4096, 1048576), 'unzen-sha256-bulk-20261104'),
                    (True, LINEAR_LENGTHS, 'unzen-sha256-linear-regime-20261104')):
                output = base / str(mode); self.assertEqual(m.prepare(output, linear_regime=mode), 18)
                catalog = json.loads((output / 'catalog.json').read_bytes()); rows = catalog['cases']
                self.assertEqual((catalog['lengths'], catalog['corpus_id']), (list(lengths), corpus))
                identities = {row['request_sha256'] for row in rows}
                self.assertEqual(len(identities), 18); self.assertFalse(all_requests & identities); all_requests.update(identities)
                self.assertEqual({row['corpus_id'] for row in rows}, {corpus})
                self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
                self.assertEqual(catalog['helper_source_sha256'], {n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()})
                if not mode: self.assertEqual({r['sample_id']: r['request_sha256'] for r in rows}, DEFAULT_REQUEST_SHA256)
                by = {r['sample_id']: r for r in rows}
                for row in rows:
                    raw = (output / row['request_path']).read_bytes()
                    self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                    self.assertEqual(raw, m.encoded(json.loads(raw)))
                    self.assertEqual(row['parameters']['transaction_gas_limit'], by[row['baseline_sample_id']]['parameters']['transaction_gas_limit'])
                with self.assertRaises(FileExistsError): m.prepare(output, linear_regime=mode)
            self.assertEqual(len(all_requests), 36)
            for invalid in (None, 0, 1, 'true', [], {}):
                output = base / 'invalid'
                with self.assertRaises(ValueError): m.prepare(output, linear_regime=invalid)
                self.assertFalse(output.exists())
            command = [sys.executable, '-S', str(CASES / 'sha256_bulk.py'), '--linear-regime', '--output-dir', str(base / 'cli')]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), {'prepared': 18})
            catalog = json.loads((base / 'cli/catalog.json').read_bytes())
            self.assertEqual(catalog['lengths'], list(LINEAR_LENGTHS))
            self.assertEqual(catalog['corpus_id'], 'unzen-sha256-linear-regime-20261104')
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)

    def test_linear_eighteen_actual_bytes_gas_copy_and_output(self):
        m = producer()
        for args in LINEAR_GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); records = qualify(self, row); p = row['parameters']
                self.assertEqual(row['corpus_id'], 'unzen-sha256-linear-regime-20261104')
                words = {1269664: 39680, 1269665: 39681, 2097152: 65539}[args[1]]
                native = {1269664: 476184, 1269665: 476196, 2097152: 786492}[args[1]]
                self.assertEqual(p['memory_words'], words)
                self.assertEqual(p['gas_accounting_per_user']['memory_expansion_gas'], 3 * words + words * words // 512)
                self.assertEqual(p['gas_accounting_per_user']['native_gas_per_call'], native)
                for result in records:
                    self.assertEqual(sum(c['length'] for c in result['copies']) + 32, args[1])
                    self.assertEqual(result['copies'][-1]['length'], {1269664: 221088, 1269665: 221089, 2097152: 1048576}[args[1]])
                    self.assertEqual(result['copies'][-1]['destination'] + result['copies'][-1]['length'], 64 + args[1])
                    self.assertEqual(result['memory'][64 + args[1] + 32:], bytes(32 * words - (96 + args[1])))

    def test_linear_full_plus_one_and_fixed_shape(self):
        m = producer(); from sha256_bulk_vm import execute, decode
        with patch.object(m, 'expected_xor', side_effect=AssertionError('digest in sizing')):
            maxima = {(pattern, length): m.full_repetitions(pattern, length) for pattern, length in itertools.product(('zero', 'a5'), LINEAR_LENGTHS)}
        for (pattern, length), maximum in maxima.items():
            self.assertTrue(m.fits(pattern, length, maximum)); self.assertFalse(m.fits(pattern, length, maximum + 1))
            shapes = set(); sizes = set()
            for tag, r in itertools.product((0, 1), (0, 1, maximum, maximum + 1)):
                code, counts = m._runtime(pattern, length, r, tag)
                shapes.add(tuple(op for _, op, _ in decode(code))); sizes.add(len(code))
                if r == maximum + 1:
                    result = execute(code, 50_000_000, pattern=pattern, length=length, tag=tag, repetitions=r)
                    self.assertEqual((result['terminal'], result['counter']), (0, r))
                    self.assertEqual(result['counts'], +Counter(counts))
                    gas = m.gas_accounting(pattern, length, r)
                    self.assertEqual(result['gas'], gas['gross_gas'])
                    envelope = gas['gross_gas'] + (gas['native_gas_per_call'] + 62) // 63 + 2301
                    self.assertTrue(envelope > min(15_000_000, 16_777_216) or
                        167355 + 2 * gas['gross_gas'] > 30_000_000 or
                        any(31_000_000 - 167355 - j * gas['gross_gas'] < envelope for j in range(2)))
            self.assertEqual((len(shapes), len(sizes)), (1, 1))
            self.assertEqual(m.gas_accounting('zero', length, 1), m.gas_accounting('a5', length, 1))
        for length in (0, 32, 63, 65, 1269663, 1269666, 2097151, 2097153, True, 1269665.0):
            with self.assertRaises(ValueError): m.make_case('zero', length, 'zero')

    def test_linear_partial_copy_output_and_return_guards(self):
        m = producer(); from sha256_bulk_vm import decode, execute
        length = 1269665; code, _ = m._runtime('a5', length, 2)
        def run(image=code, **kwargs):
            return execute(image, 50_000_000, pattern='a5', length=length, tag=0, repetitions=2, **kwargs)
        self.assertEqual(run()['terminal'], 0)
        for override in ({'status': 0}, {'return_size': 31}, {'return_size': 33}, {'write_size': 0}, {'write_size': 31}):
            with self.subTest(override=override): self.assertEqual(run(override=override)['terminal'], 0xfe)
        with self.assertRaisesRegex(AssertionError, 'per-call returned digest'): run(override={'wrong_digest': True})
        single, _ = m._runtime('a5', length, 1)
        with self.assertRaisesRegex(AssertionError, 'forwarded gas'):
            execute(single, m.gas_accounting('a5', length, 1)['gross_gas'], pattern='a5', length=length, tag=0, repetitions=1)
        instructions = list(decode(code))
        last_copy = max(i for i, (_, op, _) in enumerate(instructions) if op == 0x5e)
        at, op, value = instructions[last_copy - 3]
        self.assertEqual((op, value), (0x62, 221089))
        for wrong_size in (221088, 221090):
            wrong = bytearray(code); wrong[at + 1:at + 4] = wrong_size.to_bytes(3, 'big')
            with self.assertRaisesRegex(AssertionError, 'MCOPY schedule'): run(bytes(wrong))
        call = next(i for i, (_, op, _) in enumerate(instructions) if op == 0xfa)
        at, op, value = instructions[call - 5]
        self.assertEqual((op, value), (0x62, 64 + length))
        wrong = bytearray(code); wrong[at + 1:at + 4] = (64 + length - 1).to_bytes(3, 'big')
        with self.assertRaisesRegex(AssertionError, 'STATICCALL operands'): run(bytes(wrong))

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
