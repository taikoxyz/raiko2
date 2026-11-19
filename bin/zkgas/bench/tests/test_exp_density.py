"""Finite EXP panel: independent byte math, gas, mutation and native gates."""
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
CLASSES = ('dense2', 'sparse24', 'dense24', 'dense25', 'dense32')
GRID = tuple(itertools.product(CLASSES, ('zero', 'full')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'exp_density.py'
    assert path.is_file(), 'EXP density producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('exp_density', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from exp_density_vm import run_users, operands, arithmetic
    p = row['parameters']; program = row['request']['program']
    base, exponent = operands(p['exponent_class'])
    test.assertEqual([p[k] for k in ('base_hex', 'exponent_hex', 'expected_result_hex')],
        [f'0x{v:064x}' for v in (base, exponent, arithmetic(base, exponent))])
    h, w = exponent.bit_length(), exponent.bit_count()
    test.assertEqual((p['exponent_bit_length'], p['popcount'], p['exponent_bytes'], p['source_wrapping_mul_count']), (h, w, (h + 7) // 8, h + w))
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
            test.assertEqual((site['base'], site['exponent'], site['result']), (base, exponent, arithmetic(base, exponent)))
            test.assertEqual(site['visits'], 1 if site['phase'] == 'warmup' else p['repetitions'])
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    test.assertEqual(p['gas_accounting_per_user']['memory_expansion_gas'], 6)
    return results


class ExpDensityTests(unittest.TestCase):
    def test_b_all_10_two_owner_programs(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['repetitions'], 0 if args[1] == 'zero' else p['full_repetitions'])
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(args[0], p['full_repetitions'])['gross_gas'] + 1000)
                self.assertIn('charged_block_budget', p['full_plus_one_constraint_failures'])
                if args[1] == 'full': self.assertTrue(p['near_full'])

    def test_a_gas_only_full_plus_one_and_direct_cache_differential(self):
        m = producer(); from exp_density_vm import execute, decode
        for exponent_class in CLASSES:
            with patch.object(m, '_runtime', side_effect=AssertionError('runtime used by sizing')):
                m.gas_accounting(exponent_class, 1_000_000); n = m.full_repetitions(exponent_class)
                self.assertTrue(m.fits(exponent_class, n)); self.assertFalse(m.fits(exponent_class, n + 1))
            sizes = set(); shapes = set()
            for tag, r in itertools.product((0, 1), (0, 1, 2, n + 1)):
                code, counts = m._runtime(exponent_class, r, tag); sizes.add(len(code)); shapes.add(tuple(i[1] for i in decode(code)))
                result = execute(code, 50_000_000, exponent_class=exponent_class, repetitions=r, tag=tag)
                self.assertEqual(result['counts'], +Counter(counts)); self.assertEqual(result['gas'], m.gas_accounting(exponent_class, r)['gross_gas'])
                if r <= 2:
                    self.assertEqual(result, execute(code, 50_000_000, exponent_class=exponent_class, repetitions=r, tag=tag, cache=False))
                else:
                    gas = result['gas']; envelope = gas + 1000
                    self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas > 30_000_000 or any(31_000_000 - 167355 - j * gas < envelope for j in range(2)))
            self.assertEqual((len(sizes), len(shapes)), (1, 1))

    def test_full_width_math_domains(self):
        m = producer()
        from exp_density_vm import arithmetic, operands, exp_gas
        mask = (1 << 256) - 1
        for cls in CLASSES:
            base, exponent = operands(cls)
            self.assertEqual(m._operands(cls), (base, exponent, pow(base, exponent, 1 << 256)))
            self.assertEqual(arithmetic(base, exponent), pow(base, exponent, 1 << 256))
            self.assertEqual(exp_gas(exponent), 10 + 50 * ((exponent.bit_length() + 7) // 8))
        for base, exponent in itertools.product((0, 1, 2, int('a5' * 32, 16)), (0, 1, 127, 128, 255, 256, 511)):
            self.assertEqual(arithmetic(base, exponent), pow(base, exponent, 1 << 256))
        self.assertEqual(arithmetic(0, 0), 1)
        self.assertEqual([exp_gas(e) for e in (0, 1, 127, 128, 255, 256, 511)], [10, 60, 60, 60, 60, 110, 110])
        for invalid in (True, -1, mask + 1, None, 1.0):
            with self.assertRaises(AssertionError): arithmetic(invalid, 1)
            with self.assertRaises(AssertionError): arithmetic(1, invalid)
        self.assertEqual(exp_gas(operands('sparse24')[1]), exp_gas(operands('dense24')[1]))

    def test_domains_and_separate_caps(self):
        m = producer()
        for cls in (True, 1, None, [], 'dense0'):
            for fn in (m.make_case, m.full_repetitions, m._operands):
                with self.assertRaises(ValueError): fn(cls)
        for r in (True, -1, 1.0, None, 1_000_001):
            for fn in (m._runtime, m.gas_accounting, m.fits):
                with self.assertRaises(ValueError): fn('dense24', r)
        for tag in (True, -1, 2, 0.0, None):
            with self.assertRaises(ValueError): m._runtime('dense24', 0, tag)
        for level in (True, None, [], 'single'):
            with self.assertRaises(ValueError): m.make_case(level=level)
        for field in ('TRANSACTION_GAS_CAP', 'EIP7825_GAS_CAP', 'BLOCK_BUDGET', 'HEADER_GAS_LIMIT'):
            with patch.object(m, field, 21000): self.assertFalse(m.fits('dense24', 0))

    def test_byte_mutations_and_schedule(self):
        m = producer(); from exp_density_vm import execute, decode, run_users
        for exponent_class in ('dense2', 'dense32'):
            code, _ = m._runtime(exponent_class, 2, 1); ins = list(decode(code))
            def run(image): return execute(image, 50_000_000, exponent_class=exponent_class, repetitions=2, tag=1)
            def replace(at, value):
                wrong = bytearray(code); wrong[at] = value; return bytes(wrong)
            body = next(i for i in range(len(ins) - 255) if [x[1] for x in ins[i:i + 4]] == [0x81, 0x81, 0x0a, 0x50] and ins[i - 1][1] == 0x57)
            warmup = next(i for i, (_, opcode, _) in enumerate(ins) if opcode == 0x0a)
            for index in (warmup - 2, warmup - 1, warmup, body, body + 1, body + 2, body + 3, body + 127, body + 255):
                with self.assertRaises(AssertionError): run(replace(ins[index][0], 0x52 if index == body + 3 else 0x80))
            # Feeding a result back instead of POP, swapping operand order, and
            # omitting EXP must fail both modes before any cache can hide them.
            for index, opcode in ((body + 3, 0x90), (body + 1, 0x90), (body + 2, 0x50)):
                for cache in (False, True):
                    with self.assertRaises(AssertionError):
                        execute(replace(ins[index][0], opcode), 50_000_000,
                            exponent_class=exponent_class, repetitions=2, tag=1, cache=cache)
            # Delete/duplicate site with relocation: reject malformed pure bodies, including R0.
            for mode in ('skip', 'duplicate'):
                with self.assertRaises(AssertionError): run(rewrite_block(code, ins[body][0], ins[body + 4][0], mode))
            zero_code, _ = m._runtime(exponent_class, 0, 1)
            for mode in ('skip', 'duplicate'):
                wrong = rewrite_block(zero_code, ins[body][0], ins[body + 4][0], mode)
                with self.assertRaises(AssertionError):
                    execute(wrong, 50_000_000, exponent_class=exponent_class, repetitions=0, tag=1)
            for at, opcode, _ in ins:
                if opcode in (0x14, 0x15, 0x56, 0x57, 0x5b, 0):
                    with self.assertRaises(AssertionError): run(replace(at, 0x80))
            # Every immediate is part of initialization, operand/result, loop, tag or final guard intent.
            for at, opcode, value in ins:
                if opcode in (0x7f, 0x62, 0x61):
                    wrong = bytearray(code); wrong[at + opcode - 0x5f] ^= 1
                    with self.assertRaises(AssertionError): run(bytes(wrong))
            with self.assertRaisesRegex(AssertionError, 'out of gas'):
                execute(code, m.gas_accounting(exponent_class, 2)['gross_gas'] - 1, exponent_class=exponent_class, repetitions=2, tag=1)
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
        with self.assertRaises(AssertionError): run_users(intent_mutant(m.make_case('dense32', 'zero')))

    def test_exports_helpers_cli(self):
        m = producer()
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'cases'; self.assertEqual(m.prepare(output), 10)
            catalog = json.loads((output / 'catalog.json').read_bytes())
            self.assertEqual(catalog['source_root'], 'bin/zkgas/bench')
            self.assertEqual(set(catalog['source_sha256']), {f'cases/{n}.py' for n in ('initcode_preparation', 'spawn_memory', 'storage_fanout', 'storage_mutation', 'top_level_create')})
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertEqual(len({r['request_sha256'] for r in catalog['cases']}), 10)
            for row in catalog['cases']:
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(json.loads(raw), m.make_case(row['parameters']['exponent_class'], row['parameters']['workload_level'])['request'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for invalid in (None, True, 'cases'):
                with self.assertRaises(ValueError): m.prepare(invalid)
            for model in ('A' * 64, 'a' * 63, None, True):
                with self.assertRaises(ValueError): m.prepare(Path(tmp) / 'bad', model_id=model)
                self.assertFalse((Path(tmp) / 'bad').exists())
            command = [sys.executable, '-S', '-B', str(CASES / 'exp_density.py'), '--output-dir', str(Path(tmp) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 10})
            self.assertEqual(json.loads((Path(tmp) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)
            self.assertNotEqual(subprocess.run(command + ['--unexpected'], capture_output=True, timeout=60).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('ZKGAS_FIXTURE_BIN'),
        'set ZKGAS_FIXTURE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['ZKGAS_FIXTURE_BIN'])
        build = binary.with_name('native-build.json')
        self.assertEqual(hashlib.sha256(build.read_bytes()).hexdigest(), '1e1a7f4dbd141ce97359114b737d744ee71eb37b78e974038179eedf25cd8023')
        provenance = json.loads(build.read_bytes()); self.assertEqual(len(provenance['source_file_sha256']), 81)
        workspace = CASES.parents[3]
        for name, expected in provenance['source_file_sha256'].items():
            self.assertEqual(hashlib.sha256((workspace / name).read_bytes()).hexdigest(), expected, name)
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from artifacts import identity, read_json
        from exp_density_vm import run_users
        base = Path(os.environ['EXP_DENSITY_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = (('dense32', 'zero', False), ('dense24', 'full', False), ('dense32', 'zero', True))
        for exponent_class, level, mutant in specimens:
            row = m.make_case(exponent_class, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            mutation_preflight = None
            if mutant:
                row = intent_mutant(row)
                with self.assertRaises(AssertionError) as rejected: run_users(row)
                from exp_density_vm import decode, arithmetic, operands, exp_gas
                base_operand, old_exponent = operands('dense32'); new_exponent = old_exponent - 2**254
                before = list(decode(bytes.fromhex(json.loads(original_request)['program']['prestate_contracts'][0]['code_hex'][2:])))
                after = list(decode(bytes.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])))
                differences = [dict(pc=a[0], opcode=a[1], original_hex=f'0x{a[2]:064x}', mutated_hex=f'0x{b[2]:064x}')
                    for a, b in zip(before, after) if a != b]
                self.assertEqual(len(differences), 2)
                self.assertTrue(all(d['opcode'] == 0x7f for d in differences))
                mutation_preflight = dict(owner_index=0, decoded_immediate_changes=differences,
                    original_intent_rejected=str(rejected.exception), original_intent_preserved=True,
                    original_exponent_hex=f'0x{old_exponent:064x}', mutated_exponent_hex=f'0x{new_exponent:064x}',
                    exact_modular_result_hex=f'0x{arithmetic(base_operand, old_exponent):064x}',
                    exact_modular_results_equal=arithmetic(base_operand, old_exponent) == arithmetic(base_operand, new_exponent),
                    event_gas_equal=exp_gas(old_exponent) == exp_gas(new_exponent), canonical_eligible=False)
            name = f'{exponent_class}-{level}-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            self.assertLessEqual(source.stat().st_size, 32 * 1024 * 1024)
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{n: Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/exp_density.py': Path(m.__file__), 'tests/test_exp_density.py': Path(__file__),
                'tests/exp_density_vm.py': Path(__file__).with_name('exp_density_vm.py'),
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
                specimen_role='native-accepted-exponent-intent-mutant' if mutant else 'original-positive',
                mutation='One owner EXP exponent MASK replaced by MASK-2^254 in initial and final operand guard contexts; same gas/result. Original exponent/content intent retained.' if mutant else None,
                mutation_preflight=mutation_preflight,
                artifact_caps_bytes=dict(request=32 * 1024 * 1024, native_output=128 * 1024 * 1024, benchmark_input=64 * 1024 * 1024),
                streams='combined stdout/stderr native.log, bounded by unchanged process_guard',
                observability='Native status/gas/routes plus complete execution_result replay; no direct native stack/operand export.',
                disk_preflight=dict(free_bytes=free, required_free_bytes=576 * 1024 * 1024),
                native_declared_source_verification=dict(status='all81-match',
                    declared_count=81, rust_count=sum(n.endswith('.rs') for n in provenance['source_file_sha256']),
                    compiled_workspace_input_count=provenance['compiled_workspace_input_count'],
                    source_file_sha256=provenance['source_file_sha256']),
                source_root='bin/zkgas/bench', source_sha256={n: hashlib.sha256(path.read_bytes()).hexdigest() for n, path in source_paths.items()},
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
            self.assertEqual(manifest['exit_code'], 0)
            identity(growing[0], limit=128 * 1024 * 1024); identity(growing[1], limit=64 * 1024 * 1024)
            report = read_json(growing[2], limit=128 * 1024 * 1024); receipts = report['receipts_by_block'][0]
            self.assertEqual(report['guest_input_sha256'], hashlib.sha256(growing[1].read_bytes()).hexdigest())
            self.assertEqual(report['guest_input_bincode_length'], growing[1].stat().st_size)
            self.assertEqual(report['construction_request_sha256'], manifest['request_sha256'])
            self.assertEqual(report['execution_profile'], 'evm-gas-only')
            self.assertTrue(report['public_output']); self.assertTrue(report['public_values'])
            self.assertEqual([r['success'] for r in receipts], [True] * 3)
            self.assertEqual([r['gas_used'] for r in receipts], [167355] + [row['parameters']['expected_user_charged_gas']] * 2)
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            initialized = report['prestate_contracts']['initialized_storage']; self.assertEqual(len(initialized), 2)
            for entry in initialized:
                self.assertEqual((entry['initial_storage_entry_count'], entry['final_storage_entry_count']), (0, 0))
                self.assertEqual(entry['initial_storage_root'], entry['final_storage_root'])


def rewrite_block(code, start, end, mode):
    from exp_density_vm import decode
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
    """Change one owner's two decoded exponent contexts, preserving original intent."""
    from exp_density_vm import decode, arithmetic, operands, exp_gas
    result = copy.deepcopy(row); base, exponent = operands('dense32'); changed_exponent = exponent - 2**254
    assert arithmetic(base, exponent) == arithmetic(base, changed_exponent)
    assert exp_gas(exponent) == exp_gas(changed_exponent) and exponent.bit_length() == changed_exponent.bit_length() == 256
    contract = result['request']['program']['prestate_contracts'][0]
    code = bytearray.fromhex(contract['code_hex'][2:]); ins = list(decode(code)); contexts = []
    for index, (at, op, value) in enumerate(ins):
        if op != 0x7f or value != exponent: continue
        before = [v[1] for v in ins[max(0, index - 1):index]]
        after = [v[1] for v in ins[index + 1:index + 4]]
        if before == [0x52] and after == [0x7f, 0x81, 0x81]: contexts.append(at)
        elif before == [0x80] and after == [0x14, 0x15, 0x61]: contexts.append(at)
        else: raise AssertionError('unexpected exponent immediate context')
    assert len(contexts) == 2
    for at in contexts: code[at + 1:at + 33] = changed_exponent.to_bytes(32, 'big')
    contract['code_hex'] = '0x' + code.hex()
    assert result['parameters'] == row['parameters']
    assert result['request']['program']['prestate_contracts'][1] == row['request']['program']['prestate_contracts'][1]
    return result


if __name__ == '__main__': unittest.main()
