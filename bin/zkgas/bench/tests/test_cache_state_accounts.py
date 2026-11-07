"""Literal byte execution and tiny request tests; no native/prover invocation."""
from collections import Counter
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
CASES = BENCH / 'cases'


def producer():
    source = CASES / 'cache_state_accounts.py'
    assert source.is_file(), 'cache-state producer API is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('cache_state_accounts', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def execute(code, balances=None):
    """Separate byte decoder/interpreter: no assembler, count or gas imports."""
    instructions = {}; pc = 0
    while pc < len(code):
        start = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        instructions[start] = (op, int.from_bytes(code[pc:pc + width], 'big'), pc + width)
        pc += width
    destinations = {pc for pc, (op, _, _) in instructions.items() if op == 0x5b}
    stack = []; reads = []; warm = set(); counts = Counter(); pc = 0; gas = 21000
    for _ in range(1000):
        assert pc in instructions, 'invalid instruction boundary'
        op, immediate, pc = instructions[pc]
        counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            stack.append(immediate); gas += 3
        elif op == 0x31:
            address = stack.pop(); assert address < 1 << 160, 'Address20'
            gas += 100 if address in warm else 2600
            warm.add(address); reads.append(f'0x{address:040x}')
            stack.append((balances or {}).get(address, 0))
        elif op == 0x57:
            dest, condition = stack.pop(), stack.pop(); gas += 10
            # A false branch still has a source-valid common failure target.
            assert dest in destinations and code[dest:dest + 2] == b'\x5b\xfe', 'guard destination'
            if condition: pc = dest
        elif op == 0x5b: gas += 1
        elif op == 0xfe: raise AssertionError('INVALID guard reached')
        elif op == 0xf3:
            offset, size = stack.pop(), stack.pop()
            assert offset == size == 0 and not stack, 'empty return/stack'
            return dict(reads=reads, counts=dict(counts), gas=gas, memory_bytes=0)
        else: raise AssertionError(f'unexpected opcode {op:02x}')
    raise AssertionError('instruction bound')


