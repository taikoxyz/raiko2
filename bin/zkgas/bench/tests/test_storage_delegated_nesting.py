"""Actual-byte delegated-frame checks and immutable pre-change CALL identities."""
from collections import Counter
import copy
import hashlib
import itertools
import inspect
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_storage_dirty_keys import trajectory, NATIVE_SHA256
from test_storage_mutation import decode, execute

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))
import storage_caught_nesting as common

BASE = 0x3000000000000000000000000000000000010000
LEAF = f'0x{BASE:040x}'
GRID = tuple(itertools.product(('stop', 'revert'), (16, 1024), (1, 4), ('zero', 'full')))
# Independently reviewed catalog captured before shared-helper edits. Tuple fields:
# args, canonical construction request SHA256, leaf runtime SHA256, full core row SHA256.
CALL_GOLDENS = [[['stop', 16, 1, 'zero'],
  'a80c5497b028c275b20592a281115a49cdd36dfd1b2aaf41d628c7337c95c2fe',
  'bcfbddd822dacc811a5eaecdfafb44876477e517ad741197eb091f1d5b1d1350',
  '05e19a8fa1e8e51eee65b51d211eeb528fc84d966dc046ab956312946e6ee9a8'],
 [['stop', 16, 1, 'full'],
  'ca6c94d02bde9949afeb047c3af46cbffa3a4b01d1225087fbbdbb23e0c2f9d0',
  '08ac1b01595762bef630f6d129dd15898c457cebdcc9c0d4d521e35e0302bf46',
  '88f94baca1a0605eda96cbdf74378fb718f5d5f436ea14601c079742c9d4a653'],
 [['stop', 16, 4, 'zero'],
  'd9ac58ef69ad0b15e79e0420a09080d7ff537f92f92de1fa84c853a7472d0220',
  'bcfbddd822dacc811a5eaecdfafb44876477e517ad741197eb091f1d5b1d1350',
  '963d301089736c8fcd6671e93ec31a60e0c4da53ecd04c5eccf85344e282c8d9'],
 [['stop', 16, 4, 'full'],
  'b1de739a3a510b38bc730347982069d60b7c79857afffc3b5fe8d5f238239239',
  '77d6157ebd2284c3a84fb2de5b646eb7942db14f4f9927b008e48e5038f15ef3',
  'e09676da6dad22c059f36f5e139f0a05ba96c7a436d9a8c09bb17e9c8a3fe82b'],
 [['stop', 1024, 1, 'zero'],
  '920efa68d256bf7024186061005ef6fbeab7bc63eaf4e40f37af7ccf940f8fd2',
  'eb2241f143b4b0c4964f4816183367cdd7478f3f5bc6e0ee432c0fa782b45938',
  'd6c8e2f5bc866ced073ad3afe94c6d44f6966efc69967bd67c78c29f390cf757'],
 [['stop', 1024, 1, 'full'],
  'a8e71aceda09e7eaaa0b2ffb840abe3dc225986edb9af3202f8b001da97916fb',
  'f2fbbb69e06e1aedcac9b85f61cf2113793072182c41d9c6230dcb74afab54a9',
  '8c93aafba0e39b54fe3d0d11c207e8f62bdeff5a096cd5d61ad2c3c5fbede87d'],
 [['stop', 1024, 4, 'zero'],
  '92b8505f9ed229a967c1bc95eac395e9045cbee54f665b718fb574ceba7e5169',
  'eb2241f143b4b0c4964f4816183367cdd7478f3f5bc6e0ee432c0fa782b45938',
  'f16d0d9fd430cdeabca46f87375fab9b243da36551d902a50f070e93b9c2bcd8'],
 [['stop', 1024, 4, 'full'],
  '1478dbf4fc189dde44a26cf3fde39747067caa491dc058c21b7320e5fe8f2560',
  '71f06c15f1f64256529f0e8253a573563f8f2644a5d7376fbe0c317c8cea34c8',
  '9c02082071d421cd71e52c38fa8a20686562578ac513df24bff1f0175bba507d'],
 [['revert', 16, 1, 'zero'],
  'efbf8df5636984147bbf547a33839d11ce3444293d7f312ecd8883f0753d073b',
  'ba92381032d6b7db0f02321ce05c94b1aea913c286b78dfccdd41db68e275dc8',
  'eeb64a513d050246d480cc2f57a98c454516301fa0a947e3cc44a941670021e3'],
 [['revert', 16, 1, 'full'],
  'fa1e0d22318c7f43b6778f5cc81dab417b5dcbbbbee480effcf229b52124052d',
  'cc4afc3359e306d87286a8fd233dc99a8986a264443a4daabaefe66b12e15f45',
  '44a411f7936555017dab7f032c987a915c6fe6bc3efa3fb1282f2709ee719f71'],
 [['revert', 16, 4, 'zero'],
  '0bda603124391b5c54476e32cc40ba8711010b6fb966cca27307b11b8fd0c459',
  'ba92381032d6b7db0f02321ce05c94b1aea913c286b78dfccdd41db68e275dc8',
  '0859bbe573fa71d9085f621ee958826e5a7d50a38697f8ad4399788515c857c2'],
 [['revert', 16, 4, 'full'],
  'bc6a69233705ae7888a8efb77d583d918d5cc47f422f0d59d18404a8e26dba49',
  'a2c54877d17cee1ce53a403dbdcb95ac1dd9af59caed817963129b4d21f351d4',
  'bdfc90725c43447e305539addc776dee7df431a41f31066c246f263ff1057447'],
 [['revert', 1024, 1, 'zero'],
  '4d71066e23335d21437de1876c8ab8977df411c78a976813fbb75eb8c7ea68a5',
  '7a0cb0b6c4c199e4fda10fe2a854fa3dc43110f7c7d7b60374f1e787a16eb01c',
  '21c7498c0c39b9d85076b3c8d15718e5d28c12748a87b1dfae3ef9f64ed55578'],
 [['revert', 1024, 1, 'full'],
  'c0c24b58f19b19440063cd12760dbc76704ed10e419c094c1f6307ee3ad4cde0',
  'e19fb83a444c9ea6233ee4b0460ffec39a12d1ef271456a8456479395971c78b',
  'ab810b89f8633c8238a005796b13ae001ed227c93224f81b67ea7bf13ca54b6f'],
 [['revert', 1024, 4, 'zero'],
  'c459893b7d013070f3d32a6760c4f1593c9b06401f03039dd71be9d203f63fac',
  '7a0cb0b6c4c199e4fda10fe2a854fa3dc43110f7c7d7b60374f1e787a16eb01c',
  '2b477831b8a3974fa8d234615eac342146b4947103d10c599984b5e1a54d5133'],
 [['revert', 1024, 4, 'full'],
  'aeb647c3d5b21d2abad4ad7f3956e5fe466ca27ef46d0a91e9a19aa8062136e3',
  'e19fb83a444c9ea6233ee4b0460ffec39a12d1ef271456a8456479395971c78b',
  'a8833d396649263ff95aec24dabfaeba6e8dacd43dda4b250654efd2c2acc490']]


