"""Synthetic-only direct-rustc CLI contract checks; no SDK or actual fixtures."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'hash_probe_catalog.rs'
QUERY = '0x1122334455667788' + '00' * 24
ZERO = '0x' + '0' * 16


def independent_hash(key, seed, shared):
    limbs = [(int(key, 16) >> (64 * i)) & ((1 << 64) - 1) for i in range(4)]
    def fold(x, y):
        product = x * y
        return (product & ((1 << 64) - 1)) ^ (product >> 64)
    return fold(limbs[2] ^ fold(limbs[0] ^ seed, limbs[1] ^ shared), limbs[3] ^ shared)


def independent_address_hash(value, seed, shared):
    raw = bytes.fromhex(value[2:])
    if len(raw) != 20:
        raise ValueError('Address20 length')
    def product_fold(left, right):
        product = left * right
        return (product & ((1 << 64) - 1)) ^ (product >> 64)
    words = [int.from_bytes(raw[:8], 'little'), int.from_bytes(raw[8:16], 'little'), int.from_bytes(raw[16:], 'little')]
    return product_fold(product_fold(seed ^ words[0], shared ^ words[1]) ^ words[2], shared)


class HashProbeCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not SOURCE.is_file(): raise AssertionError('maintained hash/search source missing')
        cls.tmp = tempfile.TemporaryDirectory(prefix='hash-probe-cli-')
        cls.binary = Path(cls.tmp.name) / 'hash-probe-catalog'
        built = subprocess.run(['rustc', '--edition=2024', '-O', str(SOURCE), '-o', str(cls.binary)], capture_output=True, timeout=60)
        if built.returncode: raise AssertionError(built.stderr.decode())

    @classmethod
    def tearDownClass(cls): cls.tmp.cleanup()

    def cli(self, *args):
        return subprocess.run([str(self.binary), *map(str, args)], capture_output=True, timeout=10)

    def search_args(self, output, trials=1000):
        return ['search', '--seed', ZERO, '--shared', ZERO, '--query', QUERY, '--bucket-mask', '1', '--max-trials', str(trials), '--max-seconds', '1', '--output', str(output)]

    def test_address20_hash_uses_raw_bytes_and_independent_fold_formula(self):
        keys = ['0x' + '00' * 20, '0x' + 'ff' * 20,
                '0x0123456789abcdef102030405060708090a0b0c0',
                '0x11223344' + '00' * 12 + '55667788']
        for seed, shared in [(0, 0), (17, 29), (0x0123456789abcdef, 0xfedcba9876543210)]:
            args = ['hash', '--seed', f'0x{seed:016x}', '--shared', f'0x{shared:016x}']
            for key in keys:
                args += ['--key', key]
            result = self.cli(*args, '--key-kind', 'address20')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['hashes'], [
                {'key': key, 'hash': f'0x{independent_address_hash(key, seed, shared):016x}'} for key in keys])

    def test_address20_search_long_and_short_prefix_preserve_catalog_domain(self):
        query = '0x11223344' + '00' * 12 + '55667788'
        with tempfile.TemporaryDirectory() as tmp:
            for domain in ('low32', 'high32'):
                out = Path(tmp) / (domain + '.json')
                args = self.search_args(out)
                args[args.index('--query') + 1] = query
                result = self.cli(*args, '--key-kind', 'address20', '--domain', domain)
                self.assertEqual(result.returncode, 0, result.stderr)
                meta = json.loads(result.stdout)
                self.assertEqual(meta['key_kind'], 'address20')
                keys = json.loads(out.read_text())['keys']
                self.assertEqual((len(keys), len(set(keys)), keys[-1]), (512, 512, query))
                for key in keys:
                    raw = bytes.fromhex(key[2:])
                    self.assertEqual(len(raw), 20)
                    self.assertEqual(sum(byte != 0 for byte in raw), 8)
                    self.assertEqual(independent_address_hash(key, 0, 0), 0)
                    if domain == 'low32':
                        self.assertEqual(raw[:16], bytes.fromhex(query[2:])[:16])
                    else:
                        self.assertEqual(raw[4:], bytes.fromhex(query[2:])[4:])

    def test_key_kind_preserves_strict_encoding_and_default_u256(self):
        args = ['hash', '--seed', ZERO, '--shared', ZERO, '--key', QUERY]
        implicit = self.cli(*args)
        explicit = self.cli(*args, '--key-kind', 'u256')
        self.assertEqual(explicit.returncode, 0, explicit.stderr)
        self.assertEqual(implicit.stdout, explicit.stdout)
        address = '0x11223344' + '00' * 12 + '55667788'
        for kind, key in [('address20', QUERY), ('u256', address), ('address20', address.upper()), ('other', QUERY)]:
            result = self.cli('hash', '--seed', ZERO, '--shared', ZERO, '--key', key, '--key-kind', kind)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, b'')

    def test_hash_batch_full_limbs_overflow_and_1024_bound(self):
        keys = ['0x' + '00' * 32, '0x' + 'ff' * 32, '0x' + '0123456789abcdef' * 4, QUERY]
        for seed, shared in ((0, 0), ((1 << 64) - 1, (1 << 64) - 2), (17, 29)):
            args = ['hash', '--seed', f'0x{seed:016x}', '--shared', f'0x{shared:016x}']
            for key in keys: args += ['--key', key]
            result = self.cli(*args); self.assertEqual(result.returncode, 0, result.stderr)
            obj = json.loads(result.stdout)
            self.assertEqual(obj['hashes'], [{'key': key, 'hash': f'0x{independent_hash(key, seed, shared):016x}'} for key in keys])
        args = ['hash', '--seed', ZERO, '--shared', ZERO] + ['--key', QUERY] * 1024
        good = self.cli(*args); self.assertEqual(good.returncode, 0); self.assertLessEqual(len(good.stdout), 256 * 1024)
        bad = self.cli(*args, '--key', QUERY); self.assertEqual(bad.returncode, 1); self.assertEqual(bad.stdout, b'')

    def test_invalid_cli_has_no_output(self):
        good = ['hash', '--seed', ZERO, '--shared', ZERO, '--key', QUERY]
        for args in ([], ['other'], good + ['--seed', ZERO], good + ['--unexpected', '1'], good[:-1], good[:5],
                     ['hash', '--seed', '0x1', '--shared', ZERO, '--key', QUERY],
                     good[:-1] + [QUERY.upper()], good[:-1] + ['0x' + 'g' * 64]):
            result = self.cli(*args); self.assertEqual(result.returncode, 1, args); self.assertEqual(result.stdout, b'')
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'catalog.json'; args = self.search_args(out)
            for flag, value in (('--bucket-mask', '0'), ('--bucket-mask', '2'), ('--bucket-mask', '131071'),
                                ('--max-trials', '0'), ('--max-trials', '268435457'), ('--max-seconds', '0'),
                                ('--max-seconds', '121'), ('--max-seconds', '1.0'), ('--query', '0x' + '00' * 32),
                                ('--query', QUERY[:-1] + '1')):
                changed = args.copy(); changed[changed.index(flag) + 1] = value
                result = self.cli(*changed); self.assertEqual(result.returncode, 1, (flag, value)); self.assertFalse(out.exists()); self.assertEqual(result.stdout, b'')

    def test_low32_domain_varies_comparison_prefix_and_preserves_byte_cost(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'short-prefix.json'
            result = self.cli(*self.search_args(out), '--domain', 'low32')
            self.assertEqual(result.returncode, 0, result.stderr)
            meta = json.loads(result.stdout)
            self.assertEqual(meta['candidate_domain'], 'low32')
            keys = json.loads(out.read_bytes())['keys']
            self.assertEqual((len(keys), len(set(keys)), keys[-1]), (512, 512, QUERY))
            self.assertEqual(len({int(k, 16) & 0xffffffff for k in keys}), 512)
            self.assertTrue(all(sum(b != 0 for b in bytes.fromhex(k[2:])) == 8 for k in keys))
            self.assertTrue(all(independent_hash(k, 0, 0) == 0 for k in keys))
            invalid = Path(tmp) / 'invalid.json'
            bad = self.cli(*self.search_args(invalid), '--domain', 'other')
            self.assertEqual(bad.returncode, 1)
            self.assertFalse(invalid.exists())

    def test_synthetic_search_catalog_permutation_and_exhaustion(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'catalog.json'
            result = self.cli(*self.search_args(out)); self.assertEqual(result.returncode, 0, result.stderr)
            meta = json.loads(result.stdout); self.assertEqual((meta['status'], meta['found']), ('complete', 512))
            self.assertLessEqual(meta['trials'], 1000); self.assertLessEqual(len(result.stdout), 16 * 1024)
            obj = json.loads(out.read_bytes()); self.assertEqual(set(obj), {'schema_version', 'keys'}); self.assertEqual(obj['schema_version'], 1)
            keys = obj['keys']; self.assertEqual((len(keys), len(set(keys)), keys[-1]), (512, 512, QUERY)); self.assertLessEqual(out.stat().st_size, 128 * 1024)
            expected = []
            for counter in range(meta['trials']):
                z = (counter + 0x9e3779b9) & 0xffffffff
                z = ((z ^ (z >> 16)) * 0x85ebca6b) & 0xffffffff
                z = ((z ^ (z >> 13)) * 0xc2b2ae35) & 0xffffffff; z ^= z >> 16
                high = (z << 32) | 0x55667788
                key = f'0x{high:016x}' + '00' * 24
                if all(high.to_bytes(8, 'big')) and key != QUERY: expected.append(key)
            self.assertEqual(keys, expected[:511] + [QUERY])
            self.assertTrue(all(k[10:] == QUERY[10:] for k in keys))
            self.assertTrue(all(independent_hash(k, 0, 0) == 0 for k in keys))
            incomplete = Path(tmp) / 'incomplete.json'; failed = self.cli(*self.search_args(incomplete, 1))
            self.assertEqual(failed.returncode, 2); self.assertFalse(incomplete.exists())
            obj = json.loads(failed.stdout); self.assertEqual((obj['status'], obj['trials'], obj['found']), ('trial_limit', 1, 2))
            self.assertEqual(sorted(p.name for p in Path(tmp).iterdir()), ['catalog.json'])

    def test_synthetic_time_budget_and_filesystem_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); out = root / 'timed.json'; args = self.search_args(out, 268435456)
            # Wide synthetic operands avoid the small-product degeneracy of S=17/G=29:
            # F(17,29)=493 restricts the upper product and makes low16 matches cheap.
            args[args.index('--seed') + 1] = '0x0123456789abcdef'
            args[args.index('--shared') + 1] = '0xfedcba9876543210'
            args[args.index('--bucket-mask') + 1] = '65535'
            result = self.cli(*args); self.assertEqual(result.returncode, 2, result.stderr)
            obj = json.loads(result.stdout); self.assertIn(obj['status'], ('time_limit', 'trial_limit'))
            self.assertLess(obj['found'], 512); self.assertFalse(out.exists()); self.assertEqual(list(root.iterdir()), [])
            if obj['status'] == 'time_limit': self.assertGreaterEqual(obj['elapsed_ns'], 1000000000)
            for output in (root / 'absent' / 'catalog.json', root):
                result = self.cli(*self.search_args(output)); self.assertEqual(result.returncode, 1); self.assertEqual(result.stdout, b'')
            self.assertEqual(list(root.iterdir()), [])

    def test_existing_symlink_and_concurrent_outputs_are_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); target = root / 'existing'; target.write_bytes(b'unchanged')
            link = root / 'link'; link.symlink_to(target)
            dangling = root / 'dangling'; dangling.symlink_to(root / 'absent')
            for out in (target, link, dangling):
                result = self.cli(*self.search_args(out)); self.assertEqual(result.returncode, 1); self.assertEqual(result.stdout, b'')
            self.assertEqual(target.read_bytes(), b'unchanged'); self.assertTrue(dangling.is_symlink())
            out = root / 'race.json'; args = [str(self.binary), *self.search_args(out)]
            jobs = [subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(2)]
            results = [(*p.communicate(timeout=10), p.returncode) for p in jobs]
            self.assertEqual(sorted(r[2] for r in results), [0, 1]); self.assertEqual(len(json.loads(out.read_bytes())['keys']), 512)
            self.assertEqual(sorted(p.name for p in root.iterdir()), ['dangling', 'existing', 'link', 'race.json'])


if __name__ == '__main__': unittest.main()