class CacheStateAccountsTests(unittest.TestCase):
    def test_reference_tiny_request_literal_execution_and_order(self):
        m = producer(); row = m.make_case(user_count=3); p = row['parameters']
        request = row['request']; program = request['program']
        self.assertEqual(request['schema_version'], 1)
        self.assertTrue(request['expected_user_success'])
        self.assertEqual(program['code_hex'], '0x00')
        self.assertEqual(program['input_hex'], '0x')
        self.assertEqual(program['block_count'], 1)
        self.assertFalse({'access_list', 'storage_state', 'authorizations'} & program.keys())
        recipients = [f'0x{0x4300000000000000000000000000000000000000 + i:040x}' for i in range(1, 4)]
        self.assertEqual(program['transactions'], dict(kind='call', count_per_block=3,
            gas_limit=50000, recipients=recipients))
        expected_targets = ['0x' + hashlib.sha256(b'cache-state-absent-control\0' + i.to_bytes(4, 'big')).digest()[:20].hex()
                            for i in range(1, 25)]
        self.assertEqual(p['ordered_targets'], expected_targets)
        counts = Counter()
        for i, contract in enumerate(program['prestate_contracts']):
            self.assertEqual(set(contract), {'address', 'code_hex'})
            self.assertEqual(contract['address'], recipients[i])
            code = bytes.fromhex(contract['code_hex'][2:]); result = execute(code)
            self.assertEqual(len(code), 215)
            self.assertEqual(code[-2:], b'\x5b\xfe')
            self.assertEqual(result['reads'], expected_targets[8*i:8*i+8])
            self.assertEqual(result['gas'], 41934)
            self.assertEqual(result['memory_bytes'], 0)
            counts.update(result['counts'])
        actual_routes = {key: count for key, count in program['expected_route_counts'].items() if count}
        self.assertEqual(actual_routes, dict(counts))
        self.assertEqual(program['expected_route_counts']['opcode:0xfe'], 0)
        self.assertEqual(program['expected_route_counts']['opcode:0x5b'], 0)
        self.assertEqual(p['expected_block_charged_gas'], 167355 + 3 * 41934)
        self.assertEqual(p['expected_caller_nonce_after'], 3)
        raw = b''.join(bytes.fromhex(address[2:]) for address in expected_targets)
        self.assertEqual(p['ordered_targets_sha256'], hashlib.sha256(raw).hexdigest())
        self.assertIn(p['ordered_targets_sha256'], row['sample_id'])
        self.assertIn(p['ordered_targets_sha256'], row['corpus_id'])

    def test_explicit_targets_preserve_unsigned_user_envelopes_not_code(self):
        m = producer(); reference = m.make_case(user_count=2)
        targets = list(reversed(reference['parameters']['ordered_targets']))
        row = m.make_case(targets=targets, user_count=2); frozen = copy.deepcopy(row)
        a, b = reference['request']['program'], row['request']['program']
        self.assertEqual(a['transactions'], b['transactions'])
        for field in ('code_hex', 'input_hex', 'block_count', 'expected_route_counts'):
            self.assertEqual(a[field], b[field])
        self.assertNotEqual(a['prestate_contracts'], b['prestate_contracts'])
        self.assertNotEqual(reference['sample_id'], row['sample_id'])
        self.assertEqual(row, m.make_case(targets=tuple(targets), user_count=2))
        for i, contract in enumerate(b['prestate_contracts']):
            self.assertEqual(execute(bytes.fromhex(contract['code_hex'][2:]))['reads'], targets[8*i:8*i+8])
        targets[0] = targets[1]
        self.assertEqual(row, frozen)
        self.assertIsNone(row['parameters']['pressure_hypothesis_realized'])
        self.assertFalse(row['parameters']['computed_catalog_is_observed_placement'])

    def test_guard_values_jump_targets_and_malformed_byte_negatives(self):
        m = producer(); row = m.make_case(user_count=1)
        code = bytes.fromhex(row['request']['program']['prestate_contracts'][0]['code_hex'][2:])
        for address in row['parameters']['ordered_targets']:
            with self.subTest(address=address), self.assertRaisesRegex(AssertionError, 'INVALID guard'):
                execute(code, {int(address, 16): 1})
        bad = bytearray(code); bad[23:25] = (1).to_bytes(2, 'big')
        with self.assertRaisesRegex(AssertionError, 'guard destination'): execute(bytes(bad))
        with self.assertRaisesRegex(AssertionError, 'truncated PUSH'): execute(b'\x73\x01')
        bad = bytearray(code); bad[25] = 0x50
        with self.assertRaisesRegex(AssertionError, 'unexpected opcode'): execute(bytes(bad))

    def test_strict_types_counts_exclusions_and_no_collision_retry(self):
        m = producer(); targets = m.make_case(user_count=1)['parameters']['ordered_targets']
        for users in (True, False, None, '2', 1.0, 0, -1, 513):
            with self.subTest(users=users), self.assertRaises(ValueError): m.make_case(user_count=users)
        for value in ([], targets[:-1], targets+[targets[0]], targets[0], set(targets), {}, [True]*8,
                      [b'\0'*20]*8, [1]*8):
            with self.subTest(kind=type(value).__name__), self.assertRaises(ValueError):
                m.make_case(targets=value, user_count=1)
        for replacement in (targets[1], '0x00', '0X'+'11'*20, '0x'+'GG'*20, '0x'+'AA'*20,
                            '0x'+'00'*19+'01', '0x'+'00'*20, '0x'+'66'*20,
                            '0x'+'88'*20, '0x'+'99'*20, '0x'+'33'*20,
                            '0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a',
                            '0x1670000000000000000000000000000000010001',
                            '0x0000777735367b36bc9b61c50022d9d0700db4ec',
                            '0x4300000000000000000000000000000000000001'):
            bad = list(targets); bad[0] = replacement
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                m.make_case(targets=bad, user_count=1)
        class Collision:
            def digest(self): return bytes.fromhex('66'*20) + bytes(12)
        with patch.object(m.hashlib, 'sha256', return_value=Collision()) as hashed:
            with self.assertRaises(ValueError): m.make_case(user_count=1)
            self.assertEqual(hashed.call_count, 8, 'no resampling/filtering/search')

    def test_fixed_full_shape_arithmetic_without_building_512_case(self):
        m = producer(); gas = m.gas_accounting()
        self.assertEqual(gas['gross_user_gas'], 41934)
        self.assertEqual(gas['charged_user_gas'], 41934)
        self.assertEqual(gas['calldata_floor_gas'], 21000)
        self.assertEqual(gas['expected_block_charged_gas'], 21637563)
        self.assertEqual(gas['transaction_gas_limit'], 50000)
        self.assertEqual(gas['header_gas_limit'], 31000000)
        self.assertEqual(gas['charged_gas_budget'], 30000000)
        self.assertEqual(gas['last_user_remaining_header_gas'], 9404371)
        self.assertLessEqual(gas['transaction_gas_limit'], gas['last_user_remaining_header_gas'])
        self.assertEqual(gas['refund_counter'], 0)
        self.assertEqual(gas['memory_expansion_gas'], 0)

    def test_tiny_export_existing_catalog_and_source_identity_contract(self):
        m = producer()
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'tiny'
            self.assertEqual(m.prepare(output, user_count=2), 1)
            catalog = json.loads((output/'catalog.json').read_bytes())
            self.assertEqual(len(catalog['cases']), 1)
            case = catalog['cases'][0]; raw = (output/case['request_path']).read_bytes()
            self.assertEqual(case['request_sha256'], hashlib.sha256(raw).hexdigest())
            self.assertEqual(json.loads(raw), m.make_case(user_count=2)['request'])
            self.assertEqual(catalog['generator_sha256'], hashlib.sha256(Path(m.__file__).read_bytes()).hexdigest())
            for name, module in m.source_modules().items():
                self.assertEqual(catalog['source_sha256'][name], hashlib.sha256(Path(module.__file__).read_bytes()).hexdigest())
            sys.path.insert(0, str(BENCH))
            spec = importlib.util.spec_from_file_location('cache_workload_contract', BENCH/'workload.py')
            workload = importlib.util.module_from_spec(spec); spec.loader.exec_module(workload)
            self.assertEqual(workload.load_catalog(output/'catalog.json'), catalog['cases'])
            with self.assertRaises(FileExistsError): m.prepare(output, user_count=2)
            fresh = Path(folder)/'rejected'
            with self.assertRaises(ValueError): m.prepare(fresh, user_count=2, model_id='bad')
            self.assertFalse(fresh.exists())
            request = json.loads(raw); request['program']['input_hex'] = '0x00'
            (output/case['request_path']).write_text(json.dumps(request))
            with self.assertRaisesRegex(Exception, 'request identity mismatch'):
                workload.load_catalog(output/'catalog.json')


if __name__ == '__main__':
    unittest.main()
