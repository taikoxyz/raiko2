"""Independent actual-byte checks for caught many-key leaf storage work."""
from collections import Counter
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
import unittest

from test_storage_dirty_keys import trajectory, MODEL, NATIVE_SHA256
from test_storage_mutation import decode, execute

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('stop', 'revert'), (16, 1024), (1, 4), ('zero', 'full')))
BASE = 0x3000000000000000000000000000000000010000


def address(i, level=None):
    return f'0x{BASE + i + (0 if level is None else 0x10000 + level * 0x100):040x}'


def qualify(test, row, execution=None, *, enforce_bounds=True):
    from storage_caught_vm import run_users
    p = row['parameters']; mode = p['mode']; k = p['keys_per_account']; d = p['call_depth']; r = p['rounds_per_key']
    test.assertIn((mode, k, d, p['workload_level']), GRID)
    program = row['request']['program']; contracts = program['prestate_contracts']
    expected_addresses = [address(i) for i in range(4)] + [address(i, level) for level in range(d) for i in range(4)]
    test.assertEqual([c['address'] for c in contracts], expected_addresses)
    test.assertEqual(len(contracts), 4 * (d + 1)); test.assertEqual(program['code_hex'], '0x00')
    test.assertEqual((program['input_hex'], program['block_count']), ('0x', 1))
    test.assertEqual(program['transactions'], dict(kind='call', count_per_block=4, gas_limit=p['transaction_gas_limit'], recipients=[address(i, 0) for i in range(4)]))
    test.assertIs(row['request']['expected_user_success'], True)
    initial = {}
    for i, c in enumerate(contracts):
        state = {int(key, 16): int(value, 16) for key, value in c['initial_storage'].items()}
        start = (i + 1) * 65536 + 1
        test.assertEqual(state, {0: start} | dict.fromkeys(range(start, start + k), 1) if i < 4 else {})
        initial[c['address']] = state
    records, states = run_users(row) if execution is None else execution
    test.assertEqual(states, initial, 'every complete leaf and wrapper map restores')
    routes = Counter(); spent = 167355; prefixes = []
    for i, record in enumerate(records):
        test.assertEqual(record['recipient'], address(i, 0)); root = record['result']
        test.assertEqual((root['terminal'], root['stack'], root['returndata']), (0, [], b''))
        test.assertEqual(len(record['frames']), d + 1)
        leaf = record['frames'][-1]; test.assertEqual(leaf['owner'], address(i))
        leaf_gas = trajectory(test, leaf['result'], states[address(i)], keys=k, rounds=r, mode=mode, account=i)
        required = leaf_gas['gross_gas'] - 21000 + 2301; requirements=[required]
        for _ in range(d):
            required=2621+max((64*required+62)//63,required+46); requirements.append(required)
        test.assertGreaterEqual(leaf['budget'],requirements[0])
        gross = leaf_gas['gross_gas'] + 2667 * d
        refund = 2800 * k if mode == 'stop' else 0; applied = min(refund, gross // 5)
        test.assertEqual((root['gas'], root['refund'], root['applied'], root['charged']), (gross, refund, applied, gross - applied))
        gas = p['gas_accounting_per_user']
        for field, value in dict(gross_gas=gross, refund_counter=refund, refund_applied=applied, charged_gas=gross-applied,
                                 refund_counter_before_terminal=2800*k).items():
            test.assertEqual(gas[field], value, field); test.assertEqual(p['expected_user_' + field], value)
        for level, frame in enumerate(record['frames'][:-1]):
            test.assertEqual(frame['owner'], address(i, level)); call = frame['call']
            target = address(i, level + 1) if level + 1 < d else address(i)
            test.assertEqual(call['target'], target)
            test.assertEqual(call['requested'], 12_000_000)
            test.assertEqual(call['forwarded'], min(12_000_000, call['available'] - call['available'] // 64))
            test.assertGreaterEqual(call['forwarded'], call['child_used'])
            test.assertGreaterEqual(call['forwarded'], requirements[d-level-1])
            test.assertEqual(call['child_remaining'], call['forwarded'] - call['child_used'])
            test.assertGreaterEqual(call['parent_remaining_after_child'], 46)
            test.assertEqual(call['child_status'], int(level + 1 < d or mode == 'stop'))
            test.assertEqual(frame['ordinary_pre'], 21); test.assertEqual(frame['ordinary_post'], 46)
            test.assertEqual(frame['cold_target_fee'], 2600)
        routes.update(root['counts']); prefixes.append(31_000_000 - spent); spent += root['charged']
    test.assertEqual(routes, +Counter(program['expected_route_counts']))
    test.assertEqual(routes['opcode:0xf1'], 4*d)
    test.assertEqual(routes['opcode:0x00'], 4*(d + int(mode == 'stop')))
    test.assertEqual(routes['opcode:0xfd'], 4*int(mode == 'revert'))
    fields = dict(account_count=4*(d+1), storage_owner_count=4, wrapper_account_count=4*d,
        expected_distinct_account_slot_pairs=4*(k+1), expected_distinct_write_pairs=4*k,
        expected_storage_reads=4*(2+6*k), expected_storage_writes=4*k*(2+2*r),
        expected_marker=k*r, expected_user_success=True, expected_user_receipt_statuses=[True]*4,
        expected_block_charged_gas=spent, header_prefix_remaining_gas=prefixes,
        expected_leaf_terminal='STOP' if mode == 'stop' else 'REVERT', expected_terminal='STOP',
        expected_call_count=4*d, expected_wrapper_stops=4*d, expected_leaf_stops=4*int(mode=='stop'),
        expected_leaf_reverts=4*int(mode=='revert'))
    for field, value in fields.items(): test.assertEqual(p[field], value, field)
    test.assertEqual(p['leaf_owners'], [address(i) for i in range(4)])
    test.assertEqual(p['storage_owner_addresses'], p['leaf_owners'])
    test.assertEqual(p['depth'], d)
    test.assertEqual(p['storage_entry_count_by_account'], {owner:len(state) for owner,state in initial.items()})
    test.assertFalse(p['shared_runtime']); test.assertTrue(p['shared_leaf_runtime'])
    test.assertEqual(p['wrapper_levels'], [[address(i, level) for i in range(4)] for level in range(d)])
    for role in p['account_roles']:
        test.assertEqual(role['storage_proof_required'], role['role'] == 'leaf')
        test.assertTrue(role['account_proof_required']); test.assertTrue(role['code_proof_required'])
        contract=next(c for c in contracts if c['address']==role['address'])
        code=bytes.fromhex(contract['code_hex'][2:])
        test.assertEqual((role['runtime_bytes'],role['runtime_sha256']), (len(code),hashlib.sha256(code).hexdigest()))
        test.assertLessEqual(len(code),24576)
        test.assertEqual(set(role['storage_targets']),set(contract['initial_storage']))
        if role['role']=='wrapper':
            level=role['level']; i=role['chain']
            test.assertEqual(role['next_target'],address(i,level+1) if level+1<d else address(i))
            test.assertEqual(role['expected_child_status'],int(level+1<d or mode=='stop'))
    if enforce_bounds:
        test.assertLessEqual(spent, 30_000_000); test.assertLessEqual(p['transaction_gas_limit'], 15_000_000)
        for prefix in prefixes: test.assertGreaterEqual(prefix, p['transaction_gas_limit'])
    return records, states


class StorageCaughtNestingTests(unittest.TestCase):
    def load(self):
        path = CASES / 'storage_caught_nesting.py'
        self.assertTrue(path.is_file(), 'caught nested storage producer is missing')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('storage_caught_nesting_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def test_all_16_literal_frames_and_old_leaf_identity(self):
        m = self.load()
        import storage_dirty_keys as dirty
        for args in GRID:
            with self.subTest(args=args):
                row = m.make_case(*args); qualify(self, row); p = row['parameters']
                code, _ = dirty._runtime(args[0], args[1], p['rounds_per_key'])
                for c in row['request']['program']['prestate_contracts'][:4]: self.assertEqual(c['code_hex'], '0x'+code.hex())
                full = m.make_case(*args[:3], 'full')
                self.assertEqual(p['transaction_gas_limit'], full['parameters']['transaction_gas_limit'])
                self.assertEqual([c['initial_storage'] for c in row['request']['program']['prestate_contracts']], [c['initial_storage'] for c in full['request']['program']['prestate_contracts']])

    def test_boundaries_r_plus_one_and_envelope_recurrence(self):
        m = self.load()
        from storage_caught_vm import run_users
        for mode, k, d in itertools.product(('stop', 'revert'), (16, 1024), (1, 4)):
            full = m.full_rounds(mode, k, d)
            # Batch boundaries use K16: K1024/R63 already exceeds the fixed12M child request.
            boundaries = (0, 1, 63, 64, 65, full, full+1) if k == 16 else (0, 1, full, full+1)
            for r in boundaries:
                gas = m.gas_accounting(mode, k, d, r); required = gas['leaf_gross_gas'] - 21000 + 2301
                for _ in range(d): required = 2621 + max((64*required+62)//63, required+46)
                self.assertEqual(gas['transaction_envelope'], 21000+required)
                row = m._make_case(mode, k, d, r, full)
                # R+1 and fixed loop boundaries execute under their own sufficient envelope.
                row['request']['program']['transactions']['gas_limit'] = gas['transaction_envelope']
                row['parameters']['transaction_gas_limit'] = gas['transaction_envelope']
                # One complete chain suffices here; all four owners are checked in the16 grid.
                record = run_users(row, limit=1)[0][0]
                trajectory(self, record['frames'][-1]['result'], record['state'], keys=k, rounds=r, mode=mode)
                self.assertEqual(record['result']['gas'], gas['gross_gas'])
            self.assertTrue(m.fits(mode, k, d, full)); self.assertFalse(m.fits(mode, k, d, full+1))
            g = m.gas_accounting(mode, k, d, full+1)
            self.assertTrue(g['transaction_envelope'] > 15_000_000 or 167355+4*g['charged_gas'] > 30_000_000 or any(31_000_000-167355-j*g['charged_gas'] < g['transaction_envelope'] for j in range(4)))
        with self.assertRaisesRegex(AssertionError, 'SSTORE sentry'):
            execute(bytes.fromhex('600160005500'), {0:1}, 21000+6+2300)
        self.assertEqual(execute(bytes.fromhex('600160005500'), {0:1}, 21000+6+2301)['terminal'], 0)

    def test_strict_domains(self):
        m = self.load()
        for args in ((True,), ('STOP',), ('stop',True), ('stop',1), ('stop',16,True), ('stop',16,2), ('stop',16,1,True), ('stop',16,1,'FULL')):
            with self.subTest(args=args), self.assertRaises(ValueError): m.make_case(*args)
        for fn in (m.gas_accounting, m.fits):
            for r in (True, -1, 1.0, 65537, None):
                with self.assertRaises(ValueError): fn('stop',16,1,r)
            for args in (('STOP',16,1,0), ('stop',True,1,0), ('stop',16,True,0), ('stop',16,2,0)):
                with self.assertRaises(ValueError): fn(*args)
        for args in (('STOP',16,1),('stop',True,1),('stop',16,True),('stop',16,2)):
            with self.assertRaises(ValueError): m.full_rounds(*args)
        self.assertEqual(m.make_case(), m.make_case('stop',16,1,'full'))

    def test_wrapper_operands_and_semantic_negatives(self):
        m = self.load()
        for mode, kind in itertools.product(('stop', 'revert'), ('target','status','gas','range','value','leaf-value','marker','metadata')):
            row = m.make_case(mode,16,1,'zero'); contracts=row['request']['program']['prestate_contracts']; c=contracts[4]
            code=bytearray.fromhex(c['code_hex'][2:]); instructions=list(decode(code))
            if kind=='target':
                at=next(at for at,op,_ in instructions if op==0x73); code[at+1:at+21]=(BASE+99).to_bytes(20,'big')
            elif kind in ('gas','value','range'):
                push=[(at,op,value) for at,op,value in instructions if op==0x62]
                at=push[5 if kind=='gas' else 4 if kind=='value' else 0][0]
                code[at+1:at+4]=(100 if kind=='gas' else 1).to_bytes(3,'big')
            elif kind=='status':
                at=next(at for at,op,_ in instructions if op==0xf1)+1; code[at+1:at+4]=int(mode=='revert').to_bytes(3,'big')
            elif kind in ('leaf-value','marker'):
                for leaf in contracts[:4]:
                    leaf_code=bytearray.fromhex(leaf['code_hex'][2:])
                    for at,op,value in decode(leaf_code):
                        if (kind=='leaf-value' and op==0x62 and value==5) or (kind=='marker' and op==0x63):
                            leaf_code[at+1:at+1+op-0x5f]=(9 if kind=='leaf-value' else value+1).to_bytes(op-0x5f,'big')
                    leaf['code_hex']='0x'+leaf_code.hex()
            else: row['parameters']['storage_owner_count']+=1
            c['code_hex']='0x'+code.hex()
            with self.subTest(mode=mode,kind=kind), self.assertRaises(AssertionError): qualify(self,row)

    def test_catalog_provenance_and_cli(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); out=root/'cases'; self.assertEqual(m.prepare(out),16)
            catalog=json.loads((out/'catalog.json').read_bytes()); rows=catalog['cases']; by={r['sample_id']:r for r in rows}
            self.assertEqual(catalog['corpus_id'],'unzen-storage-caught-nesting-20261103'); self.assertEqual(catalog['frozen_model_id'],MODEL)
            self.assertEqual(catalog['generator_sha256'],hashlib.sha256((CASES/'storage_caught_nesting.py').read_bytes()).hexdigest())
            self.assertEqual(catalog['source_sha256'],{name:hashlib.sha256((CASES/name).read_bytes()).hexdigest() for name in m.SOURCES})
            self.assertEqual(catalog['helper_source_sha256'],{name:hashlib.sha256(inspect.getsource(helper).encode()).hexdigest() for name,helper in m.helpers().items()})
            self.assertEqual(len(by),16); self.assertEqual(len({r['request_sha256'] for r in rows}),16)
            for row in rows:
                p=row['parameters']; self.assertEqual(row['sample_id'],f"storage-caught-nesting-{p['mode']}-k{p['keys_per_account']}-d{p['call_depth']}-r{p['rounds_per_key']}")
                self.assertIn(row['baseline_sample_id'],by); self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                self.assertEqual(hashlib.sha256((out/row['request_path']).read_bytes()).hexdigest(),row['request_sha256'])
            self.assertEqual(len(catalog['direct_references']),8)
            for ref in catalog['direct_references']:
                import storage_dirty_keys as dirty
                direct=dirty.make_case(ref['mode'],ref['keys_per_account'],ref['workload_level'])
                self.assertEqual(ref['sample_id'],direct['sample_id']); self.assertEqual(ref['request_sha256'],hashlib.sha256(m.encoded(direct['request'])).hexdigest())
            with self.assertRaises(FileExistsError): m.prepare(out)
            for bad in (None,True,'A'*64,'a'*63):
                with self.assertRaises(ValueError): m.prepare(root/'bad',model_id=bad)
                self.assertFalse((root/'bad').exists())
            for bad in (None,True,'cases'):
                with self.assertRaises(ValueError): m.prepare(bad)
            process=subprocess.run([sys.executable,'-S',str(CASES/'storage_caught_nesting.py'),'--output-dir',str(root/'cli')],capture_output=True,text=True,timeout=30)
            self.assertEqual(process.returncode,0,process.stderr); self.assertEqual(json.loads(process.stdout),{'prepared':16})

    @unittest.skipUnless(os.environ.get('STORAGE_CAUGHT_NATIVE_BIN'),'optional frozen native')
    def test_frozen_native(self):
        m=self.load(); binary=Path(os.environ['STORAGE_CAUGHT_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(),NATIVE_SHA256)
        for args, mutant in itertools.product((('stop',16,1),('revert',1024,4)),(False,True)):
            row=m.make_case(*args); p=row['parameters']
            if mutant:
                # Native accepts dirty7->9->5, but the literal trajectory must reject it.
                for leaf in row['request']['program']['prestate_contracts'][:4]:
                    code=bytearray.fromhex(leaf['code_hex'][2:])
                    for at,op,value in decode(code):
                        if op==0x62 and value==7: code[at+1:at+4]=(9).to_bytes(3,'big')
                    leaf['code_hex']='0x'+code.hex()
            with tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary); source=root/'request.json'; source.write_text(json.dumps(row['request']))
                process=subprocess.run([str(binary),'--input',str(source),'--output',str(root/'input.json'),'--output-bincode',str(root/'input.bin'),'--report',str(root/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                self.assertEqual(process.returncode,0,process.stderr.decode())
                report=json.loads((root/'native.json').read_text()); receipts=report['receipts_by_block'][0]
                self.assertEqual([r['success'] for r in receipts],[True]*5)
                self.assertEqual([r['gas_used'] for r in receipts],[167355]+[p['expected_user_charged_gas']]*4)
                self.assertEqual(report['terminal_counts'],{'stop':4*(args[2]+int(args[0]=='stop'))} | ({'revert':4} if args[0]=='revert' else {}))
                for route,count in row['request']['program']['expected_route_counts'].items():
                    self.assertEqual(report['observed_route_counts'].get(route,0),count)
                    self.assertEqual(report['attempted_route_counts'].get(route,0),count)
                diagnostics=report['prestate_contracts']['initialized_storage']
                self.assertEqual(len(diagnostics),4*(args[2]+1))
                for i,diag in enumerate(diagnostics):
                    self.assertEqual(diag['initial_storage_root'],diag['final_storage_root'])
                    self.assertEqual(diag['initial_storage_entry_count'],args[1]+1 if i<4 else 0)
                if mutant:
                    with self.assertRaises(AssertionError): qualify(self,row)
                else: qualify(self,row)


if __name__=='__main__': unittest.main()
