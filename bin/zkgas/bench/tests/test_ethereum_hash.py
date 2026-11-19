"""Ethereum Keccak-256 KATs mandatory without optional third-party modules."""
import hashlib
import importlib.util
from pathlib import Path
import sys
import unittest

try:
    from Crypto.Hash import keccak as crypto_keccak
except ImportError:
    crypto_keccak = None


class EthereumHashTests(unittest.TestCase):
    def load(self):
        path = Path(__file__).with_name('ethereum_hash.py')
        self.assertTrue(path.is_file(), 'stdlib Ethereum Keccak helper is missing')
        sys.path.insert(0, str(path.parent))
        spec = importlib.util.spec_from_file_location('ethereum_hash_under_test', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_public_ethereum_known_answers_are_not_sha3(self):
        m = self.load()
        cases = [
            (b'', 'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470'),
            (bytes(32), '290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563'),
            (b'\x80', '56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421'),
        ]
        for data, expected in cases:
            with self.subTest(data=data):
                actual = m.keccak256(data); self.assertEqual(actual.hex(), expected)
                self.assertEqual(len(actual), 32); self.assertNotEqual(actual, hashlib.sha3_256(data).digest())

    def test_fixed_reference_vectors_cover_rate_padding_without_crypto(self):
        # Frozen independent reference digests; this test also runs under python -S.
        expected = {
            135: '67ac26d3e79a18e88f212385a7bad88e6f9cfd25b5bfadbc2c4d0cf95457e613',
            136: 'e0ab45aba084eb3ddec8f785b6993a3555833dd941733756a685525a84fe012c',
            137: '54e8f24f3543727f3825a7e1dd0e9c4d01b5254f5af2ae0fa3644212495d92b2',
            272: '57e0d7506f2c4eb5f09f7318721fbd04852849690c86ced13e7642fb17035478',
        }
        m = self.load()
        for length, digest in expected.items():
            data = bytes((index*17+3) % 256 for index in range(length))
            with self.subTest(length=length): self.assertEqual(m.keccak256(data).hex(), digest)

    def test_repeated_inputs_use_a_finite_cache_and_strict_bytes_domain(self):
        m = self.load(); m.keccak256.cache_clear(); value = bytes(32)
        expected = m.keccak256(value); before = m.keccak256.cache_info()
        self.assertEqual(m.keccak256(value), expected); after = m.keccak256.cache_info()
        self.assertEqual(after.hits, before.hits+1); self.assertGreater(after.maxsize, 0)
        self.assertLessEqual(after.maxsize, 65536); self.assertLessEqual(after.currsize, after.maxsize)
        for invalid in ('', 32, None, bytearray(32), memoryview(bytes(32))):
            with self.subTest(invalid=type(invalid)), self.assertRaises(TypeError): m.keccak256(invalid)

    @unittest.skipUnless(crypto_keccak is not None, 'optional local Crypto cross-check; all KATs remain mandatory')
    def test_optional_crypto_crosscheck_padding_and_multiple_rate_blocks(self):
        m = self.load()
        for length in (0, 1, 31, 32, 134, 135, 136, 137, 255, 256, 271, 272, 273, 1024):
            for data in (bytes(length), bytes((index*17+3) % 256 for index in range(length))):
                with self.subTest(length=length, data=data[:8]):
                    self.assertEqual(m.keccak256(data), crypto_keccak.new(digest_bits=256, data=data).digest())


if __name__ == '__main__': unittest.main()
