"""Decode dense BLOCKHASH targets/control flow independently and qualify frozen hosts."""
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


def instructions(code):
    rows=[];pc=0
    while pc<len(code):
        start=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code);rows.append((start,op,code[pc:pc+width]));pc+=width
    return rows


def execute(code, number=600):
    """Small independent EVM, with distinct nonzero hashes only for last256 ancestors."""
    stack=[];pc=0;counts=Counter();queries=[]
    for _ in range(4_000_000):
        op=code[pc];pc+=1;counts[op]+=1
        if 0x60<=op<=0x7f:
            width=op-0x5f;stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width
        elif op==0x5f:stack.append(0)
        elif op==0x43:stack.append(number)
        elif op in (0x01,0x03):
            a,b=stack.pop(),stack.pop();stack.append((a+b if op==0x01 else a-b)%2**256)
        elif op==0x80:stack.append(stack[-1])
        elif op==0x81:stack.append(stack[-2])
        elif op==0x40:
            target=stack.pop();queries.append(target);stack.append(target+0xabc if number-256<=target<number else 0)
        elif op==0x15:stack.append(int(stack.pop()==0))
        elif op==0x14:stack.append(int(stack.pop()==stack.pop()))
        elif op==0x50:stack.pop()
        elif op in (0x56,0x57):
            target=stack.pop();take=op==0x56 or stack.pop()!=0
            if take:assert code[target]==0x5b;pc=target
        elif op==0x5b:pass
        elif op in (0x00,0xfe):return op,{f'opcode:0x{k:02x}':v for k,v in counts.items()},queries,stack
        else:raise AssertionError(f'unsupported opcode {op:02x}')
    raise AssertionError('dense BLOCKHASH program did not terminate')


