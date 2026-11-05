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

if __name__ == '__main__': unittest.main()
