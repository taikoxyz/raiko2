"""Finite calldata catalogs: byte intent, cold/warm gas, and bounded exports."""
from collections import Counter
import copy
import hashlib
import importlib.util
import inspect
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'
QUERY = 0x1122334455667788 << 192


def producer():
    path = CASES / 'warm_read_catalogs.py'
    assert path.is_file(), 'warm-read catalog producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('warm_read_catalogs', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from warm_read_catalogs_vm import run_users
    result = run_users(row)[0]; p = row['parameters']; program = row['request']['program']
    test.assertEqual(result['gas'], p['expected_user_charged_gas'])
    test.assertEqual(result['counts'], +Counter(program['expected_route_counts']))
    test.assertEqual((result['cold_reads'], result['warm_reads']), (512, 64 * p['repetitions'] + 1))
    test.assertEqual(result['seed_keys'], p['ordered_keys'])
    test.assertEqual((result['stack'], result['counter'], result['peak_memory_bytes']), ([], p['repetitions'], 128))
    test.assertEqual(result['read_events'], 513 + 64 * p['repetitions'])
    test.assertEqual(p['runtime_sha256'], hashlib.sha256(bytes.fromhex(program['code_hex'][2:])).hexdigest())
    test.assertEqual(p['calldata_sha256'], hashlib.sha256(bytes.fromhex(program['input_hex'][2:])).hexdigest())
    test.assertEqual(p['expected_block_charged_gas'], 167355 + result['gas'])
    test.assertFalse(p['near_full']); test.assertEqual(p['header_prefix_remaining_gas'], [30832645])
    return result


