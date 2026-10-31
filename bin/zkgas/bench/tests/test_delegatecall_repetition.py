"""Independent tiny-frame decoder and frozen native gates for DELEGATECALL repetition."""
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
MODEL='a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e'


def execute(code,override_status=None,override_return_size=None):
    """Interpret actual bytecode, including CREATE initcode and each tiny child frame."""
    completed,attempted=Counter(),Counter();calls=[];children=[];exceptional=0
    def frame(body,child=False):
        nonlocal exceptional
        stack=[];memory=bytearray();pc=0;return_size=0
        def grow(end):
            if end>len(memory):memory.extend(bytes(end-len(memory)))
        for _ in range(2_000_000):
            op=body[pc] if pc<len(body) else 0;pc+=1;key=f'opcode:0x{op:02x}';attempted[key]+=1
            if op==0xfe:exceptional+=1;return False,b''
            completed[key]+=1
            if op==0x5f:stack.append(0)
            elif 0x60<=op<=0x7f:
                width=op-0x5f;assert pc+width<=len(body);stack.append(int.from_bytes(body[pc:pc+width],'big'));pc+=width
            elif 0x80<=op<=0x8f:stack.append(stack[-(op-0x7f)])
            elif op==0x01:stack.append((stack.pop()+stack.pop())%2**256)
            elif op==0x14:stack.append(int(stack.pop()==stack.pop()))
            elif op==0x15:stack.append(int(stack.pop()==0))
            elif op==0x50:stack.pop()
            elif op==0x52:
                offset,value=stack.pop(),stack.pop();grow(offset+32);memory[offset:offset+32]=value.to_bytes(32,'big')
            elif op in (0x56,0x57):
                target=stack.pop();take=op==0x56 or stack.pop()!=0
                if take:assert body[target]==0x5b;pc=target
            elif op==0x5b:pass
            elif op==0x3d:stack.append(return_size)
            elif op==0xf0:
                value,offset,size=stack.pop(),stack.pop(),stack.pop();assert value==0;grow(offset+size)
                ok,runtime=frame(bytes(memory[offset:offset+size]),True);assert ok
                children.append(runtime);stack.append(0x1234);return_size=0
            elif op==0xf4:
                gas,address,io,ins,oo,outs=[stack.pop() for _ in range(6)]
                assert (gas,address,io,ins,oo,outs)==(64,0x1234,0,0,0,0)
                ok,result=frame(children[0],True);calls.append((ok,len(result),gas))
                stack.append(int(ok) if override_status is None else override_status)
                return_size=len(result) if override_return_size is None else override_return_size
            elif op==0:assert child or not stack;return True,b''
            elif op in (0xf3,0xfd):
                offset,size=stack.pop(),stack.pop();grow(offset+size);return op==0xf3,bytes(memory[offset:offset+size])
            else:raise AssertionError(f'unsupported opcode {op:02x}')
        raise AssertionError('unterminated parent loop')
    ok,result=frame(code)
    return {'success':ok,'completed':dict(completed),'attempted':dict(attempted),'children':children,'calls':calls,'exceptional':exceptional,'result':result}


