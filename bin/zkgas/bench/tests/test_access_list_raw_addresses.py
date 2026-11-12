"""Ordered raw owner vectors; no concentration or execution policy."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import sys
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'


def producer():
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('raw_owner_cases', CASES/'access_list_values.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RawAddressesTests(unittest.TestCase):
    def test_raw_zero_extreme_order_duplicates_and_content_identity(self):
        m = producer()
        self.assertTrue(hasattr(m, 'make_raw_addresses_case'), 'raw-address API is missing')
        addresses = ['0x'+'00'*20, '0x'+'ff'*20, '0x'+'00'*20, '0x'+'00'*19+'01']
        row = m.make_raw_addresses_case(addresses)
        raw = b''.join(bytes.fromhex(x[2:]) for x in addresses)
        expected_digest = hashlib.sha256(raw).hexdigest()
        self.assertEqual(row['request']['program']['access_list'],
                         dict(pattern='raw_addresses', count=4, addresses=addresses))
        p = row['parameters']
        self.assertEqual(p['expected_access_list_counts_per_block'], dict(
            raw_address_items=4, raw_storage_keys=0, unique_addresses=3, unique_storage_pairs=0))
        self.assertEqual(p['ordered_addresses_sha256'], expected_digest)
        self.assertEqual(p['address_encoding'], '20-byte concatenation in raw order')
        self.assertIn(expected_digest, row['sample_id'])
        self.assertIn(expected_digest, row['corpus_id'])
        self.assertNotIn('storage_keys', row['request']['program']['access_list'])
        self.assertEqual(row['request']['program']['code_hex'], '0x60006000f3')
        self.assertEqual(row['request']['program']['input_hex'], '0x')
        self.assertEqual(p['transaction_gas_limit'], 15_000_000)
        self.assertFalse(p['storage_preloads'])
        self.assertNotIn('prestate_contracts', row['request']['program'])
        frozen = copy.deepcopy(row)
        addresses.append('0x'+'01'*20)
        self.assertEqual(row, frozen)
        self.assertEqual(m.make_raw_addresses_case(tuple(addresses[:-1])), row)
        self.assertEqual(m.make_raw_addresses_case([a[:2]+a[2:].upper() for a in addresses[:-1]]), row)
        self.assertNotEqual(m.make_raw_addresses_case(list(reversed(addresses[:-1])))['sample_id'], row['sample_id'])

    def test_strict_vectors_bounds_and_block_count(self):
        m = producer()
        self.assertTrue(hasattr(m, 'make_raw_addresses_case'), 'raw-address API is missing')
        zero = '0x'+'00'*20
        for value in (None, {}, zero, set([zero]), [], [zero]*8193, [True], [0], [1.0],
                      [None], [bytes(20)], ['0x00'], ['00'*20], ['0X'+'00'*20],
                      ['0x'+'00'*21], ['0x'+'gg'*20], ['0x'+'00'*19+' 0']):
            with self.subTest(value_type=type(value).__name__), self.assertRaises(ValueError):
                m.make_raw_addresses_case(value)
        for count in (1, 8192):
            row = m.make_raw_addresses_case([zero]*count, 32)
            self.assertEqual(row['request']['program']['access_list']['count'], count)
            self.assertEqual(row['parameters']['expected_access_list_counts_per_block']['unique_addresses'], 1)
            self.assertEqual(row['request']['program']['block_count'], 32)
            self.assertIn('-b32', row['sample_id'])
        for blocks in (True, False, 0, 33, -1, 1.0, '1', None):
            with self.assertRaises(ValueError):
                m.make_raw_addresses_case([zero], blocks)
        with self.assertRaises(ValueError):
            m.make_case('raw_addresses', 1)


if __name__ == '__main__':
    unittest.main()
