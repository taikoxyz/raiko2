"""Independent point intent, actual wrapper bytes, envelopes and retained native smoke."""
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
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('identity', 'nonidentity'), (2, 16, 64), ('zero', 'single', 'full')))
NATIVE_SHA256 = 'ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec'


def producer():
    path = CASES / 'bn254_pairing_dense.py'
    assert path.is_file(), 'dense pairing producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('bn254_pairing_dense', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qualify(test, row):
    from bn254_pairing_vm import run_users, validate_payload
    p = row['parameters']; program = row['request']['program']
    test.assertIn((p['kind'], p['pairs'], p['workload_level']), GRID)
    test.assertEqual(p['repetitions'], {'zero': 0, 'single': 1, 'full': p['full_repetitions']}[p['workload_level']])
    contracts = program['prestate_contracts']; test.assertEqual(len(contracts), 1)
    contract = contracts[0]; test.assertEqual(contract['initial_storage'], {})
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=2,
        gas_limit=p['transaction_gas_limit'], recipients=[contract['address']] * 2))
    test.assertEqual((program['code_hex'], program['input_hex'], program['block_count']), ('0x00', '0x', 1))
    code = bytes.fromhex(contract['code_hex'][2:]); payload = bytes.fromhex(p['pairing_input_hex'][2:])
    validate_payload(payload, p['kind'], p['pairs'], ordered=True)
    test.assertEqual(code[-len(payload):], payload)
    test.assertEqual(p['pairing_input_sha256'], hashlib.sha256(payload).hexdigest())
    test.assertEqual((p['runtime_bytes'], p['runtime_sha256']), (len(code), hashlib.sha256(code).hexdigest()))
    records = run_users(row); routes = Counter(); spent = 167355; prefixes = []
    for result in records:
        test.assertEqual(result['terminal'], 0); test.assertEqual(result['stack'], [])
        test.assertEqual(result['attempted_calls'], p['repetitions']); test.assertEqual(result['completed_calls'], p['repetitions'])
        test.assertEqual(result['copied'], payload); test.assertEqual(result['counter'], p['repetitions'])
        test.assertEqual(len(result['memory']), 32 * (2 + 6 * p['pairs']))
        test.assertEqual(result['gas'], p['expected_user_charged_gas'])
        test.assertEqual(result['gas'], p['gas_accounting_per_user']['gross_gas'])
        test.assertEqual(result['output'], 2 if p['repetitions'] == 0 else int(p['kind'] == 'identity'))
        for call in result['calls']:
            test.assertEqual(call['input'], payload); test.assertEqual(call['sentinel'], 2)
            test.assertEqual(call['native_gas'], 45000 + 34000 * p['pairs'])
            test.assertGreaterEqual(call['forwarded_gas'], call['native_gas'])
            test.assertEqual(call['forwarded_gas'], min(call['requested_gas'], call['available_gas'] - call['available_gas'] // 64))
            test.assertEqual(call['returned_gas'], call['forwarded_gas'] - call['native_gas'])
            test.assertGreater(call['gas_after_call'], 0)
        prefixes.append(31_000_000 - spent); test.assertGreaterEqual(prefixes[-1], p['transaction_gas_limit'])
        spent += result['gas']; routes.update(result['counts'])
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(spent, p['expected_block_charged_gas']); test.assertLessEqual(spent, 30_000_000)
    test.assertEqual(prefixes, p['header_prefix_remaining_gas']); test.assertEqual(p['near_full'], spent >= 29_700_000)
    test.assertLessEqual(p['transaction_gas_limit'], min(15_000_000, 16_777_216)); test.assertLessEqual(len(code), 24576)
    for op in (0x54, 0x55, 0x5c, 0x5d, 0xf0, 0xf1, 0xf4, 0xfd, 0xfe):
        test.assertEqual(routes[f'opcode:0x{op:02x}'], 0)
    test.assertEqual(p['aggregate_state_size'], 0); test.assertEqual(p['state_size_per_account'], 0)
    return records


class PairingDenseTests(unittest.TestCase):
    def test_all_eighteen_two_user_trajectories(self):
        m = producer()
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                gas = m.gas_accounting(*args[:2], p['full_repetitions'])
                self.assertEqual(p['transaction_gas_limit'], gas['gross_gas'] + (gas['native_gas_per_call'] + 62) // 63 + 2301)

    def test_independent_curve_subgroup_and_schedule(self):
        m = producer()
        from bn254_pairing_vm import validate_payload, check_g2
        check_g2()
        for kind, k in itertools.product(('identity', 'nonidentity'), (2, 16, 64)):
            payload = m.pairing_input(kind, k); self.assertEqual(len(payload), 192 * k)
            self.assertEqual(validate_payload(payload, kind, k, ordered=True), int(kind == 'identity'))
            for mutant in ('infinity', 'field', 'g2-order', 'reorder', 'truncated', 'wrong-kind'):
                wrong = bytearray(payload); selected = kind
                if mutant == 'infinity': wrong[:64] = bytes(64)
                elif mutant == 'field': wrong[:32] = m.FIELD_MODULUS.to_bytes(32, 'big')
                elif mutant == 'g2-order': wrong[64:128] = wrong[96:128] + wrong[64:96]
                elif mutant == 'reorder': wrong[:384] = wrong[192:384] + wrong[:192]
                elif mutant == 'truncated': wrong.pop()
                else: selected = 'nonidentity' if kind == 'identity' else 'identity'
                with self.subTest(kind=kind, k=k, mutant=mutant), self.assertRaises(AssertionError):
                    validate_payload(bytes(wrong), selected, k, ordered=True)
            reordered = payload[192:384] + payload[:192] + payload[384:]
            self.assertEqual(validate_payload(reordered, kind, k, ordered=False), int(kind == 'identity'))

    def test_full_plus_one_and_runtime_boundaries(self):
        m = producer(); from bn254_pairing_vm import execute
        for kind, k in itertools.product(('identity', 'nonidentity'), (2, 16, 64)):
            n = m.full_repetitions(kind, k); self.assertTrue(m.fits(kind, k, n)); self.assertFalse(m.fits(kind, k, n + 1))
            for r in (0, 1, n, n + 1):
                code, counts = m._runtime(kind, k, r)
                result = execute(code, 40_000_000, kind=kind, pairs=k)
                self.assertEqual(result['terminal'], 0); self.assertEqual(result['completed_calls'], r)
                self.assertEqual(result['counts'], +Counter(counts)); gas = m.gas_accounting(kind, k, r)
                self.assertEqual(result['gas'], gas['gross_gas'])
                self.assertEqual(gas['memory_expansion_gas'], 3 * (2 + 6 * k) + (2 + 6 * k) ** 2 // 512)
                self.assertEqual(gas['codecopy_gas'], 3 + 18 * k)
                self.assertEqual(gas['precompile_gas'], r * (100 + 45000 + 34000 * k))
            gas = m.gas_accounting(kind, k, n + 1); envelope = gas['gross_gas'] + (45000 + 34000 * k + 62) // 63 + 2301
            self.assertTrue(envelope > 15_000_000 or 167355 + 2 * gas['gross_gas'] > 30_000_000 or
                any(31_000_000 - 167355 - j * gas['gross_gas'] < envelope for j in range(2)))

    def test_strict_domains(self):
        m = producer()
        for args in ((None,), (True,), ('IDENTITY',), ('identity', True), ('identity', 2.0), ('identity', 3), ('identity', 2, True), ('identity', 2, 'small')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for fn in (m._runtime, m.gas_accounting, m.fits):
            for r in (True, -1, 1.0, 65537, None):
                with self.assertRaises(ValueError): fn('identity', 2, r)
            for kind, k in ((None, 2), ('IDENTITY', 2), ('identity', True), ('identity', 2.0), ('identity', 3)):
                with self.assertRaises(ValueError): fn(kind, k, 0)
        for kind, k in ((None, 2), ('identity', True), ('identity', 3)):
            with self.assertRaises(ValueError): m.full_repetitions(kind, k)
        self.assertEqual(m.make_case(), m.make_case('nonidentity', 2, 'full'))
        # Each admission limit is enforced independently even when another is looser.
        for field, value in (('TRANSACTION_GAS_CAP', 21000), ('EIP7825_GAS_CAP', 21000),
                             ('BLOCK_BUDGET', 167355), ('HEADER_GAS_LIMIT', 167355)):
            with self.subTest(field=field), patch.object(m, field, value):
                self.assertFalse(m.fits('identity', 2, 0))

    def test_wrapper_failure_and_literal_mutants(self):
        m = producer(); from bn254_pairing_vm import execute, decode
        code, _ = m._runtime('nonidentity', 2, 2)
        for override in ({'status': 0}, {'return_size': 31}, {'return_size': 33}, {'result': 1}, {'result': 2}, {'write_size': 0}, {'write_size': 31}):
            with self.subTest(override=override):
                self.assertEqual(execute(code, 2_000_000, kind='nonidentity', pairs=2, override=override)['terminal'], 0xfe)
        ins = list(decode(code)); self.assertLess(len(ins), len(code))
        for label, at, replacement in (
            ('target', next(at for at, op, v in ins if op == 0x62 and v == 8), 7),
            ('input-offset', next(at for at, op, v in ins if op == 0x62 and v == 32 and code[at + 4] == 0x62), 0),
            ('copy-size', next(at for at, op, v in ins if op == 0x62 and v == 384), 383),
            ('counter-step', code.index(bytes.fromhex('62000000516200000101')) + 5, 2),
        ):
            wrong = bytearray(code); wrong[at + 1:at + 4] = replacement.to_bytes(3, 'big')
            with self.subTest(label=label), self.assertRaises(AssertionError): execute(wrong, 2_000_000, kind='nonidentity', pairs=2)
        row = m.make_case('nonidentity', 2, 'single'); payload = bytes.fromhex(row['parameters']['pairing_input_hex'][2:])
        wrong = code[:-len(payload)] + payload[192:] + payload[:192]
        with self.assertRaises(AssertionError): execute(wrong, 2_000_000, kind='nonidentity', pairs=2)
        with self.assertRaisesRegex(AssertionError, 'gas'): execute(code, 21000, kind='nonidentity', pairs=2)
        # Unreachable bytes are data, including a final PUSH32 without32 following bytes.
        self.assertEqual(execute(code + b'\x7f', 2_000_000, kind='nonidentity', pairs=2)['terminal'], 0)

    def test_catalog_cli_and_provenance(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'matrix'; self.assertEqual(m.prepare(output), 18)
            catalog = json.loads((output / 'catalog.json').read_bytes()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 18); self.assertEqual(len({r['request_sha256'] for r in rows}), 18)
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256((CASES / 'bn254_pairing_dense.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'], {name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            self.assertEqual(catalog['helper_source_sha256'], {name: hashlib.sha256(inspect.getsource(fn).encode()).hexdigest() for name, fn in m.helpers().items()})
            self.assertIn('precompile_workload._point_add', catalog['helper_source_sha256'])
            for row in rows:
                raw = (output / row['request_path']).read_bytes()
                self.assertEqual(hashlib.sha256(raw).hexdigest(), row['request_sha256'])
                self.assertEqual(raw, (json.dumps(json.loads(raw), indent=2, sort_keys=True) + '\n').encode())
                self.assertEqual(row['parameters']['transaction_gas_limit'], by[row['baseline_sample_id']]['parameters']['transaction_gas_limit'])
                self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for bad in (None, True, 'A' * 64, '0' * 63):
                target = Path(temporary) / 'invalid'
                with self.assertRaises(ValueError): m.prepare(target, model_id=bad)
                self.assertFalse(target.exists())
            for bad in (None, True, 'matrix'):
                with self.assertRaises(ValueError): m.prepare(bad)
            command = [sys.executable, '-S', str(CASES / 'bn254_pairing_dense.py'), '--output-dir', str(Path(temporary) / 'cli'), '--model-id', 'a' * 64]
            process = subprocess.run(command, capture_output=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr); self.assertEqual(json.loads(process.stdout), {'prepared': 18})
            self.assertEqual(json.loads((Path(temporary) / 'cli/catalog.json').read_bytes())['frozen_model_id'], 'a' * 64)
            self.assertNotEqual(subprocess.run(command, capture_output=True, timeout=30).returncode, 0)

    @unittest.skipUnless(os.environ.get('BN254_PAIRING_NATIVE_BIN'), 'set BN254_PAIRING_NATIVE_BIN for frozen native smoke')
    def test_frozen_native_three_specimens(self):
        m = producer(); binary = Path(os.environ['BN254_PAIRING_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), NATIVE_SHA256)
        base = Path(os.environ['BN254_PAIRING_NATIVE_OUTPUT_DIR']); base.mkdir(parents=True, exist_ok=False)
        for name, kind, pairs, mutant in (('nonidentity-k2', 'nonidentity', 2, False), ('identity-k64', 'identity', 64, False), ('reordered-k2', 'nonidentity', 2, True)):
            row = m.make_case(kind, pairs, 'single'); p = row['parameters']
            if mutant:
                c = row['request']['program']['prestate_contracts'][0]; code = bytes.fromhex(c['code_hex'][2:]); tail = 192 * pairs
                c['code_hex'] = '0x' + (code[:-tail] + code[-tail + 192:] + code[-tail:-tail + 192]).hex()
                from bn254_pairing_vm import run_users
                with self.assertRaisesRegex(AssertionError, 'literal ordered point schedule'): run_users(row)
            root = base / name; root.mkdir(); source = root / 'request.json'; source.write_bytes(m.encoded(row['request']))
            command = [str(binary), '--input', str(source), '--output', str(root / 'input.json'), '--output-bincode', str(root / 'input.bin'), '--report', str(root / 'native.json'), '--execution-profile', 'evm-gas-only']
            manifest = dict(command=command, start_utc=datetime.now(timezone.utc).isoformat(), timeout_seconds=180,
                native_sha256=NATIVE_SHA256, request_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                generator_sha256=hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest(),
                test_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                oracle_sha256=hashlib.sha256(Path(__file__).with_name('bn254_pairing_vm.py').read_bytes()).hexdigest(),
                source_sha256={name: hashlib.sha256(Path(mod.__file__).read_bytes()).hexdigest() for name, mod in m.source_modules().items()})
            (root / 'process.json').write_bytes(m.encoded(manifest)); start = time.monotonic()
            try:
                process = subprocess.run(command, capture_output=True, timeout=180)
                stdout, stderr = process.stdout, process.stderr; manifest.update(exit_code=process.returncode, timed_out=False)
            except subprocess.TimeoutExpired as error:
                stdout, stderr = error.stdout or b'', error.stderr or b''; manifest.update(exit_code=None, timed_out=True)
            except OSError as error:
                stdout, stderr = b'', str(error).encode(); manifest.update(exit_code=None, timed_out=False, launch_error=str(error))
            finally:
                manifest['elapsed_seconds'] = time.monotonic() - start
                (root / 'stdout.log').write_bytes(stdout); (root / 'stderr.log').write_bytes(stderr)
                manifest['files_sha256'] = {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in root.iterdir() if f.is_file() and f.name != 'process.json'}
                (root / 'process.json').write_bytes(m.encoded(manifest))
            self.assertEqual(manifest['exit_code'], 0, stderr.decode(errors='replace'))
            report = json.loads((root / 'native.json').read_bytes()); receipts = report['receipts_by_block'][0]
            self.assertEqual([r['success'] for r in receipts], [True] * 3)
            self.assertEqual([r['gas_used'] for r in receipts], [167355] + [p['expected_user_charged_gas']] * 2)
            for field in ('observed_route_counts', 'attempted_route_counts'):
                self.assertEqual(+Counter({k: v for k, v in report[field].items() if k.startswith('opcode:')}), +Counter(row['request']['program']['expected_route_counts']))
            for entry in report['prestate_contracts']['initialized_storage']:
                self.assertEqual((entry['initial_storage_entry_count'], entry['final_storage_entry_count']), (0, 0))
                self.assertEqual(entry['initial_storage_root'], entry['final_storage_root'])


if __name__ == '__main__': unittest.main()
