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
    @staticmethod
    def capacity_keys(count):
        # Deterministic raw values, no hashing/search against a guest table.
        keys = []; low = 0x01010101
        while len(keys) < count - 1:
            if all(low.to_bytes(4, 'big')):
                keys.append(f'0x{(0x11223344 << 224) | low:064x}')
            low += 1
        return keys + [f'0x{QUERY:064x}']

    def test_capacity_boundaries_and_independent_byte_execution(self):
        m = producer()
        self.assertIn('key_count', inspect.signature(m.make_probe_case).parameters)
        from warm_read_catalogs_vm import execute, run_users
        for k in (1, 448, 449, 896, 897, 1024):
            keys = self.capacity_keys(k)
            zero = m.make_custom_case(keys, 'zero', key_count=k)
            row = m.make_probe_case(keys, 512, key_count=k)
            self.assertEqual(row['parameters']['key_count'], k)
            self.assertIn(f'-k{k}-', row['sample_id'])
            self.assertEqual(row['baseline_sample_id'], zero['sample_id'])
            self.assertEqual(row['parameters']['transaction_gas_limit'], zero['parameters']['transaction_gas_limit'])
            for candidate, r in ((zero, 0), (row, 512)):
                p = candidate['parameters']; result = run_users(candidate)[0]
                self.assertEqual((result['cold_reads'], result['warm_reads']), (k, 64*r+1))
                self.assertEqual(result['seed_keys'], keys)
                self.assertEqual(result['gas'], p['expected_user_charged_gas'])
                self.assertEqual(result['counts'], +Counter(candidate['request']['program']['expected_route_counts']))
                self.assertEqual(p['outer_calldata_bytes'], 32*k)
                self.assertEqual(p['gas_accounting_per_user']['intrinsic_gas'], 21000+224*k)
                self.assertEqual(p['gas_accounting_per_user']['eip7623_floor_gas'], 21000+560*k)
                self.assertEqual(p['expected_persistent_storage_reads'], k+1+64*r)
                self.assertEqual(result['gas'], 21359 + 2416*k + 6775*r)
            small = m.make_probe_case(keys, 2, key_count=k); program = small['request']['program']
            args = (bytes.fromhex(program['code_hex'][2:]), bytes.fromhex(program['input_hex'][2:]), small['parameters']['transaction_gas_limit'])
            self.assertEqual(execute(*args, repetitions=2, key_count=k, cache=False), execute(*args, repetitions=2, key_count=k))
            self.assertEqual(m.make_case('mixed', 'zero', key_count=k)['parameters']['accessed_unique'], k)

    def test_capacity_invalid_counts_export_cli_and_metadata_tampering(self):
        m = producer()
        self.assertIn('key_count', inspect.signature(m.prepare).parameters)
        from warm_read_catalogs_vm import run_users
        keys = self.capacity_keys(449)
        for k in (True, False, None, 1.0, '449', 0, -1, 1025):
            with self.subTest(k=k), self.assertRaises(ValueError): m.make_probe_case(keys, 2, key_count=k)
            with self.subTest(gas_k=k), self.assertRaises(ValueError): m.gas_accounting(2, key_count=k)
        with self.assertRaises(ValueError): m.make_custom_case(keys, key_count=448)
        row = m.make_probe_case(keys, 2, key_count=449)
        for field, value in (('key_count', 448), ('key_count', True), ('seed_reads', 448),
                             ('accessed_unique_keys', 448), ('outer_calldata_bytes', 14336),
                             ('expected_user_charged_gas', 1), ('expected_persistent_storage_reads', 1)):
            bad = copy.deepcopy(row); bad['parameters'][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row); del bad['parameters']['key_count']
        with self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row); bad['parameters']['gas_accounting_per_user']['cold_sload_count'] = 448
        with self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row); bad['request']['program']['input_hex'] = bad['request']['program']['input_hex'][:-64]
        with self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row); bad['parameters']['gas_accounting_per_user']['intrinsic_gas'] = float(bad['parameters']['gas_accounting_per_user']['intrinsic_gas'])
        with self.subTest(gas_type='float'), self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row); bad['parameters']['initial_state_size'] = 1
        with self.subTest(initial_state_size=1), self.assertRaises(AssertionError): run_users(bad)
        bad = copy.deepcopy(row)
        bad['request']['program']['code_hex'] = m.make_probe_case(self.capacity_keys(448), 2, key_count=448)['request']['program']['code_hex']
        bad['parameters']['runtime_sha256'] = m.digest(bytes.fromhex(bad['request']['program']['code_hex'][2:]))
        with self.assertRaises(AssertionError): run_users(bad)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp); external = base/'keys.json'
            external.write_bytes(m.encoded(dict(schema_version=1, keys=keys)))
            for k in (True, 1.0, 0, 1025, 448):
                with self.subTest(export_k=k), self.assertRaises(ValueError):
                    m.prepare(base/'bad', catalog_file=external, key_count=k, probe_repetitions=512)
                self.assertFalse((base/'bad').exists())
            target = base/'out'
            self.assertEqual(m.prepare(target, catalog_file=external, key_count=449, probe_repetitions=512), 3)
            with self.assertRaises(FileExistsError): m.prepare(target, catalog_file=external, key_count=449)
            rows = json.loads((target/'catalog.json').read_bytes())['cases']
            self.assertEqual([r['parameters']['key_count'] for r in rows], [449]*3)
            command = [sys.executable, '-S', '-B', str(CASES/'warm_read_catalogs.py'), '--output-dir', str(base/'cli'), '--catalog-file', str(external), '--key-count', '449', '--probe-repetitions', '512']
            completed = subprocess.run(command, capture_output=True, timeout=60)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(json.loads(completed.stdout), {'prepared': 3})
            command[command.index('--output-dir')+1] = str(base/'cli-bad')
            command[command.index('--key-count')+1] = '448'
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=60).returncode, 0)
            self.assertFalse((base/'cli-bad').exists())

    def test_custom_short_comparison_prefix_preserves_runtime_and_evm_work(self):
        m = producer()
        original_keys = m._catalog('mixed')
        keys = [f'0x{(int(k, 16) >> 192):064x}' for k in original_keys[:-1]] + [f'0x{QUERY:064x}']
        row = m.make_probe_case(keys, 2)
        old = m.make_probe_case(original_keys, 2)
        self.assertEqual(row['request']['program']['code_hex'], old['request']['program']['code_hex'])
        self.assertEqual(row['parameters']['gas_accounting_per_user'], old['parameters']['gas_accounting_per_user'])
        from warm_read_catalogs_vm import run_users
        executed = run_users(row)[0]
        self.assertEqual(executed['seed_keys'], keys)
        self.assertEqual((executed['cold_reads'], executed['warm_reads']), (512, 129))

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
        request_hashes = {
            ('ordered', 'zero'): '56328b2b146535a8ac1b3f4b052e422b610265271f5f3edb49854241738add74',
            ('ordered', 'full'): '00bf96ae968cb34900d2bb602989d89c3d732c8a47da7fd4a993f6560c9544fb',
            ('mixed', 'zero'): 'e82617cfcb428b07687740443482f7275b5ab9c8af544c25816e4fed61101bf7',
            ('mixed', 'full'): '0cc8a5de1d00fa5845e76c84537eaca02ee43a4b6b6f5a3cb28f1e09cc854283',
        }
        for (catalog, level), sha in frozen.items():
            row = m.make_case(catalog, level)
            self.assertEqual(m.digest(m.encoded(row)), sha)
            self.assertEqual(m.digest(m.encoded(row['request'])), request_hashes[catalog, level])
            self.assertEqual(row, m.make_case(catalog, level, key_count=512))
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


