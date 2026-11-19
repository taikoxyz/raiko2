"""Independent tail analysis and bounded concrete DELEGATECALL regressions."""
from contextlib import nullcontext
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from ethereum_hash import keccak256
from test_multiaccount_code import BASE, PARENT, PREFIX, instructions, run

CASES = Path(__file__).resolve().parents[1] / 'cases'
PROFILES = ('stop', 'jumpdest', 'mixed', 'push32', 'jumpdest-copy', 'jumpdest-pad32')
LENGTHS = (4096, 24576)


def verify_tail(code, length, profile, index):
    """Decode independently, including PUSH data and truncated final PUSH padding."""
    assert len(code) == length, 'runtime length'
    assert code[:9] == PREFIX, 'executed prefix'
    terminal = {'jumpdest-copy': 1, 'jumpdest-pad32': 0x7f}.get(profile, 0)
    assert code[-6] == 0x63, 'salt instruction'
    assert code[-5:-1] == (index + 1).to_bytes(4, 'big'), 'salt identity'
    assert code[-1] == terminal, 'final opcode'
    middle = code[9:-6]
    if profile == 'push32':
        groups, remainder = divmod(length - 15, 33)
        assert middle == (b'\x7f' + b'\x5b' * 32) * groups + bytes(remainder), 'PUSH tail pattern'
        expected_jumps = set()
    elif profile == 'stop':
        assert not any(middle), 'STOP tail pattern'
        expected_jumps = set()
    elif profile == 'mixed':
        assert all(op == (0x5b if i % 2 == 0 else 1) for i, op in enumerate(middle)), 'mixed tail pattern'
        expected_jumps = set(range(9, length - 6, 2))
    else:
        assert middle == b'\x5b' * len(middle), 'JUMPDEST tail pattern'
        expected_jumps = set(range(9, length - 6))
    jumps = set(); boundaries = set(); pc = 0
    while pc < len(code):
        boundaries.add(pc); op = code[pc]
        if op == 0x5b: jumps.add(pc)
        pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
    assert length - 6 in boundaries, 'suffix instruction boundary'
    assert jumps == expected_jumps, 'jump table including immediate exclusion'
    assert pc - length == (32 if profile == 'jumpdest-pad32' else 0), 'analysis padding'
    # The scanner's overflow is followed by a synthetic STOP for non-STOP
    # terminals: ADD copies/pads one byte, incomplete PUSH32 copies/pads33.
    padding = pc - length + (code[max(boundaries)] != 0)
    assert padding == {'jumpdest-copy': 1, 'jumpdest-pad32': 33}.get(profile, 0), 'padded analysis length'
    return jumps


