"""Finite division panel: independent byte math, gas, mutation and native gates."""
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
OPS = ('div', 'sdiv', 'mod', 'smod')
CLASSES = ('limb1', 'limb2', 'limb3', 'limb4')
GRID = tuple(itertools.product(OPS, CLASSES, ('zero', 'full')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'division_density.py'
    assert path.is_file(), 'division density producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('division_density', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from division_density_vm import run_users, operands, arithmetic
    p = row['parameters']; program = row['request']['program']
    a, b = operands(p['operation'], p['divisor_class'])
    test.assertEqual([p[k] for k in ('operand_a_hex', 'operand_b_hex', 'expected_result_hex')],
        [f'0x{v:064x}' for v in (a, b, arithmetic(p['operation'], a, b))])
    test.assertEqual(p['bulk_operations'], 64 * p['repetitions'])
    test.assertEqual(p['target_operations_per_user'], 64 * p['repetitions'] + 1)
    results = run_users(row); routes = Counter(); spent = 167355; prefixes = []
    for tag, (contract, result) in enumerate(zip(program['prestate_contracts'], results)):
        code = bytes.fromhex(contract['code_hex'][2:])
        test.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'][tag])
        test.assertEqual(len(code), p['runtime_bytes']); test.assertLess(len(code), 24576)
        test.assertEqual((result['terminal'], result['stack'], result['counter']), (0, [], p['repetitions']))
        test.assertEqual(result['target_operations'], 64 * p['repetitions'] + 1)
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['counter_progression'], list(range(p['repetitions'] + 1)))
        test.assertEqual(len(result['sites']), 65)
        for site in result['sites']:
            test.assertEqual((site['numerator'], site['divisor'], site['result']), (a, b, arithmetic(p['operation'], a, b)))
            test.assertEqual(site['visits'], 1 if site['phase'] == 'warmup' else p['repetitions'])
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    test.assertEqual(p['gas_accounting_per_user']['memory_expansion_gas'], 6)
    return results


class DivisionDensityTests(unittest.TestCase):
    def test_explicit_repetitions_drive_bytes_counts_guards_and_gas(self):
        m = producer(); from division_density_vm import execute
        for operation, limb in itertools.product(OPS, CLASSES):
            maximum = m.full_repetitions(operation, limb)
            full = m.make_case(operation, limb)
            with self.assertRaises(ValueError): m.make_case(operation, limb, repetitions=maximum + 1)
            for repetitions in (0, 3, maximum // 2, maximum):
                with self.subTest(operation=operation, limb=limb, repetitions=repetitions):
                    row = m.make_case(operation, limb, repetitions=repetitions)
                    p = row['parameters']; results = qualify(self, row)
                    self.assertEqual(p['workload_level'], 'explicit')
                    self.assertEqual(p['repetitions'], repetitions)
                    self.assertEqual(p['expected_counter'], repetitions)
                    self.assertEqual(p['full_repetitions'], maximum)
                    self.assertEqual(row['role'], 'control' if repetitions == 0 else 'candidate')
                    self.assertEqual(p['transaction_gas_limit'], full['parameters']['transaction_gas_limit'])
                    self.assertEqual(row['request']['program']['transactions']['gas_limit'], p['transaction_gas_limit'])
                    if repetitions == 3:
                        for tag, contract in enumerate(row['request']['program']['prestate_contracts']):
                            code = bytes.fromhex(contract['code_hex'][2:])
                            self.assertEqual(results[tag], execute(code, p['transaction_gas_limit'],
                                operation=operation, divisor_class=limb, repetitions=3, tag=tag, cache=False))
                    if repetitions in (0, maximum):
                        expected = m.make_case(operation, limb, 'zero' if repetitions == 0 else 'full')
                        expected['parameters']['workload_level'] = 'explicit'
                        self.assertEqual(row, expected)
        self.assertEqual(m.make_case(), m.make_case(repetitions=None))

    def test_explicit_repetitions_reject_invalid_or_ambiguous_overrides(self):
        m = producer(); maximum = m.full_repetitions('div', 'limb1')
        for repetitions in (True, False, -1, 1.0, '1', [], maximum + 1):
            with self.subTest(repetitions=repetitions), self.assertRaises(ValueError):
                m.make_case(repetitions=repetitions)
        with self.assertRaises(ValueError): m.make_case(level='zero', repetitions=0)
        with self.assertRaises(TypeError): m.make_case('div', 'limb1', 'full', 3)

    def test_all_32_two_owner_programs(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['repetitions'], 0 if args[2] == 'zero' else p['full_repetitions'])
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(*args[:2], p['full_repetitions'])['gross_gas'] + 1000)
                if args[2] == 'full': self.assertTrue(p['near_full'])

    def test_gas_only_full_plus_one_and_direct_cache_differential(self):
        m = producer(); from division_density_vm import execute, decode
        for op, limb in itertools.product(OPS, CLASSES):
            with patch.object(m, '_runtime', side_effect=AssertionError('runtime used by sizing')):
                m.gas_accounting(op, limb, 1_000_000); n = m.full_repetitions(op, limb)
                self.assertTrue(m.fits(op, limb, n)); self.assertFalse(m.fits(op, limb, n + 1))
            sizes = set(); shapes = set()
            for tag, r in itertools.product((0, 1), (0, 1, 2, n + 1)):
                code, counts = m._runtime(op, limb, r, tag); sizes.add(len(code)); shapes.add(tuple(i[1] for i in decode(code)))
                result = execute(code, 50_000_000, operation=op, divisor_class=limb, repetitions=r, tag=tag)
                self.assertEqual(result['counts'], +Counter(counts)); self.assertEqual(result['gas'], m.gas_accounting(op, limb, r)['gross_gas'])
                if r <= 2:
                    self.assertEqual(result, execute(code, 50_000_000, operation=op, divisor_class=limb, repetitions=r, tag=tag, cache=False))
                else:
                    gas = result['gas']; envelope = gas + 1000
                    self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas > 30_000_000 or any(31_000_000 - 167355 - j * gas < envelope for j in range(2)))
            self.assertEqual((len(sizes), len(shapes)), (1, 1))

    def test_signed_and_unsigned_math_domains(self):
        producer()
        from division_density_vm import arithmetic
        mask = (1 << 256) - 1; enc = lambda v: v & mask
        vectors = [(0, 0), (1, 0), (-1, 0), (-(1 << 255), -1), (2, 3), (3, 3), (-7, 3), (7, -3), (-7, -3)]
        vectors += [(sign * ((1 << bits) + 1), (1 << bits) + 1) for sign in (-1, 1) for bits in (64, 128, 192)]
        for a, b in vectors:
            q = 0 if b == 0 else (abs(a) // abs(b)) * (-1 if (a < 0) != (b < 0) else 1)
            self.assertEqual(arithmetic('sdiv', enc(a), enc(b)), enc(q))
            self.assertEqual(arithmetic('smod', enc(a), enc(b)), enc(0 if b == 0 else a - q * b))
            self.assertEqual(arithmetic('div', enc(a), enc(b)), 0 if enc(b) == 0 else enc(a) // enc(b))
            self.assertEqual(arithmetic('mod', enc(a), enc(b)), 0 if enc(b) == 0 else enc(a) % enc(b))

    def test_domains_and_separate_caps(self):
        m = producer()
        for op in (True, 1, None, [], 'DIV'):
            for fn in (m.make_case, m.full_repetitions):
                with self.assertRaises(ValueError): fn(op, 'limb1')
        for limb in (True, 1, None, [], 'limb0'):
            for fn in (m.make_case, m.full_repetitions):
                with self.assertRaises(ValueError): fn('div', limb)
        for r in (True, -1, 1.0, None, 1_000_001):
            for fn in (m._runtime, m.gas_accounting, m.fits):
                with self.assertRaises(ValueError): fn('div', 'limb1', r)
        for tag in (True, -1, 2, 0.0, None):
            with self.assertRaises(ValueError): m._runtime('div', 'limb1', 0, tag)
        for level in (True, None, [], 'single'):
            with self.assertRaises(ValueError): m.make_case(level=level)
        for field in ('TRANSACTION_GAS_CAP', 'EIP7825_GAS_CAP', 'BLOCK_BUDGET', 'HEADER_GAS_LIMIT'):
            with patch.object(m, field, 21000): self.assertFalse(m.fits('div', 'limb1', 0))

    def test_byte_mutations_and_schedule(self):
        m = producer(); from division_density_vm import execute, decode, run_users
        for op in OPS:
            code, _ = m._runtime(op, 'limb3', 2, 1); ins = list(decode(code))
            def run(image): return execute(image, 50_000_000, operation=op, divisor_class='limb3', repetitions=2, tag=1)
            def replace(at, value):
                wrong = bytearray(code); wrong[at] = value; return bytes(wrong)
            body = next(i for i in range(len(ins) - 255) if [x[1] for x in ins[i:i + 4]] == [0x81, 0x81, m.OPERATIONS[op], 0x50] and ins[i - 1][1] == 0x57)
            warmup = next(i for i, (_, opcode, _) in enumerate(ins) if opcode == m.OPERATIONS[op])
            for index in (warmup - 2, warmup - 1, warmup, body, body + 1, body + 2, body + 3, body + 127, body + 254):
                with self.assertRaises(AssertionError): run(replace(ins[index][0], 0x52 if index == body + 3 else 0x80))
            # Delete/duplicate site with relocation: reject malformed pure bodies, including R0.
            for mode in ('skip', 'duplicate'):
                with self.assertRaises(AssertionError): run(rewrite_block(code, ins[body][0], ins[body + 4][0], mode))
            zero_code, _ = m._runtime(op, 'limb3', 0, 1)
            for mode in ('skip', 'duplicate'):
                wrong = rewrite_block(zero_code, ins[body][0], ins[body + 4][0], mode)
                with self.assertRaises(AssertionError):
                    execute(wrong, 50_000_000, operation=op, divisor_class='limb3', repetitions=0, tag=1)
            for at, opcode, _ in ins:
                if opcode in (0x14, 0x15, 0x56, 0x57, 0x5b, 0):
                    with self.assertRaises(AssertionError): run(replace(at, 0x80))
            # Every immediate is part of initialization, operand/result, loop, tag or final guard intent.
            for at, opcode, value in ins:
                if opcode in (0x7f, 0x62, 0x61):
                    wrong = bytearray(code); wrong[at + opcode - 0x5f] ^= 1
                    with self.assertRaises(AssertionError): run(bytes(wrong))
            with self.assertRaisesRegex(AssertionError, 'out of gas'):
                execute(code, m.gas_accounting(op, 'limb3', 2)['gross_gas'] - 1, operation=op, divisor_class='limb3', repetitions=2, tag=1)
        for mutation in ('recipient', 'owner', 'storage', 'code', 'count', 'calldata', 'primary', 'gas-low', 'gas-cap', 'counter-intent'):
            row = m.make_case(level='zero'); program = row['request']['program']; contracts = program['prestate_contracts']
            if mutation == 'recipient': program['transactions']['recipients'].reverse()
            elif mutation == 'owner': contracts[1]['address'] = contracts[0]['address']
            elif mutation == 'storage': contracts[0]['initial_storage'] = {'0x00': '0x01'}
            elif mutation == 'code': contracts[1]['code_hex'] = contracts[0]['code_hex']
            elif mutation == 'calldata': program['input_hex'] = '0x01'
            elif mutation == 'primary': program['code_hex'] = '0xfe'
            elif mutation == 'gas-low': program['transactions']['gas_limit'] = row['parameters']['transaction_gas_limit'] = 21000
            elif mutation == 'gas-cap': program['transactions']['gas_limit'] = row['parameters']['transaction_gas_limit'] = 15_000_001
            elif mutation == 'counter-intent': row['parameters']['repetitions'] = 1
            else: program['transactions']['count_per_block'] = 1
            with self.assertRaises(AssertionError): run_users(row)
        with self.assertRaises(AssertionError): run_users(intent_mutant(m.make_case('sdiv', 'limb1', 'zero')))

    def test_exports_helpers_cli(self):
        m = producer()
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'cases'; self.assertEqual(m.prepare(output), 32)
            catalog = json.loads((output / 'catalog.json').read_bytes())
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertEqual(len({r['request_sha256'] for r in catalog['cases']}), 32)
            for row in catalog['cases']:
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(json.loads(raw), m.make_case(row['parameters']['operation'], row['parameters']['divisor_class'], row['parameters']['workload_level'])['request'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for invalid in (None, True, 'cases'):
                with self.assertRaises(ValueError): m.prepare(invalid)
            for model in ('A' * 64, 'a' * 63, None, True):
                with self.assertRaises(ValueError): m.prepare(Path(tmp) / 'bad', model_id=model)
            command = [sys.executable, '-S', '-B', str(CASES / 'division_density.py'), '--output-dir', str(Path(tmp) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 32})
            self.assertEqual(json.loads((Path(tmp) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('DIVISION_DENSITY_NATIVE_BIN'),
        'set DIVISION_DENSITY_NATIVE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['DIVISION_DENSITY_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from division_density_vm import run_users
        base = Path(os.environ['DIVISION_DENSITY_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = (('div', 'limb2', 'full', False), ('smod', 'limb3', 'zero', False), ('sdiv', 'limb1', 'zero', True))
        for operation, divisor_class, level, mutant in specimens:
            row = m.make_case(operation, divisor_class, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            if mutant:
                row = intent_mutant(row)
                with self.assertRaises(AssertionError): run_users(row)
            name = f'{operation}-{divisor_class}-{level}-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{f'cases/{n}': Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/division_density.py': Path(m.__file__), 'tests/test_division_density.py': Path(__file__),
                'tests/division_density_vm.py': Path(__file__).with_name('division_density_vm.py'),
                **{f'{n}.py': CASES.parent / f'{n}.py' for n in ('process_guard', 'artifacts', 'shared_memory')}}
            for relative, path in source_paths.items():
                target = sources / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(path.read_bytes())
            build = binary.with_name('native-build.json'); self.assertTrue(build.is_file())
            (root / 'native-build.json').write_bytes(build.read_bytes())
            growing = [root / n for n in ('input.json', 'input.bin', 'native.json')]
            command = [str(binary), '--input', str(source), '--output', str(growing[0]), '--output-bincode', str(growing[1]),
                '--report', str(growing[2]), '--execution-profile', 'evm-gas-only']
            free = shutil.disk_usage(root).free; self.assertGreaterEqual(free, 576 * 1024 * 1024)
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), limits=limits,
                native_sha256=NATIVE_SHA256, native_build_sha256=hashlib.sha256(build.read_bytes()).hexdigest(),
                request_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                requested_intent_request_sha256=hashlib.sha256(original_request).hexdigest(),
                row_sha256=hashlib.sha256(m.encoded(row)).hexdigest(),
                requested_intent_row_sha256=hashlib.sha256(original_row).hexdigest(),
                specimen_role='native-accepted-divisor5-intent-mutant' if mutant else 'original-positive',
                mutation='SDIV divisor3 replaced by5; warmup result and retained-divisor guard recomputed. Original ordered-operand intent remains divisor3.' if mutant else None,
                streams='combined stdout/stderr native.log, bounded by unchanged process_guard',
                observability='Native status/gas/routes plus complete execution_result replay; no direct native stack/operand export.',
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
    from division_density_vm import decode
    ins = list(decode(code)); selected = [at for at, _, _ in ins if start <= at < end]
    layout = [at for at, _, _ in ins if mode != 'skip' or not start <= at < end]
    if mode == 'duplicate': layout[layout.index(end):layout.index(end)] = selected
    widths = {at: 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0) for at, op, _ in ins}
    offsets = {}; cursor = 0
    for at in layout: offsets.setdefault(at, cursor); cursor += widths[at]
    jumps = {at for at, op, _ in ins if op == 0x5b}; image = bytearray()
    for at in layout:
        piece = code[at:at + widths[at]]
        if piece[0] == 0x61 and int.from_bytes(piece[1:], 'big') in jumps:
            piece = piece[:1] + offsets[int.from_bytes(piece[1:], 'big')].to_bytes(2, 'big')
        image.extend(piece)
    return bytes(image)


def intent_mutant(row):
    """Native-accepted divisor5 with result/final guards changed; original divisor3 intent."""
    from division_density_vm import decode, arithmetic, operands
    result = copy.deepcopy(row); a, b = operands('sdiv', 'limb1'); old = arithmetic('sdiv', a, b); new = arithmetic('sdiv', a, 5)
    for contract in result['request']['program']['prestate_contracts']:
        code = bytearray.fromhex(contract['code_hex'][2:])
        for at, op, value in decode(code):
            if op == 0x7f and value in (b, old): code[at + 1:at + 33] = (5 if value == b else new).to_bytes(32, 'big')
        contract['code_hex'] = '0x' + code.hex()
    return result


if __name__ == '__main__': unittest.main()
