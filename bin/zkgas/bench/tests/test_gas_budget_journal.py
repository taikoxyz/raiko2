"""Finite gas-budget journal probes: independent concrete frames, storage and gas."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from ethereum_hash import keccak256
from test_storage_hash_composition import decode, execute, assert_semantics

CASES = Path(__file__).resolve().parents[1]/'cases'
PROFILES = ('dirty-direct', 'dirty-revert', 'mixed-revert')
# Static costs exclude memory expansion, storage, and the intrinsic transaction cost.
STATIC = {0:0, 1:3, 2:5, 0x14:3, 0x15:3, 0x20:36, 0x30:2, 0x35:3, 0x36:2,
          0x3d:2, 0x50:2, 0x51:3, 0x52:3, 0x56:8, 0x57:10, 0x5b:1,
          0xf4:100, 0xfd:0}


def independent_gas(result, users):
    static = sum(count*(3 if 0x60 <= op <= 0x7f else STATIC[op])
                 for op, count in result['counts'].items() if op not in (0x54, 0x55))
    # Reconstruct costs from actual values and per-user ordered access history.
    reads = sum(2100 if index == 0 else 100 for history in result['read_history'] for index, _ in enumerate(history))
    writes = 0; refund = 0
    for (depth, pc, key, before, value, original), count in result['writes'].items():
        fee = 100 if before == value or before != original else 20000 if original == 0 else 2900
        writes += fee*count
        if original == 0 and before != 0 and value == 0:
            # All original-zero clears occur in the successful outer frame.
            assert depth == 0; refund += (20000-100)*count
    memory = sum(3*((len(frame[3])+31)//32)+((len(frame[3])+31)//32)**2//512 for frame in result['frames'])
    gross = 21000*users+static+reads+writes+memory
    assert gross % users == 0 and refund % users == 0
    per_user = gross//users; per_refund = min(refund//users, per_user//5)
    return dict(gross_user_gas=per_user, refund_counter=refund//users, applied_refund=per_refund,
                charged_user_gas=per_user-per_refund)


def assert_journal(test, result, profile, rounds, users):
    kernel, layout = profile.split('-'); assert_semantics(test, result, kernel, layout, rounds, users)
    actual = Counter()
    for (depth, pc, key, before, value, original), count in result['writes'].items():
        test.assertEqual(key, 2); actual[depth,before,value,original] += count
    target = int(layout == 'revert')
    expected = Counter({(0,0,5,0):users, (target,5,7,0):users,
        (target,7,11,0):rounds*users, (target,11,7,0):rounds*users,
        (target,7,5,0):users, (0,5,0,0):users})
    test.assertEqual(actual, +expected)


class GasBudgetJournalTests(unittest.TestCase):
    def load(self):
        path = CASES/'gas_budget_journal.py'; self.assertTrue(path.is_file(), 'gas-budget journal producer is missing')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('journal_budget_cases', path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_twelve_case_matrix_exact_baselines_hashes_and_exclusive_prepare(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(m.prepare(root), 12)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by={r['sample_id']:r for r in rows}
            self.assertEqual({(r['parameters']['profile'],r['parameters']['packing'],r['parameters']['level']) for r in rows}, {(p,t,l) for p in PROFILES for t in (4,16) for l in ('zero','full')})
            self.assertEqual(len(by),12); self.assertEqual(len({r['request_sha256'] for r in rows}),12)
            self.assertEqual(catalog['frozen_model_id'],m.MODEL)
            for name, value in catalog['source_sha256'].items(): self.assertEqual(m.digest((CASES/name).read_bytes()),value)
            for row in rows:
                p=row['parameters']; raw=(root/row['request_path']).read_bytes(); request=json.loads(raw); program=request['program']
                self.assertEqual(m.digest(raw),row['request_sha256']); self.assertEqual(program['block_count'],1); self.assertEqual(program['input_hex'],'0x')
                self.assertEqual(program['transactions'],{'kind':'call','count_per_block':p['packing'],'gas_limit':p['transaction_gas_limit']})
                self.assertNotIn('storage_state',program); self.assertTrue(request['expected_user_success']); self.assertTrue(row['holdout_only']); self.assertFalse(row['actuals_used_for_fitting'])
                base=by[row['baseline_sample_id']]['parameters']; self.assertEqual((base['profile'],base['packing'],base['level']), (p['profile'],p['packing'],'zero'))
                self.assertEqual(base['transaction_gas_limit'],p['transaction_gas_limit']); self.assertEqual(p['all_rounds_per_proposal'],p['rounds_per_user']*p['packing'])
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other=Path(temporary)/'other';m.prepare(other,model_id='1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (root/'requests').iterdir()}, {p.name:p.read_bytes() for p in (other/'requests').iterdir()})

    def test_bounded_concrete_semantics_gas_and_unchanged_original_bytecode(self):
        m=self.load(); import storage_hash_composition as old
        self.assertEqual(keccak256(bytes(32)).hex(),'290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563')
        for profile in PROFILES:
            kind,layout=profile.split('-')
            for n in (0,64,128):
                code,counts=m._runtime(profile,n); original=old.make_case(kind,layout,n)
                self.assertEqual(code.hex(),original['request']['program']['code_hex'][2:])
                result=execute(code,transactions=2);assert_journal(self,result,profile,n,2)
                self.assertEqual({f'opcode:0x{op:02x}':count for op,count in result['counts'].items()}, {op:count*2 for op,count in counts.items() if count})
                expected=independent_gas(result,2);declared=m._gas(profile,n)
                for key,value in expected.items(): self.assertEqual(declared[key],value)
        self.assertEqual(m._gas('dirty-revert',0)['gross_user_gas'],44427)
        self.assertEqual(m._gas('dirty-revert',32768)['gross_user_gas'],7019403)
        self.assertEqual(m._gas('dirty-revert',32768)['charged_user_gas'],6999503)

    def test_full_maximal_batches_refund_eip150_and_fixed_layout(self):
        m=self.load()
        for profile in PROFILES:
            for packing in (4,16):
                row=m.make_case(profile,packing); p=row['parameters']; n=p['rounds_per_user'];gas=p['gas']; nxt=m._gas(profile,n+64)
                self.assertEqual(n%64,0); self.assertGreater(n,0)
                self.assertLessEqual(gas['charged_block_gas'],29_990_000);self.assertGreaterEqual(gas['charged_block_gas'],29_700_000)
                self.assertGreater(167355+packing*nxt['charged_user_gas'],29_990_000)
                self.assertEqual(gas['charged_block_gas'],167355+packing*gas['charged_user_gas'])
                self.assertEqual(gas['applied_refund'],19900);self.assertEqual(gas['refund_counter'],19900)
                self.assertGreaterEqual(p['transaction_gas_limit'],gas['gross_user_gas']+2301);self.assertLessEqual(p['transaction_gas_limit'],15_000_000)
                self.assertGreaterEqual(gas['minimum_native_remaining_before_user'],p['transaction_gas_limit'])
                if profile.endswith('revert'):
                    self.assertGreaterEqual(p['transaction_gas_limit'],(gas['gross_user_gas']*64+62)//63+1024)
                    self.assertEqual(p['forwarded_gas'],12_000_000)
                zero=m.make_case(profile,packing,'zero');z=zero['parameters'];self.assertEqual(z['transaction_gas_limit'],p['transaction_gas_limit'])
                self.assertEqual(z['gas']['applied_refund'],z['gas']['gross_user_gas']//5)
                code=bytes.fromhex(row['request']['program']['code_hex'][2:]);zcode=bytes.fromhex(zero['request']['program']['code_hex'][2:])
                self.assertEqual([(pc,op) for pc,op,_ in decode(code)],[(pc,op) for pc,op,_ in decode(zcode)])
                changed=[(int.from_bytes(a[2],'big'),int.from_bytes(b[2],'big')) for a,b in zip(decode(zcode),decode(code)) if a[2] != b[2]]
                self.assertEqual(sorted(changed),sorted([(0,n//64),(0,n)]))
                one,_=m._runtime(profile,64);zc=execute(zcode)['counts'];oc=execute(one)['counts']
                expected=Counter({op:zc[op]+(oc[op]-zc[op])*(n//64) for op in set(zc)|set(oc)})
                self.assertEqual({f'opcode:0x{op:02x}':count*packing for op,count in (+expected).items()}, {op:count for op,count in row['request']['program']['expected_route_counts'].items() if count})
                self.assertEqual(p['native_header_gas_limit'],31_000_000);self.assertEqual(p['charged_block_gas_budget'],30_000_000)
                self.assertEqual(row['request']['program']['expected_route_counts']['opcode:0x55'],packing*(4+2*n))
        self.assertGreater(m.make_case('dirty-revert',4)['parameters']['rounds_per_user'],32768)
        with self.assertRaises(ValueError): m.old.make_case('dirty','revert',34880)

    def test_wrong_read_write_loop_bound_and_marker_are_detected(self):
        m=self.load(); original,_=m._runtime('mixed-revert',128); decoded=decode(original)
        for kind in ('read-key','write-key','write-value','loop-bound','marker'):
            code=bytearray(original)
            if kind=='read-key':
                i=next(i for i,(_,op,_) in enumerate(decoded) if op==0x54);code[decoded[i-1][0]+3]=3
            elif kind=='write-key':
                i=next(i for i,(_,op,data) in enumerate(decoded) if op==0x62 and int.from_bytes(data,'big')==11);code[decoded[i+1][0]+3]=3
            elif kind=='write-value':
                pc=next(pc for pc,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==11);code[pc+3]=5
            elif kind=='loop-bound':
                pc=next(pc for pc,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==2 and original[pc+4]==0x14);code[pc+3]=1
            else:
                i=next(i for i,(_,op,data) in enumerate(decoded) if op==0x62 and int.from_bytes(data,'big')==128 and decoded[i-1][1]==2 and decoded[i+1][1]==0x52);code[decoded[i][0]+3]=160
            result=execute(bytes(code))
            if kind in ('read-key','write-key','write-value'):
                self.assertEqual(result['terminals'],[0]);self.assertEqual(result['counts'],execute(original)['counts'])
            else:self.assertEqual(result['terminals'],[0xfe])
            with self.subTest(kind=kind),self.assertRaises(AssertionError):assert_journal(self,result,'mixed-revert',128,1)

    def test_strict_domains_without_partial_output(self):
        m=self.load()
        for args in [('bad',4,'full'),(True,4,'full'),('dirty-direct',True,'full'),('dirty-direct',8,'full'),('dirty-revert',4,True),('mixed-revert',16,'bad')]:
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('',True,None,'A'*64,'g'*64):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'opt in frozen native/quote smoke')
    def test_native_full_spotchecks_receipt_gas_and_semantic_negatives(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            def native(request,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(request))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            for profile,packing,level in [('dirty-direct',4,'full'),('dirty-revert',4,'full'),('mixed-revert',16,'full'),('dirty-revert',4,'zero')]:
                row=m.make_case(profile,packing,level);result,folder=native(row['request'],f'{profile}-{packing}-{level}');self.assertEqual(result.returncode,0,result.stderr.decode());report=json.loads((folder/'native.json').read_text());p=row['parameters']
                receipts=report['receipts_by_block'][0];self.assertEqual(len(receipts),packing+1);self.assertTrue(all(r['success'] for r in receipts));self.assertEqual(receipts[0]['gas_used'],167355)
                self.assertEqual([r['gas_used'] for r in receipts[1:]],[p['gas']['charged_user_gas']]*packing);self.assertEqual(report['blocks'][0]['block_gas_used'],p['gas']['charged_block_gas'])
                for op,count in row['request']['program']['expected_route_counts'].items():self.assertEqual(report['observed_route_counts'].get(op,0),count)
                quote=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','gas-budget-journal-smoke','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(quote.returncode,0,quote.stderr.decode());self.assertEqual(json.loads((folder/'quote.jsonl').read_text())['quote']['model_id'],m.MODEL)
            # Gross consumed gas alone is insufficient: SSTORE requires gasleft>2300.
            row=m.make_case('dirty-direct',4);program=row['request']['program']
            old_limit=row['parameters']['gas']['gross_user_gas']+1024
            program['transactions']['gas_limit']=old_limit
            failed,folder=native(row['request'],'insufficient-sstore-sentry')
            self.assertNotEqual(failed.returncode,0);self.assertIn(b'expected_user_success',failed.stderr)
            self.assertFalse((folder/'input.bin').exists());self.assertFalse((folder/'native.json').exists())
            row['request']['expected_user_success']=False;program['expected_route_counts']={'opcode:0xfe':0}
            diagnostic,folder=native(row['request'],'sstore-sentry-diagnostic-only')
            self.assertEqual(diagnostic.returncode,0,diagnostic.stderr.decode());report=json.loads((folder/'native.json').read_text())
            self.assertEqual([(r['success'],r['gas_used']) for r in report['receipts_by_block'][0][1:]],[(False,old_limit)]*4)
            self.assertEqual(report['attempted_route_counts']['opcode:0x55']-report['observed_route_counts']['opcode:0x55'],4)
            row=m.make_case('dirty-revert',4,'zero');image,counts=m._runtime('dirty-revert',64)
            code=bytearray(image);decoded=decode(code)
            i=next(i for i,(_,op,data) in enumerate(decoded) if op==0x62 and int.from_bytes(data,'big')==128 and decoded[i-1][1]==2 and decoded[i+1][1]==0x52)
            code[decoded[i][0]+3]=160;program=row['request']['program'];program['code_hex']='0x'+code.hex()
            program['expected_route_counts']={name:0 for name in program['expected_route_counts']}
            program['expected_route_counts'].update({name:count*4 for name,count in counts.items()})
            result,folder=native(row['request'],'wrong-marker');self.assertNotEqual(result.returncode,0);self.assertFalse((folder/'input.bin').exists())



if __name__=='__main__':unittest.main()
