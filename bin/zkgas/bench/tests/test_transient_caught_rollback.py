"""Literal tiny caught-transient checks; no native/proving execution."""
import ast
from collections import Counter
import copy
from pathlib import Path
import sys
import types
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'


def load_case(name):
    """Execute current local dependencies from their actual source buffers."""
    path = CASES / (name + '.py')
    if name in sys.modules:
        return sys.modules[name]
    raw = path.read_bytes()
    module = types.ModuleType(name)
    module.__file__ = str(path)
    sys.modules[name] = module
    for node in ast.parse(raw).body:
        names = ([x.name for x in node.names] if isinstance(node, ast.Import)
                 else [node.module] if isinstance(node, ast.ImportFrom) else [])
        for dependency in names:
            if dependency and (CASES / (dependency + '.py')).is_file():
                load_case(dependency)
    exec(compile(raw, str(path), 'exec'), module.__dict__)
    return module


def replace_runtime(row, index, code):
    """Keep intentional byte mutants authenticated so execution, not hash drift, rejects them."""
    import hashlib
    row['request']['program']['prestate_contracts'][index]['code_hex']='0x'+bytes(code).hex()
    role='parent' if index==0 else 'leaf'
    row['parameters'][role+'_runtime_sha256']=hashlib.sha256(code).hexdigest()
    row['parameters'][role+'_runtime_bytes']=len(code)