class CodeAnalysisTailTests(unittest.TestCase):
    def load(self):
        path = CASES / 'code_analysis_tail.py'
        self.assertTrue(path.is_file(), 'code-analysis tail producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('tail_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_catalog_matrix_pairs_patterns_provenance_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'catalog'
            self.assertEqual(m.prepare(root), 24)
            catalog = json.loads((root / 'catalog.json').read_text()); rows = catalog['cases']
            by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 24); self.assertEqual(len({r['request_sha256'] for r in rows}), 24)
            self.assertEqual(catalog['frozen_model_id'], 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef')
            self.assertEqual(catalog['profiles'], list(PROFILES))
            self.assertEqual(catalog['target_count'], 512)
            self.assertEqual(catalog['code_lengths'], list(LENGTHS))
            self.assertEqual(catalog['levels'], ['zero', 'full'])
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'code_analysis_tail.py').read_bytes()).hexdigest())
            sources = ('multiaccount_code.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256((CASES / name).read_bytes()).hexdigest() for name in sources})
            self.assertEqual({(r['parameters']['runtime_bytes_per_target'], r['parameters']['tail_profile'], r['parameters']['workload_level']) for r in rows},
                             {(length, profile, level) for length in LENGTHS for profile in PROFILES for level in ('zero', 'full')})
            for row in rows:
                p = row['parameters']; raw = (root / row['request_path']).read_bytes()
                self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                req = json.loads(raw); program = req['program']
                self.assertTrue(row['sample_id'].startswith('code-analysis-tail-'))
                self.assertEqual(p['target_count'], 512); self.assertEqual(p['child_call_kind'], 'delegatecall')
                self.assertEqual(program['transactions'], {'kind': 'call', 'count_per_block': 4, 'gas_limit': p['transaction_gas_limit']})
                self.assertEqual((program['block_count'], program['input_hex']), (1, '0x'))
                self.assertTrue(req['expected_user_success']); self.assertTrue(row['holdout_only'])
                self.assertEqual(p['header_gas_limit'], 31_000_000)
                baseline = by[row['baseline_sample_id']]
                self.assertEqual(baseline['parameters']['calls_per_user'], 0)
                self.assertEqual(baseline['parameters']['tail_profile'], p['tail_profile'])
                self.assertEqual(baseline['parameters']['transaction_gas_limit'], p['transaction_gas_limit'])
                length = p['runtime_bytes_per_target']; profile = p['tail_profile']; contracts = program['prestate_contracts']
                self.assertEqual(len(contracts), 512); hashes = set()
                for index, contract in enumerate(contracts):
                    self.assertEqual(contract['address'], f'0x{BASE + index:040x}')
                    code = bytes.fromhex(contract['code_hex'][2:]); verify_tail(code, length, profile, index)
                    hashes.add(hashlib.sha256(code).digest())
                self.assertEqual(len(hashes), 512)
                paired = json.loads((root / baseline['request_path']).read_text())['program']
                self.assertEqual(program['transactions'], paired['transactions'])
                self.assertEqual(contracts, paired['prestate_contracts'])
                a, b = (bytes.fromhex(x['code_hex'][2:]) for x in (program, paired))
                self.assertEqual(len(a), len(b))
                if p['workload_level'] == 'full':
                    bounds = [pc for pc, op, _ in instructions(b) if op == 0x62][1:]
                    differences = [i for i, (x, y) in enumerate(zip(a, b)) if x != y]
                    self.assertTrue(differences)
                    self.assertTrue(all(any(pc < i <= pc + 3 for pc in bounds) for i in differences))
                n = p['calls_per_user']
                self.assertEqual(p['expected_configured_unique_code_count'], 512)
                self.assertEqual(p['expected_witnessed_target_code_count'], 512 if n else 0)
                self.assertEqual(p['expected_witnessed_target_code_bytes'], 512 * length if n else 0)
            before = (root / 'catalog.json').read_bytes()
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual((root / 'catalog.json').read_bytes(), before)

    def test_concrete_full_zero_context_routes_and_independent_gas(self):
        m = self.load()
        # Two real full loops bound test time; every profile's child is also executed.
        for length, profile in ((4096, 'stop'), (24576, 'jumpdest-pad32')):
            for level in ('zero', 'full'):
                row = m.make_case(length, profile, level); p = row['parameters']; program = row['request']['program']
                contracts = {int(c['address'], 16): bytes.fromhex(c['code_hex'][2:]) for c in program['prestate_contracts']}
                for i, code in enumerate(contracts.values()): verify_tail(code, length, profile, i)
                n = p['calls_per_user']
                result = run(bytes.fromhex(program['code_hex'][2:]), contracts, 512, n, current_address=PARENT)
                expected = 21054 + 265 * n + 2500 * min(n, 512) + (9 if n else 0)
                self.assertEqual(result['terminal'], 0); self.assertEqual(result['ending_counter'], n)
                self.assertEqual(result['gas'] + 21000, expected)
                self.assertEqual(p['expected_user_charged_gas'], expected)
                self.assertEqual(p['expected_block_charged_gas'], 167355 + 4 * expected)
                self.assertEqual((result['calls'], result['cold'], result['warm']), (n, min(n, 512), max(0, n - 512)))
                self.assertEqual({f'opcode:0x{k:02x}': v * 4 for k, v in result['counts'].items()},
                                 {k: v for k, v in program['expected_route_counts'].items() if v})
                self.assertEqual(result['counts'][0xf4], n); self.assertEqual(result['counts'][0xf1], 0)
                if n:
                    self.assertGreaterEqual(167355 + 4 * expected, 29_700_000)
                    self.assertLessEqual(167355 + 4 * expected, 30_000_000)
                    self.assertGreater(167355 + 4 * (expected + 265), 30_000_000)
        for profile in PROFILES:
            for length in LENGTHS:
                code = m.child_code(length, profile, 511); verify_tail(code, length, profile, 511)
                result = run(code, {}, 0, 0, current_address=PARENT, child=True)
                self.assertEqual((result['terminal'], result['end_pc'], result['gas']), (0xf3, 9, 17))
                self.assertEqual(result['output'], PARENT.to_bytes(32, 'big'))

    def test_semantic_verifier_rejects_unreachable_and_executed_mutants(self):
        m = self.load(); code = m.child_code(4096, 'jumpdest-pad32', 0)
        # Wrong unique salt, content, executed prefix, suffix instruction and terminal.
        for at, value in ((4094, 2), (9, 0), (0, 0x33), (4090, 0x64), (4095, 0)):
            bad = bytearray(code); bad[at] = value
            with self.subTest(at=at), self.assertRaises(AssertionError): verify_tail(bytes(bad), 4096, 'jumpdest-pad32', 0)

    def test_strict_inputs_and_invalid_model_leave_no_output(self):
        m = self.load()
        for length in (True, None, 32, 4095, 24577, '4096'):
            with self.subTest(length=length), self.assertRaises(ValueError): m.make_case(length=length)
        for profile in (True, None, [], {}, 'PUSH32', 'unknown'):
            with self.subTest(profile=profile), self.assertRaises(ValueError): m.make_case(profile=profile)
        for level in (True, None, [], 'n0'):
            with self.subTest(level=level), self.assertRaises(ValueError): m.make_case(level=level)
        for index in (True, None, -1, 512, '1'):
            with self.subTest(index=index), self.assertRaises(ValueError): m.child_code(4096, 'stop', index)
        with tempfile.TemporaryDirectory() as temporary:
            for model in (None, True, '', 'A' * 64, 'g' * 64):
                path = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(path, model_id=model)
                self.assertFalse(path.exists())
            # CLI validation exercises the actual entrypoint without another large catalog.
            path = Path(temporary) / 'cli'
            result = subprocess.run([sys.executable, '-S', str(CASES / 'code_analysis_tail.py'), '--output-dir', str(path), '--model-id', 'invalid'], capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0); self.assertFalse(path.exists())

    @unittest.skipUnless(os.environ.get('CODE_ANALYSIS_NATIVE_BIN'), 'set frozen native executable for opt-in matrix smoke')
    def test_frozen_native_profiles_sizes_and_selected_levels(self):
        m = self.load(); levels = os.environ.get('CODE_ANALYSIS_NATIVE_LEVELS', 'zero,full').split(',')
        self.assertTrue(levels and set(levels) <= {'zero', 'full'})
        retained = os.environ.get('CODE_ANALYSIS_NATIVE_ARTIFACT_DIR')
        context = nullcontext(retained) if retained else tempfile.TemporaryDirectory()
        with context as temporary:
            root = Path(temporary)
            if retained: root.mkdir(parents=True, exist_ok=False)
            for profile in PROFILES:
                for length in LENGTHS:
                    for level in levels:
                        row = m.make_case(length, profile, level)
                        # Without an explicit retention directory, release each
                        # native case before starting the next large-code input.
                        case_context = nullcontext(root / row['sample_id']) if retained else tempfile.TemporaryDirectory(dir=root)
                        with case_context as case_directory:
                            folder = Path(case_directory)
                            if retained: folder.mkdir()
                            self.check_native_case(row, folder)

    def check_native_case(self, row, folder):
        p = row['parameters']
        source = folder / 'request.json'; source.write_text(json.dumps(row['request']))
        result = subprocess.run([os.environ['CODE_ANALYSIS_NATIVE_BIN'], '--input', str(source), '--output', str(folder / 'input.json'), '--output-bincode', str(folder / 'input.bin'), '--report', str(folder / 'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
        (folder / 'native-process.json').write_text(json.dumps({'exit_code': result.returncode, 'stdout': result.stdout.decode(), 'stderr': result.stderr.decode()}))
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        report = json.loads((folder / 'native.json').read_text())
        self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
        self.assertEqual([r['gas_used'] for r in report['receipts_by_block'][0]], [167355] + [p['expected_user_charged_gas']] * 4)
        self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'], 0)
        self.assertEqual({k: v for k, v in report['observed_route_counts'].items() if v}, {k: v for k, v in row['request']['program']['expected_route_counts'].items() if v})
        prestate = report['prestate_contracts']
        for field, expected in (('configured_account_count', 512), ('configured_unique_code_bytes', 512 * p['runtime_bytes_per_target']), ('witnessed_account_count', p['expected_witnessed_target_accounts']), ('witnessed_unique_code_count', p['expected_witnessed_target_code_count']), ('witnessed_code_bytes', p['expected_witnessed_target_code_bytes'])):
            self.assertEqual(prestate[field], expected)
        guest = json.loads((folder / 'input.json').read_text())
        codes = [bytes.fromhex(c[2:]) for c in guest['witnesses'][0]['witness']['codes']]
        hashes = [keccak256(c) for c in codes]
        self.assertEqual(len(hashes), len(set(hashes)))
        expected = {bytes.fromhex(c['code_hex'][2:]) for c in row['request']['program']['prestate_contracts']} if p['calls_per_user'] else set()
        self.assertEqual({c for c in codes if c[:9] == PREFIX}, expected)


if __name__ == '__main__': unittest.main()
