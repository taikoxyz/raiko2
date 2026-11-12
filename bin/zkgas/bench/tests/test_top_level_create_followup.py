"""Finite deployment-size/content/repetition followups and real native qualification."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


def interpret_return(code):
    """Small independent decoder for straight-line PUSH/MSTORE8/MCOPY/RETURN data."""
    stack, memory, counts = [], bytearray(), {}
    def extend(end):
        if end > len(memory): memory.extend(bytes(end-len(memory)))
    pc = 0
    while pc < len(code):
        op = code[pc]; pc += 1; key = f'opcode:0x{op:02x}'
        counts[key] = counts.get(key, 0)+1
        if 0x60 <= op <= 0x7f:
            width = op-0x5f
            if pc+width > len(code): raise AssertionError('truncated PUSH')
            stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width
        elif op == 0x53:
            offset, value = stack.pop(), stack.pop(); extend(offset+1); memory[offset] = value & 255
        elif op == 0x5e:
            destination, source, length = stack.pop(), stack.pop(), stack.pop()
            extend(max(destination+length, source+length)); memory[destination:destination+length] = bytes(memory[source:source+length])
        elif op == 0xf3:
            offset, length = stack.pop(), stack.pop(); extend(offset+length)
            if pc != len(code): raise AssertionError('unexpected code after RETURN')
            return bytes(memory[offset:offset+length]), counts
        else: raise AssertionError(f'unsupported straight-line opcode {op}')
    raise AssertionError('missing RETURN')


class TopLevelCreateFollowupTests(unittest.TestCase):
    def load(self):
        path = CASES/'top_level_create_followup.py'
        self.assertTrue(path.is_file(), 'followup producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('create_followup_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_exact_38_matrix_reference_replays_and_identity_closure(self):
        cases = self.load()
        expected = {('create', 'return_runtime', size, 1, 1, 8_000_000, None)
                    for size in (0, 1, 31, 33, 1024, 4095, 4096, 4097, 8192, 16384, 24575, 24576)}
        expected |= {('create', 'return_runtime', 0, 1, 3, 8_000_000, None)}
        expected |= {('create', 'return_runtime', 24576, n, b, 8_000_000, None) for n, b in ((2, 1), (4, 1), (1, 3), (2, 3))}
        expected |= {('create', 'return_runtime', 0, n, b, 8_000_000, None) for n, b in ((2, 1), (4, 1), (2, 3))}
        expected |= {('create', 'return_runtime', 256, n, 1, 300_000, None) for n in (1, 16, 128)}
        expected |= {('create', 'return_runtime', 1024, n, 1, 500_000, None) for n in (1, 16, 64)}
        expected |= {('create', 'filled_runtime', size, 1, 1, 8_000_000, seed) for size in (4096, 24576) for seed in (0, 0x5b, 0x7f)}
        expected |= {('call', 'return_runtime', size, 1, 1, 8_000_000, None) for size in (0, 4096, 24576, 24577)}
        expected |= {('call', 'return_runtime', size, 4, 1, 8_000_000, None) for size in (0, 24576)}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(cases.prepare(root), 38)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by_id = {r['sample_id']:r for r in rows}
            self.assertEqual(len(by_id), 38)
            keys = set(); requests = set(); refs = []; repeated = []
            for row in rows:
                p = row['parameters']; keys.add((p['transaction_kind'], p['shape'], p['returned_runtime_bytes'], p['count_per_block'], p['block_count'], p['transaction_gas_limit'], p['fill_byte']))
                raw = (root/row['request_path']).read_bytes(); requests.add(raw)
                self.assertEqual(row['request_sha256'], cases.digest(raw))
                request = json.loads(raw); self.assertTrue(request['expected_user_success'])
                self.assertEqual(request['program']['input_hex'], '0x')
                self.assertFalse({'access_list', 'authorizations', 'storage_state'} & request['program'].keys())
                base = by_id[row['baseline_sample_id']]['parameters']
                self.assertEqual(base['block_count'], p['block_count'])
                self.assertEqual(base['transaction_gas_limit'], p['transaction_gas_limit'])
                self.assertEqual(base['transaction_kind'], p['transaction_kind'])
                if row['reference_role'] == 'previously_observed_reference_replay':
                    refs.append(row)
                    original = cases.original.make_case('create', 'return_runtime', block_count=p['block_count'], runtime_bytes=p['returned_runtime_bytes'], gas_limit=8_000_000)
                    self.assertEqual(raw, cases.encoded(original['request']))
                    self.assertEqual(row['prior_observed_reference_sample_id'], original['sample_id'])
                else:
                    self.assertEqual(row['reference_role'], 'new_model_development_case')
                    self.assertIsNone(row['prior_observed_reference_sample_id'])
                self.assertFalse(row['holdout_only'])
                if 'repeat_reference_sample_id' in row:
                    repeated.append(row); previous = by_id[row['repeat_reference_sample_id']]['parameters']
                    self.assertEqual(previous['returned_runtime_bytes'], 24576)
                    self.assertEqual(previous['block_count'], p['block_count'])
                    self.assertLess(previous['count_per_block'], p['count_per_block'])
            self.assertEqual(keys, expected); self.assertEqual(len(requests), 38)
            self.assertEqual(len(refs), 3); self.assertEqual(len(repeated), 3)
            self.assertEqual(catalog['previously_observed_reference_replay_count'], 3)
            self.assertEqual(catalog['new_model_development_case_count'], 35)
            self.assertEqual(catalog['generator_sha256'], cases.digest((CASES/'top_level_create_followup.py').read_bytes()))
            self.assertEqual(catalog['base_generator_sha256'], cases.digest((CASES/'top_level_create.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'], cases.digest((CASES/'spawn_memory.py').read_bytes()))
            before = {str(p.relative_to(root)):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): cases.prepare(root)
            self.assertEqual(before, {str(p.relative_to(root)):p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_mcopy_fill_has_independent_output_and_equal_paths_across_contents(self):
        cases = self.load()
        for size, copies, code_length in ((4096, 12, 172), (24576, 15, 211)):
            maps, codes = [], []
            for seed in (0, 0x5b, 0x7f):
                row = cases.make_case('create', 'filled_runtime', runtime_bytes=size, fill_byte=seed)
                program = row['request']['program']; code = bytes.fromhex(program['code_hex'][2:])
                result, counts = interpret_return(code); self.assertEqual(result, bytes([seed])*size)
                self.assertEqual(len(code), code_length); self.assertEqual(counts['opcode:0x5e'], copies)
                self.assertEqual(counts['opcode:0x62'], 3*copies+3)
                self.assertEqual(program['transactions']['expected_runtime_hex'], '0x'+result.hex())
                for op, expected in program['expected_route_counts'].items(): self.assertEqual(counts.get(op, 0), expected)
                maps.append(counts); codes.append(code)
            self.assertEqual(maps[0], maps[1]); self.assertEqual(maps[1], maps[2])
            self.assertEqual(codes[0][:1]+codes[0][2:], codes[1][:1]+codes[1][2:])
            self.assertEqual(codes[1][:1]+codes[1][2:], codes[2][:1]+codes[2][2:])
        row = cases.make_case('call', 'return_runtime', runtime_bytes=24577)
        result, counts = interpret_return(bytes.fromhex(row['request']['program']['code_hex'][2:]))
        self.assertEqual(result, bytes(24577)); self.assertTrue(row['request']['expected_user_success'])
        self.assertNotIn('expected_runtime_hex', row['request']['program']['transactions'])

    def test_typed_bounds_and_unknown_shapes_are_rejected(self):
        cases = self.load()
        for kwargs in [dict(kind='unknown'), dict(kind=True), dict(shape='unknown'), dict(shape=[]),
                       dict(count=0), dict(count=True), dict(count=2049), dict(block_count=True), dict(block_count=33),
                       dict(gas_limit=True), dict(gas_limit=20999), dict(gas_limit=15_000_001),
                       dict(runtime_bytes=True), dict(runtime_bytes=-1), dict(runtime_bytes=24577),
                       dict(kind='call', runtime_bytes=24578), dict(fill_byte=0),
                       dict(shape='filled_runtime', runtime_bytes=0, fill_byte=0),
                       dict(shape='filled_runtime', fill_byte=True), dict(shape='filled_runtime', fill_byte=1),
                       dict(shape='filled_runtime'), dict(kind='call', shape='filled_runtime', fill_byte=0)]:
            defaults = dict(kind='create', shape='return_runtime', runtime_bytes=4096); defaults.update(kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError): cases.make_case(**defaults)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'), 'set frozen native for followup qualification')
    def test_all_38_native_inclusion_state_gas_and_opcode_routes(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); cases.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                p = row['parameters']; count, blocks = p['count_per_block'], p['block_count']
                folder = root/row['sample_id']; folder.mkdir()
                result = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(root/'cases'/row['request_path']), '--output', str(folder/'input.json'), '--output-bincode', str(folder/'input.bin'), '--report', str(folder/'native.json'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                self.assertEqual(result.returncode, 0, f"{row['sample_id']}: {result.stderr.decode()}")
                report = json.loads((folder/'native.json').read_text()); guest = json.loads((folder/'input.json').read_text())
                request = json.loads((root/'cases'/row['request_path']).read_text()); users = []
                self.assertEqual(report['transaction_count'], blocks*(count+1))
                user_gas = 0
                for b, receipts in enumerate(report['receipts_by_block']):
                    self.assertEqual(len(receipts), count+1)
                    self.assertTrue(all(r['success'] and r['gas_used'] <= (1_000_000 if i == 0 else p['transaction_gas_limit']) for i, r in enumerate(receipts)))
                    charged = sum(r['gas_used'] for r in receipts[1:]); user_gas += charged
                    self.assertEqual(report['blocks'][b]['user_transaction_gas_used'], charged)
                    self.assertEqual(report['blocks'][b]['block_gas_used'], charged+receipts[0]['gas_used'])
                    self.assertEqual(len(guest['witnesses'][b]['block']['body']['transactions']), count+1)
                    users.extend(report['transactions']['blocks'][b]['users'])
                self.assertEqual(user_gas, report['user_transaction_gas_used_total'])
                self.assertEqual([u['nonce'] for u in users], list(range(count*blocks)))
                self.assertEqual(sum(b['exceptional_instructions'] for b in report['observations_by_block']), 0)
                for op, expected in request['program']['expected_route_counts'].items(): self.assertEqual(report['observed_route_counts'].get(op, 0), expected)
                if p['transaction_kind'] == 'create':
                    self.assertTrue(all(u['to'] is None and u['create_address'] is not None and u['poststate']['nonce'] == 1 for u in users))
                    runtime = request['program']['transactions']['expected_runtime_hex']
                    initial_codes = guest['witnesses'][0]['witness']['codes']
                    self.assertEqual(len(initial_codes), 2)  # Anchor and existing checkpoint STOP.
                    self.assertEqual(initial_codes[-1], '0x00')
                    self.assertNotIn(request['program']['code_hex'], initial_codes)
                    if p['returned_runtime_bytes'] > 0:
                        # One returned zero byte shares the checkpoint code; it is
                        # already required by the honest initial witness.
                        if runtime != '0x00': self.assertNotIn(runtime, initial_codes)
                        for witness in guest['witnesses'][1:]: self.assertIn(runtime, witness['witness']['codes'])
                else:
                    self.assertTrue(all(u['create_address'] is None for u in users))
                if row['reference_role'] == 'previously_observed_reference_replay':
                    original = cases.original.make_case('create', 'return_runtime', block_count=blocks, runtime_bytes=p['returned_runtime_bytes'], gas_limit=8_000_000)
                    original_file = folder/'original-request.json'; original_file.write_bytes(cases.encoded(original['request']))
                    replay = subprocess.run([os.environ['NATIVE_FIXTURE_BIN'], '--input', str(original_file), '--output', str(folder/'reference.json'), '--output-bincode', str(folder/'reference.bin'), '--execution-profile', 'evm-gas-only'], capture_output=True, timeout=180)
                    self.assertEqual(replay.returncode, 0, replay.stderr.decode())
                    self.assertEqual((folder/'input.bin').read_bytes(), (folder/'reference.bin').read_bytes())


if __name__ == '__main__': unittest.main()
