"""Cold code read recipe: independent bytes, cold gas, content and export gates."""
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
import weakref
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('balance', 'size'), (16, 128, 512), ('zero', 'selected')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
BUILD_SHA256 = '1e1a7f4dbd141ce97359114b737d744ee71eb37b78e974038179eedf25cd8023'


def producer():
    path = CASES / 'cold_code_reads.py'
    assert path.is_file(), 'cold code read producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('cold_code_reads', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from cold_code_reads_vm import run
    result = run(row); p = row['parameters']; program = row['request']['program']
    test.assertEqual(result['counts'], +Counter(program['expected_route_counts']))
    test.assertEqual((result['counter'], result['cold_count'], result['warm_count']), (p['visits'], p['visits'], 0))
    test.assertEqual(result['gas'], p['expected_user_gross_gas'])
    test.assertEqual(result['gas'], p['expected_user_charged_gas'])
    test.assertEqual(p['expected_block_charged_gas'], 167355 + result['gas'])
    test.assertEqual((p['expected_user_refund_counter'], p['expected_user_refund_applied']), (0, 0))
    test.assertEqual(p['gas_accounting_per_user']['memory_expansion_gas'], 3)
    test.assertEqual(p['gas_accounting_per_user']['cold_account_gas'], 2600 * p['visits'])
    test.assertEqual(p['near_full'], p['expected_block_charged_gas'] >= 29_700_000)
    test.assertLessEqual(len(result['samples']), 8)
    sequence = hashlib.sha256(); counters = hashlib.sha256()
    for index in range(p['visits']):
        sequence.update((0x4000000000000000000000000000000000010000 + index).to_bytes(20, 'big'))
        sequence.update((0 if p['operation'] == 'balance' else 24576).to_bytes(32, 'big'))
    for index in range(p['visits'] + 1): counters.update(index.to_bytes(32, 'big'))
    test.assertEqual(result['ordered_address_value_sha256'], sequence.hexdigest())
    test.assertEqual(result['counter_sequence_sha256'], counters.hexdigest())
    return result


def intent_mutant(row):
    result = copy.deepcopy(row); contract = result['request']['program']['prestate_contracts'][0]
    code = bytes.fromhex(contract['code_hex'][2:]); assert len(code) == 24576 and code[-1] == 0x5b
    contract['code_hex'] = '0x' + (code[:-1] + b'\0').hex()
    return result


