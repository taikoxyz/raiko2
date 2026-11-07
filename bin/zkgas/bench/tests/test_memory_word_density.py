"""Finite word-memory panel: actual-byte memory, gas, mutation and native gates."""
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
GROUPS = tuple((op, arena, offset) for op in ('store', 'load') for arena in (256, 1048576) for offset in ((0, 1) if op == 'store' else (1,)))
GRID = tuple((*group, level) for group in GROUPS for level in ('zero', 'full'))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'memory_word_density.py'
    assert path.is_file(), 'word-memory producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('memory_word_density', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from memory_word_density_vm import run_users
    p = row['parameters']; program = row['request']['program']; results = run_users(row)
    test.assertEqual((p['value_a_hex'], p['value_b_hex']), ('0x' + 'a5' * 32, '0x' + '5a' * 32))
    routes = Counter(); spent = 167355; prefixes = []
    for tag, (contract, result) in enumerate(zip(program['prestate_contracts'], results)):
        code = bytes.fromhex(contract['code_hex'][2:])
        test.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'][tag])
        test.assertEqual(len(code), p['runtime_bytes']); test.assertLess(len(code), 24576)
        test.assertEqual((result['terminal'], result['stack'], result['counter']), (0, [], p['repetitions']))
        test.assertEqual(result['target_operations'], 64 * p['repetitions'] + (2 if p['operation'] == 'store' else 1))
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['peak_memory_bytes'], p['arena_bytes'])
        test.assertEqual(result['memory_expansion_gas'], 3 * (p['arena_bytes'] // 32) + (p['arena_bytes'] // 32) ** 2 // 512)
        test.assertEqual(result['counter_progression'], list(range(p['repetitions'] + 1)))
        test.assertEqual(result['target_word_hex'], p['value_b_hex'])
        test.assertEqual(result['last_word_hex'], '0x' + '00' * 32)
        test.assertEqual(len(result['sites']), 66 if p['operation'] == 'store' else 65)
        for site in result['sites']:
            test.assertEqual(site['offset'], p['residue'])
            test.assertEqual(site['visits'], 1 if site['phase'] == 'warmup' else p['repetitions'])
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertEqual(p['target_operations_per_user'], 64 * p['repetitions'] + p['warmup_target_operations_per_user'])
    test.assertEqual(p['body_target_operations_per_user'], 64 * p['repetitions'])
    opcode = 'opcode:0x52' if p['operation'] == 'store' else 'opcode:0x51'
    test.assertEqual(p['non_target_same_opcode_operations_per_user'] + p['target_operations_per_user'], results[0]['counts'][opcode])
    return results


class MemoryWordDensityTests(unittest.TestCase):
    def test_a_all_twelve_two_owner_programs(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['repetitions'], 0 if args[3] == 'zero' else p['full_repetitions'])
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(*args[:3], p['full_repetitions'])['gross_gas'] + 1000)
                self.assertEqual(row['sample_id'], f'memory-word-{args[0]}-a{args[1]}-o{args[2]}-n{64 * p["repetitions"]}')
                self.assertEqual(p['counter_memory_offset'], 64); self.assertEqual(p['protected_memory_offset'], 96)
                if args[3] == 'full': self.assertTrue(p['near_full'])

    def test_b_direct_cache_differential_full_plus_one(self):
        m = producer(); from memory_word_density_vm import execute, decode
        for group in GROUPS:
            with patch.object(m, '_runtime', side_effect=AssertionError('runtime sizing')):
                m.gas_accounting(*group, 1_000_000); n = m.full_repetitions(*group)
                self.assertTrue(m.fits(*group, n)); self.assertFalse(m.fits(*group, n + 1))
            shapes = set(); sizes = set()
            for tag, r in itertools.product((0, 1), (0, 1, 2, n + 1)):
                code, counts = m._runtime(*group, r, tag); shapes.add(tuple(x[1] for x in decode(code))); sizes.add(len(code))
                args = dict(operation=group[0], arena_bytes=group[1], residue=group[2], repetitions=r, tag=tag)
                result = execute(code, 50_000_000, **args)
                self.assertEqual(result['counts'], +Counter(counts)); self.assertEqual(result['gas'], m.gas_accounting(*group, r)['gross_gas'])
                if r <= 2: self.assertEqual(result, execute(code, 50_000_000, cache=False, **args))
                else:
                    g = result['gas']; e = g + 1000
                    failures = [key for key, failed in (('charged_block_budget', 167355 + 2*g > 30_000_000), ('transaction_fixture_cap', e > 15_000_000), ('eip7825_cap', e > 16_777_216), ('ordered_header_prefix', any(31_000_000 - 167355 - j*g < e for j in range(2))), ('runtime_size', len(code) >= 24576)) if failed]
                    self.assertEqual(failures, m.make_case(*group)['parameters']['full_plus_one_constraint_failures']); self.assertTrue(failures)
            self.assertEqual((len(shapes), len(sizes)), (1, 1))

    def test_domains_separate_caps_and_memory_rounding(self):
        m = producer()
        for op in (True, 1, None, [], 'MSTORE'):
            with self.assertRaises(ValueError): m.make_case(op)
        for arena in (True, 255, 257, 1048577, 256.0, None):
            with self.assertRaises(ValueError): m.make_case(arena_bytes=arena)
        for residue in (True, -1, 2, 1.0, None):
            with self.assertRaises(ValueError): m.make_case(residue=residue)
        with self.assertRaises(ValueError): m.make_case('load', 256, 0)
        for r in (True, -1, 1.0, None, 1_000_001):
            for fn in (m._runtime, m.gas_accounting, m.fits):
                with self.assertRaises(ValueError): fn('store', 256, 1, r)
        for tag in (True, -1, 2, 0.0, None):
            with self.assertRaises(ValueError): m._runtime('store', 256, 1, 0, tag)
        for level in (True, None, [], 'single'):
            with self.assertRaises(ValueError): m.make_case(level=level)
        for field in ('TRANSACTION_GAS_CAP', 'EIP7825_GAS_CAP', 'BLOCK_BUDGET', 'HEADER_GAS_LIMIT'):
            with patch.object(m, field, 21000): self.assertFalse(m.fits('store', 256, 1, 0))

    def test_exports_streamed_metadata_cli_and_helpers(self):
        m = producer()
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'cases'; self.assertEqual(m.prepare(output), 12)
            catalog = json.loads((output / 'catalog.json').read_bytes())
            self.assertEqual(catalog['source_root'], 'bin/zkgas/bench')
            self.assertEqual(set(catalog['source_sha256']), {f'cases/{n}.py' for n in ('initcode_preparation', 'spawn_memory', 'storage_fanout', 'storage_mutation', 'top_level_create')})
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertEqual(len({r['request_sha256'] for r in catalog['cases']}), 12)
            for row in catalog['cases']:
                self.assertNotIn('request', row)
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(json.loads(raw), m.make_case(row['parameters']['operation'], row['parameters']['arena_bytes'], row['parameters']['residue'], row['parameters']['workload_level'])['request'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for invalid in (None, True, 'cases'):
                with self.assertRaises(ValueError): m.prepare(invalid)
            for model in ('A' * 64, 'a' * 63, None, True):
                with self.assertRaises(ValueError): m.prepare(Path(tmp) / 'bad', model_id=model)
                self.assertFalse((Path(tmp) / 'bad').exists())
            command = [sys.executable, '-S', '-B', str(CASES / 'memory_word_density.py'), '--output-dir', str(Path(tmp) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 12})
            self.assertEqual(json.loads((Path(tmp) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)
            self.assertNotEqual(subprocess.run(command + ['--unexpected'], capture_output=True, timeout=60).returncode, 0)


    def test_byte_memory_effects_mutants_and_schedule(self):
        m = producer(); from memory_word_density_vm import execute, decode, run_users, structure, prove_body, apply_body, A, B
        for op, arena, offset in GROUPS:
            for tag, r in itertools.product((0, 1), (0, 2)):
                code, _ = m._runtime(op, arena, offset, r, tag); ins = list(decode(code))
                args = dict(operation=op, arena_bytes=arena, residue=offset, repetitions=r, tag=tag)
                warmup, loop, body, end, targets = structure(ins, op, arena, offset, r, tag)
                body_ins = [i for i in ins if body <= i[0] < end]
                stack = [A, B, offset] if op == 'store' else [offset]
                proof = prove_body(body_ins, stack, B.to_bytes(32, 'big'), operation=op, offset=offset)
                memory = bytearray(arena); memory[offset:offset + 32] = B.to_bytes(32, 'big'); memory[64:96] = (7).to_bytes(32, 'big'); memory[96:128] = tag.to_bytes(32, 'big')
                before = bytes(memory); apply_body(proof, stack, memory)
                self.assertEqual(bytes(memory), before); self.assertEqual(proof['writes'], 64 if op == 'store' else 0)
                self.assertEqual(proof['reads'], 64 if op == 'load' else 0); self.assertEqual(proof['memory_growth'], 0)
                self.assertEqual(proof['gas'], 64 * (9 if op == 'store' else 8))
                self.assertEqual([e['value'] for e in proof['effects']], ([A, B] * 32 if op == 'store' else [B] * 64))
                # Memory feedback is part of the cached transition precondition.
                memory[offset] ^= 1
                with self.assertRaisesRegex(AssertionError, 'memory'): apply_body(proof, stack, memory)
                for badstack in (stack + [0], stack[:-1], ([A, B, 64] if op == 'store' else [64])):
                    with self.assertRaises(AssertionError): prove_body(body_ins, badstack, B.to_bytes(32, 'big'), operation=op, offset=offset)
                with self.assertRaises(AssertionError): prove_body(body_ins, stack, A.to_bytes(32, 'big'), operation=op, offset=offset)
                # Exact actual body: changed DUP depths, omitted read/write,
                # extra stack item, value/offset feedback and aliasing all fail.
                for index in (0, 1, 2, 63, 95, 191):
                    wrong = bytearray(code); wrong[body_ins[index][0]] = 0x80 if body_ins[index][1] != 0x80 else 0x81
                    with self.assertRaises(AssertionError): execute(bytes(wrong), 50_000_000, **args)
                for mode in ('skip', 'duplicate'):
                    with self.assertRaises(AssertionError): execute(rewrite_block(code, body, body_ins[3][0], mode), 50_000_000, **args)
                # Every meaningful immediate changes alone, including arena,
                # counter/tag address, A/B/offset and final retained guards.
                for pc, opcode, value in ins:
                    if opcode not in (0x60, 0x62, 0x7f): continue
                    wrong = bytearray(code); width = opcode - 0x5f
                    wrong[pc + 1:pc + 1 + width] = (value ^ 1).to_bytes(width, 'big')
                    with self.assertRaises(AssertionError): execute(bytes(wrong), 50_000_000, **args)
        row = m.make_case('store', 256, 1, 'zero')
        for mutation in ('recipient', 'owner', 'storage', 'code', 'calldata', 'primary', 'gas-low', 'gas-cap', 'counter-intent', 'count', 'tag'):
            wrong = copy.deepcopy(row); program = wrong['request']['program']; contracts = program['prestate_contracts']
            if mutation == 'recipient': program['transactions']['recipients'].reverse()
            elif mutation == 'owner': contracts[0]['address'] = contracts[1]['address']
            elif mutation == 'storage': contracts[0]['initial_storage'] = {'0x00': '0x01'}
            elif mutation == 'code': contracts[1]['code_hex'] = contracts[0]['code_hex']
            elif mutation == 'calldata': program['input_hex'] = '0x01'
            elif mutation == 'primary': program['code_hex'] = '0xfe'
            elif mutation == 'gas-low': program['transactions']['gas_limit'] = wrong['parameters']['transaction_gas_limit'] = 21000
            elif mutation == 'gas-cap': program['transactions']['gas_limit'] = wrong['parameters']['transaction_gas_limit'] = 15_000_001
            elif mutation == 'counter-intent': wrong['parameters']['repetitions'] = 1
            elif mutation == 'tag': wrong['parameters']['owner_tags'].reverse()
            else: program['transactions']['count_per_block'] = 1
            with self.assertRaises(AssertionError): run_users(wrong)
        original = m.make_case('store', 1048576, 1, 'full'); mutant = intent_mutant(original)
        with self.assertRaises(AssertionError): run_users(mutant)
        changed = bytes.fromhex(mutant['request']['program']['prestate_contracts'][0]['code_hex'][2:])
        result = execute(changed, original['parameters']['transaction_gas_limit'], operation='store', arena_bytes=1048576, residue=1,
            repetitions=original['parameters']['repetitions'], tag=0, expected_a=int.from_bytes(b'\xaa' * 32, 'big'))
        self.assertEqual(result['gas'], original['parameters']['expected_user_charged_gas'])
        self.assertEqual(result['target_word_hex'], original['parameters']['value_b_hex'])


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('ZKGAS_FIXTURE_BIN'),
        'set ZKGAS_FIXTURE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['ZKGAS_FIXTURE_BIN'])
        build = binary.with_name('native-build.json')
        self.assertEqual(hashlib.sha256(build.read_bytes()).hexdigest(), '1e1a7f4dbd141ce97359114b737d744ee71eb37b78e974038179eedf25cd8023')
        provenance = json.loads(build.read_bytes()); self.assertEqual(len(provenance['source_file_sha256']), 81)
        self.assertEqual(sum(n.endswith('.rs') for n in provenance['source_file_sha256']), 66)
        self.assertEqual(provenance['compiled_workspace_input_count'], 70)
        workspace = CASES.parents[3]
        for name, expected in provenance['source_file_sha256'].items():
            self.assertEqual(hashlib.sha256((workspace / name).read_bytes()).hexdigest(), expected, name)
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from artifacts import identity, read_json
        from memory_word_density_vm import run_users
        base = Path(os.environ['MEMORY_WORD_DENSITY_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = (('store', 1048576, 1, 'full', False), ('load', 256, 1, 'zero', False), ('store', 1048576, 1, 'full', True))
        for operation, arena, residue, level, mutant in specimens:
            row = m.make_case(operation, arena, residue, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            mutation_preflight = None
            if mutant:
                row = intent_mutant(row)
                with self.assertRaises(AssertionError) as rejected: run_users(row)
                from memory_word_density_vm import decode, execute, A, B
                changed_a = int.from_bytes(b'\xaa' * 32, 'big')
                original_code = bytes.fromhex(json.loads(original_request)['program']['prestate_contracts'][0]['code_hex'][2:])
                changed_code = bytes.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])
                before = list(decode(original_code)); after = list(decode(changed_code))
                differences = [dict(pc=a[0], opcode=a[1], width=a[1] - 0x5f, original_hex=f'0x{a[2]:064x}', mutated_hex=f'0x{b[2]:064x}')
                    for a, b in zip(before, after) if a != b]
                self.assertEqual(len(differences), 3)
                self.assertTrue(all(d['original_hex'] == f'0x{A:064x}' and d['mutated_hex'] == f'0x{changed_a:064x}' for d in differences))
                accepted = execute(changed_code, row['parameters']['transaction_gas_limit'], operation=operation, arena_bytes=arena,
                    residue=residue, repetitions=row['parameters']['repetitions'], tag=0, expected_a=changed_a)
                self.assertEqual(accepted['gas'], row['parameters']['expected_user_charged_gas'])
                self.assertEqual(accepted['target_word_hex'], f'0x{B:064x}')
                self.assertEqual(accepted['counts'], +Counter(m._runtime(operation, arena, residue, row['parameters']['repetitions'])[1]))
                mutation_preflight = dict(owner_index=0, decoded_immediate_changes=differences,
                    contexts=['initial-retained-A', 'warmup-A-expected-guard', 'final-retained-A-guard'],
                    original_intent_rejected=str(rejected.exception), original_intent_preserved=True,
                    original_a_hex=f'0x{A:064x}', mutated_a_hex=f'0x{changed_a:064x}',
                    internally_consistent_changed_a_accepted=True, final_b_gas_routes_unchanged=True, canonical_eligible=False)
            name = f'{operation}-a{arena}-o{residue}-{level}-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            self.assertLessEqual(source.stat().st_size, 32 * 1024 * 1024)
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{n: Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/memory_word_density.py': Path(m.__file__), 'tests/test_memory_word_density.py': Path(__file__),
                'tests/memory_word_density_vm.py': Path(__file__).with_name('memory_word_density_vm.py'),
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
                specimen_role='native-accepted-A-value-intent-mutant' if mutant else 'original-positive',
                mutation='One owner A changed a5*32 to aa*32 in exactly decoded initial/warmup-A/final-A guard contexts; B, gas and routes unchanged. Original A/content intent retained.' if mutant else None,
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
    from memory_word_density_vm import decode
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
    """Change one owner's three exact decoded A contexts; preserve original intent."""
    from memory_word_density_vm import decode, A
    result = copy.deepcopy(row); contract = result['request']['program']['prestate_contracts'][0]
    code = bytearray.fromhex(contract['code_hex'][2:]); ins = list(decode(code)); contexts = []
    for index, (at, op, value) in enumerate(ins):
        if not 0x60 <= op <= 0x7f or value != A: continue
        before = [v[1] for v in ins[max(0, index - 1):index]]
        after = [v[1] for v in ins[index + 1:index + 4]]
        if before == [0x52] and after == [0x7f, 0x62, 0x82]: context = 'initial-retained-A'
        elif before == [0x51] and after == [0x14, 0x15, 0x61]: context = 'warmup-A-expected-guard'
        elif before == [0x80] and after == [0x14, 0x15, 0x61]: context = 'final-retained-A-guard'
        else: raise AssertionError('unexpected A immediate context')
        width = op - 0x5f; assert width == 32, 'decoded A width'
        contexts.append((context, at, width))
    assert [c[0] for c in contexts] == ['initial-retained-A', 'warmup-A-expected-guard', 'final-retained-A-guard']
    for _, at, width in contexts: code[at + 1:at + 1 + width] = b'\xaa' * width
    contract['code_hex'] = '0x' + code.hex()
    assert result['parameters'] == row['parameters']
    assert result['request']['program']['prestate_contracts'][1] == row['request']['program']['prestate_contracts'][1]
    return result


if __name__ == '__main__': unittest.main()