class CaughtTransientTests(unittest.TestCase):
    def test_two_transactions_really_catch_and_restore_each_child(self):
        path = CASES / 'transient_caught_rollback.py'
        self.assertTrue(path.is_file(), 'caught transient composition producer is missing')
        producer = load_case('transient_caught_rollback')
        from transient_caught_rollback_vm import run_users
        row = producer.make_case(key_count=2, caught_calls=2, user_count=2,
                                 raw_key_layout={'L1': 1, 'L3': 0})
        users = run_users(row)
        contracts=row['request']['program']['prestate_contracts']
        self.assertEqual([c['address'] for c in contracts], ['0x'+'65'*20,'0x'+'67'*20])
        self.assertNotIn('0x'+'66'*20,[c['address'] for c in contracts])
        self.assertEqual(len(users), 2)
        routes = Counter()
        for user in users:
            self.assertEqual(user['terminal'], 0)
            self.assertEqual(user['stack'], [])
            self.assertEqual(user['transient_before'], {})
            self.assertEqual(user['transient_after'], {})
            self.assertEqual(user['parent_final_transient'], {0: 1})
            self.assertEqual(len(user['calls']), 2)
            self.assertEqual([call['status'] for call in user['calls']], [0, 0])
            self.assertEqual([call['terminal'] for call in user['calls']], [0xfd, 0xfd])
            self.assertTrue(all(call['returndata'] == b'' for call in user['calls']))
            self.assertTrue(all(call['forwarded'] > call['gas'] for call in user['calls']))
            self.assertEqual(user['refund'], 0)
            routes.update(user['counts'])
        self.assertEqual(routes, +Counter(row['request']['program']['expected_route_counts']))
        self.assertEqual(routes['opcode:0xf4'], 4)
        self.assertEqual(routes['opcode:0xfd'], 4)
        self.assertEqual(routes['opcode:0x00'], 2)
        self.assertEqual([u['gas'] for u in users], [26769, 26769])
        self.assertEqual(row['parameters']['gas_accounting_per_user']['gross_gas'], 26769)
        for user in users:
            self.assertEqual([c['target_fee'] for c in user['calls']], [2600, 100])
            self.assertEqual([c['gas'] for c in user['calls']], [836, 836])
            for call in user['calls']:
                self.assertEqual(call['forwarded'], min(call['requested'], call['available']-call['available']//64))
                self.assertEqual(call['remaining'], call['forwarded']-call['gas'])
                self.assertEqual(call['transient_before'], call['transient_after'])
                self.assertEqual(call['storage_owner'], row['parameters']['parent_owner'])
            reads = [e for e in user['transient_events'] if e['kind']=='read' and e['key']==0]
            self.assertEqual([e['value'] for e in reads], [0, 1, 1])
            query = int(row['parameters']['query_key'], 16)
            self.assertEqual([e['value'] for e in user['transient_events'] if e['kind']=='read' and e['key']==query], [0, 1, 0, 0, 1, 0])

    def test_emitted_scalar_gas_matches_literal_tiny_frames(self):
        from transient_caught_rollback_vm import run_users
        m = load_case('transient_caught_rollback')
        for k in (1, 2, 3):
            for calls in (1, 2, 3):
                for word in (0, (1 << 64)-1):
                    row = m.make_case(key_count=k, caught_calls=calls, user_count=1,
                                      raw_key_layout={'L1': word, 'L3': 0})
                    result, = run_users(row)
                    self.assertEqual(result['gas'], row['parameters']['expected_user_charged_gas'])
                    self.assertEqual(result['gross_gas'], row['parameters']['expected_user_gross_gas'])
                    self.assertEqual(result['counts'], +Counter(row['request']['program']['expected_route_counts']))
                    self.assertTrue(all(c['gas']==173*k+490 for c in result['calls']))
                    self.assertEqual(row['parameters']['gas_accounting_per_user']['ordinary_parent_gas'],131+169*calls)

    def test_strict_bounds_before_any_assembler(self):
        m = load_case('transient_caught_rollback')
        valid = dict(key_count=2, caught_calls=2, user_count=2, raw_key_layout={'L1': 0, 'L3': 0})
        from unittest.mock import patch
        bad = [(key, value) for key, values in [('key_count',(0,513,True,1.0)),
              ('caught_calls',(0,129,True,1.0)), ('user_count',(0,3,True,1.0)),
              ('raw_key_layout',(None, {'L1': 0}, {'L1': True,'L3': 0},
                                 {'L1': 1<<64,'L3': 0}, {'L1': 0,'L3': 1}))] for value in values]
        with patch.object(m.transient, '_runtime', side_effect=AssertionError('assembly must not start')):
            for key, value in bad:
                args = valid | {key: value}
                with self.subTest(key=key,value=value), self.assertRaises(ValueError):
                    m.make_case(**args)

    def test_wrong_call_guard_terminal_and_sentinel_are_rejected(self):
        from transient_caught_rollback_vm import decode, run_users
        m = load_case('transient_caught_rollback')
        original = m.make_case(key_count=2,caught_calls=2,user_count=2,raw_key_layout={'L1': 1,'L3': 0})
        contracts = original['request']['program']['prestate_contracts']
        parent = bytes.fromhex(contracts[0]['code_hex'][2:]); leaf = bytes.fromhex(contracts[1]['code_hex'][2:])
        instructions = list(decode(parent)); call = next(i for i,x in enumerate(instructions) if x[1]==0xf4)
        changes = [(0, instructions[call][0], 0xf1, 'CALL changes storage owner'),
                   (0, instructions[call+1][0]+3, 1, 'guard INVALID'),
                   (1, next(pc for pc,op,_ in decode(leaf) if op==0xfd), 0, 'child STOP'),
                   (1, next(pc for pc,op,_ in decode(leaf) if op==0xfd), 0xfe, 'guard INVALID')]
        # Guard after returned query must demand0, not1; sentinel must be written1.
        read = next(i for i in range(call+1,len(instructions)) if instructions[i][1]==0x5c)
        changes.append((0,instructions[read+1][0]+3,1,'guard INVALID'))
        write = next(i for i,x in enumerate(instructions) if x[1]==0x5d)
        changes.append((0,instructions[write-2][0]+3,0,'guard INVALID'))
        for contract, offset, value, error in changes:
            row=copy.deepcopy(original);code=bytearray(parent if contract==0 else leaf);code[offset]=value
            replace_runtime(row,contract,code)
            with self.subTest(contract=contract,offset=offset), self.assertRaisesRegex(AssertionError,error):
                run_users(row)

    def test_child_out_of_gas_is_not_a_successful_caught_revert(self):
        from transient_caught_rollback_vm import decode, run_users
        row=load_case('transient_caught_rollback').make_case(key_count=2,caught_calls=2,user_count=1,raw_key_layout={'L1':0,'L3':0})
        contract=row['request']['program']['prestate_contracts'][0];code=bytearray.fromhex(contract['code_hex'][2:]);ins=list(decode(code))
        call=next(i for i,x in enumerate(ins) if x[1]==0xf4);request=ins[call-1][0]
        code[request+1:request+4]=(100).to_bytes(3,'big');replace_runtime(row,0,code)
        with self.assertRaisesRegex(AssertionError,'out of gas'):
            run_users(row)

    def test_fixed_future_geometry_budget_is_scalar_only(self):
        # No K512/calls128 runtime, keyset, calldata or request is constructed.
        k, calls = 512, 128
        nonzero_ordinal_bytes = 510 + 257
        nonzero = nonzero_ordinal_bytes + 8*k
        zero = 32*k - nonzero
        intrinsic = 21000 + 4*zero + 16*nonzero
        floor = 21000 + 10*(zero+4*nonzero)
        leaf = 173*k+490
        parent = 131+169*calls
        memory = 3*(k+4)+(k+4)**2//512
        gross = intrinsic+parent+memory+3*k+100*(1+2*calls)+100+2600+100*(calls-1)+calls*leaf
        self.assertEqual((leaf,intrinsic,floor,memory,gross),(89066,144892,330730,2068,11611807))
        self.assertLess(gross,12_500_000)
        remaining = 12_501_000-gross
        self.assertEqual(remaining-remaining//64,875300)
        self.assertGreater(remaining-remaining//64,leaf)
        self.assertEqual(2_000_000+2*12_501_000,27_002_000)
        self.assertGreaterEqual(30_000_000-gross,12_501_000)

    def test_new_shared_options_are_strict_and_defaults_still_stop(self):
        transient=load_case('transient_read_catalogs');caught=load_case('storage_caught_nesting')
        from transient_caught_rollback_vm import decode
        from initcode_preparation import _Factory
        code,counts=transient._runtime(0,key_count=2)
        self.assertEqual(counts['opcode:0x00'],1)
        self.assertEqual(counts.get('opcode:0xfd',0),0)
        revert,routes=transient._runtime(0,key_count=2,terminal='revert')
        self.assertEqual(routes['opcode:0xfd'],1)
        self.assertEqual(routes.get('opcode:0x00',0),0)
        self.assertTrue(any(op==0xfd for _,op,_ in decode(revert)))
        for bad in (True,None,'return','invalid'):
            with self.assertRaises(ValueError):transient._runtime(0,key_count=2,terminal=bad)
        for args in ({'input_offset':True},{'input_size':-1},{'input_offset':1<<24},
                     {'input_size':131073},{'times':0},{'times':True}):
            with self.assertRaises(ValueError):caught._emit_call(_Factory(),caught.address(0),0,**args)

    def test_input_range_keyset_and_count_mutations_do_not_hide_in_rollback(self):
        from transient_caught_rollback_vm import run_users
        row=load_case('transient_caught_rollback').make_case(key_count=2,caught_calls=2,user_count=2,raw_key_layout={'L1':0,'L3':0})
        for kind in ('first_key_zero','duplicate','wrong_count','missing_second_user'):
            bad=copy.deepcopy(row)
            if kind in ('first_key_zero','duplicate'):
                data=bytearray.fromhex(bad['request']['program']['input_hex'][2:])
                data[:32]=bytes(32) if kind=='first_key_zero' else data[32:64]
                bad['request']['program']['input_hex']='0x'+data.hex()
            elif kind=='wrong_count':bad['parameters']['caught_calls']=3
            else:bad['request']['program']['transactions']['recipients'].pop()
            with self.subTest(kind=kind), self.assertRaises(AssertionError):run_users(bad)

    def test_declared_case_scope_and_core_metadata_must_match_actual_execution(self):
        from transient_caught_rollback_vm import run_users
        row=load_case('transient_caught_rollback').make_case(key_count=2,caught_calls=2,user_count=2,raw_key_layout={'L1':1,'L3':0})
        # The formerly accepted counterexample reduces both executable fields,
        # while preserving the complete two-user declaration and doubled routes.
        bad=copy.deepcopy(row)
        bad['request']['program']['transactions']['recipients'].pop()
        bad['request']['program']['transactions']['counts_by_block']=[0,1]
        with self.assertRaises(AssertionError):run_users(bad)

    def test_core_metadata_contradictions_fail_closed(self):
        from transient_caught_rollback_vm import run_users
        original=load_case('transient_caught_rollback').make_case(key_count=2,caught_calls=2,user_count=2,raw_key_layout={'L1':1,'L3':0})
        for field,value in [('all_user_transactions',True),('all_user_transactions',1),
                ('user_transactions_by_block',[0,1]),('user_transactions_by_block',[False,2]),
                ('parent_owner',original['parameters']['leaf_code_address']),
                ('leaf_code_address',original['parameters']['parent_owner']),
                ('expected_user_gross_gas',0),('expected_user_charged_gas',0),
                ('expected_user_refund_counter',True),('expected_user_refund_applied',1),
                ('expected_child_status',1),('expected_leaf_return_bytes',True),
                ('expected_leaf_terminal','STOP'),('expected_terminal','REVERT'),
                ('query_key','0x'+'00'*32),('parent_runtime_sha256','0'*64),
                ('leaf_runtime_bytes',1),('expected_user_receipt_statuses',[False,False])]:
            row=copy.deepcopy(original);row['parameters'][field]=value
            with self.subTest(field=field,value=value),self.assertRaises(AssertionError):run_users(row)
        for value in ({},{'opcode:0xf4':4},{'opcode:0xf4':True}):
            row=copy.deepcopy(original);row['request']['program']['expected_route_counts']=value
            with self.subTest(routes=value),self.assertRaises(AssertionError):run_users(row)
        for field,value in [('gross_gas',0),('charged_gas',True),('refund_counter',True),('leaf_execution_gas',1)]:
            row=copy.deepcopy(original);row['parameters']['gas_accounting_per_user'][field]=value
            with self.subTest(gas=field),self.assertRaises(AssertionError):run_users(row)

    def test_calldata_floor_must_fit_envelope_even_when_execution_fits(self):
        from transient_caught_rollback_vm import run_users
        row=load_case('transient_caught_rollback').make_case(key_count=32,caught_calls=1,user_count=1,raw_key_layout={'L1':(1<<64)-1,'L3':0})
        self.assertEqual(row['parameters']['expected_user_gross_gas'],38084)
        self.assertEqual(row['parameters']['expected_user_charged_gas'],39880)
        limit=38184
        row['request']['program']['transactions']['gas_limit']=limit
        row['parameters']['transaction_gas_limit']=limit
        with self.assertRaisesRegex(AssertionError,'charged calldata floor exceeds'):
            run_users(row)


if __name__ == '__main__':
    unittest.main()
