"""Account20 catalog contract: bytes, independent execution, bounds and immutable exports."""
from collections import Counter
import copy
import hashlib
import importlib.util
import inspect
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'
QUERY = '0x1122334400000000000000000000000055667788'
MODEL = '5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11'


def producer():
    path = CASES / 'account_read_catalogs.py'
    assert path.is_file(), 'account-read producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('account_read_catalogs', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from account_read_catalogs_vm import run_users
    result = run_users(row)[0]; p = row['parameters']
    test.assertEqual(result['counts'], +Counter(row['request']['program']['expected_route_counts']))
    test.assertEqual(result['gas'], p['expected_user_charged_gas'])
    test.assertEqual(result['seed_addresses'], p['ordered_addresses'])
    test.assertEqual((result['cold_reads'], result['warm_reads']), (512, 64 * p['repetitions'] + 1))
    test.assertEqual((result['stack'], result['counter'], result['peak_memory_bytes']), ([], p['repetitions'], 128))
    return result


class AccountReadCatalogTests(unittest.TestCase):
    def test_default_bytes_account_domain_and_literal_gas(self):
        m = producer()
        for operation in ('balance', 'extcodesize'):
            for level, r, user_gas in [('zero', 0, 1514351), ('probe', 512, 4983151)]:
                row = m.make_case(operation, level); p = row['parameters']; program = row['request']['program']
                data = bytes.fromhex(program['input_hex'][2:]); addresses = p['ordered_addresses']
                self.assertEqual((len(addresses), len(set(addresses)), addresses[-1]), (512, 512, QUERY))
                self.assertTrue(all(len(a) == 42 and a.startswith('0x11223344' + '00' * 12) for a in addresses))
                self.assertTrue(all(sum(b != 0 for b in bytes.fromhex(a[2:])) == 8 for a in addresses))
                self.assertEqual(data, b''.join(bytes(12) + bytes.fromhex(a[2:]) for a in addresses))
                self.assertEqual((len(data), data.count(0)), (16384, 12288))
                self.assertEqual(p['repetitions'], r); self.assertEqual(p['expected_user_charged_gas'], user_gas)
                self.assertEqual(p['expected_block_charged_gas'], user_gas + 167355)
                self.assertEqual(p['gas_accounting_per_user']['intrinsic_gas'], 135688)
                self.assertEqual(p['gas_accounting_per_user']['eip7623_floor_gas'], 307720)
                self.assertEqual(p['gas_accounting_per_user']['cold_account_gas'], 512 * 2600)
                self.assertEqual(p['transaction_gas_limit'], 14997601)
                self.assertEqual(m.full_repetitions(operation), 1990)
                self.assertFalse(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertEqual(row['data_role'], 'development_probe')
                self.assertNotIn('prestate_contracts', program); self.assertNotIn('storage_state', program)
                self.assertNotIn('access_list', program); self.assertNotIn('authorizations', program)
                qualify(self, row)

    def test_custom_short_prefix_same_runtime_and_actual_abi_targets(self):
        m = producer(); original = m.make_case(); addresses = original['parameters']['ordered_addresses']
        short = ['0x' + '00' * 12 + a[2:10] + a[-8:] for a in addresses[:-1]] + [QUERY]
        row = m.make_case(keys=short); qualify(self, row)
        self.assertEqual(row['request']['program']['code_hex'], original['request']['program']['code_hex'])
        self.assertEqual(row['parameters']['gas_accounting_per_user'], original['parameters']['gas_accounting_per_user'])
        self.assertNotEqual(row['request']['program']['input_hex'], original['request']['program']['input_hex'])
        self.assertNotEqual(row['sample_id'], original['sample_id'])
        self.assertTrue(row['sample_id'].startswith('account-read-balance-'))

    def test_raw_byte_oracle_rejects_mutations_and_nonzero_feedback(self):
        m = producer(); from account_read_catalogs_vm import execute, run_users
        for op in ('balance', 'extcodesize'):
            row = m.make_case(op, repetitions=2); qualify(self, row)
            code = bytes.fromhex(row['request']['program']['code_hex'][2:]); data = bytes.fromhex(row['request']['program']['input_hex'][2:])
            for broken in [code[:-1], code + b'\x00', code.replace(bytes([0x31 if op == 'balance' else 0x3b]), b'\x54', 1)]:
                with self.assertRaises(AssertionError): execute(broken, data, 15000000, operation=op, repetitions=2)
            with self.assertRaises(AssertionError): execute(code, data, 15000000, operation=op, repetitions=2, account_values={int(QUERY, 16): 1})
            for field in ('prestate_contracts', 'storage_state', 'access_list', 'authorizations'):
                bad = copy.deepcopy(row); bad['request']['program'][field] = []
                with self.assertRaises(AssertionError): run_users(bad)
            for field, value in [('target_balance', 1), ('target_code_bytes', 1), ('read_events', 1), ('expected_block_charged_gas', 1), ('calldata_zero_bytes', 1)]:
                bad = copy.deepcopy(row); bad['parameters'][field] = value
                with self.assertRaises(AssertionError): run_users(bad)
            bad = copy.deepcopy(row); bad['parameters']['ordered_addresses'][:2] = reversed(bad['parameters']['ordered_addresses'][:2])
            with self.assertRaises(AssertionError): run_users(bad)
            for changed in [bytes([1]) + data[1:], data[:-1], data[:32] + data[64:]]:
                with self.assertRaises(AssertionError): execute(code, changed, 15000000, operation=op, repetitions=2)

    def test_input_validation_cannot_confuse_u256_and_address20(self):
        m = producer(); keys = m.make_case()['parameters']['ordered_addresses']
        variants = [True, iter(keys), keys[:-1], keys + keys[:1], [True] + keys[1:], [keys[0].upper()] + keys[1:], [keys[1]] + keys[1:], keys[::-1], ['0x' + '00' * 12 + keys[0][2:]] + keys[1:], ['0x' + '66' * 20] + keys[1:], ['0x' + '00' * 19 + '01'] + keys[1:]]
        for bad in variants:
            with self.assertRaises(ValueError): m.make_case(keys=bad)
        for op in (True, None, 'sload', 'size'):
            with self.assertRaises(ValueError): m.make_case(op)
        for level in (True, None, 'full'):
            with self.assertRaises(ValueError): m.make_case(level=level)
        for r in (True, -1, 0, 1.0, None, 1990, 1000001):
            with self.assertRaises(ValueError): m.make_case(repetitions=r)
        for r in (1, 2, 1989): qualify(self, m.make_case(repetitions=r))

    def test_prepare_pair_provenance_and_no_overwrite(self):
        m = producer(); self.assertEqual(m.MODEL, MODEL)
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp); external = base/'keys.json'
            raw = m.encoded({'schema_version': 1, 'keys': m.make_case()['parameters']['ordered_addresses']}); external.write_bytes(raw)
            for custom in (False, True):
                output = base/('custom' if custom else 'default')
                self.assertEqual(m.prepare(output, catalog_file=external if custom else None), 2)
                catalog = json.loads((output/'catalog.json').read_bytes())
                self.assertEqual(catalog['frozen_model_id'], MODEL); self.assertEqual(catalog['levels'], ['zero', 'probe'])
                self.assertFalse(catalog['holdout_only']); self.assertFalse(catalog['actuals_used_for_fitting'])
                self.assertNotIn(str(base), json.dumps(catalog))
                for name, module in m.source_modules().items(): self.assertEqual(catalog['source_sha256'][name], hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest())
                for name, fn in m.helpers().items(): self.assertEqual(catalog['helper_source_sha256'][name], hashlib.sha256(inspect.getsource(fn).encode()).hexdigest())
                if custom: self.assertEqual(catalog['external_catalog_json_sha256'], hashlib.sha256(raw).hexdigest())
                self.assertEqual(len({r['sample_id'] for r in catalog['cases']}), 2)
                for row in catalog['cases']:
                    request_raw = (output/row['request_path']).read_bytes()
                    self.assertEqual(hashlib.sha256(request_raw).hexdigest(), row['request_sha256'])
                    qualified = row | {'request': json.loads(request_raw)}; qualify(self, qualified)
                with self.assertRaises(FileExistsError): m.prepare(output)
            for model in (True, None, 'A'*64, 'a'*63):
                with self.assertRaises(ValueError): m.prepare(base/'bad', model_id=model)
                self.assertFalse((base/'bad').exists())
            for value in [{'schema_version': True, 'keys': []}, {'schema_version': 1, 'keys': [], 'extra': 0}]:
                external.write_text(json.dumps(value))
                with self.assertRaises(ValueError): m.prepare(base/'bad', catalog_file=external)
                self.assertFalse((base/'bad').exists())
            external.write_bytes(b'{"schema_version":1,"schema_version":1,"keys":[]}')
            with self.assertRaises(ValueError): m.prepare(base/'duplicate-json', catalog_file=external)
            self.assertFalse((base/'duplicate-json').exists())
            for invalid in [0, True, -1, 1990, 1.0]:
                with self.assertRaises(ValueError): m.prepare(base/'bad-repetitions', probe_repetitions=invalid)
                self.assertFalse((base/'bad-repetitions').exists())
            for invalid in [None, True, 'path']:
                with self.assertRaises(ValueError): m.prepare(invalid)
            external.write_bytes(b' ' * (128*1024+1))
            with patch.object(m.json, 'loads', side_effect=AssertionError('must reject size before JSON parse')):
                with self.assertRaises(ValueError): m.prepare(base/'bad', catalog_file=external)
            external.write_bytes(raw)
            command=[sys.executable,'-S','-B',str(CASES/'account_read_catalogs.py'),'--output-dir',str(base/'cli'),'--catalog-file',str(external),'--probe-repetitions','2']
            result=subprocess.run(command,capture_output=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr); self.assertEqual(json.loads(result.stdout),{'prepared':2})
            self.assertNotEqual(subprocess.run(command,capture_output=True,timeout=30).returncode,0)

