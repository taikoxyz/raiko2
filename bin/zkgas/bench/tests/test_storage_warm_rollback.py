"""Literal storage warm-set and ancestor-journal regressions (stdlib only)."""
from collections import Counter
import copy
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_storage_mutation import decode, execute
from test_storage_dirty_keys import events, NATIVE_SHA256

CASES = Path(__file__).resolve().parents[1] / 'cases'
GRID = tuple(itertools.product(('leaf', 'ancestor'), (16, 256), (2, 4), (False, True), ('zero', 'full')))
BASE = 0x3000000000000000000000000000000000010000


def address(i, level=None):
    return f'0x{BASE+i+(0 if level is None else 0x10000+level*0x100):040x}'


# Captured from independently reviewed prior catalogs BEFORE changing shared helpers.
OLD_GOLDENS = (('storage-caught-nesting-stop-k16-d1-r0',
  'a80c5497b028c275b20592a281115a49cdd36dfd1b2aaf41d628c7337c95c2fe',
  '18065108fadf979c6c2d32b02cdc964698765538bd084dff4b0f6d9a9190dbe9'),
 ('storage-caught-nesting-stop-k16-d1-r2107',
  'ca6c94d02bde9949afeb047c3af46cbffa3a4b01d1225087fbbdbb23e0c2f9d0',
  '8333d7aae7bef34ce7ab96fd4a09888107aabaa43f3743d3213d50e22649dde4'),
 ('storage-caught-nesting-stop-k16-d4-r0',
  'd9ac58ef69ad0b15e79e0420a09080d7ff537f92f92de1fa84c853a7472d0220',
  '21287bbc89b5a4d9ad90aa05467957c6b2e4898b134f183705af9b691d045fce'),
 ('storage-caught-nesting-stop-k16-d4-r2105',
  'b1de739a3a510b38bc730347982069d60b7c79857afffc3b5fe8d5f238239239',
  '77525979d836abd413040a878cf691be0f82fc09de61b0415f29dea85d0512dd'),
 ('storage-caught-nesting-stop-k1024-d1-r0',
  '920efa68d256bf7024186061005ef6fbeab7bc63eaf4e40f37af7ccf940f8fd2',
  '9d58643709a901038821578fae983e8093d6aed17a59f66a41df2a41c1c262a3'),
 ('storage-caught-nesting-stop-k1024-d1-r12',
  'a8e71aceda09e7eaaa0b2ffb840abe3dc225986edb9af3202f8b001da97916fb',
  'ee0a2d817eb0566e7f00773eed368561fd2ade220821405e3bbf8ace648f6cca'),
 ('storage-caught-nesting-stop-k1024-d4-r0',
  '92b8505f9ed229a967c1bc95eac395e9045cbee54f665b718fb574ceba7e5169',
  '334eeb40947f3983c76fe3c782cc82a164d60e9c4fadc322e7745bd9c36221c5'),
 ('storage-caught-nesting-stop-k1024-d4-r11',
  '1478dbf4fc189dde44a26cf3fde39747067caa491dc058c21b7320e5fe8f2560',
  '2c7c5084faa270c625e6b5eb33fad877b6402bc233490a78ef674631cfd23388'),
 ('storage-caught-nesting-revert-k16-d1-r0',
  'efbf8df5636984147bbf547a33839d11ce3444293d7f312ecd8883f0753d073b',
  'd8829b92064dc3378804a07edf63c58ae78ce3ad8222c942559549aaa9df0358'),
 ('storage-caught-nesting-revert-k16-d1-r2094',
  'fa1e0d22318c7f43b6778f5cc81dab417b5dcbbbbee480effcf229b52124052d',
  '9c79e87c9ce918ca942f31f260a62bff0aa72f990f4b84003b3bf8e0adec80ca'),
 ('storage-caught-nesting-revert-k16-d4-r0',
  '0bda603124391b5c54476e32cc40ba8711010b6fb966cca27307b11b8fd0c459',
  'd81a2eeb504e889eef90529c57de1a018632f60ed91de4980a1371b781e22246'),
 ('storage-caught-nesting-revert-k16-d4-r2092',
  'bc6a69233705ae7888a8efb77d583d918d5cc47f422f0d59d18404a8e26dba49',
  'fb892d6096a90dd1ae2e8ae24c45f45448f39c541baa304e075f08792c32c35f'),
 ('storage-caught-nesting-revert-k1024-d1-r0',
  '4d71066e23335d21437de1876c8ab8977df411c78a976813fbb75eb8c7ea68a5',
  '9c62d24c3726c389104384c48e2f21114c0d2031cdd7377c924f49791c723c4f'),
 ('storage-caught-nesting-revert-k1024-d1-r5',
  'c0c24b58f19b19440063cd12760dbc76704ed10e419c094c1f6307ee3ad4cde0',
  '411a5e02cbc54458d243b7f95b05c84e6a64cf7c459f9b55f712437993f69938'),
 ('storage-caught-nesting-revert-k1024-d4-r0',
  'c459893b7d013070f3d32a6760c4f1593c9b06401f03039dd71be9d203f63fac',
  '7524de3ea3b909a897b594b29d7a23bbfcbadf0425ee549784c47f4ac9514195'),
 ('storage-caught-nesting-revert-k1024-d4-r5',
  'aeb647c3d5b21d2abad4ad7f3956e5fe466ca27ef46d0a91e9a19aa8062136e3',
  'd4877a654936f4a0abfa1c34760fae02bf172650648ef8ddecb6019ee304c988'),
 ('storage-delegated-nesting-stop-k16-d1-r0',
  'b6f769fe6a64471cf23a330cf52cf615559452e88ec2b10aa3647e90796e6694',
  'a50e89b5ebd74b85d3a73c116de946148a7f3a070e701cc1101b093a76232e93'),
 ('storage-delegated-nesting-stop-k16-d1-r2107',
  '796a938b33269853ab4d933d966c584b7fee7b1ca3dd07a6d4dc67a79ac1fd8b',
  '0cd7085da15629971fe3a7bf9a549152928f5b11786b3209809a442a7b053533'),
 ('storage-delegated-nesting-stop-k16-d4-r0',
  'a4ac1a543ce012a89a9c2a7bb29eb3e64b06b04e2ee4dfff5b75c50e0ac58758',
  '4276e23426766e9ae5b92901120984314590eca0d2054bef0f97f7f1afc63413'),
 ('storage-delegated-nesting-stop-k16-d4-r2105',
  '3b7352554af2faff2a643ad734df14bc559f0a9f40e0dedbb342b4f9a603a0e1',
  '878e74c4e0a620579946ef0e7f8a0f82b7e0fe605e5d370daf30b593a25dbd96'),
 ('storage-delegated-nesting-stop-k1024-d1-r0',
  '48931935afba6d04627e74787edc321c595d5fa9f77e950cd6205c8676fff07b',
  '8fa70dd551156f77b10fe5e933b50db2a77d3c615c097e66d9fa3c8c38b56ac3'),
 ('storage-delegated-nesting-stop-k1024-d1-r12',
  '185b7443eca9131de17875ded1c54b76596fab1a4f2d85861bb5a8837dcbb372',
  'f5d0b3113d86ef55c2c01edbf82599652eba1a42ad2aee8f7b76e6a001afa7d8'),
 ('storage-delegated-nesting-stop-k1024-d4-r0',
  'deef7edc5c55c0df77dfe56b57ca15b4e638e7478866e1218d3103e6b11feb86',
  'b6015d7e31ab638858e241dd5f4136c8232cc49798d76a4a88fba8ab44c69ea4'),
 ('storage-delegated-nesting-stop-k1024-d4-r11',
  'd6fbafa92b4f05190e1000d88b22fd210af18377c3fed3be46e41646c0307d4c',
  '26feab5d033df6d88109eaa97df46e9e6f430aea6666e9d27e4468307c7ffa22'),
 ('storage-delegated-nesting-revert-k16-d1-r0',
  'ed0742a8ad3d419456345b414115add6253cc35e1c27fcd1807455ced03725e2',
  '69f5f3a96670ebb406e62224eab613b1de9b94490ea0d4f37d273174fdf4893e'),
 ('storage-delegated-nesting-revert-k16-d1-r2094',
  '95c1add79bab375da10472310d8b1dbdc5d6f49a67a40342f96d659a23b4828e',
  'bc7ef0864a056f91197280ed3cb197265bc11d3d3a59d3537fbeab29d61ce7ee'),
 ('storage-delegated-nesting-revert-k16-d4-r0',
  'f2caa09368ee977e5784d3d24acb7604ef5ae843063f139ee547c3a1f20ecc10',
  '6dc977ba5d9cc92f1df849bb835991d0c1a69095c44a5364a3a3317c79c0dfef'),
 ('storage-delegated-nesting-revert-k16-d4-r2092',
  'a3c83e217bf190c96c17ba8a7e787fe81a7b169f4b12798b635149f3856b03b6',
  'f73fc8c733a6e84edfd48aee8ff3b3092e7063eb15cdebe38dcdab18020a2cca'),
 ('storage-delegated-nesting-revert-k1024-d1-r0',
  '4dd8c628df0ca727f7198d80ce019e8f0a44555ad53ba5f88dd35b43cbaf4ef1',
  '27a9faf407ee28709df06a1d88777cb21b4f399aec71540c0620374464979844'),
 ('storage-delegated-nesting-revert-k1024-d1-r5',
  '64476fa3aaea8e3ea02ca569f233902438d0808c905f9e4ba3ec3e80cf700644',
  'fc605f1a1e1bdbdd71d2a33e9d6799804195ba2948009a0e33dcf09257011465'),
 ('storage-delegated-nesting-revert-k1024-d4-r0',
  'ae9ffe1b8d9f8e7dbca1604d1ea0194b3df58ea8918de6c9ac040104c39fcf78',
  'ae6c4b8452a3d526929b37fab48c5629dfd782134d22c7691175cd738706c5f8'),
 ('storage-delegated-nesting-revert-k1024-d4-r5',
  '6a91306b288fd2bbf0969057053f7a28f4d79d0e50e504bb13af8f70c13be515',
  '9fd5870adef99ecedacf2c40c29926c465d851e206d2f2e30cb87f0660ad55d4'))


