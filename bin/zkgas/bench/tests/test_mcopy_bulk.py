"""Focused bulk-MCOPY byte-intent, gas and frozen native qualification."""
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
LAYOUTS = ('same-residue-disjoint', 'different-residue-disjoint', 'overlap-right', 'overlap-left')
GRID = tuple(itertools.product(LAYOUTS, ('zero', 'single', 'full')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'mcopy_bulk.py'
    assert path.is_file(), 'MCOPY bulk producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('mcopy_bulk', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from mcopy_bulk_vm import run_users
    p = row['parameters']; program = row['request']['program']
    test.assertIn((p['layout'], p['workload_level']), GRID)
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
        test.assertEqual(result['counter'], p['repetitions']); test.assertEqual(result['loop_copies'], p['repetitions'])
        test.assertEqual(result['memory_sha256'], p['expected_final_memory_sha256'][tag])
        test.assertEqual(result['guards'], p['expected_guards'][tag])
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['mcopy_gas'], p['gas_accounting_per_user']['mcopy_gas'])
        test.assertEqual(len(result['memory']), 32 * p['memory_words'])
        test.assertEqual(result['memory'][32:64], b'\x7b' * 32)
        padding = 64 + p['payload_extent_bytes']; test.assertEqual(result['memory'][padding:], b'\xee' * (len(result['memory']) - padding))
        if p['layout'] == 'different-residue-disjoint': test.assertEqual(result['memory'][65600], 0xcd)
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    for op in (0x54, 0x55, 0x5c, 0x5d, 0xf0, 0xf1, 0xf4, 0xfa, 0xfd, 0xfe): test.assertEqual(routes[f'opcode:0x{op:02x}'], 0)
    return results


class McopyBulkTests(unittest.TestCase):
    def test_all_twelve_two_owner_trajectories(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(args[0], p['full_repetitions'])['gross_gas'] + 1000)

    def test_full_plus_one_and_gas_only_sizing(self):
        m = producer(); from mcopy_bulk_vm import execute, decode
        with patch.object(m, 'expected_memory', side_effect=AssertionError('byte simulation during sizing')):
            for layout in LAYOUTS:
                m.gas_accounting(layout, 1_000_000); maximum = m.full_repetitions(layout)
                self.assertTrue(m.fits(layout, maximum)); self.assertFalse(m.fits(layout, maximum + 1))
        for layout in LAYOUTS:
            maximum = m.full_repetitions(layout); shapes = set(); sizes = set()
            for tag, r in itertools.product((0, 1), (0, 1, maximum, maximum + 1)):
                code, counts = m._runtime(layout, r, tag); shapes.add(tuple(op for _, op, _ in decode(code))); sizes.add(len(code))
                if r == maximum + 1:
                    result = execute(code, 50_000_000, layout=layout, tag=tag, repetitions=r)
                    self.assertEqual((result['terminal'], result['counter'], result['loop_copies']), (0, r, r))
                    self.assertEqual(result['counts'], +Counter(counts)); gas = m.gas_accounting(layout, r)
                    self.assertEqual(result['gas'], gas['gross_gas']); self.assertEqual(result['mcopy_gas'], gas['mcopy_gas'])
                    w = len(result['memory']) // 32; self.assertEqual(gas['memory_expansion_gas'], 3 * w + w * w // 512)
                    envelope = gas['gross_gas'] + 1000
                    self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas['gross_gas'] > 30_000_000 or any(31_000_000 - 167355 - j * gas['gross_gas'] < envelope for j in range(2)))
            self.assertEqual((len(shapes), len(sizes)), (1, 1))

    def test_strict_domains_and_separate_caps(self):
        m = producer()
        for layout in (True, None, 0, 'overlap', [], 'SAME-RESIDUE-DISJOINT'):
            for fn in (m.make_case, m.full_repetitions):
                with self.assertRaises(ValueError): fn(layout)
        for level in (True, None, 0, 'max', []):
            with self.assertRaises(ValueError): m.make_case(LAYOUTS[0], level)
        for fn in (m._runtime, m.gas_accounting, m.fits, m.expected_memory):
            for r in (True, -1, 1.0, None, 1_000_001):
                with self.assertRaises(ValueError): fn(LAYOUTS[0], r)
        for tag in (True, -1, 2, 0.0, None):
            for fn in (m._runtime, m.expected_memory):
                with self.assertRaises(ValueError): fn(LAYOUTS[0], 0, tag)
        for field, value in (('TRANSACTION_GAS_CAP', 21000), ('EIP7825_GAS_CAP', 21000), ('BLOCK_BUDGET', 167355), ('HEADER_GAS_LIMIT', 167355)):
            with patch.object(m, field, value): self.assertFalse(m.fits(LAYOUTS[0], 0))

    def test_semantic_negatives(self):
        m = producer(); from mcopy_bulk_vm import execute, decode, run_users
        for layout in LAYOUTS:
            code, _ = m._runtime(layout, 2, 1); ins = list(decode(code))
            def run(image, **kw): return execute(image, 50_000_000, layout=layout, tag=1, repetitions=2, **kw)
            def change(at, width, value):
                wrong = bytearray(code); wrong[at + 1:at + 1 + width] = value.to_bytes(width, 'big'); return bytes(wrong)
            self.assertEqual(run(code)['terminal'], 0)
            loop = next(i for i, (_, op, _) in enumerate(ins) if op == 0x57) + 1
            ci = next(i for i in range(loop, len(ins)) if ins[i][1] == 0x5e)
            for index in (ci - 1, ci - 2, ci - 3):
                at, op, value = ins[index]
                with self.assertRaisesRegex(AssertionError, 'copy schedule'): run(change(at, op - 0x5f, value + 1))
            # Preparation source/destination/length, A seed, padding seed, protected word.
            prep = next(i for i, (_, op, _) in enumerate(ins) if op == 0x5e)
            for index in (prep - 1, prep - 2, prep - 3):
                at, op, value = ins[index]
                with self.assertRaises(AssertionError): run(change(at, op - 0x5f, value + 1))
            for at, op, value in ins[:loop]:
                if op == 0x7f:
                    with self.assertRaises(AssertionError): run(change(at, 32, value ^ 1))
            if layout == 'different-residue-disjoint':
                at = next(at for at, op, value in ins if op == 0x60 and value == 0xcd)
                with self.assertRaisesRegex(AssertionError, 'prepared memory'): run(change(at, 1, 0xcc))
            for mode in ('skip', 'duplicate'):
                wrong = rewrite_block(code, ins[ci - 3][0], ins[ci + 1][0], mode)
                with self.assertRaises(AssertionError): run(wrong)
            # Counter increment, exit bound, expected counter and each byte guard.
            needle = bytes.fromhex('62000000516200000101'); at = code.index(needle) + 5
            with self.assertRaises(AssertionError): run(change(at, 3, 2))
            for at, op, value in ins:
                if op == 0x7f and value == 2:
                    with self.assertRaises(AssertionError): run(change(at, 32, 1))
            for at, op, value in ins[ci + 1:]:
                if op == 0x7f:
                    with self.assertRaises(AssertionError): run(change(at, 32, value ^ 1))
            with self.assertRaisesRegex(AssertionError, 'out of gas'):
                execute(code, m.gas_accounting(layout, 2)['gross_gas'] - 1, layout=layout, tag=1, repetitions=2)
            if layout.startswith('overlap'):
                bad_direction = 'forward' if layout == 'overlap-right' else 'backward'
                with self.assertRaisesRegex(AssertionError, 'copy content'): run(code, copy_mode=bad_direction)
                # Opposite overlap direction and self-copy differ from the declared tuple.
                for source, destination in ((65, 64), (64, 65), (64, 64)):
                    expected = (ins[ci - 2][2], ins[ci - 1][2])
                    if (source, destination) == expected: continue
                    wrong = bytearray(code)
                    for index, value in ((ci - 2, source), (ci - 1, destination)):
                        pos, op, _ = ins[index]; wrong[pos + 1:pos + 1 + op - 0x5f] = value.to_bytes(op - 0x5f, 'big')
                    with self.assertRaisesRegex(AssertionError, 'copy schedule'): run(bytes(wrong))
        for mutation in ('recipient', 'owner', 'storage', 'code', 'count', 'calldata'):
            row = m.make_case(level='single'); p = row['request']['program']; contracts = p['prestate_contracts']
            if mutation == 'recipient': p['transactions']['recipients'].reverse()
            elif mutation == 'owner': contracts[1]['address'] = contracts[0]['address']
            elif mutation == 'storage': contracts[0]['initial_storage'] = {'0x00': '0x01'}
            elif mutation == 'code': contracts[1]['code_hex'] = contracts[0]['code_hex']
            elif mutation == 'calldata': p['input_hex'] = '0x01'
            else: p['transactions']['count_per_block'] = 1
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError): run_users(row)
        with self.assertRaisesRegex(AssertionError, 'copy schedule'): run_users(intent_mutant(m.make_case('overlap-left', 'single')))

    def test_exports_helpers_and_cli(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'cases'; self.assertEqual(m.prepare(output), 12)
            catalog = json.loads((output / 'catalog.json').read_bytes()); self.assertEqual(catalog['corpus_id'], 'unzen-mcopy-bulk-20261104')
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest() for name, module in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertEqual(len(set(r['sample_id'] for r in catalog['cases'])), 12)
            self.assertEqual(len(set(r['request_sha256'] for r in catalog['cases'])), 12)
            for saved in catalog['cases']:
                raw = (output / saved['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), saved['request_sha256'])
                row = saved | {'request': json.loads(raw)}; qualify(self, row)
                self.assertEqual(row['request'], m.make_case(saved['parameters']['layout'], saved['parameters']['workload_level'])['request'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for model in ('A' * 64, 'a' * 63, None, True):
                with self.assertRaises(ValueError): m.prepare(Path(temporary) / 'bad', model_id=model)
            for invalid in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(invalid)
            command = [sys.executable, '-S', '-B', str(CASES / 'mcopy_bulk.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 12})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)

    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('MCOPY_BULK_NATIVE_BIN'),
        'set MCOPY_BULK_NATIVE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['MCOPY_BULK_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from mcopy_bulk_vm import run_users
        base = Path(os.environ['MCOPY_BULK_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = (('same-residue-disjoint', 'single', False), ('overlap-right', 'full', False), ('overlap-left', 'single', True))
        for layout, level, mutant in specimens:
            row = m.make_case(layout, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            if mutant:
                row = intent_mutant(row)
                with self.assertRaisesRegex(AssertionError, 'copy schedule'): run_users(row)
            name = f'{layout}-{level}-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{f'cases/{n}': Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/mcopy_bulk.py': Path(m.__file__), 'tests/test_mcopy_bulk.py': Path(__file__),
                'tests/mcopy_bulk_vm.py': Path(__file__).with_name('mcopy_bulk_vm.py'),
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
                specimen_role='native-accepted-self-copy-intent-mutant' if mutant else 'original-positive',
                mutation='Loop MCOPY source65 replaced by64, making src==dst64; final byte guards recomputed for self-copy. Original intent remains overlap-left.' if mutant else None,
                streams='combined stdout/stderr native.log, bounded by unchanged process_guard',
                observability='Native status/gas/routes plus complete execution_result replay; no native final-memory export.',
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
    """Test-only deletion/duplication with relocation of actual jump immediates."""
    from mcopy_bulk_vm import decode
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
    """Native-valid self-copy with recomputed guards, rejected by original intent."""
    from mcopy_bulk_vm import decode
    assert row['parameters']['layout'] == 'overlap-left' and row['parameters']['repetitions'] == 1
    m = producer(); result = copy.deepcopy(row)
    for tag, contract in enumerate(result['request']['program']['prestate_contracts']):
        code = bytearray.fromhex(contract['code_hex'][2:]); ins = list(decode(code))
        loop = next(i for i, (_, op, _) in enumerate(ins) if op == 0x57) + 1
        ci = next(i for i in range(loop, len(ins)) if ins[i][1] == 0x5e)
        at, op, value = ins[ci - 2]; assert (op, value) == (0x62, 65); code[at + 1:at + 4] = (64).to_bytes(3, 'big')
        memory = m.expected_memory('overlap-left', 0, tag); guards = m.guard_offsets('overlap-left')
        done = next(i for i in range(ci, len(ins)) if ins[i][1] == 0x5b)
        expected_pushes = [(at, value) for at, op, value in ins[done:] if op == 0x7f]
        assert len(expected_pushes) == 1 + len(guards)
        for (at, _), offset in zip(expected_pushes[1:], guards): code[at + 1:at + 33] = memory[offset:offset + 32]
        contract['code_hex'] = '0x' + code.hex()
    return result


if __name__ == '__main__': unittest.main()