class ColdCodeReadsTests(unittest.TestCase):
    def test_all_twelve_actual_bytes_gas_order_and_envelopes(self):
        m = producer(); envelopes = {}
        for operation, footprint, level in GRID:
            with self.subTest(operation=operation, footprint=footprint, level=level):
                row = m.make_case(operation, footprint, level); qualify(self, row); p = row['parameters']
                self.assertEqual(p['visits'], 0 if level == 'zero' else footprint)
                self.assertEqual(row['sample_id'], f'cold-code-read-{operation}-k{footprint}-n{p["visits"]}')
                self.assertEqual(row['baseline_sample_id'], f'cold-code-read-{operation}-k{footprint}-n0')
                self.assertEqual(row['role'], 'control' if level == 'zero' else 'candidate')
                self.assertEqual(len(row['request']['program']['prestate_contracts']), footprint)
                self.assertFalse(p['near_full']); self.assertLess(len(m.encoded(row['request'])), 32 * 1024 * 1024)
                envelopes.setdefault(footprint, p['transaction_gas_limit']); self.assertEqual(envelopes[footprint], p['transaction_gas_limit'])
        for footprint, envelope in envelopes.items():
            self.assertEqual(envelope, max(m.gas_accounting(op, footprint, footprint)['gross_gas'] for op in ('balance', 'size')) + 1000)

    def test_direct_small_visits_and_fixed_shapes(self):
        m = producer(); from cold_code_reads_vm import execute, decode
        for operation in ('balance', 'size'):
            shapes = set(); sizes = set(); contracts = m.make_case(operation, 16, 'selected')['request']['program']['prestate_contracts']
            for visits in (0, 1, 2, 16):
                code, counts = m._runtime(operation, 16, visits)
                result = execute(code, 15_000_000, operation=operation, footprint=16, visits=visits, contracts=contracts)
                self.assertEqual(result['gas'], m.gas_accounting(operation, 16, visits)['gross_gas'])
                self.assertEqual(result['counts'], +Counter(counts)); self.assertEqual(result['counter'], visits)
                shapes.add(tuple(op for _, op, _ in decode(code))); sizes.add(len(code))
            self.assertEqual((len(shapes), len(sizes)), (1, 1))
        # Manual tiny-byte gas fixture: one memory word; loop/static overhead plus2600 per read.
        self.assertEqual(m.gas_accounting('size', 16, 0)['gross_gas'], 21067)
        self.assertEqual(m.gas_accounting('size', 16, 1)['gross_gas'], 23756)

    def test_mutations_reject_wrong_account_count_content_guards_and_missing_read(self):
        m = producer(); from cold_code_reads_vm import run, execute, decode
        row = m.make_case('size', 16, 'selected'); code = bytes.fromhex(row['request']['program']['code_hex'][2:])
        for field, value in (('footprint', 128), ('visits', 15), ('primary_code_owner_address', '0x' + '77' * 20), ('all_user_transactions', 2)):
            wrong = copy.deepcopy(row); wrong['parameters'][field] = value
            with self.assertRaises(AssertionError): run(wrong)
        for kind in ('base', 'count', 'length', 'tag', 'pattern', 'storage', 'duplicate', 'tail'):
            wrong = copy.deepcopy(row); contracts = wrong['request']['program']['prestate_contracts']
            if kind == 'base': contracts[0]['address'] = '0x' + '55' * 20
            elif kind == 'count': contracts.pop()
            elif kind == 'duplicate': contracts[1] = copy.deepcopy(contracts[0])
            elif kind == 'storage': contracts[0]['initial_storage'] = {'0x1': '0x1'}
            elif kind == 'tail': wrong = intent_mutant(row)
            else:
                body = bytearray.fromhex(contracts[0]['code_hex'][2:])
                if kind == 'length': body.pop()
                elif kind == 'tag': body[32] = 2
                elif kind == 'pattern': body[0] = 0x60
                contracts[0]['code_hex'] = '0x' + body.hex()
            with self.subTest(kind=kind), self.assertRaises(AssertionError): run(wrong)
            # Even an attacker-updated content hash cannot replace the literal code pattern/tag intent.
            wrong['parameters']['target_code_sha256'] = [hashlib.sha256(bytes.fromhex(c['code_hex'][2:])).hexdigest() for c in wrong['request']['program']['prestate_contracts']]
            with self.subTest(rehashed_kind=kind), self.assertRaises(AssertionError): run(wrong)
        instructions = list(decode(code))
        for at, op, value in instructions:
            if op in (0x31, 0x3b, 0x52, 0x14, 0x57, 0x56) or op == 0x7f and value in (16, 24576):
                wrong = bytearray(code); wrong[at] = 0x50 if op in (0x31, 0x3b) else 0x5b
                with self.assertRaises(AssertionError):
                    execute(bytes(wrong), 15_000_000, operation='size', footprint=16, visits=16, contracts=row['request']['program']['prestate_contracts'])
        base_pc = next(at for at, op, _ in instructions if op == 0x73)
        wrong = bytearray(code); wrong[base_pc + 20] ^= 1
        with self.assertRaises(AssertionError): execute(bytes(wrong), 15_000_000, operation='size', footprint=16, visits=16, contracts=row['request']['program']['prestate_contracts'])
        for args in (('balance', 16, 16), ('size', 16, 15)):
            with self.assertRaises(AssertionError): execute(code, 15_000_000, operation=args[0], footprint=args[1], visits=args[2], contracts=row['request']['program']['prestate_contracts'])
        with self.assertRaises(AssertionError): execute(code, 21000, operation='size', footprint=16, visits=16, contracts=row['request']['program']['prestate_contracts'])
        # Same-length wrong content passes EXTCODESIZE math, still fails original intent.
        self.assertEqual(len(bytes.fromhex(intent_mutant(row)['request']['program']['prestate_contracts'][0]['code_hex'][2:])), 24576)

    def test_domains_before_any_artifacts(self):
        m = producer()
        for operation in (True, None, [], 0, 'EXTCODESIZE'):
            for fn in (m.make_case, m._runtime, m.gas_accounting):
                with self.assertRaises(ValueError): fn(operation, 16, *(() if fn is m.make_case else (0,)))
        for footprint in (True, None, [], 0, 1, 513, 16.0):
            for fn in (m.make_case, m._runtime, m.gas_accounting):
                with self.assertRaises(ValueError): fn('size', footprint, *(() if fn is m.make_case else (0,)))
        for visits in (True, None, [], -1, 17, 0.0):
            for fn in (m._runtime, m.gas_accounting):
                with self.assertRaises(ValueError): fn('size', 16, visits)
        for level in (True, None, [], 'full'):
            with self.assertRaises(ValueError): m.make_case(level=level)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'bad'
            for model in (True, None, '', 'A' * 64, 'a' * 63):
                with self.assertRaises(ValueError): m.prepare(path, model_id=model)
                self.assertFalse(path.exists())
            for output in (None, True, str(path)):
                with self.assertRaises(ValueError): m.prepare(output)
        for cap in ('TRANSACTION_GAS_CAP', 'EIP7825_GAS_CAP', 'BLOCK_BUDGET', 'HEADER_GAS_LIMIT'):
            with patch.object(m, cap, 21000), self.assertRaises(ValueError): m.make_case()

    def test_streaming_export_cli_commitments_and_fresh_paths(self):
        m = producer(); active = []; original_make = m.make_case; original_helpers = m.helpers()
        class Request(dict): pass
        def track(*args):
            self.assertTrue(all(ref() is None for ref in active), 'previous request retained at next case')
            row = original_make(*args); row['request'] = Request(row['request']); active.append(weakref.ref(row['request']))
            return row
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'export'
            with patch.object(m, 'make_case', new=track), patch.object(m, 'helpers', return_value=original_helpers):
                self.assertEqual(m.prepare(output), 12)
            self.assertTrue(all(ref() is None for ref in active))
            catalog = json.loads((output / 'catalog.json').read_bytes())
            self.assertEqual(catalog['source_root'], 'bin/zkgas/bench'); self.assertEqual(catalog['corpus_id'], 'unzen-cold-code-reads-20261104')
            self.assertEqual(len(catalog['cases']), 12); self.assertEqual(len({r['request_sha256'] for r in catalog['cases']}), 12)
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            for row in catalog['cases']:
                self.assertNotIn('request', row); raw = (output / row['request_path']).read_bytes()
                self.assertLess(len(raw), 32 * 1024 * 1024); self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertNotIn('code_hex', row['parameters'])
                p = row['parameters']; self.assertEqual(json.loads(raw), m.make_case(p['operation'], p['footprint'], p['workload_level'])['request'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            empty = Path(tmp) / 'empty'; empty.mkdir()
            with self.assertRaises(FileExistsError): m.prepare(empty)
        # Clean the first ~123MiB export before CLI export/native stages.
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'cli'
            command = [sys.executable, '-S', '-B', str(CASES / 'cold_code_reads.py'), '--output-dir', str(output), '--model-id', 'a' * 64]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 12})
            self.assertEqual(json.loads((output / 'catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('ZKGAS_FIXTURE_BIN'),
        'set ZKGAS_FIXTURE_BIN for optional frozen-native smoke')
    def test_frozen_native_exact_three_specimens(self):
        m = producer(); binary = Path(os.environ['ZKGAS_FIXTURE_BIN']); build = binary.with_name('native-build.json')
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        self.assertEqual(hashlib.sha256(build.read_bytes()).hexdigest(), BUILD_SHA256)
        provenance = json.loads(build.read_bytes()); self.assertEqual(len(provenance['source_file_sha256']), 81)
        self.assertEqual(sum(name.endswith('.rs') for name in provenance['source_file_sha256']), 66)
        self.assertEqual(provenance['compiled_workspace_input_count'], 70)
        workspace = CASES.parents[3]
        for name, expected in provenance['source_file_sha256'].items():
            self.assertEqual(hashlib.sha256((workspace / name).read_bytes()).hexdigest(), expected, name)
        # Read-only BASE comparison also excludes unrelated edits to the five stable helpers.
        helper_baseline = {}
        for relative, module in m.source_modules().items():
            name = 'bin/zkgas/bench/' + relative
            frozen = subprocess.run(['git', '-C', str(workspace), 'show',
                '0a8d3f4cd5c8e3b0d283b2be4642bc91cbb44ae5:' + name], check=True, capture_output=True, timeout=10).stdout
            self.assertEqual(Path(module.__file__).read_bytes(), frozen, name)
            helper_baseline[relative] = hashlib.sha256(frozen).hexdigest()
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from cold_code_reads_vm import run
        base = Path(os.environ['COLD_CODE_READS_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        specimens = (('size', 16, False), ('balance', 128, False), ('size', 16, True))
        for operation, footprint, mutant in specimens:
            row = m.make_case(operation, footprint, 'selected'); qualify(self, row)
            original_request = m.encoded(row['request']); original_row = m.encoded(row)
            if mutant:
                row = intent_mutant(row)
                with self.assertRaises(AssertionError): run(row)
            name = f'{operation}-k{footprint}-selected-' + ('intent-mutant' if mutant else 'positive')
            root = base / name; root.mkdir(); source = root / 'request.json'; raw = m.encoded(row['request'])
            self.assertLess(len(raw), 32 * 1024 * 1024); source.write_bytes(raw); del raw
            (root / 'requested-intent.json').write_bytes(original_request)
            (root / 'requested-intent-row.json').write_bytes(original_row); (root / 'row.json').write_bytes(m.encoded(row))
            sources = root / 'sources'; sources.mkdir()
            source_paths = {**{n: Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/cold_code_reads.py': Path(m.__file__), 'tests/test_cold_code_reads.py': Path(__file__),
                'tests/cold_code_reads_vm.py': Path(__file__).with_name('cold_code_reads_vm.py'),
                **{f'{n}.py': CASES.parent / f'{n}.py' for n in ('process_guard', 'artifacts', 'shared_memory')}}
            for relative, path in source_paths.items():
                target = sources / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(path.read_bytes())
            (root / 'native-build.json').write_bytes(build.read_bytes())
            growing = [root / n for n in ('input.json', 'input.bin', 'native.json')]
            command = [str(binary), '--input', str(source), '--output', str(growing[0]), '--output-bincode', str(growing[1]),
                '--report', str(growing[2]), '--execution-profile', 'evm-gas-only']
            free = shutil.disk_usage(root).free; self.assertGreaterEqual(free, 576 * 1024 * 1024)
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), limits=limits,
                native_sha256=NATIVE_SHA256, native_build_sha256=BUILD_SHA256,
                request_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                requested_intent_request_sha256=hashlib.sha256(original_request).hexdigest(),
                row_sha256=hashlib.sha256(m.encoded(row)).hexdigest(), requested_intent_row_sha256=hashlib.sha256(original_row).hexdigest(),
                specimen_role='native-accepted-wrong-code-content-intent-mutant' if mutant else 'original-positive',
                canonical_bridge_eligible=not mutant,
                mutation='One configured target final JUMPDEST replaced by STOP; same24576 length and original content intent retained.' if mutant else None,
                observability='Native gas/status/routes and complete minimized replay; no direct native per-read values/order export.',
                streams='combined stdout/stderr native.log bounded by unchanged process_guard',
                disk_preflight=dict(free_bytes=free, required_free_bytes=576 * 1024 * 1024),
                uncompressed_caps_bytes=dict(request=32 * 1024 * 1024, canonical_json=128 * 1024 * 1024,
                    native_report=128 * 1024 * 1024, benchmark_input=64 * 1024 * 1024, combined_growing_stage=192 * 1024 * 1024),
                native_declared_source_verification=dict(status='all81-match', declared_count=81,
                    rust_count=sum(n.endswith('.rs') for n in provenance['source_file_sha256']),
                    compiled_workspace_input_count=provenance['compiled_workspace_input_count'], source_file_sha256=provenance['source_file_sha256']),
                stable_helpers_base_commit='0a8d3f4cd5c8e3b0d283b2be4642bc91cbb44ae5', stable_helpers_base_sha256=helper_baseline,
                source_root='bin/zkgas/bench', source_sha256={n: hashlib.sha256(path.read_bytes()).hexdigest() for n, path in source_paths.items()},
                helper_source_sha256={n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()},
                outcome='not-launched', exit_code=None, elapsed_seconds=None, output_paths=[str(path.relative_to(root)) for path in growing])
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
            # A source/native failure stops this test; it never retries or advances silently.
            self.assertEqual(manifest['exit_code'], 0)
            self.assertLessEqual(growing[0].stat().st_size, 128 * 1024 * 1024)
            self.assertLessEqual(growing[1].stat().st_size, 64 * 1024 * 1024)
            self.assertLessEqual(growing[2].stat().st_size, 128 * 1024 * 1024)
            self.assertLessEqual(sum(path.stat().st_size for path in growing), 192 * 1024 * 1024)
            report = json.loads(growing[2].read_bytes()); receipts = report['receipts_by_block'][0]
            self.assertEqual(report['guest_input_sha256'], hashlib.sha256(growing[1].read_bytes()).hexdigest())
            self.assertEqual(report['guest_input_bincode_length'], growing[1].stat().st_size)
            self.assertEqual(report['construction_request_sha256'], manifest['request_sha256'])
            self.assertEqual(report['execution_profile'], 'evm-gas-only'); self.assertTrue(report['public_output']); self.assertTrue(report['public_values'])
            self.assertEqual([r['success'] for r in receipts], [True, True])
            self.assertEqual([r['gas_used'] for r in receipts], [167355, row['parameters']['expected_user_charged_gas']])
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            users = report['transactions']['blocks'][0]['users']; self.assertEqual(len(users), 1)
            self.assertEqual((users[0]['nonce'], users[0]['to'], users[0]['gas_limit']), (0, '0x' + '66' * 20, row['parameters']['transaction_gas_limit']))
            initialized = report['prestate_contracts']['initialized_storage']; self.assertEqual(len(initialized), footprint)
            for entry in initialized:
                self.assertEqual((entry['initial_storage_entry_count'], entry['final_storage_entry_count']), (0, 0))
                self.assertEqual(entry['initial_storage_root'], entry['final_storage_root'])
            canonical = json.loads(growing[0].read_bytes())
            witness_hashes = {hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()
                for witness in canonical['witnesses'] for code in witness['witness']['codes']}
            configured = {hashlib.sha256(bytes.fromhex(c['code_hex'][2:])).hexdigest() for c in row['request']['program']['prestate_contracts']}
            intersection = configured & witness_hashes
            minimality = dict(operation=operation, configured_target_code_sha256=sorted(configured),
                witness_code_sha256=sorted(witness_hashes), target_code_intersection_sha256=sorted(intersection),
                expected_target_code_count=footprint if operation == 'size' else 0,
                actual_target_code_count=len(intersection), target_body_minimality_passed=intersection == (configured if operation == 'size' else set()),
                requested_original_content_intent_passed=not mutant, canonical_bridge_eligible=not mutant,
                input_json_sha256=hashlib.sha256(growing[0].read_bytes()).hexdigest(), input_bincode_sha256=report['guest_input_sha256'],
                native_report_sha256=hashlib.sha256(growing[2].read_bytes()).hexdigest())
            (root / 'qualification.json').write_bytes(m.encoded(minimality))
            self.assertTrue(minimality['target_body_minimality_passed'], 'unused BALANCE target bodies or missing EXTCODESIZE bodies: diagnose constructor before measuring')
            self.assertIn(row['parameters']['runtime_sha256'], witness_hashes)
            self.assertEqual(report['prestate_contracts']['witnessed_unique_code_count'], footprint if operation == 'size' else 0)


if __name__ == '__main__': unittest.main()