class BlockhashDensityTests(unittest.TestCase):
    def load(self):
        path=CASES/'blockhash_density.py'
        self.assertTrue(path.is_file(),'BLOCKHASH density producer is not implemented')
        sys.path.insert(0,str(CASES));spec=importlib.util.spec_from_file_location('blockhash_cases',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

    def test_twelve_matrix_interfaces_hashes_references_and_no_overwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root),12)
            catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual(len(by),12);self.assertEqual(catalog['frozen_model_id'],m.MODEL)
            self.assertEqual({(r['parameters']['mode'],r['parameters']['bulk_queries']) for r in rows},{(mode,n) for mode in ('nearest','farthest','expired') for n in (0,64,4096,262144)})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'blockhash_density.py').read_bytes()))
            self.assertEqual(set(catalog['source_sha256']),{'initcode_preparation.py','top_level_create.py','spawn_memory.py'})
            for name,sha in catalog['source_sha256'].items():self.assertEqual(sha,m.digest((CASES/name).read_bytes()))
            for row in rows:
                p=row['parameters'];raw=(root/row['request_path']).read_bytes();request=json.loads(raw)
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertEqual(request,m.make_case(p['mode'],p['bulk_queries'])['request'])
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000})
                self.assertEqual(request['program']['block_count'],1);self.assertEqual(request['program']['input_hex'],'0x');self.assertTrue(request['expected_user_success'])
                self.assertEqual(p['total_blockhash_queries'],p['bulk_queries']+1);self.assertEqual(p['batch_count']*64,p['bulk_queries'])
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['bulk_queries'],0);self.assertEqual(by[row['baseline_sample_id']]['parameters']['mode'],p['mode'])
                if p['bulk_queries']>64:self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['bulk_queries'],64)
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})
            other=Path(temporary)/'other';self.assertEqual(m.prepare(other,model_id='1'*64),12)
            self.assertEqual(json.loads((other/'catalog.json').read_text())['frozen_model_id'],'1'*64)
            self.assertEqual({p.name:p.read_bytes() for p in (root/'requests').iterdir()},{p.name:p.read_bytes() for p in (other/'requests').iterdir()})

    def test_independent_targets_fixed_body_full_counts_and_semantic_guard(self):
        m=self.load()
        for mode,distance in [('nearest',1),('farthest',256),('expired',257),('current',0),('future',-1)]:
            codes=[]
            for n in (0,64,4096,262144) if mode in ('nearest','farthest','expired') else (0,64):
                row=m.make_case(mode,n);code=bytes.fromhex(row['request']['program']['code_hex'][2:]);codes.append(code);decoded=instructions(code)
                self.assertEqual(sum(op==0x40 for _,op,_ in decoded),65,'N0 retains warmup and unused64-query body')
                self.assertEqual(sum(op==0x81 for _,op,_ in decoded),64);self.assertEqual(sum(op==0x57 for _,op,_ in decoded),2)
                terminal,counts,targets,stack=execute(code);self.assertEqual(terminal,0);self.assertEqual(stack,[]);self.assertEqual(targets,[600-distance]*(n+1))
                for key,value in row['request']['program']['expected_route_counts'].items():self.assertEqual(counts.get(key,0),value,(mode,n,key))
                self.assertEqual(counts.get('opcode:0x40',0),n+1);self.assertEqual(counts.get('opcode:0x57'),n//64+2)
                self.assertEqual(row['parameters']['expected_blockhash_nonzero'],distance in (1,256))
            self.assertEqual(len({len(c) for c in codes}),1)
            self.assertLessEqual(sum(a!=b for a,b in zip(codes[0],codes[-1])),3,'only PUSH3 batch counter changes')
        # Deliberately query an expired height while retaining the nonzero guard.
        code=bytearray.fromhex(m.make_case('nearest',64)['request']['program']['code_hex'][2:]);code[1:4]=(257).to_bytes(3,'big')
        self.assertEqual(execute(code)[0],0xfe)

    def test_typed_bounds_and_invalid_metadata(self):
        m=self.load()
        for mode,n in [(True,0),(None,0),('unknown',0),('nearest',True),('nearest',None),('nearest',-64),('nearest',1),('nearest',63),('nearest',262145),('nearest',262208)]:
            with self.subTest(mode=mode,n=n),self.assertRaises(ValueError):m.make_case(mode,n)
        for invalid in (None,1,'push0'):
            with self.subTest(prelude=invalid),self.assertRaises(ValueError):m.make_case(push0_prelude=invalid)
        with tempfile.TemporaryDirectory() as temporary:
            for model in (None,True,'','A'*64,'g'*64):
                with self.subTest(model=model),self.assertRaises(ValueError):m.prepare(Path(temporary)/'invalid',model_id=model)
            self.assertFalse((Path(temporary)/'invalid').exists())

    def test_prospective_profile_has_new_inputs_and_exact_prelude_counts(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'cases';self.assertEqual(m.prepare(root,profile='prospective'),6)
            catalog=json.loads((root/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual({(r['parameters']['mode'],r['parameters']['bulk_queries']) for r in rows},{(mode,n) for mode in ('farthest','expired') for n in (0,8192,196608)})
            for row in rows:
                p=row['parameters'];request=json.loads((root/row['request_path']).read_text());original=m.make_case(p['mode'],p['bulk_queries'])
                self.assertTrue(row['holdout_only']);self.assertEqual(row['data_role'],'prospective_validation_probe')
                self.assertEqual(p['stack_prelude'],'push0-pop');self.assertNotEqual(request['program']['code_hex'],original['request']['program']['code_hex'])
                code=bytes.fromhex(request['program']['code_hex'][2:]);self.assertEqual(code[:2],bytes.fromhex('5f50'))
                terminal,counts,targets,stack=execute(code)
                self.assertEqual((terminal,stack),(0,[]));self.assertEqual(targets,[600-p['target_distance']]*(p['bulk_queries']+1))
                self.assertEqual({k:v for k,v in request['program']['expected_route_counts'].items() if v},counts)
                self.assertEqual(counts['opcode:0x5f'],1);self.assertEqual(counts['opcode:0x50'],original['request']['program']['expected_route_counts']['opcode:0x50']+1)
                self.assertIn(row['baseline_sample_id'],by)
                if p['bulk_queries']>8192:self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['bulk_queries'],8192)
            with self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',profile='unknown')

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native and quote for BLOCKHASH qualification')
    def test_actual_twelve_native_quote_ancestor_and_rejection_gates(self):
        m=self.load();model=os.environ.get('ZKGAS_EXPECTED_MODEL_ID',m.MODEL)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases',model_id=model)
            def run(request,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(request))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            rows=json.loads((root/'cases/catalog.json').read_text())['cases']+[m.make_case(mode,64) for mode in ('current','future')]
            for row in rows:
                request=row.get('request') or json.loads((root/'cases'/row['request_path']).read_text());p=row['parameters'];result,folder=run(request,row['sample_id']);self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());self.assertEqual(native['transaction_count'],2);self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0)
                self.assertEqual(native['observed_route_counts'].get('opcode:0x40'),p['total_blockhash_queries']);self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],0)
                self.assertEqual({key:value for key,value in native['observed_route_counts'].items() if value},
                    {key:value for key,value in request['program']['expected_route_counts'].items() if value})
                guest=json.loads((folder/'input.json').read_text());ancestors=guest['proposal_ancestor_headers'];height=guest['witnesses'][0]['block']['header']['number']
                self.assertIs(type(height),int)
                self.assertEqual([int(r['header']['number'],16) for r in ancestors],list(range(height-256,height)))
                for a,b in zip(ancestors,ancestors[1:]):self.assertEqual(b['header']['parentHash'],a['hash'])
                target=height-p['target_distance'];self.assertEqual(any(int(r['header']['number'],16)==target for r in ancestors),p['expected_blockhash_nonzero'])
                quoted=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','blockhash-density-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(quoted.returncode,0,quoted.stderr.decode());quote=json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(quote['model_id'],model);self.assertEqual(quote['guest_input_sha256'],native['guest_input_sha256']);self.assertEqual(quote['public_output'],native['public_output'])
                self.assertEqual(quote['blocks'][0]['node']['pricing']['code_preparation'],{'nonempty_events':0,'total_bytes':0})
            for mutation in ('short-execution','wrong-semantic-target','parent-oog'):
                row=m.make_case('nearest',64);request=row['request'];code=bytearray.fromhex(request['program']['code_hex'][2:])
                if mutation=='short-execution':
                    # Only batch-count immediate: the semantic warmup still succeeds.
                    at=next(at for at,op,data in instructions(code) if op==0x62 and int.from_bytes(data,'big')==1 and at>4);code[at+1:at+4]=bytes(3)
                elif mutation=='wrong-semantic-target':code[1:4]=(257).to_bytes(3,'big')
                else:request['program']['transactions']['gas_limit']=21000
                request['program']['code_hex']='0x'+code.hex();result,folder=run(request,mutation)
                self.assertNotEqual(result.returncode,0);self.assertFalse((folder/'input.bin').exists());self.assertFalse((folder/'native.json').exists())


if __name__=='__main__':unittest.main()