def address(i, level=0):
    return f'0x{BASE+i+0x10000+level*0x100:040x}'


def qualify(test, row, execution=None, *, enforce_bounds=True):
    from storage_caught_vm import run_users
    p=row['parameters']; mode=p['mode']; k=p['keys_per_account']; d=p['call_depth']; r=p['rounds_per_key']
    test.assertIn((mode,k,d,p['workload_level']),GRID)
    program=row['request']['program']; contracts=program['prestate_contracts']
    owners=[address(i) for i in range(4)]
    expected=[LEAF]+[address(i,j) for j in range(d) for i in range(4)]
    test.assertEqual([c['address'] for c in contracts],expected)
    test.assertEqual(len(contracts),4*d+1)
    test.assertEqual(program['transactions'],dict(kind='call',count_per_block=4,gas_limit=p['transaction_gas_limit'],recipients=owners))
    test.assertEqual((program['code_hex'],program['input_hex'],program['block_count']),('0x00','0x',1))
    initial={}
    for c in contracts:
        owner=c['address']; i=owners.index(owner) if owner in owners else None
        state={int(key,16):int(value,16) for key,value in c['initial_storage'].items()}
        start=65537 if i is not None else 0
        test.assertEqual(state,({0:start}|dict.fromkeys(range(start,start+k),1)) if i is not None else {})
        initial[owner]=state
    records,states=run_users(row) if execution is None else execution
    test.assertEqual(states,initial)
    routes=Counter(); pairs=set(); writes=set(); collapsed=set(); spent=167355; prefixes=[]
    for i,record in enumerate(records):
        owner=owners[i]; root=record['result']; frames=record['frames']; leaf=frames[-1]
        test.assertEqual(record['recipient'],owner)
        test.assertEqual((root['terminal'],root['stack'],root['returndata']),(0,[],b''))
        test.assertEqual(len(frames),d+1)
        test.assertEqual((leaf['code_address'],leaf['storage_address']),(LEAF,owner))
        leaf_gas=trajectory(test,leaf['result'],states[owner],keys=k,rounds=r,mode=mode,account=0)
        test.assertEqual(record['state'],states[owner])
        required=leaf_gas['gross_gas']-21000+2301; requirements=[required]
        for _ in range(d):
            required=2618+max((64*required+62)//63,required+46); requirements.append(required)
        test.assertGreaterEqual(leaf['budget'],requirements[0])
        gross=leaf_gas['gross_gas']+2664*d; refund=2800*k if mode=='stop' else 0; applied=min(refund,gross//5)
        test.assertEqual((root['gas'],root['refund'],root['applied'],root['charged']),(gross,refund,applied,gross-applied))
        gas=p['gas_accounting_per_user']
        test.assertEqual(gas['minimum_frame_envelopes_leaf_to_root'],requirements)
        test.assertEqual(gas['leaf_refund_counter_before_terminal'],2800*k)
        test.assertEqual(p['expected_leaf_refund_counter_before_terminal'],2800*k)
        for field,value in dict(gross_gas=gross,refund_counter=refund,refund_applied=applied,charged_gas=gross-applied,refund_counter_before_terminal=refund).items():
            test.assertEqual(gas[field],value,field); test.assertEqual(p['expected_user_'+field],value,field)
        for j,frame in enumerate(frames[:-1]):
            test.assertEqual((frame['code_address'],frame['storage_address']),(address(i,j),owner))
            call=frame['call']; target=address(i,j+1) if j+1<d else LEAF
            test.assertEqual((call['kind'],call['target'],call['requested']),('delegatecall',target,12_000_000))
            test.assertEqual(call['forwarded'],min(12_000_000,call['available']-call['available']//64))
            test.assertGreaterEqual(call['forwarded'],requirements[d-j-1])
            test.assertEqual(call['child_remaining'],call['forwarded']-call['child_used'])
            test.assertGreaterEqual(call['parent_remaining_after_child'],46)
            test.assertEqual(call['child_status'],int(j+1<d or mode=='stop'))
            test.assertEqual((frame['ordinary_pre'],frame['ordinary_post'],frame['cold_target_fee']),(18,46,2600))
        for event in leaf['result']['reads']+leaf['result']['writes']:
            pairs.add((leaf['storage_address'],event[0])); collapsed.add((leaf['code_address'],event[0]))
        writes.update((leaf['storage_address'],event[0]) for event in leaf['result']['writes'])
        routes.update(root['counts']); prefixes.append(31_000_000-spent); spent+=root['charged']
    test.assertEqual(len(pairs),4*(k+1)); test.assertEqual(len(writes),4*k)
    test.assertEqual(len({(LEAF,key) for _,key in writes}),k)
    # Equal raw slots at four owners collapse incorrectly under code-address attribution.
    test.assertEqual(len(collapsed),k+1)
    test.assertNotEqual(pairs,collapsed)
    test.assertEqual(routes,+Counter(program['expected_route_counts']))
    test.assertEqual(routes['opcode:0xf4'],4*d); test.assertEqual(routes['opcode:0xf1'],0)
    test.assertEqual(routes['opcode:0x00'],4*(d+int(mode=='stop')))
    test.assertEqual(routes['opcode:0xfd'],4*int(mode=='revert'))
    fields=dict(account_count=4*d+1,total_account_count=4*d+1,storage_owner_count=4,wrapper_account_count=4*d,
        storage_owner_addresses=owners,leaf_owners=[LEAF],leaf_code_addresses=[LEAF],call_kind='delegatecall',
        expected_distinct_account_slot_pairs=4*(k+1),expected_distinct_write_pairs=4*k,
        expected_storage_reads=4*(2+6*k),expected_storage_writes=4*k*(2+2*r),expected_marker=k*r,
        expected_block_charged_gas=spent,header_prefix_remaining_gas=prefixes,near_full=spent>=29_700_000,
        start_phase_schedule=[65537]*4,final_phase_schedule=[65537]*4,
        expected_terminal='STOP',expected_leaf_terminal='STOP' if mode=='stop' else 'REVERT',
        expected_user_receipt_statuses=[True]*4,expected_call_count=4*d,
        expected_wrapper_stops=4*d,expected_leaf_stops=4*int(mode=='stop'),expected_leaf_reverts=4*int(mode=='revert'))
    for field,value in fields.items(): test.assertEqual(p[field],value,field)
    for field in ('phase_base_by_account','initial_phase_by_account','expected_final_phase_by_account','body_key_schedule_by_account'):
        test.assertEqual(set(p[field]),set(owners),field)
    for i,owner in enumerate(owners):
        test.assertEqual(p['phase_base_by_account'][owner],65536)
        test.assertEqual(p['initial_phase_by_account'][owner],65537)
        test.assertEqual(p['expected_final_phase_by_account'][owner],65537)
        test.assertEqual(p['body_key_schedule_by_account'][owner],list(range(65537,65537+k)))
    test.assertEqual(p['storage_entry_count_by_account'],{owner:len(state) for owner,state in initial.items()})
    test.assertEqual(len(p['account_roles']),len(contracts))
    for role,c in zip(p['account_roles'],contracts):
        owner=c['address']; test.assertEqual(role['address'],owner)
        test.assertEqual(role['storage_proof_required'],owner in owners)
        test.assertEqual(set(role['storage_targets']),set(c['initial_storage']))
        test.assertTrue(role['account_proof_required']); test.assertTrue(role['code_proof_required'])
        code=bytes.fromhex(c['code_hex'][2:])
        test.assertEqual((role['runtime_bytes'],role['runtime_sha256']),(len(code),hashlib.sha256(code).hexdigest()))
        test.assertLessEqual(len(code),24576)
        if owner==LEAF:
            test.assertEqual(role['role'],'leaf'); test.assertEqual(c['initial_storage'],{})
        else:
            test.assertEqual(role['role'],'wrapper'); i=role['chain']; j=role['level']
            test.assertEqual(role['storage_address'],address(i))
            test.assertEqual(role['next_target'],address(i,j+1) if j+1<d else LEAF)
            test.assertEqual(role['expected_child_status'],int(j+1<d or mode=='stop'))
    if enforce_bounds:
        test.assertLessEqual(spent,30_000_000); test.assertLessEqual(p['transaction_gas_limit'],15_000_000)
        for prefix in prefixes: test.assertGreaterEqual(prefix,p['transaction_gas_limit'])
    return records,states


class StorageDelegatedNestingTests(unittest.TestCase):
    def load(self):
        self.assertTrue((CASES/'storage_delegated_nesting.py').is_file(), 'delegated producer missing')
        import storage_delegated_nesting
        return storage_delegated_nesting

    def test_shared_keyword_and_immutable_prior_call_goldens(self):
        self.assertIn('call_kind',inspect.signature(common._wrapper_gas).parameters)
        self.assertEqual(common._wrapper_gas(call_kind='delegatecall'),(18,2600,46))
        for args,request,runtime,core in CALL_GOLDENS:
            row=common.make_case(*args); raw=row.pop('request')
            self.assertEqual(common.digest(common.encoded(raw)),request)
            self.assertEqual(row['parameters']['runtime_sha256'],runtime)
            self.assertEqual(common.digest(common.encoded(row)),core)
            self.assertEqual(common.make_case(*args),common.make_case(*args,call_kind='call'))

    def test_all_16_literal_trajectories_contexts_and_leaf_identity(self):
        m=self.load()
        import storage_dirty_keys as dirty
        for args in GRID:
            with self.subTest(args=args):
                row=m.make_case(*args); qualify(self,row); p=row['parameters']
                code,_=dirty._runtime(args[0],args[1],p['rounds_per_key'])
                self.assertEqual(row['request']['program']['prestate_contracts'][0]['code_hex'],'0x'+code.hex())
                full=m.make_case(*args[:3],'full')
                self.assertEqual(p['transaction_gas_limit'],full['parameters']['transaction_gas_limit'])
                self.assertEqual([c['initial_storage'] for c in row['request']['program']['prestate_contracts']],[c['initial_storage'] for c in full['request']['program']['prestate_contracts']])

    def test_full_plus_one_and_batch_boundary_envelopes(self):
        from storage_caught_vm import run_users
        self.load()
        for mode,k,d in itertools.product(('stop','revert'),(16,1024),(1,4)):
            full=common.full_rounds(mode,k,d,call_kind='delegatecall')
            for r in ((0,1,63,64,65,full,full+1) if k==16 else (0,1,full,full+1)):
                gas=common.gas_accounting(mode,k,d,r,call_kind='delegatecall')
                row=common._make_case(mode,k,d,r,full,call_kind='delegatecall')
                row['request']['program']['transactions']['gas_limit']=gas['transaction_envelope']
                record=run_users(row,limit=1)[0][0]
                trajectory(self,record['frames'][-1]['result'],record['state'],keys=k,rounds=r,mode=mode)
                self.assertEqual(record['result']['gas'],gas['gross_gas'])
            self.assertTrue(common.fits(mode,k,d,full,call_kind='delegatecall'))
            self.assertFalse(common.fits(mode,k,d,full+1,call_kind='delegatecall'))
            gas=common.gas_accounting(mode,k,d,full+1,call_kind='delegatecall')
            self.assertTrue(gas['transaction_envelope']>15_000_000 or 167355+4*gas['charged_gas']>30_000_000 or any(31_000_000-167355-j*gas['charged_gas']<gas['transaction_envelope'] for j in range(4)))
            if k==1024:
                for r in (63,64,65): self.assertFalse(common.fits(mode,k,d,r,call_kind='delegatecall'))
        with self.assertRaisesRegex(AssertionError,'SSTORE sentry'): execute(bytes.fromhex('600160005500'),{0:1},21000+6+2300)
        self.assertEqual(execute(bytes.fromhex('600160005500'),{0:1},21000+6+2301)['terminal'],0)

    def test_strict_domains_and_keyword_only(self):
        m=self.load()
        for args in ((True,),('STOP',),('stop',True),('stop',1),('stop',16,True),('stop',16,2),('stop',16,1,True),('stop',16,1,'FULL')):
            with self.assertRaises(ValueError): m.make_case(*args)
        for kind in (True,None,'CALL','DELEGATECALL',1,''):
            for fn,args in ((common._wrapper,(LEAF,1)),(common._wrapper_gas,()),(common.gas_accounting,('stop',16,1,0)),(common.fits,('stop',16,1,0)),(common.full_rounds,('stop',16,1)),(common._make_case,('stop',16,1,0,1)),(common.make_case,())):
                with self.subTest(fn=fn.__name__,kind=kind),self.assertRaises(ValueError): fn(*args,call_kind=kind)
        for r in (True,-1,1.0,65537,None):
            for fn in (common.gas_accounting,common.fits):
                with self.assertRaises(ValueError): fn('stop',16,1,r,call_kind='delegatecall')
            with self.assertRaises(ValueError): common._make_case('stop',16,1,r,1,call_kind='delegatecall')
            with self.assertRaises(ValueError): common._make_case('stop',16,1,0,r,call_kind='delegatecall')
        with self.assertRaises(TypeError): common.make_case('stop',16,1,'full','delegatecall')
        self.assertEqual(m.make_case(),m.make_case('stop',16,1,'full'))

    def test_actual_wrapper_operands_and_semantic_negatives(self):
        m=self.load()
        for mode,kind in itertools.product(('stop','revert'),('target','status','returndata','gas','range','extra-value','call-context','leaf-value','marker','metadata','storage-role','storage-context')):
            row=m.make_case(mode,16,1,'zero'); contracts=row['request']['program']['prestate_contracts']; c=contracts[1]
            code=bytearray.fromhex(c['code_hex'][2:]); instructions=list(decode(code))
            if kind=='target':
                at=next(at for at,op,_ in instructions if op==0x73); code[at+1:at+21]=(BASE+99).to_bytes(20,'big')
            elif kind in ('gas','range'):
                push=[(at,value) for at,op,value in instructions if op==0x62]
                at=push[4 if kind=='gas' else 0][0]; code[at+1:at+4]=(100 if kind=='gas' else 1).to_bytes(3,'big')
            elif kind=='status':
                at=next(at for at,op,_ in instructions if op==0xf4)+1; code[at+1:at+4]=int(mode=='revert').to_bytes(3,'big')
            elif kind=='returndata':
                at=next(at for at,op,_ in instructions if op==0x3d)+1; code[at+1:at+4]=(1).to_bytes(3,'big')
            elif kind=='extra-value': code[:0]=bytes.fromhex('62000001')
            elif kind=='call-context':
                at=next(at for at,op,_ in instructions if op==0xf4); code[at]=0xf1
            elif kind in ('leaf-value','marker'):
                leaf=contracts[0]; changed=bytearray.fromhex(leaf['code_hex'][2:])
                for at,op,value in decode(changed):
                    if (kind=='leaf-value' and op==0x62 and value==5) or (kind=='marker' and op==0x63): changed[at+1:at+1+op-0x5f]=(9 if kind=='leaf-value' else 1).to_bytes(op-0x5f,'big')
                leaf['code_hex']='0x'+changed.hex()
            elif kind=='metadata': row['parameters']['expected_distinct_account_slot_pairs']=17
            elif kind=='storage-role': row['parameters']['account_roles'][0]['storage_proof_required']=True
            else:
                contracts[0]['initial_storage']=contracts[1]['initial_storage']; contracts[1]['initial_storage']={}
            c['code_hex']='0x'+code.hex()
            with self.subTest(mode=mode,kind=kind),self.assertRaises((AssertionError,IndexError)): qualify(self,row)

    def test_context_and_refund_observation_mutants(self):
        from storage_caught_vm import run_users
        m=self.load()
        for mode in ('stop','revert'):
            row=m.make_case(mode,16,4,'zero'); original=run_users(row)
            for kind in ('code-owner','storage-owner','refund','leaf-refund','slot-collapse','forwarding','call-kind'):
                records,states=copy.deepcopy(original); leaf=records[0]['frames'][-1]
                if kind=='code-owner': leaf['code_address']=address(0)
                elif kind=='storage-owner': leaf['storage_address']=LEAF
                elif kind=='refund': records[0]['result']['refund']+=1
                elif kind=='leaf-refund':
                    leaf['result']['refund_before_revert' if mode=='revert' else 'refund']+=1
                elif kind=='slot-collapse':
                    for record in records: record['frames'][-1]['storage_address']=address(0)
                elif kind=='forwarding': records[0]['frames'][0]['call']['forwarded']-=1
                else: records[0]['frames'][0]['call']['kind']='call'
                with self.subTest(mode=mode,kind=kind),self.assertRaises(AssertionError): qualify(self,row,(records,states))

    def test_prepare_cli_and_16_exact_old_call_references(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); out=root/'cases'; self.assertEqual(m.prepare(out),16)
            catalog=json.loads((out/'catalog.json').read_text()); refs=catalog['call_references']
            self.assertEqual(catalog['corpus_id'],'unzen-storage-delegated-nesting-20261103')
            self.assertEqual(len(refs),16); self.assertEqual(len(catalog['cases']),16)
            for ref,(args,request,runtime,_) in zip(refs,CALL_GOLDENS):
                self.assertEqual((ref['mode'],ref['keys_per_account'],ref['depth'],ref['workload_level']),tuple(args))
                self.assertEqual((ref['request_sha256'],ref['runtime_sha256']),(request,runtime))
                self.assertFalse(ref['new_measurement'])
            for row in catalog['cases']:
                self.assertEqual(row['family'],'storage-delegated-nesting')
                raw=(out/row['request_path']).read_bytes(); self.assertEqual(common.digest(raw),row['request_sha256'])
            with self.assertRaises(FileExistsError): m.prepare(out)
            for bad in (None,True,'A'*64,'a'*63):
                with self.assertRaises(ValueError): m.prepare(root/'bad',model_id=bad)
                self.assertFalse((root/'bad').exists())
            for bad in (None,True,'cases'):
                with self.assertRaises(ValueError): m.prepare(bad)
            result=subprocess.run([sys.executable,'-S',str(CASES/'storage_delegated_nesting.py'),'--output-dir',str(root/'cli')],capture_output=True,text=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr); self.assertEqual(json.loads(result.stdout),{'prepared':16})

    @unittest.skipUnless(os.environ.get('STORAGE_DELEGATED_NATIVE_BIN'),'optional frozen native')
    def test_frozen_native_positive_and_accepted_operand_mutants(self):
        m=self.load(); binary=Path(os.environ['STORAGE_DELEGATED_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(),NATIVE_SHA256)
        # Retained paths are caller configured so failed native artifacts remain available.
        archive=Path(os.environ['STORAGE_DELEGATED_NATIVE_OUTPUT']); archive.mkdir(parents=True,exist_ok=False)
        for args,mutant in itertools.product((('stop',16,1),('revert',1024,4)),(False,True)):
            row=m.make_case(*args); p=row['parameters']; root=archive/(row['sample_id']+('-mutant' if mutant else '-positive')); root.mkdir()
            if mutant:
                leaf=row['request']['program']['prestate_contracts'][0]; code=bytearray.fromhex(leaf['code_hex'][2:])
                for at,op,value in decode(code):
                    if op==0x62 and value==7: code[at+1:at+4]=(9).to_bytes(3,'big')
                leaf['code_hex']='0x'+code.hex()
            source=root/'request.json'; source.write_text(json.dumps(row['request']))
            command=[str(binary),'--input',str(source),'--output',str(root/'input.json'),'--output-bincode',str(root/'input.bin'),'--report',str(root/'native.json'),'--execution-profile','evm-gas-only']
            try:
                process=subprocess.run(command,capture_output=True,timeout=180)
            except subprocess.TimeoutExpired as error:
                (root/'stdout.log').write_bytes(error.stdout or b''); (root/'stderr.log').write_bytes(error.stderr or b'')
                raise
            (root/'stdout.log').write_bytes(process.stdout); (root/'stderr.log').write_bytes(process.stderr)
            self.assertEqual(process.returncode,0,process.stderr.decode())
            report=json.loads((root/'native.json').read_text()); receipts=report['receipts_by_block'][0]
            self.assertEqual([r['success'] for r in receipts],[True]*5)
            self.assertEqual([r['gas_used'] for r in receipts],[167355]+[p['expected_user_charged_gas']]*4)
            self.assertEqual(report['terminal_counts'],{'stop':4*(args[2]+int(args[0]=='stop'))}|({'revert':4} if args[0]=='revert' else {}))
            for route,count in row['request']['program']['expected_route_counts'].items():
                self.assertEqual(report['observed_route_counts'].get(route,0),count)
                self.assertEqual(report['attempted_route_counts'].get(route,0),count)
            diagnostics=report['prestate_contracts']['initialized_storage']; self.assertEqual(len(diagnostics),4*args[2]+1)
            for diag in diagnostics:
                self.assertEqual(diag['initial_storage_root'],diag['final_storage_root'])
            if mutant:
                with self.assertRaises(AssertionError): qualify(self,row)
            else: qualify(self,row)


if __name__=='__main__': unittest.main()
