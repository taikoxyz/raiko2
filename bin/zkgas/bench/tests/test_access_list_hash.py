"""Frozen generic U256 hash vectors; static evidence is separate from own profiles."""
import importlib
import importlib.util
from pathlib import Path
import sys
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))
# Expected hashes are from 275 exact b76 RV64 steps per vector, not this helper.
# Shared static oracle SHA: 6cd8557f2dd116a99fd9d6540c70bbb459c768756b6fd85100d06ce8ca17a9f2.
# These are unit vectors, not dynamic seed, placement or workload evidence.
VECTORS = [
    (0x0000000000000000000000000000000000000000000000000000000000000000, 0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0x0000000000000000),
    (0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff),
    (0x0123456789abcdef1032547698badcfe8899aabbccddeefffedcba9876543210, 0x1020304050607080, 0x8877665544332211, 0xfedcba9876543210, 0x5fad0b26086d4a18),
    (0x00000001aabbccdd0000000000001234d2b5ccfd88cdb855d2b5ccfd88cdb855, 0x123456789abcdef0, 0xd2b5ccfd88cdb855, 0x9988776655443322, 0x4ad733f62336e157),
    (0xabcdef01aabbccdd0000000000001234d2b5ccfd88cdb855d2b5ccfd88cdb855, 0x123456789abcdef0, 0xd2b5ccfd88cdb855, 0x9988776655443322, 0x4ad733f62336e157),
    (0xabcdef01aabbccdd0000000000000000d2b5ccfd88cdb8550000000000000000, 0x123456789abcdef0, 0xd2b5ccfd88cdb855, 0x9988776655443322, 0x76225b5becdbf4e9),
]


class AccessListHashTests(unittest.TestCase):
    def helper(self):
        self.assertIsNotNone(importlib.util.find_spec('access_list_hash'),
                             'generic warm-set hash helper is not implemented')
        return importlib.import_module('access_list_hash')

    def test_frozen_generic_hash_vectors(self):
        m = self.helper()
        for key, seed, g0, g1, expected in VECTORS:
            with self.subTest(key=hex(key)):
                self.assertEqual(m.hash_key(key, seed, g0, g1), expected)
        self.assertEqual(VECTORS[3][-1], VECTORS[4][-1])
        self.assertNotEqual(VECTORS[5][-1], VECTORS[3][-1])

    def test_catalog_has_unique_keys_and_exact_comparison_prefix(self):
        m = self.helper()
        for g0 in (0, (1 << 64) - 1, 0xd2b5ccfd88cdb855):
            for count in (1, 64, 8192):
                keys = m.concentrated_keys(g0, count)
                self.assertEqual(len(keys), count)
                self.assertEqual(len(set(keys)), count)
                first = keys[0].to_bytes(32, 'little')[:28]
                self.assertTrue(all(k.to_bytes(32, 'little')[:28] == first for k in keys))
                self.assertTrue(all((k & ((1 << 64) - 1)) == g0 for k in keys))
                self.assertTrue(all(((k >> 64) & ((1 << 64) - 1)) == g0 for k in keys))
                expected = ((4 * g0) & ((1 << 64) - 1)) ^ ((4 * g0) >> 64)
                self.assertEqual({m.hash_key(k, 0x123456789abcdef0, g0, 0x9988776655443322)
                                  for k in keys}, {expected})

    def test_dispersed_control_is_stable_and_not_a_distribution_certificate(self):
        m = self.helper()
        keys = m.dispersed_keys(8192)
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(keys[:64], m.dispersed_keys(64))
        self.assertTrue(all(type(k) is int and 0 <= k < (1 << 256) for k in keys))
        self.assertEqual(len({m.hash_key(k, 0x123456789abcdef0,
                                       0xd2b5ccfd88cdb855, 0x9988776655443322)
                              for k in keys[:64]}), 64)
        # This one fixed unit seed does not establish the actual guest distribution.

    def test_strict_keys_seeds_and_counts(self):
        m = self.helper()
        for bad in (True, None, '1', 1.0, -1, 1 << 256):
            with self.assertRaises(ValueError):
                m.hash_key(bad, 0, 0, 0)
        for index in (1, 2, 3):
            for bad in (True, None, '1', 1.0, -1, 1 << 64):
                args = [0, 0, 0, 0]
                args[index] = bad
                with self.assertRaises(ValueError):
                    m.hash_key(*args)
        for bad in (True, None, '1', 1.0, -1, 1 << 64):
            with self.assertRaises(ValueError):
                m.concentrated_keys(bad, 1)
        for bad in (True, None, '1', 1.0, 0, -1, 8193):
            with self.assertRaises(ValueError):
                m.concentrated_keys(0, bad)
            with self.assertRaises(ValueError):
                m.dispersed_keys(bad)


if __name__ == '__main__':
    unittest.main()
