"""Independent decoding and frozen-native qualification for repeated initcode factories."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'
MODEL = 'a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e'


def execute_factory(code, fail_creation=False):
    """Independent small EVM for generated control flow, memory and child-input decoding."""
    stack, memory, counts, children, pc = [], bytearray(), Counter(), [], 0
    def grow(end):
        if end > len(memory): memory.extend(bytes(end-len(memory)))
    for _ in range(200_000):
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op-0x5f; assert pc+width <= len(code)
            stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width
        elif op == 0x80: stack.append(stack[-1])
        elif op == 0x01: stack.append((stack.pop()+stack.pop()) % 2**256)
        elif op == 0x14: stack.append(int(stack.pop()==stack.pop()))
        elif op == 0x15: stack.append(int(stack.pop()==0))
        elif op == 0x50: stack.pop()
        elif op == 0x53:
            offset, value = stack.pop(), stack.pop(); grow(offset+1); memory[offset] = value & 255
        elif op == 0x5e:
            destination, source, length = stack.pop(), stack.pop(), stack.pop()
            grow(max(source+length,destination+length)); memory[destination:destination+length] = bytes(memory[source:source+length])
        elif op in (0x56,0x57):
            target = stack.pop(); take = op==0x56 or stack.pop()!=0
            if take: assert code[target]==0x5b; pc=target
        elif op == 0x5b: pass
        elif op in (0xf0,0xf5):
            value,offset,length = stack.pop(),stack.pop(),stack.pop()
            salt = stack.pop() if op==0xf5 else None
            assert value==0; grow(offset+length); image=bytes(memory[offset:offset+length]); children.append((image,salt))
            if fail_creation: stack.append(0); continue
            if not image or image[0]==0: counts['opcode:0x00'] += 1
            else:
                assert image[:5]==bytes.fromhex('60016000f3')
                counts['opcode:0x60'] += 2; counts['opcode:0xf3'] += 1
            stack.append(len(children))  # Stand-in success address; real addresses checked natively.
        elif op in (0x00,0xfd,0xfe):
            if op==0xfd: assert stack.pop()==stack.pop()==0
            return children,dict(counts),op,bytes(memory),stack
        else: raise AssertionError(f'unsupported opcode {op:02x}')
    raise AssertionError('factory did not terminate')


class InitcodePreparationTests(unittest.TestCase):
    def load(self):
        path=CASES/'initcode_preparation.py'
        self.assertTrue(path.is_file(),'repeated initcode producer is not implemented')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('initcode_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_exact_24_matrix_hashes_references_and_no_overwrite(self):
        m=self.load()
        expected={(f,l,n,'jumpdest',0,False) for f in ('create','create2') for l in (4096,49152) for n in (1,16,128)}
        expected|={(f,49152,16,p,0,False) for f in ('create','create2') for p in ('zero','push32')}
        expected|={(f,49152,0,'jumpdest',0,False) for f in ('create','create2')}
        expected|={(f,49152,16,'jumpdest',0,True) for f in ('create','create2')}
        expected|={(f,49152,16,'jumpdest',1,False) for f in ('create','create2')}
        expected|={(f,0,16,'zero',0,False) for f in ('create','create2')}
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root),24)
            catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];ids={r['sample_id'] for r in rows};keys=set();requests=set()
            self.assertEqual(len(ids),24);self.assertEqual(catalog['frozen_model_id'],MODEL)
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'initcode_preparation.py').read_bytes()))
            for name in ('top_level_create.py','spawn_memory.py'):self.assertEqual(catalog['source_sha256'][name],m.digest((CASES/name).read_bytes()))
            for row in rows:
                p=row['parameters'];keys.add((p['creation_family'],p['initcode_bytes'],p['repetitions'],p['tail_pattern'],p['child_output_bytes'],p['outer_revert']))
                raw=(root/row['request_path']).read_bytes();request=json.loads(raw);requests.add(raw);self.assertEqual(m.digest(raw),row['request_sha256'])
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000})
                self.assertEqual(request['program']['block_count'],1);self.assertEqual(request['program']['input_hex'],'0x')
                self.assertEqual(request['expected_user_success'],not p['outer_revert']);self.assertIn(row['baseline_sample_id'],ids)
                base=next(r for r in rows if r['sample_id']==row['baseline_sample_id']);self.assertEqual(base['parameters']['creation_family'],p['creation_family'])
                self.assertEqual(p['factory_runtime_sha256'],m.digest(bytes.fromhex(request['program']['code_hex'][2:])))
                self.assertEqual(p['expected_factory_nonce'],0 if p['outer_revert'] else p['repetitions'])
                self.assertEqual(p['expected_code_preparation'],{'nonempty_events':p['repetitions']*p['child_output_bytes'],'total_bytes':p['repetitions']*p['child_output_bytes']})
            self.assertEqual(keys,expected);self.assertEqual(len(requests),24)
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_independent_memory_loop_child_success_and_revert_paths(self):
        m=self.load()
        for family in ('create','create2'):
            lengths=[]
            for repeats in (0,1,16,128):
                row=m.make_case(family,49152,repeats);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);lengths.append(len(code))
                children,counts,terminal,memory,stack=execute_factory(code)
                self.assertEqual(len(children),repeats);self.assertEqual(terminal,0);self.assertEqual(stack,[])
                self.assertEqual(memory[:49152],b'\x00'+b'\x5b'*49151)
                self.assertTrue(all(image==memory[:49152] for image,salt in children))
                self.assertEqual([s for _,s in children],list(range(repeats)) if family=='create2' else [None]*repeats)
                for key,value in row['request']['program']['expected_route_counts'].items():self.assertEqual(counts.get(key,0),value,(key,repeats))
            self.assertEqual(len(set(lengths)),1,'loop body must not expand with repetitions')
            for pattern in ('zero','push32'):
                row=m.make_case(family,49152,16,pattern=pattern);children,counts,terminal,_,_=execute_factory(bytes.fromhex(row['request']['program']['code_hex'][2:]))
                tail=bytes(49151) if pattern=='zero' else ((b'\x7f'+b'\x5b'*32)*1490)[:49151]
                self.assertEqual(children[0][0],b'\x00'+tail)
            for output,revert in ((0,True),(1,False)):
                row=m.make_case(family,49152,16,child_output_bytes=output,outer_revert=revert)
                children,counts,terminal,_,_=execute_factory(bytes.fromhex(row['request']['program']['code_hex'][2:]))
                self.assertEqual(terminal,0xfd if revert else 0);self.assertEqual(len(children),16)
                for key,value in row['request']['program']['expected_route_counts'].items():self.assertEqual(counts.get(key,0),value,key)
            row=m.make_case(family,0,16,pattern='zero');children,counts,terminal,_,_=execute_factory(bytes.fromhex(row['request']['program']['code_hex'][2:]))
            self.assertEqual([x for x,_ in children],[b'']*16);self.assertEqual(counts['opcode:0x00'],17)
            row=m.make_case(family,4096,1);_,_,terminal,_,_=execute_factory(bytes.fromhex(row['request']['program']['code_hex'][2:]),True)
            self.assertEqual(terminal,0xfe,'zero creation address must fail before normal/revert terminal')

    def test_typed_bounds_and_invalid_combinations(self):
        m=self.load()
        for kwargs in [dict(family=True),dict(family='unknown'),dict(length=True),dict(length=-1),dict(length=49153),dict(repeats=True),dict(repeats=-1),dict(repeats=2049),dict(pattern=[]),dict(pattern='unknown'),dict(outer_revert=1),dict(child_output_bytes=True),dict(child_output_bytes=2),dict(length=0),dict(length=4,child_output_bytes=1)]:
            args=dict(family='create',length=49152,repeats=16);args.update(kwargs)
            with self.subTest(kwargs=kwargs),self.assertRaises(ValueError):m.make_case(**args)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native and quote for all24 actual gates')
    def test_all_24_real_native_and_quote_preparation_gates(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                with self.subTest(sample=row['sample_id']):
                    p=row['parameters'];folder=root/row['sample_id'];folder.mkdir()
                    result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(root/'cases'/row['request_path']),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                    self.assertEqual(result.returncode,0,result.stderr.decode());native=json.loads((folder/'native.json').read_text())
                    self.assertEqual(native['transaction_count'],2);self.assertTrue(native['receipts_by_block'][0][0]['success'])
                    self.assertEqual(native['receipts_by_block'][0][1]['success'],not p['outer_revert'])
                    self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                    self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0)
                    self.assertEqual(native['transactions']['blocks'][0]['users'][0]['nonce'],0)
                    self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],p['expected_factory_nonce'])
                    request=json.loads((root/'cases'/row['request_path']).read_text())
                    for key,value in request['program']['expected_route_counts'].items():self.assertEqual(native['observed_route_counts'].get(key,0),value)
                    result=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','initcode-preparation-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                    self.assertEqual(result.returncode,0,result.stderr.decode());q=json.loads((folder/'quote.jsonl').read_text())['quote'];self.assertEqual(q['model_id'],MODEL)
                    self.assertEqual(q['guest_input_sha256'],native['guest_input_sha256']);self.assertEqual(q['public_output'],native['public_output'])
                    self.assertEqual(q['blocks'][0]['node']['pricing']['code_preparation'],p['expected_code_preparation'])
                    self.assertEqual(q['blocks'][0]['prove']['code_preparation'],p['expected_code_preparation'])


if __name__=='__main__':unittest.main()