def qualify(test, row, execution=None, *, enforce_bounds=True):
    from storage_warm_rollback_vm import run_users
    p=row['parameters']; mode=p['exit_mode']; k=p['keys_per_account']; d=p['depth']; warm=p['prewarm']; r=p['rounds_per_key']
    test.assertIn((mode,k,d,warm,p['workload_level']),GRID)
    program=row['request']['program']; contracts=program['prestate_contracts']
    owners=[address(i,0) for i in range(4)]; expected=[address(0)]+[address(i,j) for j in range(d) for i in range(4)]
    test.assertEqual([c['address'] for c in contracts],expected)
    test.assertEqual((program['code_hex'],program['input_hex'],program['block_count']),('0x00','0x',1))
    test.assertEqual(program['transactions'],dict(kind='call',count_per_block=4,gas_limit=p['transaction_gas_limit'],recipients=owners))
    test.assertIs(row['request']['expected_user_success'],True)
    initial={}
    for c in contracts:
        state={int(a,16):int(b,16) for a,b in c['initial_storage'].items()}
        test.assertEqual(state,({0:65537}|dict.fromkeys(range(65537,65537+k),1)) if c['address'] in owners else {})
        initial[c['address']]=state
    records,states=run_users(row) if execution is None else execution
    test.assertEqual(states,initial); counts=Counter(); access=set(); written=set(); spent=167355; prefixes=[]
    traversal=[(0,65537,not warm)]+[(key,1,not warm) for key in range(65537,65537+k)]
    for i,record in enumerate(records):
        frames=record['frames']; root=record['result']; leaf=frames[-1]['result']; owner=owners[i]
        test.assertEqual(len(frames),d+1)
        test.assertEqual((root['terminal'],root['stack'],root['returndata'],root['refund'],root['applied']),(0,[],b'',0,0))
        test.assertEqual((frames[-1]['code_address'],frames[-1]['storage_address']),(address(0),owner))
        reads=[(0,65537,not warm)]; writes=[]
        for key in range(65537,65537+k):
            reads.extend(((key,1,not warm),(key,5,False))); writes.append((key,1,5,1,False))
        for key in range(65537,65537+k):
            reads.append((key,5,False))
            for _ in range(r): writes.extend(((key,5,7,1,False),(key,7,5,1,False)))
            reads.append((key,5,False))
        for key in range(65537,65537+k):
            reads.extend(((key,5,False),(key,1,False))); writes.append((key,5,1,1,False))
        reads.append((0,65537,False))
        events(test,leaf['reads'],reads,'leaf ordered reads'); events(test,leaf['writes'],writes,'attempted leaf writes')
        test.assertEqual((leaf['terminal'],len(leaf['memory']),leaf['stack']),(0xfd if mode=='leaf' else 0,192,[]))
        test.assertEqual([int.from_bytes(leaf['memory'][j:j+32],'big') for j in (0,64,96,128,160)],[k,65537,65536+k,r,k*r])
        test.assertEqual(leaf.get('refund_before_revert',leaf['refund']),2800*k)
        test.assertEqual(leaf['refund'],0 if mode=='leaf' else 2800*k)
        top=frames[0]; test.assertEqual(len(top['result']['memory']),32)
        expected_top=([(a,b,True) for a,b,_ in traversal] if warm else [])+traversal
        events(test,top['reads'],expected_top,'top pre/post warmth')
        for j,frame in enumerate(frames[:-1]):
            test.assertEqual((frame['code_address'],frame['storage_address']),(address(i,j),owner))
            call=frame['call']; test.assertEqual(call['target'],address(i,j+1) if j+1<d else address(0))
            test.assertEqual(call['requested'],12_000_000)
            test.assertEqual(call['forwarded'],min(12_000_000,call['available']-call['available']//64))
            test.assertGreaterEqual(call['forwarded'],call['child_used'])
            test.assertEqual(call['child_remaining'],call['forwarded']-call['child_used'])
            expected_status=0 if (mode=='leaf' and j==d-1) or (mode=='ancestor' and j==d-2) else 1
            test.assertEqual(call['child_status'],expected_status)
            test.assertEqual(frame['result']['terminal'],0xfd if mode=='ancestor' and j==d-1 else 0)
            test.assertEqual(frame['refund_before_terminal'],2800*k if mode=='ancestor' and j==d-1 else 0)
            test.assertEqual(frame['cold_target_fee'],2600)
        # The literal opcode stream independently determines ordinary gas.
        costs={0:0,1:3,0x14:3,0x15:3,0x3d:2,0x51:3,0x52:3,0x54:0,0x55:0,0x56:8,0x57:10,0x5b:1,0xf4:2600,0xfd:0}
        ordinary=sum(n*(3 if 0x60<=int(op[-2:],16)<=0x7f else costs[int(op[-2:],16)]) for op,n in root['counts'].items())
        storage_reads=sum(2100 if cold else 100 for _,_,cold in leaf['reads']+top['reads'])
        gross=21000+ordinary+18+3+storage_reads+k*(3000+200*r)
        test.assertEqual((root['gas'],root['charged']),(gross,gross))
        for field,value in dict(gross_gas=gross,charged_gas=gross,refund_counter=0,refund_applied=0,refund_counter_before_terminal=0).items():
            test.assertEqual(p['expected_user_'+field],value); test.assertEqual(p['gas_accounting_per_user'][field],value)
        required=leaf['gas']-21000+2301
        requirements=[required]
        for frame in reversed(frames[:-1]):
            required=frame['pre_gas']+max((64*required+62)//63,required+frame['post_gas'])
            requirements.append(required)
        test.assertEqual(p['gas_accounting_per_user']['minimum_frame_envelopes_leaf_to_root'],requirements)
        test.assertGreaterEqual(p['transaction_gas_limit'],21000+required)
        for frame,need in zip(reversed(frames[1:]),requirements[:-1]): test.assertGreaterEqual(frame['budget'],need)
        for key,_,_ in leaf['reads']+top['reads']: access.add((owner,key))
        for key,*_ in leaf['writes']: written.add((owner,key))
        counts.update(root['counts']); prefixes.append(31_000_000-spent); spent+=gross
    test.assertEqual(counts,+Counter(program['expected_route_counts']))
    test.assertEqual((counts['opcode:0xf4'],counts['opcode:0xfd'],counts['opcode:0x00']),(4*d,4,4*d))
    fields=dict(account_count=4*d+1,total_account_count=4*d+1,storage_owner_count=4,storage_owner_addresses=owners,
        expected_distinct_account_slot_pairs=4*(k+1),expected_distinct_write_pairs=4*k,
        expected_storage_reads=4*(2+6*k+(1+int(warm))*(k+1)),expected_storage_writes=4*k*(2+2*r),
        expected_block_charged_gas=spent,header_prefix_remaining_gas=prefixes,expected_user_refund_counter_before_terminal=0,
        expected_leaf_refund_counter_before_terminal=2800*k,expected_ancestor_refund_counter_before_terminal=2800*k if mode=='ancestor' else 0,
        expected_all_frame_terminal_counts={'STOP':4*d,'REVERT':4},expected_user_receipt_statuses=[True]*4)
    for field,value in fields.items(): test.assertEqual(p[field],value,field)
    test.assertEqual((len(access),len(written)),(4*(k+1),4*k))
    for role in p['account_roles']:
        c=next(c for c in contracts if c['address']==role['address']); code=bytes.fromhex(c['code_hex'][2:])
        test.assertEqual((role['runtime_bytes'],role['runtime_sha256']),(len(code),hashlib.sha256(code).hexdigest()))
        test.assertEqual(role['storage_proof_required'],role['address'] in owners)
        test.assertEqual(set(role['storage_targets']),set(c['initial_storage']))
        test.assertTrue(role['account_proof_required']); test.assertTrue(role['code_proof_required']); test.assertLessEqual(len(code),24576)
    if enforce_bounds:
        test.assertLessEqual(spent,30_000_000); test.assertLessEqual(p['transaction_gas_limit'],15_000_000)
        for remaining in prefixes: test.assertGreaterEqual(remaining,p['transaction_gas_limit'])
    return records,states


class StorageWarmRollbackTests(unittest.TestCase):
    def load(self):
        path=CASES/'storage_warm_rollback.py'
        self.assertTrue(path.is_file(),'warm rollback producer is missing')
        sys.path.insert(0,str(CASES))
        spec=importlib.util.spec_from_file_location('storage_warm_rollback_cases',path)
        module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_all_32_literal_trajectories_and_pair_equality(self):
        m=self.load(); derived={}
        for mode,k,d,warm,level in GRID:
            with self.subTest(args=(mode,k,d,warm,level)):
                row=m.make_case(mode,k,d,warm,level); records,_=qualify(self,row)
                frames=records[0]['frames']; required=frames[-1]['result']['gas']-21000+2301
                for frame in reversed(frames[:-1]): required=frame['pre_gas']+max((64*required+62)//63,required+frame['post_gas'])
                derived[(mode,k,d,warm,level)]=21000+required
                other=m.make_case('ancestor' if mode=='leaf' else 'leaf',k,d,warm,level)
                for field in ('expected_user_gross_gas','transaction_gas_limit','full_rounds_per_key','expected_all_frame_terminal_counts'):
                    self.assertEqual(row['parameters'][field],other['parameters'][field])
                self.assertEqual(row['request']['program']['expected_route_counts'],other['request']['program']['expected_route_counts'])
                full=m.make_case(mode,k,d,warm,'full')
                self.assertEqual(row['parameters']['transaction_gas_limit'],full['parameters']['transaction_gas_limit'])
                self.assertEqual([c['initial_storage'] for c in row['request']['program']['prestate_contracts']],[c['initial_storage'] for c in full['request']['program']['prestate_contracts']])

        for k,d,warm in itertools.product((16,256),(2,4),(False,True)):
            expected=max(derived[(mode,k,d,warm,'full')] for mode in ('leaf','ancestor'))
            for mode,level in itertools.product(('leaf','ancestor'),('zero','full')):
                self.assertEqual(m.make_case(mode,k,d,warm,level)['parameters']['transaction_gas_limit'],expected)

    def test_strict_domains(self):
        m=self.load()
        for args in ((None,), (True,),('LEAF',),('leaf',True),('leaf',1024),('leaf',16,True),('leaf',16,1),('leaf',16,2,1),('leaf',16,2,None),('leaf',16,2,False,True),('leaf',16,2,False,'FULL')):
            with self.subTest(args=args),self.assertRaises(ValueError): m.make_case(*args)
        for r in (True,-1,1.0,65537,None):
            with self.assertRaises(ValueError): m.gas_accounting('leaf',16,2,False,r)
        for fn,args in ((m.fits,(True,2,False,0)),(m.full_rounds,(True,2,False)),(m.fits,(16,2,1,0)),(m.full_rounds,(16,2,1))):
            with self.assertRaises(ValueError): fn(*args)
        self.assertEqual(m.make_case(),m.make_case('leaf',16,2,False,'full'))

    def test_old_32_request_runtime_core_goldens(self):
        self.load()
        import storage_caught_nesting as call
        import storage_delegated_nesting as delegated
        for sid,request_hash,core_hash in OLD_GOLDENS:
            parts=sid.split('-'); mode=parts[-4]; k=int(parts[-3][1:]); d=int(parts[-2][1:]); r=int(parts[-1][1:])
            producer=delegated if 'delegated' in sid else call
            row=producer.make_case(mode,k,d,'zero' if r==0 else 'full'); request=row.pop('request')
            encoded=lambda value: json.dumps(value,sort_keys=True,separators=(',',':')).encode()
            # Request files use spawn_memory.encoded's stable pretty representation.
            from spawn_memory import encoded as request_encoded
            self.assertEqual(hashlib.sha256(request_encoded(request)).hexdigest(),request_hash,sid)
            self.assertEqual(hashlib.sha256(encoded(row)).hexdigest(),core_hash,sid)

    def test_initial_warm_keys_is_optional_without_result_shape_drift(self):
        code=bytes.fromhex('60005400')
        cold=execute(code,{0:1},100000); warm=execute(code,{0:1},100000,initial_warm_keys={0})
        self.assertEqual(set(cold),set(warm)); self.assertEqual(cold['gas']-warm['gas'],2000)
        self.assertEqual((cold['reads'],warm['reads']),([(0,1,True)],[(0,1,False)]))
        with self.assertRaisesRegex(AssertionError,'SSTORE sentry'):
            execute(bytes.fromhex('600160005500'),{0:1},21000+6+2300)
        self.assertEqual(execute(bytes.fromhex('600160005500'),{0:1},21000+6+2301)['terminal'],0)

    def test_boundaries_and_executed_full_plus_one(self):
        m=self.load()
        from storage_warm_rollback_vm import run_users
        for k,d,warm in itertools.product((16,256),(2,4),(False,True)):
            full=m.full_rounds(k,d,warm); self.assertTrue(m.fits(k,d,warm,full)); self.assertFalse(m.fits(k,d,warm,full+1))
            for mode in ('leaf','ancestor'):
                for r in ((0,1,63,64,65,full,full+1) if k==16 else (0,1,full,full+1)):
                    with self.subTest(args=(mode,k,d,warm,r)):
                        row=m._make_case(mode,k,d,warm,r,full); gas=m.gas_accounting(mode,k,d,warm,r)
                        row['request']['program']['transactions']['gas_limit']=gas['transaction_envelope']
                        record=run_users(row,limit=1)[0][0]
                        self.assertEqual(record['result']['gas'],gas['gross_gas'])
                gas=m.gas_accounting(mode,k,d,warm,full+1)
                self.assertTrue(167355+4*gas['charged_gas']>30_000_000 or gas['transaction_envelope']>15_000_000 or any(31_000_000-167355-j*gas['charged_gas']<gas['transaction_envelope'] for j in range(4)))

    def test_semantic_negatives_and_checkpoint_warm_sets(self):
        m=self.load()
        from storage_warm_rollback_vm import run_users
        for mode,warm,kind in itertools.product(('leaf','ancestor'),(False,True),('prewarm','postvalue','context','marker','terminal','status','gas','dirty9')):
            row=m.make_case(mode,16,2,warm,'zero'); contracts=row['request']['program']['prestate_contracts']
            if kind=='prewarm': row['parameters']['prewarm']=not warm
            elif kind=='context':
                contracts[1]['initial_storage']={}; contracts[0]['initial_storage']={'0x0':'0x10001'}
            else:
                c=contracts[0] if kind in ('marker','dirty9','terminal') else contracts[1]
                code=bytearray.fromhex(c['code_hex'][2:]); instructions=list(decode(code))
                if kind=='marker':
                    at=next(at for at,op,_ in instructions if op==0x63); code[at+1:at+5]=(1).to_bytes(4,'big')
                elif kind=='dirty9':
                    # Zero R has no attempted dirty writes; use R1 for this intent negative.
                    row=m._make_case(mode,16,2,warm,1,m.full_rounds(16,2,warm)); c=row['request']['program']['prestate_contracts'][0]
                    code=bytearray.fromhex(c['code_hex'][2:])
                    for at,op,v in decode(code):
                        if op==0x62 and v==7: code[at+1:at+4]=(9).to_bytes(3,'big')
                elif kind=='terminal':
                    at=next(at for at,op,_ in instructions if op in (0,0xfd)); code[at]=0xfe
                elif kind=='gas':
                    at=next(at for at,op,v in instructions if op==0x62 and v==12_000_000); code[at+1:at+4]=(100).to_bytes(3,'big')
                elif kind=='status':
                    at=next(at for at,op,_ in instructions if op==0xf4)+1; code[at+1:at+4]=(1-int.from_bytes(code[at+1:at+4],'big')).to_bytes(3,'big')
                else:
                    # Last expected body value guard belongs to mandatory postcheck.
                    at=[at for at,op,v in instructions if op==0x62 and v==1 and code[at+4]==0x14][-1]; code[at+1:at+4]=(9).to_bytes(3,'big')
                c['code_hex']='0x'+code.hex()
            with self.subTest(args=(mode,warm,kind)),self.assertRaises(AssertionError): qualify(self,row)
        # Child warmth is rolled back; caller warmth outside the checkpoint survives.
        for mode,warm in itertools.product(('leaf','ancestor'),(False,True)):
            record=run_users(m.make_case(mode,16,2,warm,'zero'),limit=1)[0][0]
            keys={(address(0,0),key) for key in [0]+list(range(65537,65553))}
            reverting=record['frames'][-1 if mode=='leaf' else -2]
            self.assertEqual(set(reverting['warm_slots_after']),keys if warm else set())
            self.assertEqual(set(reverting['warm_slots_before']),keys if warm else set())
            if mode=='ancestor':
                self.assertTrue(set(record['frames'][-1]['warm_slots_after'])>=keys)
                self.assertEqual(record['frames'][-2]['refund_before_terminal'],44800)
                self.assertNotIn(address(0),record['frames'][-2]['warm_accounts_after'])
                self.assertIn(address(0),record['frames'][-1]['warm_accounts_after'])
            else:
                self.assertIn(address(0),record['frames'][-1]['warm_accounts_after'])

    def test_catalog_provenance_and_cli(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'cases'; self.assertEqual(m.prepare(output),32)
            catalog=json.loads((output/'catalog.json').read_text()); self.assertEqual(catalog['corpus_id'],'unzen-storage-warm-rollback-20261103')
            self.assertEqual(len({r['request_sha256'] for r in catalog['cases']}),32)
            self.assertEqual(catalog['generator_sha256'],hashlib.sha256((CASES/'storage_warm_rollback.py').read_bytes()).hexdigest())
            self.assertIn('storage_caught_nesting._emit_call',catalog['helper_source_sha256'])
            for name,expected in catalog['source_sha256'].items(): self.assertEqual(hashlib.sha256((CASES/name).read_bytes()).hexdigest(),expected)
            import inspect
            for name,helper in m.helpers().items(): self.assertEqual(hashlib.sha256(inspect.getsource(helper).encode()).hexdigest(),catalog['helper_source_sha256'][name])
            for row in catalog['cases']: self.assertEqual(hashlib.sha256((output/row['request_path']).read_bytes()).hexdigest(),row['request_sha256'])
            with self.assertRaises(FileExistsError): m.prepare(output)
            for value in (None,True,'A'*64,'0'*63):
                with self.assertRaises(ValueError): m.prepare(Path(temporary)/'bad',model_id=value)
            with self.assertRaises(ValueError): m.prepare(str(output))
            result=subprocess.run([sys.executable,'-S',str(CASES/'storage_warm_rollback.py'),'--output-dir',str(Path(temporary)/'cli')],capture_output=True,text=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr); self.assertEqual(json.loads(result.stdout),{'prepared':32})

    @unittest.skipUnless(os.environ.get('STORAGE_WARM_NATIVE_BIN'),'optional frozen native')
    def test_frozen_native_positives_and_accepted_dirty9_mutants(self):
        m=self.load(); binary=Path(os.environ['STORAGE_WARM_NATIVE_BIN'])
        self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(),NATIVE_SHA256)
        archive=Path(os.environ['STORAGE_WARM_NATIVE_OUTPUT']); archive.mkdir(parents=True,exist_ok=False)
        for args,mutant in itertools.product((('leaf',16,2,False),('ancestor',256,4,True)),(False,True)):
            row=m.make_case(*args); p=row['parameters']; root=archive/(row['sample_id']+('-dirty9' if mutant else '-positive')); root.mkdir()
            if mutant:
                c=row['request']['program']['prestate_contracts'][0]; code=bytearray.fromhex(c['code_hex'][2:])
                for at,op,value in decode(code):
                    if op==0x62 and value==7: code[at+1:at+4]=(9).to_bytes(3,'big')
                c['code_hex']='0x'+code.hex()
            source=root/'request.json'; source.write_text(json.dumps(row['request']))
            command=[str(binary),'--input',str(source),'--output',str(root/'input.json'),'--output-bincode',str(root/'input.bin'),'--report',str(root/'native.json'),'--execution-profile','evm-gas-only']
            try: process=subprocess.run(command,capture_output=True,timeout=180)
            except subprocess.TimeoutExpired as error:
                (root/'stdout.log').write_bytes(error.stdout or b''); (root/'stderr.log').write_bytes(error.stderr or b''); raise
            (root/'stdout.log').write_bytes(process.stdout); (root/'stderr.log').write_bytes(process.stderr)
            self.assertEqual(process.returncode,0,process.stderr.decode())
            report=json.loads((root/'native.json').read_text()); receipts=report['receipts_by_block'][0]
            self.assertEqual([r['success'] for r in receipts],[True]*5)
            self.assertEqual([r['gas_used'] for r in receipts],[167355]+[p['expected_user_charged_gas']]*4)
            self.assertEqual(report['terminal_counts'],{'stop':4*args[2],'revert':4})
            for route,count in row['request']['program']['expected_route_counts'].items():
                self.assertEqual(report['observed_route_counts'].get(route,0),count)
                self.assertEqual(report['attempted_route_counts'].get(route,0),count)
            for diag in report['prestate_contracts']['initialized_storage']: self.assertEqual(diag['initial_storage_root'],diag['final_storage_root'])
            if mutant:
                with self.assertRaises(AssertionError): qualify(self,row)
            else: qualify(self,row)


if __name__=='__main__': unittest.main()
