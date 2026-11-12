"""Concrete child-access, warmth checkpoint and gas oracles for fixed inputs."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_storage_hash_composition import decode

CASES = Path(__file__).resolve().parents[1]/'cases'
ADDRESS = int.from_bytes(bytes([0x66])*20, 'big')
TAG = int.from_bytes(b'cold-frame-child-v1'.ljust(32, b'\0'), 'big')
MASK = (1 << 256)-1
STATIC = {0:0, 1:3, 0x14:3, 0x15:3, 0x30:2, 0x35:3, 0x36:2,
          0x3d:2, 0x51:3, 0x52:3, 0x56:8, 0x57:10, 0x5b:1,
          0xf4:100, 0xf3:0, 0xfd:0, 0xfe:0}


def execute_cold(code, *, transactions=1):
    """Bounded concrete execution; cold flags checkpoint independently of cache."""
    counts=Counter(); accesses=[]; calls=[]; frames=[]; terminals=[]; gas=[]
    cached=set(); all_warm=[]
    def frame(data, depth, tx, warm, tally):
        checkpoint=warm.copy(); stack=[]; memory=bytearray(); returndata=b''; pc=0
        def grow(end):
            if len(memory)<end:memory.extend(bytes(end-len(memory)))
        for _ in range(2_000_000):
            at=pc;op=code[pc];pc+=1;counts[op]+=1
            if 0x60<=op<=0x7f:
                width=op-0x5f;stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width;tally['static']+=3
            else:tally['static']+=STATIC.get(op,0)
            if 0x60<=op<=0x7f:pass
            elif op in (1,0x14):
                a,b=stack.pop(),stack.pop();stack.append((a+b)&MASK if op==1 else int(a==b))
            elif op==0x15:stack.append(int(stack.pop()==0))
            elif op==0x30:stack.append(ADDRESS)
            elif op==0x35:
                offset=stack.pop();stack.append(int.from_bytes(data[offset:offset+32].ljust(32,b'\0'),'big'))
            elif op==0x36:stack.append(len(data))
            elif op==0x3d:stack.append(len(returndata))
            elif op==0x51:
                offset=stack.pop();grow(offset+32);stack.append(int.from_bytes(memory[offset:offset+32],'big'))
            elif op==0x52:
                offset,value=stack.pop(),stack.pop();grow(offset+32);memory[offset:offset+32]=value.to_bytes(32,'big')
            elif op==0x54:
                key=stack.pop();cold=key not in warm;was_cached=key in cached
                warm.add(key);cached.add(key);tally['storage']+=2100 if cold else 100
                accesses.append((tx,depth,at,key,0,cold,was_cached));stack.append(0)
            elif op in (0x56,0x57):
                target=stack.pop();take=op==0x56 or stack.pop()!=0
                if take:assert code[target]==0x5b;pc=target
            elif op==0x5b:pass
            elif op==0xf4:
                requested,target,start,size,out,outsize=(stack.pop() for _ in range(6))
                assert depth==0 and target==ADDRESS
                grow(max(start+size,out+outsize));input_data=bytes(memory[start:start+size]);before=warm.copy()
                terminal,output=frame(input_data,depth+1,tx,warm,tally)
                status=int(terminal==0xf3);returndata=output
                memory[out:out+min(outsize,len(output))]=output[:outsize]
                calls.append((tx,requested,target,start,size,out,outsize,input_data,status,output,before,warm.copy()))
                stack.append(status)
            elif op in (0xf3,0xfd):
                offset,size=stack.pop(),stack.pop();grow(offset+size);output=bytes(memory[offset:offset+size])
                before=warm.copy()
                if op==0xfd:warm.clear();warm.update(checkpoint)
                words=(len(memory)+31)//32;tally['memory']+=3*words+words*words//512
                frames.append((tx,depth,op,data,bytes(memory),before,warm.copy(),checkpoint))
                return op,output
            elif op in (0,0xfe):
                before=warm.copy()
                if op==0xfe:warm.clear();warm.update(checkpoint)
                words=(len(memory)+31)//32;tally['memory']+=3*words+words*words//512
                frames.append((tx,depth,op,data,bytes(memory),before,warm.copy(),checkpoint))
                return op,b''
            else:raise AssertionError(f'unsupported concrete opcode{op:02x}')
        raise AssertionError('cold-frame program did not terminate')
    for tx in range(transactions):
        warm=set();tally={'static':0,'storage':0,'memory':0}
        terminal,_=frame(b'',0,tx,warm,tally);terminals.append(terminal);all_warm.append(warm.copy())
        gas.append(21000+sum(tally.values()))
        if terminal!=0:break
    return dict(counts=+counts,accesses=accesses,calls=calls,frames=frames,terminals=terminals,
                gas=gas,warm_at_tx_end=all_warm,cached=cached)


def assert_cold(test,result,profile,calls,users):
    prewarm=profile=='parent-prewarm-revert';success=profile=='cold-return'
    test.assertEqual(result['terminals'],[0]*users)
    test.assertEqual(len(result['calls']),calls*users)
    test.assertEqual(result['warm_at_tx_end'],[{2}]*users)
    for tx in range(users):
        actual=[(depth,key,value,cold) for t,depth,pc,key,value,cold,cached in result['accesses'] if t==tx]
        expected=([(0,2,0,True)] if prewarm else [])
        expected += [(1,2,0,not prewarm and (not success or index==0)) for index in range(calls)]
        expected += [(0,2,0,not prewarm and (not success or calls==0))]
        test.assertEqual(actual,expected)
        # The first read of each later transaction is cold even though its slot
        # remains cached; never equate warmth reset with DB rematerialization.
        first=next(a for a in result['accesses'] if a[0]==tx)
        test.assertTrue(first[5]);test.assertEqual(first[6],tx>0)
    for tx,requested,target,start,size,out,outsize,data,status,output,before,after in result['calls']:
        test.assertEqual((requested,target,start,size,out,outsize),(12000000,ADDRESS,256,32,128,32))
        test.assertEqual(data,TAG.to_bytes(32,'big'));test.assertEqual(status,int(success));test.assertEqual(output,bytes(32))
        test.assertEqual(after,{2} if success or prewarm else set())
        test.assertEqual(before,{2} if prewarm else ({2} if success and before else set()))
    for tx,depth,terminal,data,memory,before,after,checkpoint in result['frames']:
        if depth==0:
            test.assertEqual(terminal,0);test.assertEqual(int.from_bytes(memory[64:96],'big'),calls//4)
            test.assertEqual(memory[256:288],TAG.to_bytes(32,'big'))
        else:
            test.assertEqual(terminal,0xf3 if success else 0xfd);test.assertEqual(memory,bytes(32))
            test.assertEqual(before,{2});test.assertEqual(after,{2} if success or prewarm else checkpoint)


class ColdFrameCompositionTests(unittest.TestCase):
    def load(self):
        path=CASES/'cold_frame_composition.py';self.assertTrue(path.is_file(),'cold-frame producer is missing')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('cold_frame_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_twelve_inputs_exact_interfaces_controls_hashes_and_nooverwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'cases';self.assertEqual(m.prepare(output),12)
            catalog=json.loads((output/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual(len(by),12);self.assertEqual(len({r['request_sha256'] for r in rows}),12)
            self.assertEqual({(r['parameters']['profile'],r['parameters']['packing'],r['parameters']['level']) for r in rows},
                {(p,t,l) for p in ('cold-return','cold-revert','parent-prewarm-revert') for t in (4,16) for l in ('zero','full')})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'cold_frame_composition.py').read_bytes()))
            for name,sha in catalog['source_sha256'].items():self.assertEqual(m.digest((CASES/name).read_bytes()),sha)
            for row in rows:
                p=row['parameters'];raw=(output/row['request_path']).read_bytes();request=json.loads(raw);program=request['program']
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertTrue(row['holdout_only']);self.assertFalse(row['actuals_used_for_fitting'])
                self.assertEqual(program['input_hex'],'0x');self.assertEqual(program['block_count'],1)
                self.assertEqual(program['transactions'],{'kind':'call','count_per_block':p['packing'],'gas_limit':p['transaction_gas_limit']})
                self.assertNotIn('storage_state',program);self.assertTrue(request['expected_user_success'])
                base=by[row['baseline_sample_id']]['parameters'];self.assertEqual((base['profile'],base['packing'],base['level']),(p['profile'],p['packing'],'zero'))
                self.assertEqual(base['transaction_gas_limit'],p['transaction_gas_limit']);self.assertEqual(p['all_child_calls'],p['calls_per_user']*p['packing'])
            before={p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(output)
            self.assertEqual(before,{p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()})
            other=Path(temporary)/'metadata';m.prepare(other,model_id='1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (output/'requests').iterdir()},{p.name:p.read_bytes() for p in (other/'requests').iterdir()})

    def test_ordered_warmth_rollback_and_transaction_reset_exact_gas(self):
        m=self.load()
        for profile in m.PROFILES:
            for n in (0,4,8):
                code,counts=m._runtime(profile,n);result=execute_cold(code,transactions=3)
                assert_cold(self,result,profile,n,3)
                self.assertEqual({f'opcode:0x{op:02x}':count for op,count in result['counts'].items()},
                                 {key:count*3 for key,count in counts.items() if count})
                self.assertEqual(result['gas'],[m._gas(profile,n)['charged_user_gas']]*3)
        self.assertEqual(m._gas('cold-return',0)['charged_user_gas'],23244)
        self.assertEqual(m._gas('cold-revert',0)['charged_user_gas'],23244)
        self.assertEqual(m._gas('parent-prewarm-revert',0)['charged_user_gas'],23369)
        for n in (4,8):
            self.assertEqual(m._gas('cold-revert',n)['charged_user_gas']-m._gas('cold-return',n)['charged_user_gas'],2000*n)

    def test_full_gas_quantization_envelopes_and_fixed_body_counts(self):
        m=self.load();self.assertEqual(m.UNROLL,4)
        for profile in m.PROFILES:
            for packing in (4,16):
                row=m.make_case(profile,packing);p=row['parameters'];n=p['calls_per_user'];gas=m._gas(profile,n)
                self.assertEqual(n%4,0);self.assertGreater(n,0)
                self.assertEqual(p['expected_block_gas_used'],167355+packing*gas['charged_user_gas'])
                self.assertGreaterEqual(p['expected_block_gas_used'],29700000);self.assertLessEqual(p['expected_block_gas_used'],29990000)
                self.assertGreater(167355+packing*m._gas(profile,n+4)['charged_user_gas'],29990000)
                self.assertEqual(p['expected_user_gas_used'],gas['charged_user_gas']);self.assertEqual(gas['applied_refund'],0)
                self.assertEqual(p['transaction_gas_limit'],gas['gross_user_gas']+100000)
                self.assertLessEqual(p['transaction_gas_limit'],15000000)
                self.assertGreaterEqual(31000000-167355-(packing-1)*gas['charged_user_gas'],p['transaction_gas_limit'])
                zero=m.make_case(profile,packing,'zero');self.assertEqual(zero['parameters']['transaction_gas_limit'],p['transaction_gas_limit'])
                full=bytes.fromhex(row['request']['program']['code_hex'][2:]);empty=bytes.fromhex(zero['request']['program']['code_hex'][2:])
                self.assertEqual([(pc,op) for pc,op,_ in decode(full)],[(pc,op) for pc,op,_ in decode(empty)])
                changes=[(int.from_bytes(a[2],'big'),int.from_bytes(b[2],'big')) for a,b in zip(decode(empty),decode(full)) if a[2]!=b[2]]
                self.assertEqual(changes,[(0,n//4),(0,n//4)])
                z=execute_cold(empty)['counts'];small=execute_cold(m._runtime(profile,4)[0])['counts']
                extrapolated=Counter({op:z[op]+(small[op]-z[op])*(n//4) for op in set(z)|set(small)})
                self.assertEqual({f'opcode:0x{op:02x}':count*packing for op,count in (+extrapolated).items()},
                                 {op:count for op,count in row['request']['program']['expected_route_counts'].items() if count})

    def test_wrong_child_key_and_parent_key_preserve_success_but_fail_semantics(self):
        m=self.load();original=m._runtime('cold-revert',4)[0];decoded=decode(original)
        for which in (0,1):
            code=bytearray(original);indices=[i for i,(_,op,_) in enumerate(decoded) if op==0x54]
            key_pc,op,word=decoded[indices[which]-1];self.assertEqual(int.from_bytes(word,'big'),2)
            code[key_pc+3]=3;result=execute_cold(bytes(code),transactions=2)
            self.assertEqual(result['terminals'],[0,0]);self.assertEqual(result['counts'],execute_cold(original,transactions=2)['counts'])
            with self.assertRaises(AssertionError):assert_cold(self,result,'cold-revert',4,2)
        code=bytearray(original);terminal=next(pc for pc,op,_ in decoded if op==0xfd);code[terminal]=0xf3
        self.assertEqual(execute_cold(bytes(code))['terminals'],[0xfe])
        code=bytearray(original);bound=next(pc for pc,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==1 and code[pc+4]==0x14)
        code[bound+3]=0;self.assertEqual(execute_cold(bytes(code))['terminals'],[0xfe])

    def test_strict_input_domains(self):
        m=self.load()
        for args in [('bad',4,'full'),(True,4,'full'),('cold-return',True,'full'),('cold-return',8,'full'),('cold-return',4,True),('cold-return',4,'bad')]:
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)
        for n in (-4,True,1,65540):
            with self.subTest(n=n),self.assertRaises(ValueError):m._runtime('cold-return',n)
        with tempfile.TemporaryDirectory() as temporary:
            for model in (None,True,'','A'*64,'g'*64):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'opt in frozen native/quote smoke')
    def test_native_gas_full_targets_and_quote(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            def native(row,name):
                folder=root/name;folder.mkdir();request=folder/'request.json';request.write_bytes(m.encoded(row['request']))
                process=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(request),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return process,folder
            for profile,packing,level in [('cold-return',4,'full'),('cold-revert',16,'full'),('parent-prewarm-revert',4,'full'),('cold-revert',4,'zero')]:
                row=m.make_case(profile,packing,level);process,folder=native(row,f'{profile}-{packing}-{level}')
                self.assertEqual(process.returncode,0,process.stderr.decode());report=json.loads((folder/'native.json').read_text());p=row['parameters']
                receipts=report['receipts_by_block'][0];self.assertEqual(len(receipts),packing+1);self.assertTrue(all(r['success'] for r in receipts));self.assertEqual(receipts[0]['gas_used'],167355)
                self.assertEqual([r['gas_used'] for r in receipts[1:]],[p['expected_user_gas_used']]*packing);self.assertEqual(report['blocks'][0]['block_gas_used'],p['expected_block_gas_used'])
                for op,count in row['request']['program']['expected_route_counts'].items():self.assertEqual(report['observed_route_counts'].get(op,0),count)
                quote=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--corpus-id','cold-frame-composition-smoke','--sample-id',row['sample_id'],'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                self.assertEqual(quote.returncode,0,quote.stderr.decode())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'),'opt in frozen native warmth/negative controls')
    def test_native_short_cold_difference_and_native_success_wrong_key(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);results={}
            def request(code,counts):
                row=m.make_case('cold-revert',4,'zero')
                routes={key:0 for key in row['request']['program']['expected_route_counts']}
                routes.update({key:count*4 for key,count in counts.items()})
                row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
                return row['request']
            def native(value,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(value))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            for profile in m.PROFILES:
                code,counts=m._runtime(profile,4);result,folder=native(request(code,counts),profile)
                self.assertEqual(result.returncode,0,result.stderr.decode());report=json.loads((folder/'native.json').read_text())
                results[profile]=[r['gas_used'] for r in report['receipts_by_block'][0][1:]]
                self.assertEqual(results[profile],[m._gas(profile,4)['charged_user_gas']]*4)
            self.assertEqual([a-b for a,b in zip(results['cold-revert'],results['cold-return'])],[8000]*4)
            self.assertEqual([a-b for a,b in zip(results['parent-prewarm-revert'],results['cold-return'])],[125]*4)
            original,counts=m._runtime('cold-revert',4);code=bytearray(original);decoded=decode(code)
            i=next(i for i,(_,op,_) in enumerate(decoded) if op==0x54);code[decoded[i-1][0]+3]=3
            result,folder=native(request(code,counts),'wrong-child-key')
            self.assertEqual(result.returncode,0,result.stderr.decode())
            report=json.loads((folder/'native.json').read_text());self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
            self.assertEqual([r['gas_used'] for r in report['receipts_by_block'][0][1:]],[m._gas('cold-revert',4)['charged_user_gas']]*4)
            with self.assertRaises(AssertionError):assert_cold(self,execute_cold(bytes(code),transactions=4),'cold-revert',4,4)
            code=bytearray(original);code[next(pc for pc,op,_ in decoded if op==0xfd)]=0xf3
            result,folder=native(request(code,counts),'wrong-child-terminal')
            self.assertNotEqual(result.returncode,0)


if __name__=='__main__':unittest.main()