class DelegatecallRepetitionTests(unittest.TestCase):
    def load(self):
        path=CASES/'delegatecall_repetition.py';self.assertTrue(path.is_file(),'DELEGATECALL producer is not implemented')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('delegatecall_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_exact_sixteen_matrix_catalog_hashes_envelope_and_no_overwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            out=Path(temporary)/'cases';self.assertEqual(m.prepare(out),16);catalog=json.loads((out/'catalog.json').read_text());rows=catalog['cases'];ids={r['sample_id'] for r in rows}
            self.assertEqual(len(ids),16);self.assertEqual(catalog['frozen_model_id'],MODEL)
            self.assertEqual({(r['parameters']['child_terminal'],r['parameters']['repetitions']) for r in rows},{(t,n) for t in ('stop','return0','revert0','invalid') for n in (0,1,1024,32768)})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'delegatecall_repetition.py').read_bytes()))
            for name in ('initcode_preparation.py','top_level_create.py','spawn_memory.py'):self.assertEqual(catalog['source_sha256'][name],m.digest((CASES/name).read_bytes()))
            requests=set()
            for row in rows:
                p=row['parameters'];raw=(out/row['request_path']).read_bytes();request=json.loads(raw);requests.add(raw)
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertTrue(request['expected_user_success'])
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000})
                self.assertEqual(request['program']['block_count'],1);self.assertEqual(request['program']['input_hex'],'0x')
                self.assertEqual(p['forwarded_gas'],64);self.assertEqual(p['expected_factory_nonce'],1);self.assertEqual(p['expected_child_creations'],1)
                if p['child_terminal'] in ('revert0','invalid'):
                    self.assertEqual(row['role'],'control');self.assertEqual(row['data_role'],'high_price_failure_control')
                base=next(r for r in rows if r['sample_id']==row['baseline_sample_id']);self.assertEqual(base['parameters']['child_terminal'],p['child_terminal']);self.assertEqual(base['parameters']['repetitions'],0)
                self.assertEqual(p['factory_runtime_sha256'],m.digest(bytes.fromhex(request['program']['code_hex'][2:])))
            self.assertEqual(len(requests),16)
            before={p.relative_to(out):p.read_bytes() for p in out.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(out)
            self.assertEqual(before,{p.relative_to(out):p.read_bytes() for p in out.rglob('*') if p.is_file()})

    def test_actual_parent_child_decode_counts_fixed_body_and_both_guards(self):
        m=self.load();runtimes={'stop':b'\x00','return0':bytes.fromhex('60006000f3'),'revert0':bytes.fromhex('60006000fd'),'invalid':b'\xfe'}
        for terminal,runtime in runtimes.items():
            lengths=[]
            for count in (0,1,1024,32768):
                row=m.make_case(terminal,count);program=row['request']['program'];code=bytes.fromhex(program['code_hex'][2:]);lengths.append(len(code));result=execute(code)
                self.assertTrue(result['success']);self.assertEqual(result['children'],[runtime]);self.assertEqual(len(result['calls']),count)
                self.assertEqual(result['calls'],[(terminal in ('stop','return0'),0,64)]*count)
                for key,value in program['expected_route_counts'].items():self.assertEqual(result['completed'].get(key,0),value,(terminal,count,key))
                self.assertEqual(result['completed'].get('opcode:0xfe',0),0)
                self.assertEqual(result['attempted'].get('opcode:0xfe',0),count if terminal=='invalid' else 0)
                self.assertEqual(result['exceptional'],count if terminal=='invalid' else 0)
            self.assertEqual(len(set(lengths)),1,'counter immediate must not change loop body size')
            one=m.make_case(terminal,1);code=bytes.fromhex(one['request']['program']['code_hex'][2:])
            self.assertFalse(execute(code,override_status=int(terminal not in ('stop','return0')))['success'])
            self.assertFalse(execute(code,override_return_size=1)['success'])

    def test_typed_parameters(self):
        m=self.load()
        for terminal,count in [(True,1),('unknown',1),([],1),('stop',True),('stop',-1),('stop',32769),('stop',1.5)]:
            with self.subTest(terminal=terminal,count=count),self.assertRaises(ValueError):m.make_case(terminal,count)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native and quote for all16 gates')
    def test_all_sixteen_real_native_and_current_quote(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                with self.subTest(sample=row['sample_id']):
                    folder=root/row['sample_id'];folder.mkdir();p=row['parameters']
                    result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(root/'cases'/row['request_path']),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                    self.assertEqual(result.returncode,0,result.stderr.decode());native=json.loads((folder/'native.json').read_text())
                    self.assertEqual(native['transaction_count'],2);self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                    self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],1)
                    self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],p['expected_exceptional_instructions'])
                    self.assertEqual(native['observed_route_counts'].get('opcode:0xfe',0),0)
                    self.assertEqual(native['attempted_route_counts'].get('opcode:0xfe',0),p['expected_exceptional_instructions'])
                    request=json.loads((root/'cases'/row['request_path']).read_text())
                    for key,value in request['program']['expected_route_counts'].items():self.assertEqual(native['observed_route_counts'].get(key,0),value)
                    result=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','delegatecall-repetition-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                    self.assertEqual(result.returncode,0,result.stderr.decode());q=json.loads((folder/'quote.jsonl').read_text())['quote'];self.assertEqual(q['model_id'],MODEL)
                    self.assertEqual(q['guest_input_sha256'],native['guest_input_sha256']);self.assertEqual(q['public_output'],native['public_output'])
                    self.assertEqual(q['blocks'][0]['node']['pricing']['code_preparation'],p['expected_code_preparation']);self.assertEqual(q['blocks'][0]['prove']['code_preparation'],p['expected_code_preparation'])


if __name__=='__main__':unittest.main()
