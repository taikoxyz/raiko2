"""Independent actual-byte LOG/receipt-intent qualification, not receipt-root proof."""
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
DOMAINS = ((0, 'zero'), (64, 'zero'), (64, 'a5'), (262144, 'zero'), (262144, 'a5'))
GRID = tuple((n, length, pattern, level) for n in (1, 4) for length, pattern in DOMAINS for level in ('zero', 'single', 'full'))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'log_receipt_bulk.py'
    assert path.is_file(), 'LOG bulk producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('log_receipt_bulk', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from log_receipt_vm import run_users
    p = row['parameters']; program = row['request']['program']
    test.assertIn((p['topics'], p['input_bytes'], p['pattern'], p['workload_level']), GRID)
    test.assertEqual(p['repetitions'], {'zero': 0, 'single': 1, 'full': p['full_repetitions']}[p['workload_level']])
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=2, gas_limit=p['transaction_gas_limit'], recipients=p['recipient_schedule']))
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    test.assertEqual(row['request']['schema_version'], 1); test.assertIs(row['request']['expected_user_success'], True)
    results = run_users(row); routes = Counter(); spent = 167355; prefixes = []
    for tag, (contract, result) in enumerate(zip(program['prestate_contracts'], results)):
        code = bytes.fromhex(contract['code_hex'][2:])
        test.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'][tag])
        test.assertEqual(len(code), p['runtime_bytes']); test.assertLess(len(code), 24576)
        test.assertEqual(result['terminal'], 0); test.assertEqual(result['stack'], [])
        test.assertEqual(result['counter'], p['repetitions']); test.assertEqual(result['log_count'], p['repetitions'])
        test.assertEqual(result['last_input'], int(p['expected_last_input_hex'][tag], 16))
        test.assertEqual(result['sequence_sha256'], p['expected_log_sequence_sha256'][tag])
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['mcopy_gas'], p['gas_accounting_per_user']['mcopy_gas'])
        test.assertEqual(result['log_gas'], p['repetitions'] * (375 + 375 * p['topics'] + 8 * p['input_bytes']))
        test.assertEqual(len(result['memory']), 32 * p['memory_words'])
        test.assertEqual(sum(c['length'] for c in result['copies']) + 32, max(32, p['input_bytes']))
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    for op in (0x54, 0x55, 0x5c, 0x5d, 0xf0, 0xf1, 0xf4, 0xfa, 0xfd, 0xfe): test.assertEqual(routes[f'opcode:0x{op:02x}'], 0)
    return results


