"""Independently decode arithmetic operands, modulo results and completed operation maps."""
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
A=0xeb8195ab884b4adbc98e7be3b001ebf9c6102e4accd653429a8f027175193cfc
B=0xbbb7d14f635b0739ae021b674541c9882e973094d5e2099ab396af9bd5f9debe
PAIRS={'add':{'zero':(0,0),'small':(7,11),'carry':(MASK,1),'mixed256':(A,B)},
       'mul':{'zero':(0,0),'small':(7,11),'wide':(MASK,MASK-2),'mixed256':(A,B)}}


def instructions(code):
    pc=0;rows=[]
    while pc<len(code):
        at=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code);rows.append((at,op,code[pc:pc+width]));pc+=width
    return rows


def execute(code):
    """Concrete stack oracle records actual ordered top-first pairs at every ADD/MUL PC."""
    stack=[];pc=0;counts=Counter();events=Counter()
    for _ in range(4_000_000):
        at=pc;op=code[pc];pc+=1;counts[op]+=1
        if 0x60<=op<=0x7f:
            width=op-0x5f;stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width
        elif 0x80<=op<=0x82:stack.append(stack[-(op-0x7f)])
        elif op in (0x01,0x02):
            a,b=stack.pop(),stack.pop();result=(a+b if op==0x01 else a*b)&MASK
            events[at,op,a,b,result]+=1;stack.append(result)
        elif op==0x14:stack.append(int(stack.pop()==stack.pop()))
        elif op==0x15:stack.append(int(stack.pop()==0))
        elif op==0x50:stack.pop()
        elif op in (0x56,0x57):
            target=stack.pop();take=op==0x56 or stack.pop()!=0
            if take:assert code[target]==0x5b;pc=target
        elif op==0x5b:pass
        elif op in (0x00,0xfe):return op,stack,{f'opcode:0x{k:02x}':v for k,v in counts.items()},events
        else:raise AssertionError(f'unsupported opcode {op:02x}')
    raise AssertionError('arithmetic loop did not terminate')


def target_sites(code,opcode):
    decoded=instructions(code)
    warm=[at for i,(at,op,_) in enumerate(decoded) if i>=2 and op==opcode and [r[1] for r in decoded[i-2:i]]==[0x81,0x81]]
    bulk=[at for i,(at,op,_) in enumerate(decoded) if i>=2 and op==opcode and [r[1] for r in decoded[i-2:i]]==[0x82,0x82]]
    assert len(warm)==1 and len(bulk)==64
    return warm[0],bulk


