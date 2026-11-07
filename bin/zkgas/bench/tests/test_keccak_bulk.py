"""Finite KECCAK bulk byte, gas, bounded-reference and native qualification."""
from collections import Counter
import copy
from datetime import datetime, timezone
import hashlib
import importlib.util
import inspect
import itertools
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
BENCH = Path(__file__).resolve().parents[1]
CASES = BENCH / 'cases'
DOMAINS = ((0, 'zero'),) + tuple(itertools.product((135, 136, 137, 65536), ('zero', 'a5')))
GRID = tuple((length, pattern, level) for length, pattern in DOMAINS for level in ('zero', 'single', 'full'))


def producer():
    path = CASES / 'keccak_bulk.py'
    assert path.is_file(), 'KECCAK bulk producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('keccak_bulk', path)
    module = importlib.util.module_from_spec(spec); sys.modules[spec.name] = module; spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from keccak_bulk_vm import run_users
    m = producer(); p = row['parameters']; program = row['request']['program']; results = run_users(row)
    test.assertIn((p['input_bytes'], p['pattern'], p['workload_level']), GRID)
    routes = Counter(); gas = 167355
    for tag, result in enumerate(results):
        test.assertEqual(result['terminal'], 0); test.assertEqual(result['counter'], p['repetitions'])
        test.assertEqual(result['hash_count'], p['repetitions']); test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['last_digest'], p['expected_last_digest'][tag]); test.assertEqual(result['last_input_word'], p['expected_last_input_word'][tag])
        test.assertEqual(len(result['memory']), 32 * p['memory_words']); test.assertEqual(result['guards'], p['expected_guards'][tag])
        accounting = p['gas_accounting_per_user']; words = p['memory_words']
        test.assertEqual(accounting['memory_expansion_gas'], 3 * words + words * words // 512)
        test.assertEqual(result['mcopy_gas'], accounting['preparation_mcopy_gas'])
        test.assertEqual(result['keccak_gas'], p['repetitions'] * (30 + 6 * ((p['input_bytes'] + 31) // 32)))
        test.assertEqual(result['keccak_gas'], accounting['keccak_gas'])
        test.assertEqual(result['gas'], 21000 + accounting['ordinary_opcode_gas'] + accounting['memory_expansion_gas'] + result['mcopy_gas'] + result['keccak_gas'])
        test.assertEqual(result['stack'], []); test.assertLessEqual(len(result['hash_samples']), 8)
        test.assertLessEqual(result['peak_pending_inputs'], 1); test.assertLessEqual(result['peak_pending_bytes'], p['input_bytes'])
        test.assertLessEqual(result['reference_calls'], 1); routes.update(result['counts']); gas += result['gas']
        code = bytes.fromhex(program['prestate_contracts'][tag]['code_hex'][2:])
        test.assertEqual(hashlib.sha256(code).hexdigest(), p['runtime_sha256'][tag]); test.assertLess(len(code), 24576)
    test.assertEqual(routes, +Counter(program['expected_route_counts'])); test.assertEqual(gas, p['expected_block_charged_gas'])
    test.assertLessEqual(gas, 30_000_000); test.assertEqual(p['near_full'], gas >= 29_700_000)
    test.assertEqual(p['hash_permutations_per_call'], 0 if not p['input_bytes'] else p['input_bytes'] // 136 + 1)
    test.assertEqual(p['transaction_gas_limit'], m.gas_accounting(p['input_bytes'], p['pattern'], p['full_repetitions'])['gross_gas'] + 1000)
    for remaining in p['header_prefix_remaining_gas']: test.assertGreaterEqual(remaining, p['transaction_gas_limit'])
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216))
    for op in (0x54, 0x55, 0x5c, 0x5d, 0xf0, 0xf1, 0xf4, 0xfa, 0xfd, 0xfe): test.assertEqual(routes[f'opcode:0x{op:02x}'], 0)
    return results


class KeccakBulkTests(unittest.TestCase):
    def test_all_27_two_owners(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args): qualify(self, m.make_case(*args))

    def test_full_plus_one_gas_only_solver(self):
        m = producer(); from keccak_bulk_vm import execute, decode
        with patch.object(m, 'expected_message', side_effect=AssertionError('sizing hashed bytes')), patch.object(m, '_hash', side_effect=AssertionError('sizing hashed bytes')):
            for length, pattern in DOMAINS:
                m.gas_accounting(length, pattern, 1_000_000); maximum = m.full_repetitions(length, pattern)
                self.assertTrue(m.fits(length, pattern, maximum)); self.assertFalse(m.fits(length, pattern, maximum + 1))
        for length, pattern in DOMAINS:
            r = m.full_repetitions(length, pattern) + 1; shapes = set()
            for tag in (0, 1):
                code, counts = m._runtime(length, pattern, r, tag)
                result = execute(code, 50_000_000, length=length, pattern=pattern, repetitions=r, tag=tag)
                self.assertEqual(result['gas'], m.gas_accounting(length, pattern, r)['gross_gas']); self.assertEqual(result['counts'], +Counter(counts))
                envelope = result['gas'] + 1000
                self.assertTrue(envelope > 15_000_000 or envelope > 16_777_216 or 167355 + 2 * result['gas'] > 30_000_000 or any(31_000_000 - 167355 - j * result['gas'] < envelope for j in range(2)))
                shapes.add(tuple(op for _, op, _ in decode(code)))
            sizes = set()
            for count, tag in itertools.product((0, 1, r - 1, r), (0, 1)):
                image, _ = m._runtime(length, pattern, count, tag); sizes.add(len(image)); shapes.add(tuple(op for _, op, _ in decode(image)))
            self.assertEqual((len(shapes), len(sizes)), (1, 1))

    def test_direct_lazy_all_domains_and_feedback(self):
        m = producer(); from keccak_bulk_vm import execute, decode
        for (length, pattern), tag, r in itertools.product(DOMAINS, (0, 1), (0, 1, 2)):
            code, _ = m._runtime(length, pattern, r, tag)
            lazy = execute(code, 50_000_000, length=length, pattern=pattern, repetitions=r, tag=tag)
            direct = execute(code, 50_000_000, length=length, pattern=pattern, repetitions=r, tag=tag, lazy=False)
            for key in ('memory', 'gas', 'counts', 'hash_sequence_sha256', 'counter_sequence_sha256', 'hash_count', 'guards'):
                self.assertEqual(lazy[key], direct[key], (length, pattern, tag, r, key))
            self.assertEqual(direct['reference_calls'], r)
            owner = bytes.fromhex(f'{0x3000000000000000000000000000000000010000 + tag:040x}')
            records = b''.join(owner + i.to_bytes(8, 'big') + (64).to_bytes(8, 'big') + length.to_bytes(8, 'big') + hashlib.sha256(m.expected_message(length, pattern, i, tag)).digest() for i in range(r))
            self.assertEqual(lazy['hash_sequence_sha256'], hashlib.sha256(records).hexdigest())
            self.assertEqual(lazy['counter_sequence_sha256'], hashlib.sha256(b''.join(i.to_bytes(8, 'big') for i in range(r + 1))).hexdigest())
            self.assertEqual([sample['index'] for sample in lazy['hash_samples']], list(range(r)))
        # Generic raw executions exercise hash feedback, partial overwrite and branch/arithmetic reads.
        seed = bytes.fromhex('60006000526020600020602052')
        suffixes = ('60205160000160005200', '600160215360205160005200', '602060202060005200',
                    '602051600114601b5700005b00', '60206020601f5e60205160005200', '600160005260205160005200')
        for suffix in suffixes:
            code = seed + bytes.fromhex(suffix)
            a = execute(code, 100000, length=0, pattern='zero', repetitions=0, tag=0, strict=False)
            b = execute(code, 100000, length=0, pattern='zero', repetitions=0, tag=0, strict=False, lazy=False)
            self.assertEqual((a['memory'], a['counts'], a['gas']), (b['memory'], b['counts'], b['gas']))
            self.assertGreater(a['reference_calls'], 0)

    def test_domains_caps_and_unique_messages(self):
        m = producer()
        for length, pattern in ((True, 'zero'), (1, 'zero'), (0, 'a5'), (136, b'a5'), (136, [])):
            for fn in (m.make_case, m.full_repetitions):
                with self.assertRaises(ValueError): fn(length, pattern)
        for bad in (True, -1, None, 1.0, 1_000_001):
            for fn in (m._runtime, m.gas_accounting, m.fits):
                with self.assertRaises(ValueError): fn(136, 'a5', bad)
            with self.assertRaises(ValueError): m.expected_message(136, 'a5', bad)
        for bad in (True, -1, 2, None, 0.0):
            with self.assertRaises(ValueError): m._runtime(136, 'a5', 1, bad)
        for bad in ('max', None, True, []):
            with self.assertRaises(ValueError): m.make_case(136, 'a5', bad)
        for field, value in (('TRANSACTION_GAS_CAP', 21000), ('EIP7825_GAS_CAP', 21000), ('BLOCK_BUDGET', 167355), ('HEADER_GAS_LIMIT', 167355)):
            with patch.object(m, field, value): self.assertFalse(m.fits(136, 'a5', 0))
        for length, pattern in DOMAINS:
            messages = [m.expected_message(length, pattern, index, tag) for tag, index in itertools.product((0, 1), range(3))]
            self.assertEqual(len(set(messages)), 6 if length else 1)
            if length: self.assertEqual(messages[-1], ((1 << 128) | 2).to_bytes(32, 'big') + bytes([165 if pattern == 'a5' else 0]) * (length - 32))

    def test_semantic_negatives(self):
        m = producer(); from keccak_bulk_vm import execute, decode, run_users
        for length, pattern in DOMAINS:
            code, _ = m._runtime(length, pattern, 2, 1); ins = list(decode(code))
            def run(image): return execute(image, 50_000_000, length=length, pattern=pattern, repetitions=2, tag=1)
            def change(at, width, value):
                image = bytearray(code); image[at + 1:at + 1 + width] = value.to_bytes(width, 'big'); return bytes(image)
            hi = next(i for i, (_, op, _) in enumerate(ins) if op == 0x20)
            # Wrong hash length/offset, nonce/tag, counter bounds/step, prep/tail/guard immediates.
            indices = {hi - 1, hi - 2}
            indices.update(i for i, (_, op, _) in enumerate(ins) if op == 0x7f)
            indices.update(i for i, (_, op, _) in enumerate(ins) if op == 0x62 and i + 1 < len(ins) and ins[i + 1][1] == 1)
            for index in indices:
                at, op, value = ins[index]
                with self.subTest(domain=(length, pattern), index=index), self.assertRaises(AssertionError): run(change(at, op - 0x5f, value ^ 1))
            for i, (_, op, _) in enumerate(ins):
                if op != 0x5e: continue
                for j in (i - 1, i - 2, i - 3):
                    at, pop, value = ins[j]
                    with self.assertRaises(AssertionError): run(change(at, pop - 0x5f, value ^ 1))
            # Replacing KECCAK by POP yields skipped hashing/stack failure; duplicate consumes wrong operands.
            for replacement in (0x50, 0x80):
                image = bytearray(code); image[ins[hi][0]] = replacement
                with self.assertRaises((AssertionError, IndexError)): run(bytes(image))
            with self.assertRaisesRegex(AssertionError, 'out of gas'): execute(code, m.gas_accounting(length, pattern, 2)['gross_gas'] - 1, length=length, pattern=pattern, repetitions=2, tag=1)
        for field in ('recipient', 'owner', 'storage', 'code', 'count', 'calldata'):
            row = m.make_case(level='single'); p = row['request']['program']; c = p['prestate_contracts']
            if field == 'recipient': p['transactions']['recipients'].reverse()
            elif field == 'owner': c[1]['address'] = c[0]['address']
            elif field == 'storage': c[0]['initial_storage'] = {'0x00': '0x01'}
            elif field == 'code': c[1]['code_hex'] = c[0]['code_hex']
            elif field == 'count': p['transactions']['count_per_block'] = 1
            else: p['input_hex'] = '0x01'
            with self.assertRaises(AssertionError): run_users(row)
        with self.assertRaises(AssertionError): run_users(intent_mutant(m.make_case(136, 'a5', 'single')))

    def test_bounded_samples_and_source_exports_cli(self):
        m = producer(); from keccak_bulk_vm import execute
        code, _ = m._runtime(0, 'zero', 10000)
        result = execute(code, 50_000_000, length=0, pattern='zero', repetitions=10000, tag=0)
        self.assertEqual(result['hash_count'], 10000); self.assertEqual(len(result['hash_samples']), 8)
        self.assertEqual([record['index'] for record in result['hash_samples']], [0, 1, 2, 3, 9996, 9997, 9998, 9999])
        self.assertEqual(result['reference_calls'], 1); self.assertEqual(m.ethereum_hash.keccak256.cache_info().currsize, 0)
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'catalog'; self.assertEqual(m.prepare(output), 27)
            catalog = json.loads((output / 'catalog.json').read_bytes()); self.assertEqual(catalog['source_root'], 'bin/zkgas/bench')
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertIn('tests/ethereum_hash.py', catalog['source_sha256'])
            for name in catalog['source_sha256']: self.assertNotIn('..', Path(name).parts); self.assertFalse(Path(name).is_absolute())
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertEqual(len({row['request_sha256'] for row in catalog['cases']}), 27)
            for row in catalog['cases']:
                raw = (output / row['request_path']).read_bytes(); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for bad in (None, True, 'a' * 63, 'A' * 64):
                with self.assertRaises(ValueError): m.prepare(Path(temporary) / 'bad', model_id=bad)
            with self.assertRaises(ValueError): m.prepare('bad')
            command = [sys.executable, '-S', '-B', str(CASES / 'keccak_bulk.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 27})
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=90).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('KECCAK_BULK_NATIVE_BIN'),
        'set KECCAK_BULK_NATIVE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['KECCAK_BULK_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from keccak_bulk_vm import run_users
        base = Path(os.environ['KECCAK_BULK_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = ((0, 'zero', 'full', False), (65536, 'a5', 'single', False), (136, 'a5', 'single', True))
        for length, pattern, level, mutant in specimens:
            row = m.make_case(length, pattern, level); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            if mutant:
                row = intent_mutant(row)
                with self.assertRaisesRegex(AssertionError, 'nonce/tag/index schedule'): run_users(row)
            name = f'l{length}-{pattern}-{level}-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{n: Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/keccak_bulk.py': Path(m.__file__), 'tests/test_keccak_bulk.py': Path(__file__),
                'tests/keccak_bulk_vm.py': Path(__file__).with_name('keccak_bulk_vm.py'),
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
                specimen_role='native-accepted-prefix-intent-mutant' if mutant else 'original-positive',
                mutation='Nonce prefix additionally sets bit80; exact final nonce and Ethereum digest guards recomputed. Original tag/index intent remains unchanged and rejects.' if mutant else None,
                streams='combined stdout/stderr native.log, bounded by unchanged process_guard',
                observability='Native status/gas/routes plus complete execution_result replay; no native final-memory export.',
                disk_preflight=dict(free_bytes=free, required_free_bytes=576 * 1024 * 1024),
                source_root='bin/zkgas/bench',
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


def intent_mutant(row):
    """Accepted EVM variant changes nonce prefix bit80 and its final exact guards."""
    row = copy.deepcopy(row); m = producer(); from keccak_bulk_vm import decode
    p = row['parameters']; length = p['input_bytes']; r = p['repetitions']; pattern = p['pattern']
    for tag, contract in enumerate(row['request']['program']['prestate_contracts']):
        code = bytearray.fromhex(contract['code_hex'][2:]); ins = list(decode(code)); hi = next(i for i, (_, op, _) in enumerate(ins) if op == 0x20)
        # The prefix immediate is added to the counter immediately before MSTORE64.
        prefix = next((at, op, value) for at, op, value in ins[:hi] if op == 0x7f and value == tag << 128 and at > next(at for at, op, _ in ins if op == 0x57))
        at, op, value = prefix; code[at + 1:at + 33] = (value | (1 << 80)).to_bytes(32, 'big')
        old_word = ((tag << 128) | (r - 1)).to_bytes(32, 'big'); new_word = (int.from_bytes(old_word, 'big') | (1 << 80)).to_bytes(32, 'big')
        old_digest = m._hash(m.expected_message(length, pattern, r - 1, tag)); new_digest = m._hash(new_word + bytes([165 if pattern == 'a5' else 0]) * (length - 32))
        for at, op, value in ins[hi + 1:]:
            if op == 0x7f and value in (int.from_bytes(old_word, 'big'), int.from_bytes(old_digest, 'big')):
                replacement = new_word if value == int.from_bytes(old_word, 'big') else new_digest; code[at + 1:at + 33] = replacement
        contract['code_hex'] = '0x' + code.hex()
    return row


if __name__ == '__main__': unittest.main()