class WarmReadCatalogTests(unittest.TestCase):
    def test_default_rows_and_custom_pair_preserve_frozen_bytes(self):
        # Complete canonical row hashes include requests, gas, runtime and envelope.
        # Frozen before adding the probe option; generator/helper provenance is separate.
        m = producer()
        frozen = {
            ('ordered', 'zero'): '14a0fa70f97d2edbac542feaf9623a354b06075e71c0db3f1a8e5d0227f85520',
            ('ordered', 'full'): 'bc1234627a48b88df5045e4b502d636a56a34e2d5740ecc908171851220c8d7d',
            ('mixed', 'zero'): 'b06a5c809c05c80a74c89ca56b12f5648c3f1b1dfc492c036623c246b8d86720',
            ('mixed', 'full'): '37b8a41c7071ad1f31a1e0386ad93b0c608a252ce6d15f83aff70252f0d178ee',
        }
        for (catalog, level), sha in frozen.items():
            self.assertEqual(m.digest(m.encoded(m.make_case(catalog, level))), sha)
        keys = m._catalog('mixed')
        for level, sha in (('zero', 'e4eab90686d4372b15318c2c46c966e9f773c4088c3d52dc21857cd232056283'),
                           ('full', '23060a317db643e166c8b4c84c6e90f1a513e26366ccbdf665d0e2c58def14fb')):
            self.assertEqual(m.digest(m.encoded(m.make_custom_case(keys, level))), sha)

    def test_probe_factory_actual_bytes_and_export(self):
        m = producer(); self.assertTrue(hasattr(m, 'make_probe_case'), 'intermediate probe factory missing')
        keys = list(m._catalog('mixed')); full = m.make_custom_case(keys); maximum = m.full_repetitions()
        for r in (1, 2, 512, maximum - 1):
            row = m.make_probe_case(keys, r); qualify(self, row); p = row['parameters']
            self.assertEqual((p['repetitions'], p['expected_counter'], p['workload_level']), (r, r, 'probe'))
            self.assertEqual(row['sample_id'], full['sample_id'].rsplit('-n', 1)[0] + f'-n{64 * r}')
            self.assertEqual(row['baseline_sample_id'], m.make_custom_case(keys, 'zero')['sample_id'])
            self.assertEqual(p['transaction_gas_limit'], full['parameters']['transaction_gas_limit'])
            self.assertEqual(p['full_plus_one_constraint_failures'], ['transaction_fixture_cap'])
            self.assertEqual(row['data_role'], 'development_probe'); self.assertFalse(row['holdout_only'])
            self.assertIn('intermediate', p['fullness_scope'].lower())
        for r in (True, None, 0, -1, 1.0, maximum, maximum + 1):
            with self.assertRaises(ValueError): m.make_probe_case(keys, r)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp); external = base / 'keys.json'; external.write_bytes(m.encoded(dict(schema_version=1, keys=keys)))
            for r in (True, 0, -1, 1.0, maximum, maximum + 1):
                with self.assertRaises(ValueError): m.prepare(base / 'invalid', catalog_file=external, probe_repetitions=r)
                self.assertFalse((base / 'invalid').exists())
            with self.assertRaises(ValueError): m.prepare(base / 'invalid', probe_repetitions=512)
            self.assertFalse((base / 'invalid').exists())
            output = base / 'probe'; self.assertEqual(m.prepare(output, catalog_file=external, probe_repetitions=512), 3)
            obj = json.loads((output / 'catalog.json').read_bytes()); self.assertEqual(obj['levels'], ['zero', 'probe', 'full'])
            self.assertEqual(len({row['sample_id'] for row in obj['cases']}), 3)
            self.assertEqual([row['parameters']['repetitions'] for row in obj['cases']], [0, 512, maximum])
            self.assertEqual(len({row['parameters']['transaction_gas_limit'] for row in obj['cases']}), 1)
            command = [sys.executable, '-S', '-B', str(CASES / 'warm_read_catalogs.py'), '--output-dir', str(base / 'cli-probe'), '--catalog-file', str(external), '--probe-repetitions', '512']
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 3})
            for r in ('0', str(maximum), str(maximum + 1)):
                bad = command.copy(); bad[bad.index('--output-dir') + 1] = str(base / 'invalid-cli'); bad[-1] = r
                self.assertNotEqual(subprocess.run(bad, capture_output=True, timeout=60).returncode, 0)
                self.assertFalse((base / 'invalid-cli').exists())
            bad = command.copy(); bad[bad.index('--output-dir') + 1] = str(base / 'no-catalog-cli')
            del bad[bad.index('--catalog-file'):bad.index('--catalog-file') + 2]
            self.assertNotEqual(subprocess.run(bad, capture_output=True, timeout=60).returncode, 0)
            self.assertFalse((base / 'no-catalog-cli').exists())

    def test_a_default_catalogs_and_full_packing(self):
        m = producer(); rows = [m.make_case(c, level) for c in ('ordered', 'mixed') for level in ('zero', 'full')]
        for row in rows:
            result = qualify(self, row); p = row['parameters']; data = bytes.fromhex(row['request']['program']['input_hex'][2:])
            keys = [int(v, 16) for v in p['ordered_keys']]
            self.assertEqual((len(keys), len(set(keys)), keys[-1]), (512, 512, QUERY))
            self.assertTrue(all(k & ((1 << 192) - 1) == 0 and all(k.to_bytes(32, 'big')[:8]) for k in keys))
            self.assertEqual((len(data), data.count(0), len(data) - data.count(0)), (16384, 12288, 4096))
            gas = p['gas_accounting_per_user']; self.assertEqual(gas['intrinsic_gas'], 135688)
            self.assertEqual(gas['eip7623_floor_gas'], 307720); self.assertFalse(gas['eip7623_floor_active'])
            self.assertEqual(gas['cold_sload_gas'], 1075200)
            self.assertEqual(gas['warm_sload_gas'], (64 * p['repetitions'] + 1) * 100)
            self.assertEqual(p['full_plus_one_constraint_failures'], ['transaction_fixture_cap'])
            self.assertTrue(m.fits(p['full_repetitions'])); self.assertFalse(m.fits(p['full_repetitions'] + 1))
            self.assertEqual(p['initial_storage_entries'], 0); self.assertEqual(p['all_user_transactions'], 1)
            self.assertEqual(p['primary_code_owner_count'], 1)
            self.assertNotIn('prestate_contracts', row['request']['program'])
            self.assertEqual(p['transaction_gas_limit'], m.gas_accounting(m.full_repetitions())['gross_gas'] + 1000)
            self.assertLessEqual(result['gas'] + 1000, p['transaction_gas_limit'])
        for a, b in ((rows[0], rows[2]), (rows[1], rows[3])):
            self.assertEqual(a['request']['program']['code_hex'], b['request']['program']['code_hex'])
            self.assertEqual(a['parameters']['gas_accounting_per_user'], b['parameters']['gas_accounting_per_user'])
            self.assertNotEqual(a['request']['program']['input_hex'], b['request']['program']['input_hex'])
        self.assertEqual(rows[0]['request']['program']['input_hex'], rows[1]['request']['program']['input_hex'])
        self.assertEqual(rows[0]['parameters']['ordered_keys'][:2], [f'0x{0x0101010101010101 << 192:064x}', f'0x{0x0101010101010102 << 192:064x}'])
        # Independent published SplitMix64 recurrence, including rejected outputs.
        state = 0x243f6a8885a308d3; expected = []; seen = set()
        while len(expected) < 511:
            state = (state + 0x9e3779b97f4a7c15) & ((1 << 64) - 1); z = state
            z = ((z ^ (z >> 30)) * 0xbf58476d1ce4e5b9) & ((1 << 64) - 1)
            z = ((z ^ (z >> 27)) * 0x94d049bb133111eb) & ((1 << 64) - 1); z ^= z >> 31
            if z != 0x1122334455667788 and all(z.to_bytes(8, 'big')) and z not in seen:
                expected.append(f'0x{z << 192:064x}'); seen.add(z)
        self.assertEqual(rows[2]['parameters']['ordered_keys'], expected + [f'0x{QUERY:064x}'])

    def test_direct_small_runs_and_cache_feedback(self):
        m = producer(); from warm_read_catalogs_vm import execute, decode, structure, prove_body, apply_body
        for catalog in ('ordered', 'mixed'):
            data = bytes.fromhex(m.make_case(catalog, 'zero')['request']['program']['input_hex'][2:])
            for r in (0, 1, 2):
                code, counts = m._runtime(r)
                direct = execute(code, data, 15000000, repetitions=r, cache=False)
                cached = execute(code, data, 15000000, repetitions=r)
                self.assertEqual(direct, cached); self.assertEqual(direct['gas'], m.gas_accounting(r)['gross_gas'])
                self.assertEqual(direct['counts'], +Counter(counts))
        code, _ = m._runtime(2); ins = list(decode(code)); shape = structure(ins, 2)
        body = [i for i in ins if shape['body'] <= i[0] < shape['body_end']]
        keys = [int.from_bytes(data[i:i + 32], 'big') for i in range(0, len(data), 32)]
        warm = set(keys); storage = {}; stack = [QUERY]
        proof = prove_body(body, stack, warm, storage, QUERY)
        before = (list(stack), set(warm), dict(storage)); apply_body(proof, stack, warm, storage)
        self.assertEqual((stack, warm, storage), before)
        for bad in ([QUERY, 0], [0], []):
            with self.assertRaises(AssertionError): apply_body(proof, bad, warm, storage)
        warm.remove(QUERY)
        with self.assertRaises(AssertionError): apply_body(proof, stack, warm, storage)
        warm.add(QUERY); storage[QUERY] = 1
        with self.assertRaises(AssertionError): apply_body(proof, stack, warm, storage)
        for i in (0, 1, 2, 63, 95, 191):
            bad = bytearray(code); bad[body[i][0]] = 0x81
            with self.assertRaises(AssertionError): execute(bytes(bad), data, 15000000, repetitions=2)
        for pc, op, value in ins:
            if 0x60 <= op <= 0x7f:
                bad = bytearray(code); width = op - 0x5f
                bad[pc + 1:pc + 1 + width] = (value ^ 1).to_bytes(width, 'big')
                with self.assertRaises(AssertionError): execute(bytes(bad), data, 15000000, repetitions=2)
        for changed in (data[:-1], data[:32] + data[:32] + data[64:], data[32:] + data[:32]):
            with self.assertRaises(AssertionError): execute(code, changed, 15000000, repetitions=2)
        row = m.make_case('ordered', 'full'); mutant = intent_mutant(row)
        from warm_read_catalogs_vm import run_users
        with self.assertRaisesRegex(AssertionError, 'catalog'): run_users(mutant)
        result = execute(bytes.fromhex(mutant['request']['program']['code_hex'][2:]), bytes.fromhex(mutant['request']['program']['input_hex'][2:]), 15000000, repetitions=row['parameters']['repetitions'])
        self.assertEqual(result['gas'], row['parameters']['expected_user_charged_gas'])
        self.assertEqual(row['parameters'], mutant['parameters'])
        for field in ('prestate_contracts', 'storage_state'):
            bad = copy.deepcopy(row); bad['request']['program'][field] = []
            with self.assertRaises(AssertionError): run_users(bad)

    def test_custom_validation_and_bounded_export(self):
        m = producer(); keys = m.make_case('mixed', 'zero')['parameters']['ordered_keys']; keys = keys[1:511] + keys[:1] + keys[511:]
        row = m.make_custom_case(keys); qualify(self, row)
        expected_hash = hashlib.sha256(b''.join(int(k, 16).to_bytes(32, 'big') for k in keys)).hexdigest()
        self.assertEqual(row['parameters']['catalog_id'], 'custom-' + expected_hash)
        self.assertFalse(row['holdout_only']); self.assertEqual(row['data_role'], 'development_probe')
        invalid = [True, iter(keys), keys[:-1], keys + keys[:1], [True] + keys[1:], [keys[0].upper()] + keys[1:], [keys[1]] + keys[1:], keys[::-1], ['0x' + '01' * 32] + keys[1:]]
        for bad in invalid:
            with self.assertRaises(ValueError): m.make_custom_case(bad)
        for bad in (True, -1, 1.0, 1000001):
            with self.assertRaises(ValueError): m._runtime(bad)
        for bad in (None, True, 'other'):
            with self.assertRaises(ValueError): m.make_case(bad)
            with self.assertRaises(ValueError): m.make_case(level=bad)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp); external = base / 'catalog.json'; raw = json.dumps(dict(schema_version=1, keys=keys)).encode(); external.write_bytes(raw)
            for custom in (False, True):
                output = base / ('custom' if custom else 'default')
                self.assertEqual(m.prepare(output, catalog_file=external if custom else None), 2 if custom else 4)
                catalog = json.loads((output / 'catalog.json').read_bytes())
                self.assertEqual(catalog['source_root'], 'bin/zkgas/bench')
                self.assertEqual(len(catalog['source_sha256']), 5)
                for name, module in m.source_modules().items(): self.assertEqual(catalog['source_sha256'][name], hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest())
                for name, fn in m.helpers().items(): self.assertEqual(catalog['helper_source_sha256'][name], hashlib.sha256(inspect.getsource(fn).encode()).hexdigest())
                self.assertNotIn(str(base), json.dumps(catalog))
                if custom: self.assertEqual(catalog['external_catalog_json_sha256'], hashlib.sha256(raw).hexdigest())
                for exported in catalog['cases']:
                    request_raw = (output / exported['request_path']).read_bytes()
                    self.assertEqual(hashlib.sha256(request_raw).hexdigest(), exported['request_sha256'])
                    self.assertNotIn('request', exported)
                with self.assertRaises(FileExistsError): m.prepare(output)
            for value in ({'schema_version': True, 'keys': keys}, {'schema_version': 1, 'keys': keys, 'extra': 0}, {'schema_version': 1, 'keys': keys[:-1]}):
                external.write_text(json.dumps(value))
                with self.assertRaises(ValueError): m.prepare(base / 'invalid', catalog_file=external)
                self.assertFalse((base / 'invalid').exists())
            external.write_bytes(b' ' * (128 * 1024 + 1))
            with patch.object(m.json, 'loads', side_effect=AssertionError('must reject size before parsing')):
                with self.assertRaises(ValueError): m.prepare(base / 'oversize', catalog_file=external)
            for model in (True, None, 'A' * 64, 'a' * 63):
                with self.assertRaises(ValueError): m.prepare(base / 'bad-model', model_id=model)
                self.assertFalse((base / 'bad-model').exists())
            for output in (True, None, 'path'):
                with self.assertRaises(ValueError): m.prepare(output)
            external.write_bytes(raw)
            command = [sys.executable, '-S', '-B', str(CASES / 'warm_read_catalogs.py'), '--output-dir', str(base / 'cli'), '--catalog-file', str(external)]
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr); self.assertEqual(json.loads(result.stdout), {'prepared': 2})
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)
            self.assertNotEqual(subprocess.run(command + ['--unexpected'], capture_output=True, timeout=60).returncode, 0)


    @unittest.skipUnless(sys.platform.startswith('linux') and os.environ.get('ZKGAS_FIXTURE_BIN'), 'set ZKGAS_FIXTURE_BIN for frozen Native3')
    def test_native_exact_three_specimens(self):
        from datetime import datetime, timezone
        import shutil
        import time
        m = producer(); from warm_read_catalogs_vm import run_users, execute
        from ethereum_hash import keccak256
        from test_storage_access_growth import rlp_decode, trie_value
        sys.path.insert(0, str(CASES.parent)); import process_guard
        from artifacts import identity, read_json
        binary = Path(os.environ['ZKGAS_FIXTURE_BIN']); build = binary.with_name('native-build.json')
        native_sha = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'
        build_sha = '1e1a7f4dbd141ce97359114b737d744ee71eb37b78e974038179eedf25cd8023'
        self.assertEqual(identity(binary)[0], native_sha); self.assertEqual(identity(build)[0], build_sha)
        provenance = read_json(build); declared = provenance['source_file_sha256']; workspace = CASES.parents[3]
        self.assertEqual((len(declared), sum(n.endswith('.rs') for n in declared), provenance['compiled_workspace_input_count']), (81, 66, 70))
        for name, expected in declared.items(): self.assertEqual(identity(workspace / name)[0], expected, name)
        base = Path(os.environ['WARM_READ_CATALOGS_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        limits = dict(pss_mib=6144, timeout_seconds=180, min_free_mib=576, output_mib=192, log_mib=16)
        summary = []
        for catalog, level, mutant in (('ordered', 'full', False), ('mixed', 'zero', False), ('ordered', 'full', True)):
            original = m.make_case(catalog, level); qualify(self, original)
            original_request = m.encoded(original['request']); original_row = m.encoded(original)
            row = intent_mutant(original) if mutant else original; mutation_preflight = None
            if mutant:
                with self.assertRaises(AssertionError) as rejected: run_users(row)
                program = row['request']['program']
                result = execute(bytes.fromhex(program['code_hex'][2:]), bytes.fromhex(program['input_hex'][2:]), row['parameters']['transaction_gas_limit'], repetitions=row['parameters']['repetitions'])
                self.assertEqual(result['gas'], original['parameters']['expected_user_charged_gas'])
                self.assertEqual(program['code_hex'], original['request']['program']['code_hex'])
                mutation_preflight = dict(original_intent_rejected=str(rejected.exception), original_intent_preserved=True,
                    swapped_calldata_word_indices=[0, 1], unchanged_query_code_gas_result=True, canonical_eligible=False)
            name = f'{catalog}-{level}-' + ('intent-mutant' if mutant else 'positive'); root = base / name; root.mkdir()
            source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            self.assertLessEqual(source.stat().st_size, 32 * 1024 * 1024)
            (root / 'requested-intent.json').write_bytes(original_request); (root / 'requested-intent-row.json').write_bytes(original_row)
            (root / 'row.json').write_bytes(m.encoded(row)); sources = root / 'sources'; sources.mkdir()
            source_paths = {**{n: Path(mod.__file__) for n, mod in m.source_modules().items()},
                'cases/warm_read_catalogs.py': Path(m.__file__), 'tests/test_warm_read_catalogs.py': Path(__file__),
                'tests/warm_read_catalogs_vm.py': Path(__file__).with_name('warm_read_catalogs_vm.py'),
                'tests/ethereum_hash.py': Path(__file__).with_name('ethereum_hash.py'),
                'tests/test_storage_access_growth.py': Path(__file__).with_name('test_storage_access_growth.py'),
                **{f'{n}.py': CASES.parent / f'{n}.py' for n in ('process_guard', 'artifacts', 'shared_memory')}}
            for relative, path in source_paths.items():
                target = sources / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(path.read_bytes())
            (root / 'native-build.json').write_bytes(build.read_bytes())
            growing = [root / n for n in ('input.json', 'input.bin', 'native.json')]
            command = [str(binary), '--input', str(source), '--output', str(growing[0]), '--output-bincode', str(growing[1]), '--report', str(growing[2]), '--execution-profile', 'evm-gas-only']
            free = shutil.disk_usage(root).free; self.assertGreaterEqual(free, 576 * 1024 * 1024)
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), limits=limits, native_sha256=native_sha,
                native_build_sha256=build_sha, request_sha256=identity(source)[0], row_sha256=identity(root / 'row.json')[0],
                requested_intent_request_sha256=hashlib.sha256(original_request).hexdigest(), requested_intent_row_sha256=hashlib.sha256(original_row).hexdigest(),
                specimen_role='native-accepted-catalog-order-intent-mutant' if mutant else 'original-positive', mutation_preflight=mutation_preflight,
                artifact_caps_bytes=dict(request=32 * 1024 * 1024, native_output=128 * 1024 * 1024, benchmark_input=64 * 1024 * 1024),
                disk_preflight=dict(free_bytes=free, required_free_bytes=576 * 1024 * 1024),
                native_declared_source_verification=dict(status='all81-match', declared_count=81, rust_count=66, compiled_workspace_input_count=70, source_file_sha256=declared),
                source_root='bin/zkgas/bench', source_sha256={n: identity(path)[0] for n, path in source_paths.items()},
                helper_source_sha256={n: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for n, fn in m.helpers().items()},
                outcome='not-launched', exit_code=None, elapsed_seconds=None,
                observability='Native gas/status/routes plus whole-input replay; no per-read key/value export.')
            manifest['prelaunch_files_sha256'] = {str(f.relative_to(root)): identity(f)[0] for f in root.rglob('*') if f.is_file()}
            (root / 'process.json').write_bytes(m.encoded(manifest)); start = time.monotonic()
            try:
                outcome = process_guard.run(command, root, 'native', limits, growing=growing)
                manifest.update(exit_code=outcome['exit_code'], outcome='success')
            except BaseException as error:
                retained = read_json(root / 'native-outcome.json') if (root / 'native-outcome.json').is_file() else {}
                manifest.update(exit_code=retained.get('exit_code'), outcome='failed', error=f'{type(error).__name__}: {error}')
                raise
            finally:
                manifest['elapsed_seconds'] = time.monotonic() - start
                manifest['files_sha256'] = {str(f.relative_to(root)): identity(f)[0] for f in root.rglob('*') if f.is_file() and f.name != 'process.json'}
                (root / 'process.json').write_bytes(m.encoded(manifest))
            identity(growing[0], limit=128 * 1024 * 1024); identity(growing[1], limit=64 * 1024 * 1024)
            report = read_json(growing[2], limit=128 * 1024 * 1024); receipts = report['receipts_by_block'][0]
            self.assertEqual(report['guest_input_sha256'], identity(growing[1])[0]); self.assertEqual(report['guest_input_bincode_length'], growing[1].stat().st_size)
            self.assertEqual(report['construction_request_sha256'], manifest['request_sha256']); self.assertEqual(report['execution_profile'], 'evm-gas-only')
            self.assertTrue(report['public_output']); self.assertTrue(report['public_values'])
            self.assertEqual([r['success'] for r in receipts], [True, True]); self.assertEqual([r['gas_used'] for r in receipts], [167355, row['parameters']['expected_user_charged_gas']])
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            # Authenticate the default primary through the actual parent MPT root.
            # configured prestate_contracts/initialized_storage may be empty; those
            # lists do not establish the absence or storage root of the primary.
            guest = read_json(growing[0], limit=128 * 1024 * 1024); witness = guest['witnesses'][0]
            nodes = {keccak256(bytes.fromhex(raw[2:])): bytes.fromhex(raw[2:]) for raw in guest['proposal_state_nodes']}
            parent_root = bytes.fromhex(guest['proposal_ancestor_headers'][-1]['header']['stateRoot'][2:])
            account = rlp_decode(trie_value(parent_root, keccak256(bytes.fromhex('66' * 20)), nodes))
            self.assertEqual(account[2].hex(), '56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421')
            self.assertEqual(account[3], keccak256(bytes.fromhex(row['request']['program']['code_hex'][2:])))
            self.assertIn(row['request']['program']['code_hex'], witness['witness']['codes'])
            txs = witness['block']['body']['transactions']; self.assertEqual(len(txs), 2); tx = txs[1]['transaction']['Eip1559']
            self.assertEqual((tx['nonce'], tx['to'], tx['input'], tx['gas_limit']), (0, '0x' + '66' * 20, row['request']['program']['input_hex'], row['parameters']['transaction_gas_limit']))
            manifest.update(primary_empty_storage_root_authenticated=True, primary_code_hash_authenticated=True, primary_nonce=0,
                public_output_sha256=hashlib.sha256(m.encoded(report['public_output'])).hexdigest(), public_values_sha256=hashlib.sha256(m.encoded(report['public_values'])).hexdigest())
            (root / 'process.json').write_bytes(m.encoded(manifest))
            summary.append(dict(specimen=name, process_sha256=identity(root / 'process.json')[0], request_sha256=manifest['request_sha256'], row_sha256=manifest['row_sha256'],
                input_sha256=identity(growing[1])[0], input_json_sha256=identity(growing[0])[0], report_sha256=identity(growing[2])[0],
                bridge_eligible=not mutant, user_gas=receipts[1]['gas_used'], outcome='success'))
        (base / 'summary.json').write_bytes(m.encoded(dict(specimens=summary, invocations=3, positive_bridges=2, mutants_excluded=1)))


def intent_mutant(row):
    """Swap only first two calldata keys, keeping original catalog/order intent."""
    result = copy.deepcopy(row); program = result['request']['program']; data = bytes.fromhex(program['input_hex'][2:])
    program['input_hex'] = '0x' + (data[32:64] + data[:32] + data[64:]).hex()
    return result


if __name__ == '__main__': unittest.main()