class LogReceiptBulkTests(unittest.TestCase):
    def test_all_thirty_two_owner_trajectories(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(*args[:3], p['full_repetitions'])['gross_gas'] + 1000)

    def test_full_plus_one_independent_gas_and_fixed_shape(self):
        m = producer(); from log_receipt_vm import execute, decode
        with patch.object(m, 'expected_sequence', side_effect=AssertionError('digests during sizing')):
            maxima = {(n, length, pattern): m.full_repetitions(n, length, pattern) for n in (1, 4) for length, pattern in DOMAINS}
        for (n, length, pattern), maximum in maxima.items():
            shapes = set(); sizes = set()
            self.assertTrue(m.fits(n, length, pattern, maximum)); self.assertFalse(m.fits(n, length, pattern, maximum + 1))
            for tag, r in itertools.product((0, 1), (0, 1, maximum, maximum + 1)):
                code, counts = m._runtime(n, length, pattern, r, tag)
                shapes.add(tuple(op for _, op, _ in decode(code))); sizes.add(len(code))
                if r == maximum + 1:
                    result = execute(code, 50_000_000, topics=n, length=length, pattern=pattern, tag=tag, repetitions=r)
                    self.assertEqual((result['terminal'], result['counter'], result['log_count']), (0, r, r))
                    self.assertEqual(result['counts'], +Counter(counts)); gas = m.gas_accounting(n, length, pattern, r)
                    self.assertEqual(result['gas'], gas['gross_gas'])
                    w = 2 + max(32, length) // 32
                    self.assertEqual(gas['memory_expansion_gas'], 3 * w + w * w // 512)
                    self.assertEqual(gas['log_gas'], r * (375 + 375 * n + 8 * length))
                    self.assertEqual(gas['mcopy_gas'], result['mcopy_gas'])
                    envelope = gas['gross_gas'] + 1000
                    self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas['gross_gas'] > 30_000_000 or any(31_000_000 - 167355 - j * gas['gross_gas'] < envelope for j in range(2)))
            self.assertEqual((len(shapes), len(sizes)), (1, 1))
        for n, length in itertools.product((1, 4), (64, 262144)):
            self.assertEqual(m.gas_accounting(n, length, 'zero', 1), m.gas_accounting(n, length, 'a5', 1))

    def test_strict_domains_and_separate_constraints(self):
        m = producer()
        for args in ((True,), (0,), (2,), (1, True), (1, 32), (1, 64.0), (1, 0, 'a5'), (1, 64, True), (1, 64, 'A5'), (1, 64, 'a5', True), (1, 64, 'a5', 'partial')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for fn in (m._runtime, m.gas_accounting, m.fits):
            for r in (True, -1, 1.0, None, 1_000_001):
                with self.assertRaises(ValueError): fn(1, 64, 'zero', r)
        for tag in (True, -1, 2, 0.0, None):
            with self.assertRaises(ValueError): m._runtime(1, 64, 'zero', 0, tag)
            with self.assertRaises(ValueError): m.expected_sequence(1, 64, 'zero', 0, tag)
        for field, value in (('TRANSACTION_GAS_CAP', 21000), ('EIP7825_GAS_CAP', 21000), ('BLOCK_BUDGET', 167355), ('HEADER_GAS_LIMIT', 167355)):
            with patch.object(m, field, value): self.assertFalse(m.fits(1, 64, 'zero', 0))

    def test_semantic_negatives(self):
        m = producer(); from log_receipt_vm import execute, decode, run_users
        code, _ = m._runtime(4, 64, 'a5', 2, 1); ins = list(decode(code))
        def run(image): return execute(image, 50_000_000, topics=4, length=64, pattern='a5', tag=1, repetitions=2)
        self.assertEqual(run(code)['terminal'], 0)
        def change(at, width, value):
            wrong = bytearray(code); wrong[at + 1:at + 1 + width] = value.to_bytes(width, 'big'); return bytes(wrong)
        log = next(i for i, (_, op, _) in enumerate(ins) if op == 0xa4)
        for index, value in ((log - 1, 32), (log - 2, 32)):
            at, op, _ = ins[index]
            with self.assertRaisesRegex(AssertionError, 'LOG operands'): run(change(at, op - 0x5f, value))
        prefixes = [(at, op, v) for at, op, v in ins[:log] if op == 0x7f and v in [(1 << 128) | (j << 64) for j in range(4)]]
        for at, op, value in prefixes:
            with self.subTest(prefix=value), self.assertRaises(AssertionError): run(change(at, 32, value ^ (1 << 80)))
        a, b = prefixes[-1], prefixes[-2]
        wrong = bytearray(code); wrong[a[0] + 1:a[0] + 33] = b[2].to_bytes(32, 'big'); wrong[b[0] + 1:b[0] + 33] = a[2].to_bytes(32, 'big')
        with self.assertRaisesRegex(AssertionError, 'ordered topics'): run(bytes(wrong))
        seed = next(at for at, op, v in ins if op == 0x7f and v == int.from_bytes(b'\xa5' * 32, 'big'))
        with self.assertRaises(AssertionError): run(change(seed, 32, 0))
        copy_index = next(i for i, (_, op, _) in enumerate(ins) if op == 0x5e)
        for index in (copy_index - 1, copy_index - 2, copy_index - 3):
            at, op, value = ins[index]
            with self.assertRaises(AssertionError): run(change(at, op - 0x5f, value + 1))
        # Remove/duplicate the entire event-producing body, relocating real jump
        # immediates so failures prove semantics rather than broken PC arithmetic.
        body_start = next(i for i, (_, op, _) in enumerate(ins) if op == 0x57) + 1
        for mode in ('skip', 'duplicate'):
            wrong = rewrite_block(code, ins[body_start][0], ins[log + 1][0], mode)
            if mode == 'duplicate':
                with self.assertRaisesRegex(AssertionError, 'LOG index/count'): run(wrong)
            else: self.assertEqual(run(wrong)['terminal'], 0xfe)
        # A changed increment exits early or hits the final counter guard.
        needle = bytes.fromhex('62000000516200000101'); at = code.index(needle) + 5
        self.assertEqual(run(change(at, 3, 2))['terminal'], 0xfe)
        # Change the loop exit to one event, preserving declared final count two.
        at = next(at for at, op, value in ins if op == 0x7f and value == 2)
        self.assertEqual(run(change(at, 32, 1))['terminal'], 0xfe)
        # The final expected counter and topic index are separately enforced.
        at = next(at for at, op, value in reversed(ins) if op == 0x7f and value == 2)
        self.assertEqual(run(change(at, 32, 1))['terminal'], 0xfe)
        at, op, value = prefixes[-1]
        with self.assertRaisesRegex(AssertionError, 'ordered topics'): run(change(at, 32, value | 1))
        with self.assertRaisesRegex(AssertionError, 'out of gas'):
            execute(code, m.gas_accounting(4, 64, 'a5', 2)['gross_gas'] - 1,
                topics=4, length=64, pattern='a5', tag=1, repetitions=2)
        # Final expected last-input immediate is distinct from the runtime topic recipe.
        at = next(at for at, op, v in reversed(ins) if op == 0x7f and v == (1 << 128) | 1)
        self.assertEqual(run(change(at, 32, 0))['terminal'], 0xfe)
        for mutation in ('recipient', 'owner', 'storage', 'code', 'count'):
            row = m.make_case(1, 64, 'a5', 'single'); p = row['request']['program']; contracts = p['prestate_contracts']
            if mutation == 'recipient': p['transactions']['recipients'].reverse()
            elif mutation == 'owner': contracts[1]['address'] = contracts[0]['address']
            elif mutation == 'storage': contracts[0]['initial_storage'] = {'0x00': '0x01'}
            elif mutation == 'code': contracts[1]['code_hex'] = contracts[0]['code_hex']
            else: p['transactions']['count_per_block'] = 1
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): run_users(row)
        with self.assertRaisesRegex(AssertionError, 'ordered topics'): run_users(intent_mutant(m.make_case(1, 64, 'a5', 'single')))

    def test_catalog_cli_canonical_sources(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 30)
            catalog = json.loads((output / 'catalog.json').read_bytes()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 30); self.assertEqual(len({r['request_sha256'] for r in rows}), 30)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {n: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for n, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()})
            for row in rows:
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(raw, m.encoded(json.loads(raw)))
                self.assertEqual(row['parameters']['transaction_gas_limit'], by[row['baseline_sample_id']]['parameters']['transaction_gas_limit'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for invalid in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=invalid)
                self.assertFalse(target.exists())
            for invalid in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(invalid)
            command = [sys.executable, '-S', str(CASES / 'log_receipt_bulk.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 30})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('LOG_RECEIPT_BULK_NATIVE_BIN'),
        'set LOG_RECEIPT_BULK_NATIVE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['LOG_RECEIPT_BULK_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from log_receipt_vm import run_users
        base = Path(os.environ['LOG_RECEIPT_BULK_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=256, output_mib=192, log_mib=16)
        specimens = ((1, 0, 'zero', 'full', False), (4, 262144, 'a5', 'single', False), (1, 64, 'a5', 'single', True))
        for topics, length, pattern, level, mutant in specimens:
            row = m.make_case(topics, length, pattern, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            if mutant:
                row = intent_mutant(row)
                with self.assertRaisesRegex(AssertionError, 'ordered topics'): run_users(row)
            name = f"log{topics}-l{length}-{pattern}-{level}-" + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{f'cases/{n}': Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/log_receipt_bulk.py': Path(m.__file__), 'tests/test_log_receipt_bulk.py': Path(__file__),
                'tests/log_receipt_vm.py': Path(__file__).with_name('log_receipt_vm.py'),
                **{f'{n}.py': CASES.parent / f'{n}.py' for n in ('process_guard', 'artifacts', 'shared_memory')}}
            for relative, path in source_paths.items():
                target = sources / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(path.read_bytes())
            growing = [root / n for n in ('input.json', 'input.bin', 'native.json')]
            command = [str(binary), '--input', str(source), '--output', str(growing[0]), '--output-bincode', str(growing[1]),
                '--report', str(growing[2]), '--execution-profile', 'evm-gas-only']
            free = shutil.disk_usage(root).free; self.assertGreaterEqual(free, 576 * 1024 * 1024)
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), limits=limits,
                native_sha256=NATIVE_SHA256, request_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                requested_intent_request_sha256=hashlib.sha256(original_request).hexdigest(),
                specimen_role='native-accepted-topic-intent-mutant' if mutant else 'original-positive',
                mutation='topic0 prefix bit80 added; code gas/routes/loop guards unchanged' if mutant else None,
                streams='combined stdout/stderr native.log, bounded by unchanged process_guard',
                observability='Native status/gas/routes plus complete execution_result replay; no native per-log data/topics export.',
                disk_preflight=dict(free_bytes=free, required_free_bytes=576 * 1024 * 1024),
                source_sha256={n: hashlib.sha256(path.read_bytes()).hexdigest() for n, path in source_paths.items()},
                helper_source_sha256={n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()},
                outcome='not-launched', exit_code=None, elapsed_seconds=None,
                output_paths=[str(path.relative_to(root)) for path in growing])
            manifest['prelaunch_files_sha256'] = {str(f.relative_to(root)): hashlib.sha256(f.read_bytes()).hexdigest() for f in root.rglob('*') if f.is_file()}
            (root / 'process.json').write_bytes(m.encoded(manifest)); start = time.monotonic()
            try:
                outcome = process_guard.run(command, root, 'native', limits, growing=growing)
                manifest.update(exit_code=outcome['exit_code'], outcome='success')
            except BaseException as error:
                retained = json.loads((root / 'native-outcome.json').read_bytes()) if (root / 'native-outcome.json').is_file() else {}
                manifest.update(exit_code=retained.get('exit_code'), outcome='failed', error=f'{type(error).__name__}: {error}')
                raise
            finally:
                manifest['elapsed_seconds'] = time.monotonic() - start
                manifest['files_sha256'] = {str(f.relative_to(root)): hashlib.sha256(f.read_bytes()).hexdigest() for f in root.rglob('*') if f.is_file() and f.name != 'process.json'}
                (root / 'process.json').write_bytes(m.encoded(manifest))
            self.assertEqual(manifest['exit_code'], 0); report = json.loads(growing[2].read_bytes()); receipts = report['receipts_by_block'][0]
            self.assertEqual([r['success'] for r in receipts], [True] * 3)
            self.assertEqual([r['gas_used'] for r in receipts], [167355] + [row['parameters']['expected_user_charged_gas']] * 2)
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            initialized = report['prestate_contracts']['initialized_storage']; self.assertEqual(len(initialized), 2)
            for entry in initialized:
                self.assertEqual((entry['initial_storage_entry_count'], entry['final_storage_entry_count']), (0, 0))
                self.assertEqual(entry['initial_storage_root'], entry['final_storage_root'])


def rewrite_block(code, start, end, mode):
    """Test-only deletion/duplication with jump relocation; never production code."""
    from log_receipt_vm import decode
    ins = list(decode(code)); selected = [at for at, _, _ in ins if start <= at < end]
    assert selected and mode in ('skip', 'duplicate')
    layout = [at for at, _, _ in ins if mode != 'skip' or not start <= at < end]
    if mode == 'duplicate':
        index = layout.index(end); layout[index:index] = selected
    widths = {at: 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0) for at, op, _ in ins}
    offsets = {}; cursor = 0
    for at in layout:
        offsets.setdefault(at, cursor); cursor += widths[at]
    jumps = {at for at, op, _ in ins if op == 0x5b}; image = bytearray()
    for at in layout:
        piece = code[at:at + widths[at]]
        if piece[0] == 0x61 and int.from_bytes(piece[1:], 'big') in jumps:
            piece = piece[:1] + offsets[int.from_bytes(piece[1:], 'big')].to_bytes(2, 'big')
        image.extend(piece)
    return bytes(image)


def intent_mutant(row):
    """Native-valid topic-prefix mutation: EVM cannot read LOG topics back."""
    from log_receipt_vm import decode
    result = copy.deepcopy(row); assert row['parameters']['topics'] == 1
    for tag, contract in enumerate(result['request']['program']['prestate_contracts']):
        code = bytearray.fromhex(contract['code_hex'][2:]); ins = list(decode(code)); log = next(i for i, (_, op, _) in enumerate(ins) if op == 0xa1)
        at, op, value = ins[log - 4]; assert op == 0x7f and value == tag << 128
        code[at + 1:at + 33] = (value | (1 << 80)).to_bytes(32, 'big'); contract['code_hex'] = '0x' + code.hex()
    return result


if __name__ == '__main__': unittest.main()