class StructuredWarmTailTests(unittest.TestCase):
    def test_tiny_structured_literal_bytes_and_direct_cache_parity(self):
        m = producer()
        from warm_read_catalogs_vm import run_users, execute
        for k in (1, 2, 3, 16, 256, 257):
            for r in (0, 1, 2):
                for word in (0, 1, (1 << 64) - 1):
                    with self.subTest(k=k, r=r, word=word):
                        row = m.make_structured_case(raw_key_layout={'L1': word, 'L3': 0}, key_count=k, repetitions=r)
                        p = row['parameters']; program = row['request']['program']
                        data = bytes.fromhex(program['input_hex'][2:]); code = bytes.fromhex(program['code_hex'][2:])
                        keys = [(i << 24) | (word << 64) for i in range(1, k + 1)]
                        self.assertEqual(data, b''.join(v.to_bytes(32, 'big') for v in keys))
                        self.assertEqual(p['query_key'], f'0x{keys[-1]:064x}')
                        self.assertEqual(program['block_count'], 2)
                        self.assertEqual(program['transactions'], dict(kind='call', counts_by_block=[0, 1], gas_limit=p['transaction_gas_limit']))
                        self.assertNotIn('expected_anchor_gas', p); self.assertNotIn('expected_block_charged_gas', p)
                        result = run_users(row, structured=True)[0]
                        args = (code, data, p['transaction_gas_limit'])
                        direct = execute(*args, repetitions=r, key_count=k, query=keys[-1], structured=True, cache=False)
                        self.assertEqual(result, direct)
                        self.assertEqual((result['cold_reads'], result['warm_reads']), (k, 64 * r + 1))
                        self.assertEqual(result['stack'], []); self.assertEqual(result['peak_memory_bytes'], 128)
                        z = data.count(0); nz = len(data) - z
                        gas = p['gas_accounting_per_user']
                        self.assertEqual(gas['intrinsic_gas'], 21000 + 4*z + 16*nz)
                        self.assertEqual(gas['eip7623_floor_gas'], 21000 + 10*(z + 4*nz))
                        self.assertEqual(p['transaction_gas_limit'], result['gas'] + 1000)
                        self.assertEqual(p['source_proposal_charged_gas_upper_bound'], 2000000 + max(result['gas'], gas['eip7623_floor_gas']))
                        self.assertEqual(p['source_proposal_envelope_upper_bound'], 2000000 + p['transaction_gas_limit'])

    def test_core_query_is_explicit_and_legacy_is_strict(self):
        m = producer()
        from warm_read_catalogs_vm import decode, structure
        code, _ = m._runtime_core(2, key_count=3, query=3 << 24)
        structure(list(decode(code)), 2, key_count=3, query=3 << 24)
        with self.assertRaises(AssertionError): structure(list(decode(code)), 2, key_count=3)
        for k in (1025, 4096, True):
            with self.assertRaises(ValueError): m._runtime(0, key_count=k)
        for query in (-1, 1 << 256, True, 1.0):
            with self.assertRaises(ValueError): m._runtime_core(0, key_count=1, query=query)

    def test_structured_factory_does_not_search_or_relabel_legacy_rows(self):
        m = producer()
        with patch.object(m, 'full_repetitions', side_effect=AssertionError('legacy R search')), \
             patch.object(m, '_make_case', side_effect=AssertionError('legacy row rendering')):
            row = m.make_structured_case(raw_key_layout={'L1': 0, 'L3': 0}, key_count=2, repetitions=1)
        self.assertEqual(row['request']['program']['transactions']['counts_by_block'], [0, 1])
        self.assertNotIn('full_repetitions', row['parameters'])

    def test_strict_structured_inputs_reject_before_request(self):
        m = producer()
        with patch.object(m.original, 'make_case', side_effect=AssertionError('request constructed')):
            for layout in (None, [], {}, {'L1': 0}, {'L1': 0, 'L3': 0, 'shift': 24},
                           {'L1': True, 'L3': 0}, {'L1': -1, 'L3': 0}, {'L1': 1 << 64, 'L3': 0},
                           {'L1': 1.0, 'L3': 0}, {'L1': 0, 'L3': True}, {'L1': 0, 'L3': 1}):
                with self.subTest(layout=layout), self.assertRaises(ValueError):
                    m.make_structured_case(raw_key_layout=layout, key_count=2, repetitions=1)
            for k in (0, 4097, True, 1.0):
                with self.subTest(k=k), self.assertRaises(ValueError):
                    m.make_structured_case(raw_key_layout={'L1': 0, 'L3': 0}, key_count=k, repetitions=0)
            for r in (-1, True, 1.0, m.MAX_REPETITIONS + 1, m.MAX_REPETITIONS):
                with self.subTest(r=r), self.assertRaises(ValueError):
                    m.make_structured_case(raw_key_layout={'L1': 0, 'L3': 0}, key_count=1, repetitions=r)

    def test_structured_mutants_and_legacy_oracle_do_not_accept_new_mode(self):
        m = producer(); from warm_read_catalogs_vm import run_users, decode
        row = m.make_structured_case(raw_key_layout={'L1': 1, 'L3': 0}, key_count=3, repetitions=2)
        with self.assertRaises(AssertionError): run_users(row)
        for field, value in [('query_key', '0x'+'00'*32), ('calldata_zero_bytes', 0), ('expected_user_charged_gas', 1),
                             ('source_proposal_charged_gas_upper_bound', 0), ('block_count', 1), ('sentinel', 0), ('counter_memory_offset', 0)]:
            bad = copy.deepcopy(row); bad['parameters'][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError): run_users(bad, structured=True)
        for mode in ('reorder', 'duplicate', 'short', 'schedule', 'bool-schedule', 'query-code', 'sentinel-code', 'first-seed-code'):
            bad = copy.deepcopy(row); program = bad['request']['program']; data = bytes.fromhex(program['input_hex'][2:]); code = bytearray.fromhex(program['code_hex'][2:])
            if mode == 'reorder': data = data[32:64] + data[:32] + data[64:]
            elif mode == 'duplicate': data = data[:32] + data[:32] + data[64:]
            elif mode == 'short': data = data[:-1]
            elif mode == 'schedule': program['transactions']['counts_by_block'] = [1, 0]
            elif mode == 'bool-schedule': program['transactions']['counts_by_block'] = [False, True]
            else:
                if mode == 'query-code': at = next(at for at, op, value in decode(code) if op == 0x7f and value == (3<<24)|(1<<64))
                elif mode == 'sentinel-code': at = next(at for at, op, value in decode(code) if value == 0xa55a)
                else: at = next(at for at, op, value in decode(code) if op == 0x54); code[at] = 0x50; at = None
                if at is not None: code[at + 1] ^= 1
                program['code_hex'] = '0x' + code.hex(); bad['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            program['input_hex'] = '0x' + data.hex()
            with self.subTest(mode=mode), self.assertRaises(AssertionError): run_users(bad, structured=True)

    def test_formal_geometry_scalar_only(self):
        # No keyset, runtime or row is emitted at K4096/R512.
        k = 4096; r = 512; ordinary = 569079; nz_ordinal = 7921
        def gas(nz):
            z = 32*k - nz
            intrinsic = 21000 + 4*z + 16*nz; floor = 21000 + 10*(z + 4*nz)
            gross = intrinsic + ordinary + 12 + 2100*k + 100*(64*r + 1)
            return max(gross, floor)
        self.assertEqual(gas(nz_ordinal), 13087931)
        self.assertEqual(gas(nz_ordinal + 8*k), 13481147)
        self.assertEqual(2000000 + gas(nz_ordinal + 8*k) + 1000, 15482147)
        self.assertEqual((k + 7*16 + 7)//8, 526)


if __name__ == '__main__': unittest.main()