class ArithmeticDensityTests(unittest.TestCase):
    def load(self):
        path=CASES/'arithmetic_density.py';self.assertTrue(path.is_file(),'arithmetic density producer is not implemented')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('arithmetic_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_exact_matrix_interfaces_hashes_references_and_no_overwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root),24)
            catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual(len(by),24);self.assertEqual(catalog['frozen_model_id'],m.MODEL)
            self.assertEqual({(r['parameters']['operation'],r['parameters']['operand_class'],r['parameters']['bulk_operations']) for r in rows},
                {(op,cls,n) for op in PAIRS for cls in PAIRS[op] for n in (0,4096,262144)})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'arithmetic_density.py').read_bytes()))
            self.assertEqual(set(catalog['source_sha256']),{'initcode_preparation.py','top_level_create.py','spawn_memory.py'})
            for name,digest in catalog['source_sha256'].items():self.assertEqual(digest,m.digest((CASES/name).read_bytes()))
            for row in rows:
                raw=(root/row['request_path']).read_bytes();request=json.loads(raw);p=row['parameters'];a,b=PAIRS[p['operation']][p['operand_class']]
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertEqual((int(p['operand_a_hex'],16),int(p['operand_b_hex'],16)),(a,b))
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000})
                self.assertEqual(request['program']['input_hex'],'0x');self.assertEqual(request['program']['block_count'],1);self.assertTrue(request['expected_user_success'])
                self.assertEqual(p['target_pair_events'],p['bulk_operations']+1);self.assertEqual(p['counter_ADD_events'],p['bulk_operations']//64)
                self.assertEqual(p['completed_target_opcode_events'],p['bulk_operations']+1+(p['bulk_operations']//64 if p['operation']=='add' else 0))
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['bulk_operations'],0)
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['operand_class'],p['operand_class'])
                if p['bulk_operations']>4096:self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['bulk_operations'],4096)
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other=Path(temporary)/'metadata';m.prepare(other,model_id='1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (root/'requests').iterdir()},{p.name:p.read_bytes() for p in (other/'requests').iterdir()})

    def test_actual_ordered_operands_modulo_results_fixed_sites_and_counter(self):
        m=self.load();lengths=set()
        for operation,pairs in PAIRS.items():
            opcode=1 if operation=='add' else 2
            for cls,(a,b) in pairs.items():
                codes=[]
                for n in (0,4096,262144):
                    row=m.make_case(operation,cls,n);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);codes.append(code);lengths.add(len(code))
                    decoded=instructions(code);self.assertEqual([(op,int.from_bytes(data,'big')) for _,op,data in decoded[:2]],[(0x7f,b),(0x7f,a)])
                    self.assertEqual(sum(op==0x7f for _,op,_ in decoded),3,'operand and expected-result immediates all PUSH32')
                    warm,bulk=target_sites(code,opcode);terminal,stack,counts,events=execute(code)
                    self.assertEqual((terminal,stack),(0,[]));expected=(a+b if operation=='add' else a*b)&MASK
                    self.assertEqual(int(row['parameters']['expected_result_hex'],16),expected)
                    target={key:value for key,value in events.items() if key[0] in {warm,*bulk}}
                    self.assertEqual(target,{(warm,opcode,a,b,expected):1}|({(pc,opcode,a,b,expected):n//64 for pc in bulk} if n else {}))
                    counter={key:value for key,value in events.items() if key[0] not in {warm,*bulk}}
                    self.assertEqual(sum(counter.values()),n//64)
                    self.assertEqual({(a,b,r) for (_,op,a,b,r) in counter},{(1,i,i+1) for i in range(n//64)})
                    self.assertTrue(all(op==1 and value==1 for (_,op,_,_,_),value in counter.items()))
                    self.assertEqual(counts,{key:value for key,value in row['request']['program']['expected_route_counts'].items() if value})
                self.assertLessEqual(sum(a!=b for a,b in zip(codes[0],codes[-1])),3,'only fixed-width batch-count immediate changes')
        self.assertEqual(len(lengths),1,'same-width arithmetic programs across operations/operand classes')
        self.assertEqual((PAIRS['add']['carry'][0]+1)&MASK,0);self.assertEqual((PAIRS['mul']['wide'][0]*PAIRS['mul']['wide'][1])&MASK,3)
        self.assertNotEqual(A,B);self.assertGreaterEqual(A,1<<255);self.assertGreaterEqual(B,1<<255)

    def test_semantic_guard_short_execution_and_operand_mutations(self):
        m=self.load()
        for operation in PAIRS:
            row=m.make_case(operation,'mixed256',4096);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);decoded=instructions(code);opcode=1 if operation=='add' else 2;warm,bulk=target_sites(code,opcode)
            bad=bytearray(code);at=[at for at,op,_ in decoded if op==0x7f][2];bad[at+32]^=1
            terminal,_,_,events=execute(bytes(bad));self.assertEqual(terminal,0xfe);self.assertEqual(sum(value for key,value in events.items() if key[0] in {warm,*bulk}),1)
            bad=bytearray(code);at=next(at for at,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==64);bad[at+1:at+4]=bytes(3)
            terminal,stack,counts,events=execute(bytes(bad));self.assertEqual((terminal,stack),(0,[]));self.assertNotEqual(counts,{k:v for k,v in row['request']['program']['expected_route_counts'].items() if v})
            # Commutativity preserves result, guards and exact opcode totals, but
            # does not justify silently changing the declared ordered pair.
            swapped=code[33:66]+code[:33]+code[66:];terminal,stack,counts,events=execute(swapped)
            self.assertEqual((terminal,stack),(0,[]));self.assertEqual(counts,{k:v for k,v in row['request']['program']['expected_route_counts'].items() if v})
            self.assertEqual({(a,b) for (pc,op,a,b,_) in events if pc in {warm,*bulk}},{(B,A)})
            self.assertNotEqual((B,A),(A,B))
            # A bad DUP changes one target operand to a copy of the other.
            bad=bytearray(code);bad[bulk[0]-2]=0x81;terminal,stack,counts,events=execute(bytes(bad));self.assertEqual((terminal,stack),(0,[]))
            self.assertIn((bulk[0],opcode,A,A,(A+A if operation=='add' else A*A)&MASK),events)

    def test_typed_options_and_bounds(self):
        m=self.load()
        for op,cls,n in [(True,'zero',0),('unknown','zero',0),('add',True,0),('add','wide',64),('mul','carry',64),('add','zero',True),('mul','small',-64),('add','zero',1),('mul','wide',262208)]:
            with self.subTest(operation=op,cls=cls,count=n),self.assertRaises(ValueError):m.make_case(op,cls,n)
        with tempfile.TemporaryDirectory() as temporary:
            for model in (None,True,'','A'*64,'g'*64):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',model_id=model)
            self.assertFalse((Path(temporary)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native/quote for arithmetic qualification')
    def test_actual_24_native_quotes_and_mutation_controls(self):
        m=self.load();model=os.environ.get('ZKGAS_EXPECTED_MODEL_ID',m.MODEL)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases',model_id=model)
            def run(request,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(request))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                request=json.loads((root/'cases'/row['request_path']).read_text());result,folder=run(request,row['sample_id']);self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());self.assertEqual(native['transaction_count'],2);self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0)
                self.assertEqual({k:v for k,v in native['observed_route_counts'].items() if v},{k:v for k,v in request['program']['expected_route_counts'].items() if v})
                self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],0)
                result=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','arithmetic-density-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(result.returncode,0,result.stderr.decode());quote=json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(quote['model_id'],model);self.assertEqual(quote['public_output'],native['public_output']);self.assertEqual(quote['guest_input_sha256'],native['guest_input_sha256'])
                self.assertEqual(quote['blocks'][0]['node']['pricing']['code_preparation'],{'nonempty_events':0,'total_bytes':0})
                self.assertEqual(quote['node']['block_local_known_pgu'],quote['blocks'][0]['node']['pricing']['known_subtotal_pgu'])
            for name in ('wrong-result','short-execution','parent-oog','swapped-ordered-operands'):
                row=m.make_case('mul','mixed256',4096);request=row['request'];code=bytearray.fromhex(request['program']['code_hex'][2:]);decoded=instructions(code)
                if name=='wrong-result':at=[at for at,op,_ in decoded if op==0x7f][2];code[at+32]^=1
                elif name=='short-execution':at=next(at for at,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==64);code[at+1:at+4]=bytes(3)
                elif name=='parent-oog':request['program']['transactions']['gas_limit']=21000
                else:code=code[33:66]+code[:33]+code[66:]
                request['program']['code_hex']='0x'+code.hex();result,folder=run(request,name)
                if name=='swapped-ordered-operands':
                    self.assertEqual(result.returncode,0,result.stderr.decode());warm,bulk=target_sites(bytes(code),2);events=execute(bytes(code))[3]
                    self.assertEqual({(a,b) for (pc,_,a,b,_) in events if pc in {warm,*bulk}},{(B,A)})
                else:self.assertNotEqual(result.returncode,0);self.assertFalse((folder/'input.bin').exists());self.assertFalse((folder/'native.json').exists())


if __name__=='__main__':unittest.main()