class SizedAccountReadCatalogTests(unittest.TestCase):
    @staticmethod
    def keys(n):
        return ['0x' + 'ab' * 16 + i.to_bytes(4, 'big').hex() for i in range(1, n + 1)]

    def test_legacy_complete_rows_and_requests_stay_byte_exact(self):
        m = producer(); keys = m._catalog()
        short = ['0x' + '00' * 12 + a[2:10] + a[-8:] for a in keys[:-1]] + [QUERY]
        digest = hashlib.sha256()
        for operation in ('balance', 'extcodesize'):
            for catalog in (None, list(keys), short):
                for level, values in [('zero', (0, 512, 1989)), ('probe', (1, 2, 512, 1400, 1989))]:
                    for r in values:
                        row = m.make_case(operation, level, keys=catalog, repetitions=r)
                        digest.update(m.encoded(row)); digest.update(m.encoded(row['request']))
        self.assertEqual(digest.hexdigest(), 'b099f48d100e617f07744b0c328b96d9d89763ae516087b721e1ec9d5a012858')

    def test_sized_actual_byte_gas_and_access_list_intent(self):
        m = producer(); self.assertTrue(callable(getattr(m, 'make_sized_case', None)), 'missing sized-case API')
        from account_read_catalogs_vm import run_users
        for operation in ('balance', 'extcodesize'):
            for n in (1, 32, 1024):
                for r in (0, 1, 1024):
                    for use_al in (False, True):
                        with self.subTest(operation=operation, n=n, r=r, access_list=use_al):
                            keys = self.keys(n); row = m.make_sized_case(keys, operation=operation, repetitions=r, access_list=use_al)
                            p = row['parameters']; program = row['request']['program']; result = run_users(row)[0]
                            data = bytes.fromhex(program['input_hex'][2:]); nonzero = sum(x != 0 for x in data)
                            intrinsic = 21000 + 4 * (len(data) - nonzero) + 16 * nonzero + (2400 * n if use_al else 0)
                            self.assertEqual(p['key_count'], n); self.assertEqual(p['query_address'], keys[-1])
                            self.assertEqual((p['transaction_gas_limit'], program['transactions']['gas_limit']), (15000000, 15000000))
                            self.assertEqual(p['gas_accounting_per_user']['intrinsic_gas'], intrinsic)
                            self.assertEqual(result['eip7623_floor_gas'], 21000 + 10 * (len(data) + 3 * nonzero))
                            self.assertEqual(result['seed_addresses'], keys)
                            self.assertEqual((result['cold_reads'], result['warm_reads']), (0 if use_al else n, 64 * r + 1 + (n if use_al else 0)))
                            self.assertEqual((result['counter'], result['stack'], result['peak_memory_bytes']), (r, [], 128))
                            self.assertEqual(result['counts'], +Counter(program['expected_route_counts']))
                            self.assertEqual(result['gas'], p['expected_user_charged_gas'])
                            self.assertEqual(p['expected_block_charged_gas'], result['gas'] + 167355)
                            self.assertFalse(row['actuals_used_for_fitting']); self.assertFalse(row['holdout_only'])
                            self.assertNotIn('prestate_contracts', program); self.assertNotIn('storage_state', program)
                            if use_al: self.assertEqual(program['access_list'], dict(pattern='raw_addresses', count=n, addresses=keys))
                            else: self.assertNotIn('access_list', program)
        keys = self.keys(32); row = m.make_sized_case(keys, repetitions=1, access_list=True)
        keys[0] = keys[-1]
        self.assertNotEqual(keys, row['parameters']['ordered_addresses'])
        row['request']['program']['access_list']['addresses'][0] = keys[-1]
        self.assertNotEqual(row['request']['program']['access_list']['addresses'], row['parameters']['ordered_addresses'])

    def test_sized_domains_and_budget_fail_before_export(self):
        m = producer(); self.assertTrue(callable(getattr(m, 'make_sized_case', None)), 'missing sized-case API')
        keys = self.keys(2)
        for bad in (None, True, iter(keys), {}, [], self.keys(1025), [keys[0], keys[0]], [1], ['0x' + '00' * 19 + '01'], ['0x' + '66' * 20], [keys[0].upper()], [keys[0][:-2]], [keys[0] + '00']):
            with self.subTest(keys=type(bad).__name__):
                with self.assertRaises(ValueError): m.make_sized_case(bad)
        for bad in (True, -1, 1.0, None, 1000001):
            with self.assertRaises(ValueError): m.make_sized_case(keys, repetitions=bad)
        for bad in (True, None, 'sload'):
            with self.assertRaises(ValueError): m.make_sized_case(keys, operation=bad)
        for bad in (1, None, 'yes', 0):
            with self.assertRaises(ValueError): m.make_sized_case(keys, access_list=bad)
        with self.assertRaises(ValueError): m.make_sized_case(self.keys(1024), repetitions=1000000)
        for bad in (0, 1025, True, 1.0):
            with self.assertRaises(ValueError): m._runtime('balance', 1, key_count=bad)

    def test_sized_exact_budget_boundary_and_typed_counters(self):
        m = producer(); from account_read_catalogs_vm import run_users
        for n in (1, 1024):
            for use_al in (False, True):
                keys = self.keys(n); data = b''.join(bytes(12) + bytes.fromhex(a[2:]) for a in keys)
                intrinsic = 21000 + 4 * data.count(0) + 16 * (len(data) - data.count(0))
                base = intrinsic + (2592 if use_al else 2692) * n + 359
                maximum = (15000000 - base) // 6775
                row = m.make_sized_case(keys, repetitions=maximum, access_list=use_al)
                result = run_users(row)[0]
                self.assertEqual(result['gross_gas'], base + 6775 * maximum)
                with self.assertRaises(ValueError): m.make_sized_case(keys, repetitions=maximum + 1, access_list=use_al)
        row = m.make_sized_case(self.keys(1), repetitions=0)
        for field in ('accessed_unique_addresses', 'seed_reads'):
            for value in (True, 1.0):
                bad = copy.deepcopy(row); bad['parameters'][field] = value
                with self.assertRaises(AssertionError): run_users(bad)

    def test_sized_oracle_rejects_rehashed_bytes_metadata_and_al_mutants(self):
        m = producer(); self.assertTrue(callable(getattr(m, 'make_sized_case', None)), 'missing sized-case API')
        from account_read_catalogs_vm import run_users
        for use_al in (False, True):
            row = m.make_sized_case(self.keys(32), repetitions=1, access_list=use_al); run_users(row)
            for field, value in [('key_count', 1), ('query_address', self.keys(1)[0]), ('read_events', 1), ('expected_user_gross_gas', 1), ('sentinel', 0), ('access_list', not use_al), ('unroll_factor', 32), ('all_user_transactions', 2), ('header_gas_limit', 32000000), ('transaction_gas_limit', 15000000.0)]:
                bad = copy.deepcopy(row); bad['parameters'][field] = value
                with self.assertRaises(AssertionError): run_users(bad)
            bad = copy.deepcopy(row); bad['request']['program']['transactions']['gas_limit'] = 14999999
            with self.assertRaises(AssertionError): run_users(bad)
            bad = copy.deepcopy(row); data = bytearray.fromhex(bad['request']['program']['input_hex'][2:]); data[0] = 1
            bad['request']['program']['input_hex'] = '0x' + data.hex(); bad['parameters']['calldata_sha256'] = bad['parameters']['address_words_sha256'] = hashlib.sha256(data).hexdigest()
            with self.assertRaises(AssertionError): run_users(bad)
            bad = copy.deepcopy(row); code = bytes.fromhex(bad['request']['program']['code_hex'][2:]); marker = b'\x7f' + int(row['parameters']['query_address'], 16).to_bytes(32, 'big')
            self.assertIn(marker, code); code = code.replace(marker, b'\x7f' + int(self.keys(1)[0], 16).to_bytes(32, 'big'), 1)
            bad['request']['program']['code_hex'] = '0x' + code.hex(); bad['parameters']['runtime_sha256'] = hashlib.sha256(code).hexdigest()
            with self.assertRaises(AssertionError): run_users(bad)
            if use_al:
                for change in ('order', 'count', 'keys'):
                    bad = copy.deepcopy(row); al = bad['request']['program']['access_list']
                    if change == 'order': al['addresses'].reverse()
                    elif change == 'count': al['count'] += 1
                    else: al['storage_keys'] = []
                    with self.assertRaises(AssertionError): run_users(bad)


if __name__ == '__main__': unittest.main()
