"""Independent ordered roles, persistent originals, journal frames and gas oracles."""
from collections import Counter
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
from test_storage_hash_composition import decode

CASES=Path(__file__).resolve().parents[1]/'cases'
PHASE=4096
MASK=(1<<256)-1
ADDRESS=int.from_bytes(bytes([0x66])*20,'big')
TAG=b'order-composition-child-v1'.ljust(32,b'\0')
SEEDS=tuple(int.from_bytes(hashlib.sha256(f'zkgas-stack-seed-v1-{i}'.encode()).digest(),'big')|(1<<255) for i in range(17))
COST={0:0,1:3,2:5,0x10:3,0x14:3,0x15:3,0x30:2,0x35:3,0x36:2,0x3d:2,0x50:2,0x51:3,0x52:3,0x56:8,0x57:10,0x5b:1,0x80:3,0x9f:3,0xf4:100,0xfd:0,0xfe:0}


def roles(order,packing):
    return [1,2]*(packing//2) if order=='alternating' else [1]*(packing//2)+[2]*(packing//2) if order=='densityfirst' else [2]*(packing//2)+[1]*(packing//2)


def table(order,packing):
    return b''.join(value.to_bytes(32,'big') for value in roles(order,packing))


def execute(code,data,users):
    state={};all_txs=[]
    for index in range(users):
        original=state.copy();warm=set();refund=0;counts=Counter();reads=[];writes=[];hashes=[];swaps=Counter();frames=[];calls=[]
        def frame(calldata,depth):
            nonlocal refund
            stack=[];memory=bytearray();pc=0;gas=0;returndata=b'';checkpoint=state.copy();warm_before=warm.copy();refund_before=refund
            def grow(end):
                nonlocal gas
                before=(len(memory)+31)//32;after=(end+31)//32
                if after>before:
                    gas+=3*after+after*after//512-(3*before+before*before//512);memory.extend(bytes(after*32-len(memory)))
            for _ in range(1_000_000):
                at=pc;op=code[pc];pc+=1;counts[op]+=1
                if 0x60<=op<=0x7f:
                    width=op-0x5f;stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width;gas+=3;continue
                if op not in (0x20,0x54,0x55):gas+=COST[op]
                if op in (1,2,0x10,0x14):
                    a,b=stack.pop(),stack.pop();stack.append((a+b)&MASK if op==1 else (a*b)&MASK if op==2 else int(a<b) if op==0x10 else int(a==b))
                elif op==0x15:stack.append(int(stack.pop()==0))
                elif op==0x30:stack.append(ADDRESS)
                elif op==0x35:
                    offset=stack.pop();stack.append(int.from_bytes(calldata[offset:offset+32].ljust(32,b'\0'),'big'))
                elif op==0x36:stack.append(len(calldata))
                elif op==0x3d:stack.append(len(returndata))
                elif op==0x50:stack.pop()
                elif op==0x51:
                    offset=stack.pop();grow(offset+32);stack.append(int.from_bytes(memory[offset:offset+32],'big'))
                elif op==0x52:
                    offset,value=stack.pop(),stack.pop();grow(offset+32);memory[offset:offset+32]=value.to_bytes(32,'big')
                elif op==0x54:
                    key=stack.pop();cold=key not in warm;warm.add(key);value=state.get(key,0);gas+=2100 if cold else 100;reads.append((depth,key,value,original.get(key,0),cold));stack.append(value)
                elif op==0x55:
                    key,value=stack.pop(),stack.pop();before=state.get(key,0);orig=original.get(key,0);cold=key not in warm;warm.add(key);gas+=2100 if cold else 0
                    if before==value:gas+=100
                    elif orig==before:
                        gas+=20000 if orig==0 else 2900
                        if orig and not value:refund+=4800
                    else:
                        gas+=100
                        if orig:
                            if not before:refund-=4800
                            if not value:refund+=4800
                        if orig==value:refund+=19900 if orig==0 else 2800
                    writes.append((depth,key,before,value,orig,cold))
                    if value:state[key]=value
                    else:state.pop(key,None)
                elif op==0x20:
                    offset,length=stack.pop(),stack.pop();grow(offset+length);image=bytes(memory[offset:offset+length]);result=keccak256(image);gas+=30+6*((length+31)//32);hashes.append((depth,offset,length,image,result));stack.append(int.from_bytes(result,'big'))
                elif op==0x80:stack.append(stack[-1])
                elif op==0x9f:
                    swaps[tuple(stack)]+=1;stack[-1],stack[-17]=stack[-17],stack[-1]
                elif op in (0x56,0x57):
                    target=stack.pop();take=op==0x56 or stack.pop()!=0
                    if take:assert code[target]==0x5b;pc=target
                elif op==0x5b:pass
                elif op==0xf4:
                    forwarded,target,start,size,out,outsize=(stack.pop() for _ in range(6));assert depth==0 and target==ADDRESS;grow(max(start+size,out+outsize));childdata=bytes(memory[start:start+size]);terminal,result,childgas=frame(childdata,depth+1);gas+=childgas;returndata=result;memory[out:out+min(outsize,len(result))]=result[:outsize];status=int(terminal==0xf3);stack.append(status);calls.append((forwarded,target,childdata,status,result,state.copy()))
                elif op in (0,0xfd,0xfe):
                    result=b''
                    if op==0xfd:
                        offset,length=stack.pop(),stack.pop();grow(offset+length);result=bytes(memory[offset:offset+length])
                    if op in (0xfd,0xfe):
                        state.clear();state.update(checkpoint);warm.clear();warm.update(warm_before);refund=refund_before
                    frames.append((depth,op,bytes(memory),checkpoint,state.copy(),tuple(stack)));return op,result,gas
                else:raise AssertionError(f'unsupported opcode {op:02x}')
            raise AssertionError('bounded order runtime did not terminate')
        terminal,_,execution=frame(data,0);tokens=sum(1 if byte==0 else 4 for byte in data);intrinsic=21000+4*tokens;gross=intrinsic+execution;applied=min(refund,gross//5);floor=21000+10*tokens
        all_txs.append(dict(index=index,terminal=terminal,original=original,state=state.copy(),counts=counts,reads=reads,writes=writes,hashes=hashes,swaps=swaps,frames=frames,calls=calls,gross=gross,refund_counter=refund,applied_refund=applied,calldata_tokens=tokens,calldata_floor_candidate=floor,charged=max(gross-applied,floor)))
        if terminal!=0:break
    return all_txs


def assert_semantics(test,result,kernel,order,packing,density_batches,journal_batches):
    expected_roles=roles(order,packing);test.assertEqual(len(result),packing)
    swapped=list(SEEDS);swapped[0],swapped[-1]=swapped[-1],swapped[0]
    for index,tx in enumerate(result):
        role=expected_roles[index];test.assertEqual(tx['terminal'],0);test.assertEqual(tx['original'],{} if index==0 else {PHASE:index});test.assertEqual(tx['state'],{PHASE:index+1})
        expected_reads=[(0,PHASE,index,index,True)]
        if role==2:expected_reads += [(0,2,0,0,True),(1,2,5,0,False),(1,2,7,0,False),(0,2,5,0,False),(0,2,0,0,False)]
        expected_reads += [(0,PHASE,index+1,index,False)]
        test.assertEqual(tx['reads'],expected_reads)
        expected=[(0,PHASE,index,index+1,index,False)]
        if role==2:
            N=journal_batches*64;expected += [(0,2,0,5,0,False),(1,2,5,7,0,False)]+[(1,2,7,11,0,False),(1,2,11,7,0,False)]*N+[(1,2,7,5,0,False),(0,2,5,0,0,False)]
        test.assertEqual(len(tx['writes']),len(expected))
        for actual,wanted in zip(tx['writes'],expected):test.assertEqual(actual,wanted)
        test.assertEqual(tx['counts'][0x9f],2*2048*density_batches if role==1 else 0)
        test.assertTrue(set(tx['swaps']) <= {SEEDS,tuple(swapped)})
        test.assertEqual(len(tx['hashes']),1+(journal_batches*64 if kernel=='mixed' else 0) if role==2 else 0)
        for depth,offset,length,image,digest in tx['hashes']:test.assertEqual((depth,offset,length,image,digest),(1,0,32,bytes(32),keccak256(bytes(32))))
        test.assertEqual(len(tx['calls']),int(role==2))
        for forwarded,target,data,status,marker,state in tx['calls']:
            test.assertEqual((forwarded,target,data,status,marker,state),(12000000,ADDRESS,TAG,0,(journal_batches*64).to_bytes(32,'big'),{PHASE:index+1,2:5}))
        for depth,terminal,memory,checkpoint,after,stack in tx['frames']:
            test.assertEqual(stack,())
            if depth==1:
                test.assertEqual(checkpoint,{PHASE:index+1,2:5});test.assertEqual(after,checkpoint);test.assertEqual(terminal,0xfd)
            if role==1 and depth==0:test.assertEqual(int.from_bytes(memory[:32],'big'),density_batches)
        test.assertGreaterEqual(tx['gross']-tx['applied_refund'],tx['calldata_floor_candidate'])


def semantic_mutations(code,data):
    """Native-success wrong semantics and guard failures, using decoded operands."""
    decoded=decode(code)
    def changed(edits):
        image=bytearray(code)
        for pc,width,value in edits:image[pc+1:pc+1+width]=value.to_bytes(width,'big')
        return bytes(image)
    phase=[(pc,len(raw),PHASE+1) for pc,op,raw in decoded if 0x60<=op<=0x7f and int.from_bytes(raw,'big')==PHASE]
    assert len(phase)==3
    seeds=[(pc,32,SEEDS[1] if int.from_bytes(raw,'big')==SEEDS[0] else SEEDS[0]) for pc,op,raw in decoded if op==0x7f and int.from_bytes(raw,'big') in SEEDS[:2]]
    assert len(seeds)==4
    bulk=[];final_read=[];marker=[];counter=[]
    for i,(pc,op,raw) in enumerate(decoded):
        if op==0x55 and decoded[i-1][2]==(2).to_bytes(3,'big') and decoded[i-2][2]==(11).to_bytes(3,'big'):
            bulk.append((decoded[i-1][0],3,3))
        if op==0x54 and decoded[i-1][2]==(2).to_bytes(3,'big') and decoded[i+1][2]==bytes(3):final_read.append((decoded[i-1][0],3,3))
        if op==0x52 and decoded[i-1][2]==(128).to_bytes(3,'big') and decoded[i-2][1]==2:marker.append((decoded[i-1][0],3,160))
        if op==0x51 and decoded[i-1][2]==bytes(3) and decoded[i+1][1]==0x62 and decoded[i+2][1]==0x14 and decoded[i+3][1]==0x15:counter.append((decoded[i-1][0],3,32))
    assert len(bulk)==64 and len(final_read)==2 and len(marker)==1 and len(counter)==1
    # The first read0 checks entry before seeding; choose the parent final read0.
    return {'table-order':(code,table('densityfirst',len(data)//32)),
        'phase-key':(changed(phase),data),'paired-seeds':(changed(seeds),data),
        'bulk-write-key':(changed(bulk),data),'final-read-key':(changed(final_read[1:]),data),
        'marker-offset':(changed(marker),data),'density-counter':(changed(counter),data)}


class OrderCompositionTests(unittest.TestCase):
    def load(self):
        path=CASES/'order_composition.py';self.assertTrue(path.is_file(),'order-composition producer is missing');sys.path.insert(0,str(CASES))
        spec=importlib.util.spec_from_file_location('order_cases',path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_small_runtime_phase_originals_and_role_order(self):
        m=self.load()
        for kernel in ('dirty','mixed'):
            for packing in (2,4):
                for order in ('alternating','densityfirst','journalfirst'):
                    code,counts=m._runtime(kernel,packing,1,1);data=table(order,packing);result=execute(code,data,packing);assert_semantics(self,result,kernel,order,packing,1,1)
                    for tx,role in zip(result,roles(order,packing)):
                        self.assertEqual(+tx['counts'],Counter({int(key[9:],16):n for key,n in counts['density' if role==1 else 'journal'].items() if n}))

    def test_phase_gas_is_clean_per_transaction_and_floor_is_explicit(self):
        m=self.load();code,_=m._runtime('dirty',4,0,0)
        for order in ('alternating','journalfirst'):
            result=execute(code,table(order,4),4);assert_semantics(self,result,'dirty',order,4,0,0)
            for index,tx in enumerate(result):
                gas=m._gas_user('dirty',4,0,0,roles(order,4)[index],index)
                for key,name in [('gross_user_gas','gross'),('refund_counter','refund_counter'),('applied_refund','applied_refund'),('charged_user_gas','charged'),('calldata_floor_candidate_gas','calldata_floor_candidate')]:self.assertEqual(gas[key],tx[name])
                self.assertEqual(gas['phase_write_gas'],20000 if index==0 else 2900)

    def test_24_matrix_requests_baselines_source_hashes_and_nooverwrite(self):
        m=self.load();self.assertTrue(callable(getattr(m,'prepare',None)),'24-case preparation is missing')
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root),24);catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];by={row['sample_id']:row for row in rows}
            self.assertEqual(len(by),24);self.assertEqual(len({row['request_sha256'] for row in rows}),24)
            self.assertEqual({(row['parameters']['kernel'],row['parameters']['packing'],row['parameters']['order'],row['parameters']['level']) for row in rows},{(k,p,o,l) for k in ('dirty','mixed') for p in (4,16) for o in ('alternating','densityfirst','journalfirst') for l in ('zero','full')})
            for row in rows:
                p=row['parameters'];raw=(root/row['request_path']).read_bytes();request=json.loads(raw);program=request['program'];self.assertEqual(m.digest(raw),row['request_sha256']);self.assertEqual(bytes.fromhex(program['input_hex'][2:]),table(p['order'],p['packing']));self.assertNotIn('storage_state',program)
                self.assertEqual(program['transactions'],{'kind':'call','count_per_block':p['packing'],'gas_limit':p['transaction_gas_limit']});self.assertTrue(request['expected_user_success']);self.assertEqual(program['block_count'],1)
                base=by[row['baseline_sample_id']]['parameters'];self.assertEqual((base['kernel'],base['packing'],base['order'],base['level']),(p['kernel'],p['packing'],p['order'],'zero'));self.assertEqual(base['transaction_gas_limit'],p['transaction_gas_limit'])
                self.assertEqual(p['expected_phase_original_by_index'],list(range(p['packing'])));self.assertEqual(p['expected_phase_final'],p['packing']);self.assertEqual(p['phase_slot'],4096)
                self.assertEqual(p['expected_block_gas_used'],167355+sum(p['expected_user_gas_used_by_index']));self.assertEqual(p['expected_user_gross_gas_by_index'],[x['gross_user_gas'] for x in p['gas_by_index']])
                self.assertTrue(all(x['calldata_floor_nonbinding'] for x in p['gas_by_index']));self.assertTrue(row['holdout_only']);self.assertFalse(row['actuals_used_for_fitting'])
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})
            alternate=Path(temporary)/'metadata';m.prepare(alternate,model_id='1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (root/'requests').iterdir()},{p.name:p.read_bytes() for p in (alternate/'requests').iterdir()})

    def test_full_gas_counts_fixed_body_and_order_neutral_envelope(self):
        m=self.load();self.assertTrue(callable(getattr(m,'make_case',None)),'full gas planning is missing')
        for kernel in ('dirty','mixed'):
            for packing in (4,16):
                rows=[m.make_case(kernel,packing,order) for order in ('alternating','densityfirst','journalfirst')]
                self.assertEqual(len({r['parameters']['expected_block_gas_used'] for r in rows}),1);self.assertEqual(len({r['parameters']['transaction_gas_limit'] for r in rows}),1)
                for row in rows:
                    p=row['parameters'];self.assertGreaterEqual(p['expected_block_gas_used'],29700000);self.assertLessEqual(p['expected_block_gas_used'],30000000)
                    self.assertGreaterEqual(min(p['native_remaining_gas_before_users']),p['transaction_gas_limit']);self.assertLessEqual(p['transaction_gas_limit'],15000000)
                    self.assertEqual(p['native_header_gas_limit'],31000000);self.assertEqual(p['charged_block_gas_budget'],30000000)
                    zero=m.make_case(kernel,packing,p['order'],'zero');code=bytes.fromhex(row['request']['program']['code_hex'][2:]);zcode=bytes.fromhex(zero['request']['program']['code_hex'][2:])
                    self.assertEqual(row['request']['program']['input_hex'],zero['request']['program']['input_hex']);self.assertEqual([(pc,op) for pc,op,_ in decode(code)],[(pc,op) for pc,op,_ in decode(zcode)])
                    empty,_=m._runtime(kernel,packing,0,0);one_d,_=m._runtime(kernel,packing,1,0);one_j,_=m._runtime(kernel,packing,0,1)
                    baseline=execute(empty,table(p['order'],packing),packing);d_result=execute(one_d,table(p['order'],packing),packing);j_result=execute(one_j,table(p['order'],packing),packing)
                    expected=Counter()
                    for index,(a,b,c) in enumerate(zip(baseline,d_result,j_result)):
                        for op in set(a['counts'])|set(b['counts'])|set(c['counts']):expected[op]+=a['counts'][op]+p['density_batches']*(b['counts'][op]-a['counts'][op])+p['journal_batches']*(c['counts'][op]-a['counts'][op])
                        gross=a['gross']+p['density_batches']*(b['gross']-a['gross'])+p['journal_batches']*(c['gross']-a['gross'])
                        refund=min(a['refund_counter'],gross//5);charged=max(gross-refund,a['calldata_floor_candidate'])
                        self.assertEqual(p['expected_user_gross_gas_by_index'][index],gross);self.assertEqual(p['expected_user_gas_used_by_index'][index],charged)
                        for key in ('refund_counter','calldata_tokens','calldata_floor_candidate'):
                            self.assertEqual(a[key],b[key]);self.assertEqual(a[key],c[key])
                        if index:
                            step=(b['gross']-a['gross']) or (c['gross']-a['gross']);budget=(29990000-167355-17100)//packing
                            self.assertLessEqual(charged,budget);self.assertGreater(charged+step,budget)
                    self.assertEqual({f'opcode:0x{op:02x}':value for op,value in (+expected).items()},{op:value for op,value in row['request']['program']['expected_route_counts'].items() if value})
        alt=m.make_case('dirty',4,'alternating','zero')['parameters']['expected_block_gas_used'];jfirst=m.make_case('dirty',4,'journalfirst','zero')['parameters']['expected_block_gas_used'];self.assertNotEqual(alt,jfirst)

    def test_strict_public_domains(self):
        m=self.load();self.assertTrue(callable(getattr(m,'make_case',None)))
        for args in [('bad',4,'alternating','full'),(True,4,'alternating','full'),('dirty',True,'alternating','full'),('dirty',2,'alternating','full'),('mixed',4,'bad','full'),('dirty',16,'journalfirst',True)]:
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('X'*64,'1'*63,False):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'invalid',model_id=model)
                self.assertFalse((Path(temporary)/'invalid').exists())

    def test_native_success_mutations_need_operand_and_order_oracles(self):
        m=self.load();code,_=m._runtime('mixed',4,1,8);data=table('alternating',4);valid=execute(code,data,4);assert_semantics(self,valid,'mixed','alternating',4,1,8)
        counts=sum((tx['counts'] for tx in valid),Counter())
        for name,(wrong,image) in semantic_mutations(code,data).items():
            with self.subTest(name=name):
                result=execute(wrong,image,4)
                if name in ('marker-offset','density-counter'):
                    self.assertNotEqual(result[-1]['terminal'],0);self.assertEqual(result[-1]['state'],result[-1]['original'])
                else:
                    self.assertEqual([tx['terminal'] for tx in result],[0]*4)
                    self.assertEqual(sum((tx['counts'] for tx in result),Counter()),counts)
                    if name in ('table-order','phase-key','paired-seeds'):self.assertEqual(sum(tx['charged'] for tx in result),sum(tx['charged'] for tx in valid))
                    with self.assertRaises(AssertionError):assert_semantics(self,result,'mixed','alternating',4,1,8)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'),'set NATIVE_FIXTURE_BIN for retained native smoke')
    def test_native_full_spots(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            for kernel,packing,order,level in [('dirty',4,'alternating','full'),('mixed',16,'journalfirst','full'),('dirty',16,'journalfirst','zero')]:
                row=m.make_case(kernel,packing,order,level);root=Path(temporary)/row['sample_id'];root.mkdir();request=root/'request.json';request.write_bytes(m.encoded(row['request']))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(request),'--output',str(root/'input.json'),'--output-bincode',str(root/'input.bin'),'--report',str(root/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=60)
                self.assertEqual(result.returncode,0,result.stderr.decode()[-2000:]);report=json.loads((root/'native.json').read_text());p=row['parameters']
                self.assertEqual([x['gas_used'] for x in report['receipts_by_block'][0][1:]],p['expected_user_gas_used_by_index']);self.assertEqual(report['blocks'][0]['block_gas_used'],p['expected_block_gas_used']);self.assertTrue(all(x['success'] for x in report['receipts_by_block'][0]))
                self.assertEqual(Counter({k:v for k,v in report['observed_route_counts'].items() if v}),Counter({k:v for k,v in row['request']['program']['expected_route_counts'].items() if v}))


if __name__=='__main__':unittest.main()
