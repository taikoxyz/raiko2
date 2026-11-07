"""Independent concrete stack/memory qualification for balanced stack-operation pairs."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES=Path(__file__).resolve().parents[1]/'cases'
MASK=(1<<256)-1
SEEDS=(
    0xe13216c64d40f989e40231e5e7b8eed79c41c605441f3a85c23260c857014fc2,
    0xb02b21034cf64755f8a7d16b49321a341ec7e02d53af8c9965262fab2a92e8a3,
    0xfc4b00cd7a6c7379231f58c8d3577beb7d64b06c33a2c81f46ec0b982016223d,
    0xc8b6b425af7a4158243236dc2221495629abbac3216a115c2e65cbd8afcd65f2,
    0x886337607254191879ea411bf42751bbf547425b8cf1b2cffc95354d10731034,
    0xd684026160226c5359e8141ea438483296ec1c92883958f1fdf39ef3dfae46a2,
    0xb5dcf95660829243485e41f5c9fcb38e7fd473258d0589bbff5aa77da424c6e3,
    0x99be855a56d3f57b83fbfdfc03dd98dc395c745561aceafed1e5b0da064625d8,
    0xb52faa3208130ae0067be016c90230b6672822a9e05e54d65ae2161dcedec34b,
    0xf9562e2a3fb587d71799c8b8494f58d7250d0f1d5e697887222678be6d82a096,
    0xb992fcbf5f4aeeb08512eabf8208dcfd8d838e3bfab308ba5b49b0d90b2f3fed,
    0xbda08a276cdb58c65141defda9de0159d70bd7661ff26c048919b4fee239a86f,
    0x94ae92fdf25e8b3d858393cb8d7428ccd0376cbe3a8e2fd5acedb123f260cb79,
    0xb738c8a3ee5c34f433e642fe3822497abbf3eaf0f55e4b4cd2c1fa6b94f6dddc,
    0xdb286d843fca1fbd9c708c8f65205a1fb87b3e9e278568761e38e79317a35213,
    0xc6cb7cc024b5cf95fc27e525c17d5f0f4a18bd7b3aa171a408b0b5edaed73210,
    0x96d5ff3ab47f723d98b598b0573ef7f1acf9c7fb46aef30def8e91136fedef45,
)
PATTERNS={'push0-pop':(0x5f,0x50),'dup1-pop':(0x80,0x50),'dup3-pop':(0x82,0x50),
          'dup8-pop':(0x87,0x50),'dup16-pop':(0x8f,0x50),
          'swap1-swap1':(0x90,0x90),'swap8-swap8':(0x97,0x97),'swap16-swap16':(0x9f,0x9f)}


def instructions(code):
    rows=[];pc=0
    while pc<len(code):
        at=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code);rows.append((at,op,code[pc:pc+width]));pc+=width
    return rows


def pair_sites(code,pattern):
    a,b=PATTERNS[pattern];decoded=instructions(code);candidates=[i for i,(_,op,_) in enumerate(decoded) if op==a]
    assert len(candidates)==(128 if a==b else 64)
    selected=candidates[::2] if a==b else candidates
    starts=[decoded[i][0] for i in selected]
    assert all(decoded[i+1][:2]==(decoded[i][0]+1,b) for i in selected)
    assert starts==list(range(starts[0],starts[0]+128,2))
    return starts


def execute(code,body_pcs=()):
    stack=[];memory=bytearray();pc=0;counts=Counter();states=Counter();loads=Counter();stores=Counter()
    def grow(size):
        if len(memory)<size:memory.extend(bytes(size-len(memory)))
    for _ in range(4_000_000):
        at=pc;op=code[pc];pc+=1;counts[op]+=1
        if at in body_pcs:states[at,op,tuple(stack)]+=1
        if 0x60<=op<=0x7f:
            width=op-0x5f;stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width
        elif op==0x5f:stack.append(0)
        elif 0x80<=op<=0x8f:stack.append(stack[-(op-0x7f)])
        elif 0x90<=op<=0x9f:
            index=op-0x8f;stack[-1],stack[-1-index]=stack[-1-index],stack[-1]
        elif op==0x50:stack.pop()
        elif op==0x51:
            offset=stack.pop();grow(offset+32);value=int.from_bytes(memory[offset:offset+32],'big');loads[at,offset,value]+=1;stack.append(value)
        elif op==0x52:
            offset,value=stack.pop(),stack.pop();grow(offset+32);memory[offset:offset+32]=value.to_bytes(32,'big');stores[at,offset,value]+=1
        elif op==0x01:stack.append((stack.pop()+stack.pop())&MASK)
        elif op==0x14:stack.append(int(stack.pop()==stack.pop()))
        elif op==0x15:stack.append(int(stack.pop()==0))
        elif op in (0x56,0x57):
            target=stack.pop();take=op==0x56 or stack.pop()!=0
            if take:assert code[target]==0x5b;pc=target
        elif op==0x5b:pass
        elif op in (0x00,0xfe):return op,stack,bytes(memory),{f'opcode:0x{k:02x}':v for k,v in counts.items()},states,loads,stores
        else:raise AssertionError(f'unknown opcode{op:02x}')
    raise AssertionError('stack body failed to terminate')


def closed_counts(pattern,n):
    b=n//64;counts={0x62:4*b+4,0x52:b+1,0x7f:34,0x5b:b+2,0x51:2*b+1,0x14:b+18,
        0x61:2*b+18,0x57:b+18,0x60:b,0x01:b,0x56:b,0x15:17,0x00:1}
    for op in PATTERNS[pattern]:counts[op]=counts.get(op,0)+n
    return {f'opcode:0x{k:02x}':v for k,v in counts.items() if v}


class StackDensityTests(unittest.TestCase):
    def load(self):
        path=CASES/'stack_density.py';self.assertTrue(path.is_file(),'stack density producer is not implemented')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('stack_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_24_matrix_seeds_interfaces_hashes_and_nooverwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root),24);catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual(len(by),24);self.assertEqual({(r['parameters']['pattern'],r['parameters']['bulk_pairs']) for r in rows},{(p,n) for p in PATTERNS for n in (0,4096,262144)})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'stack_density.py').read_bytes()))
            self.assertEqual(set(catalog['source_sha256']),{'initcode_preparation.py','top_level_create.py','spawn_memory.py'})
            for row in rows:
                p=row['parameters'];raw=(root/row['request_path']).read_bytes();request=json.loads(raw)
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertEqual(tuple(int(v,16) for v in p['stack_words_bottom_to_top']),SEEDS)
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000});self.assertEqual(request['program']['input_hex'],'0x');self.assertEqual(request['program']['block_count'],1)
                self.assertTrue(request['expected_user_success']);self.assertEqual(p['body_stack_words'],17);self.assertEqual(p['counter_memory_offset'],0);self.assertEqual(p['expected_counter_final'],p['bulk_pairs']//64)
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['bulk_pairs'],0);self.assertEqual(by[row['baseline_sample_id']]['parameters']['pattern'],p['pattern'])
                if p['bulk_pairs']>4096:self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['bulk_pairs'],4096)
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other=Path(temporary)/'metadata';m.prepare(other,model_id='1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (root/'requests').iterdir()},{p.name:p.read_bytes() for p in (other/'requests').iterdir()})

    def test_concrete_stack_indices_values_memory_counter_and_closed_counts(self):
        m=self.load();lengths=set();self.assertEqual(len(set(SEEDS)),17);self.assertTrue(all(v.bit_length()==256 for v in SEEDS))
        for pattern,(a,b) in PATTERNS.items():
            codes=[]
            for n in (0,4096,262144):
                row=m.make_case(pattern,n);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);codes.append(code);lengths.add(len(code));decoded=instructions(code)
                constants=[int.from_bytes(data,'big') for _,op,data in decoded if op==0x7f];self.assertEqual(constants,list(SEEDS)+list(reversed(SEEDS)))
                sites=pair_sites(code,pattern);pcs={pc+offset for pc in sites for offset in (0,1)}
                terminal,stack,memory,counts,states,loads,stores=execute(code,pcs);self.assertEqual((terminal,stack),(0,[]));self.assertEqual(memory,(n//64).to_bytes(32,'big'))
                self.assertEqual(counts,closed_counts(pattern,n));self.assertEqual(counts,{k:v for k,v in row['request']['program']['expected_route_counts'].items() if v})
                for pc in sites:
                    self.assertEqual(states.get((pc,a,SEEDS),0),n//64,'counter must not occupy the body stack, especially DUP1')
                    changed=list(SEEDS)
                    if a==0x5f:changed.append(0)
                    elif 0x80<=a<=0x8f:changed.append(SEEDS[-(a-0x7f)])
                    else:index=a-0x8f;changed[-1],changed[-1-index]=changed[-1-index],changed[-1]
                    self.assertEqual(states.get((pc+1,b,tuple(changed)),0),n//64)
                mloads=[at for at,op,_ in decoded if op==0x51];self.assertEqual(len(mloads),2)
                self.assertEqual(loads,{**{(mloads[0],0,i):1 for i in range(n//64+1)},**{(mloads[1],0,i):1 for i in range(n//64)}})
                self.assertEqual(sum(stores.values()),n//64+1)
            self.assertLessEqual(sum(a!=b for a,b in zip(codes[0],codes[-1])),3)
        self.assertEqual(len(lengths),1,'two-byte pairs preserve code size across all eight patterns')

    def test_semantic_counter_and_access_mutations_are_detected(self):
        m=self.load();row=m.make_case('swap16-swap16',4096);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);decoded=instructions(code)
        bad=bytearray(code);at=[at for at,op,_ in decoded if op==0x7f][17];bad[at+32]^=1
        self.assertEqual(execute(bytes(bad))[0],0xfe)
        bad=bytearray(code);at=next(at for at,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==64);bad[at+1:at+4]=bytes(3)
        terminal,stack,_,counts,*_=execute(bytes(bad));self.assertEqual((terminal,stack),(0,[]));self.assertNotEqual(counts,closed_counts('swap16-swap16',4096))
        bad=bytearray(code);pc=pair_sites(code,'swap16-swap16')[0];bad[pc+1]=0x90
        self.assertEqual(execute(bytes(bad))[0],0xfe)
        row=m.make_case('dup1-pop',4096);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);pc=pair_sites(code,'dup1-pop')[0];bad=bytearray(code);bad[pc]=0x82
        terminal,stack,_,counts,states,*_=execute(bytes(bad),{pc,pc+1});self.assertEqual((terminal,stack),(0,[]));self.assertIn((pc+1,0x50,SEEDS+(SEEDS[-3],)),states)
        self.assertNotIn((pc+1,0x50,SEEDS+(SEEDS[-1],)),states,'balanced wrong DUP passes guards but accessed value oracle catches it')

    def test_typed_bounds_and_metadata_validation(self):
        m=self.load()
        for pattern,n in [(True,0),(None,0),('unknown',0),('dup2-pop',64),('dup1-pop',True),('dup1-pop',-64),('dup1-pop',1),('dup1-pop',262208)]:
            with self.subTest(pattern=pattern,n=n),self.assertRaises(ValueError):m.make_case(pattern,n)
        with tempfile.TemporaryDirectory() as temporary:
            for model in ('',None,True,'A'*64,'g'*64):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native and quote for24stack qualification')
    def test_actual_24_native_quotes_and_rejections(self):
        m=self.load();model=os.environ.get('ZKGAS_EXPECTED_MODEL_ID',m.MODEL)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases',model_id=model)
            def run(request,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(request))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                request=json.loads((root/'cases'/row['request_path']).read_text());result,folder=run(request,row['sample_id']);self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());self.assertEqual({k:v for k,v in native['observed_route_counts'].items() if v},{k:v for k,v in request['program']['expected_route_counts'].items() if v})
                self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0);self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                result=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','stack-density-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(result.returncode,0,result.stderr.decode());quote=json.loads((folder/'quote.jsonl').read_text())['quote'];self.assertEqual(quote['model_id'],model);self.assertEqual(quote['public_output'],native['public_output']);self.assertEqual(quote['guest_input_sha256'],native['guest_input_sha256'])
            for name in ('wrong-ending-word','short-loop','broken-swap','parent-oog'):
                row=m.make_case('swap16-swap16',4096);request=row['request'];code=bytearray.fromhex(request['program']['code_hex'][2:]);decoded=instructions(code)
                if name=='wrong-ending-word':at=[at for at,op,_ in decoded if op==0x7f][17];code[at+32]^=1
                elif name=='short-loop':at=next(at for at,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==64);code[at+1:at+4]=bytes(3)
                elif name=='broken-swap':code[pair_sites(bytes(code),'swap16-swap16')[0]+1]=0x90
                else:request['program']['transactions']['gas_limit']=21000
                request['program']['code_hex']='0x'+code.hex();result,folder=run(request,name);self.assertNotEqual(result.returncode,0);self.assertFalse((folder/'input.bin').exists());self.assertFalse((folder/'native.json').exists())


if __name__=='__main__':unittest.main()
