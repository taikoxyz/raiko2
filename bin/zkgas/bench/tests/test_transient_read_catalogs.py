"""Direct-write transient catalogs: literal bytes, full-state cache and floor gas."""
import copy
import hashlib
import importlib.util
import inspect
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
MODEL = '5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11'
QUERY = 0x1122334455667788 << 192


def producer():
    path = CASES / 'transient_read_catalogs.py'
    assert path.is_file(), 'transient-read producer is missing'
    sys.path.insert(0, str(CASES))
    spec = importlib.util.spec_from_file_location('transient_read_catalogs', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


class TransientReadCatalogTests(unittest.TestCase):
    def test_default_zero_probes_full_and_floor(self):
        m = producer()
        from transient_read_catalogs_vm import run_users
        zero = m.make_case('mixed', 'zero'); keys = zero['parameters']['ordered_keys']
        rows = [zero, m.make_probe_case(keys, 64), m.make_probe_case(keys, 512), m.make_custom_case(keys)]
        maximum = m.full_repetitions()
        self.assertTrue(m.fits(maximum)); self.assertFalse(m.fits(maximum+1))
        self.assertEqual(m.make_case(), m.make_case('mixed', 'full'))
        self.assertEqual(len({row['parameters']['transaction_gas_limit'] for row in rows}), 1)
        self.assertEqual(len({row['parameters']['runtime_bytes'] for row in rows}), 1)
        for row, r in zip(rows, (0,64,512,maximum)):
            result = run_users(row)[0]; p=row['parameters']; gas=p['gas_accounting_per_user']
            self.assertEqual((result['tload_count'], result['tstore_count']), (2+64*r,512))
            self.assertEqual(result['seed_keys'], keys)
            self.assertEqual(result['query_values'], [0,1])
            self.assertEqual(result['transient_state_at_stop'], dict.fromkeys(keys,1))
            self.assertEqual(result['transient_state_after_tx'], {})
            self.assertEqual(result['stack'], [])
            self.assertEqual((result['counter'], result['peak_memory_bytes']), (r,128))
            self.assertEqual(gas['intrinsic_gas'],135688); self.assertEqual(gas['eip7623_floor_gas'],307720)
            self.assertEqual(gas['transient_load_gas'],100*(2+64*r)); self.assertEqual(gas['transient_store_gas'],51200)
            self.assertEqual(result['charged_gas'], max(result['gross_gas'],307720))
            self.assertEqual((p['expected_persistent_storage_reads'],p['expected_persistent_storage_writes']),(0,0))
            self.assertNotIn('prestate_contracts',row['request']['program']); self.assertNotIn('storage_state',row['request']['program'])
            self.assertEqual(p['full_plus_one_constraint_failures'],['transaction_fixture_cap'])
            self.assertFalse(row['actuals_used_for_fitting']); self.assertFalse(row['holdout_only'])
        self.assertTrue(rows[0]['parameters']['gas_accounting_per_user']['eip7623_floor_active'])
        self.assertFalse(rows[2]['parameters']['gas_accounting_per_user']['eip7623_floor_active'])

    def test_domains_catalog_variation_and_direct_cache_parity(self):
        m=producer()
        from transient_read_catalogs_vm import execute,run_users
        for k in (1,448,449,512,896,897,1024):
            for catalog in ('mixed','ordered'):
                zero=m.make_case(catalog,'zero',key_count=k);keys=zero['parameters']['ordered_keys']
                row=m.make_probe_case(keys,2,key_count=k);p=row['parameters'];program=row['request']['program']
                args=(bytes.fromhex(program['code_hex'][2:]),bytes.fromhex(program['input_hex'][2:]),p['transaction_gas_limit'])
                direct=execute(*args,repetitions=2,key_count=k,cache=False)
                self.assertEqual(direct,execute(*args,repetitions=2,key_count=k))
                self.assertEqual(run_users(row)[0]['charged_gas'],p['expected_user_charged_gas'])
                self.assertEqual(p['expected_user_gross_gas'],21484+397*k+6775*2)
                shifted=[f'0x{int(v,16)>>192:064x}' for v in keys[:-1]]+[keys[-1]]
                alternate=m.make_probe_case(shifted,2,key_count=k)
                self.assertEqual(program['code_hex'],alternate['request']['program']['code_hex'])
                self.assertEqual(p['gas_accounting_per_user'],alternate['parameters']['gas_accounting_per_user'])
        for k in (True,False,0,-1,1025,1.0,'512',None):
            with self.subTest(k=k),self.assertRaises(ValueError):m.make_case(key_count=k)
        keys=m.make_case('mixed','zero')['parameters']['ordered_keys']
        for r in (True,False,None,0,-1,1.0,m.full_repetitions(),m.full_repetitions()+1):
            with self.subTest(r=r),self.assertRaises(ValueError):m.make_probe_case(keys,r)
        for catalog in (None,False,1,[], 'unknown'):
            with self.assertRaises(ValueError):m.make_case(catalog)
        with self.assertRaises(ValueError):m.make_custom_case(keys[:-1])
        bad=keys.copy();bad[0]=bad[1]
        with self.assertRaises(ValueError):m.make_custom_case(bad)
        bad=keys.copy();bad[-1],bad[-2]=bad[-2],bad[-1]
        with self.assertRaises(ValueError):m.make_custom_case(bad)

    def test_cache_rejects_full_state_feedback_and_byte_mutants(self):
        m=producer()
        from transient_read_catalogs_vm import decode,structure,prove_body,apply_body,execute
        row=m.make_probe_case(m.make_case('mixed','zero')['parameters']['ordered_keys'],2)
        program=row['request']['program'];code=bytes.fromhex(program['code_hex'][2:]);data=bytes.fromhex(program['input_hex'][2:])
        instructions=list(decode(code));shape=structure(instructions,2,key_count=512)
        body=[i for i in instructions if shape['body']<=i[0]<shape['body_end']]
        state={int(k,16):1 for k in row['parameters']['ordered_keys']};memory=bytearray(128)
        memory[:32]=(512).to_bytes(32,'big');memory[96:128]=(0xa55a).to_bytes(32,'big')
        proof=prove_body(body,[QUERY],state,memory,QUERY,key_count=512)
        before=([QUERY],dict(state),bytearray(memory));stack=[QUERY];apply_body(proof,stack,state,memory)
        self.assertEqual((stack,state,memory),before)
        for key in (QUERY,next(k for k in state if k!=QUERY)):
            bad=dict(state);bad[key]=2
            with self.assertRaises(AssertionError):apply_body(proof,[QUERY],bad,memory)
        bad=dict(state);del bad[next(k for k in state if k!=QUERY)]
        with self.assertRaises(AssertionError):apply_body(proof,[QUERY],bad,memory)
        with self.assertRaises(AssertionError):apply_body(proof,[QUERY,0],state,memory)
        bad=bytearray(memory);bad[127]^=1
        with self.assertRaises(AssertionError):apply_body(proof,[QUERY],state,bad)
        for index in (0,1,2,63,95,191):
            bad=bytearray(code);bad[body[index][0]]=0x81
            with self.subTest(index=index),self.assertRaises(AssertionError):execute(bytes(bad),data,15000000,repetitions=2)

    def test_metadata_and_same_success_seed_mutant_rejected(self):
        m=producer();from transient_read_catalogs_vm import run_users,decode
        row=m.make_probe_case(m.make_case('mixed','zero')['parameters']['ordered_keys'],2)
        fields={'key_count':True,'repetitions':2.0,'seed_writes':511,'expected_transient_reads':129,
                'expected_persistent_storage_writes':1,'aggregate_state_size':512,'expected_final_storage_entry_count':512,
                'transaction_nonce':True,'expected_user_charged_gas':1,'sentinel':0,'peak_memory_bytes':160,
                'full_repetitions':True,'workload_level':'zero','transaction_gas_limit':14995248.0,
                'full_plus_one_constraint_failures':[]}
        for field,value in fields.items():
            bad=copy.deepcopy(row);bad['parameters'][field]=value
            with self.subTest(field=field),self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);bad['parameters']['gas_accounting_per_user']['gross_gas']=float(bad['parameters']['expected_user_gross_gas'])
        with self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);bad['request']['program']['transactions']['gas_limit']-=1;bad['parameters']['transaction_gas_limit']-=1
        with self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);bad['request']['program']['transactions']['count_per_block']=True
        with self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);bad['request']['program']['prestate_contracts']=[]
        with self.assertRaises(AssertionError):run_users(bad)
        # Replacing direct TSTORE1 with another same-width value must fail the literal oracle,
        # even if a correspondingly changed query guard could make real EVM execution succeed.
        instructions=list(decode(bytes.fromhex(row['request']['program']['code_hex'][2:])))
        from transient_read_catalogs_vm import structure
        shape=structure(instructions,2,key_count=512);bad=copy.deepcopy(row)
        code=bytearray.fromhex(bad['request']['program']['code_hex'][2:]);pc=shape['seed_value'];code[pc+3]=2
        bad['request']['program']['code_hex']='0x'+code.hex();bad['parameters']['runtime_sha256']=hashlib.sha256(code).hexdigest()
        with self.assertRaises(AssertionError):run_users(bad)

    def test_export_and_cli_model_catalog_caps_and_no_overwrite(self):
        m=producer();self.assertIs(inspect.signature(m.prepare).parameters['model_id'].default,inspect.Parameter.empty)
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);out=root/'out'
            self.assertEqual(m.prepare(out,model_id=MODEL,probe_repetitions=64),3)
            catalog=json.loads((out/'catalog.json').read_bytes());self.assertEqual(catalog['frozen_model_id'],MODEL)
            self.assertEqual([r['parameters']['repetitions'] for r in catalog['cases']],[0,64,m.full_repetitions()])
            for row in catalog['cases']:
                raw=(out/row['request_path']).read_bytes();self.assertEqual(hashlib.sha256(raw).hexdigest(),row['request_sha256'])
            with self.assertRaises(FileExistsError):m.prepare(out,model_id=MODEL)
            keys=m.make_case('mixed','zero')['parameters']['ordered_keys'];external=root/'keys.json'
            external.write_bytes(m.encoded(dict(schema_version=1,keys=keys)))
            custom=root/'custom';self.assertEqual(m.prepare(custom,model_id=MODEL,catalog_file=external,probe_repetitions=512),3)
            obj=json.loads((custom/'catalog.json').read_bytes());self.assertEqual(obj['external_catalog_json_sha256'],hashlib.sha256(external.read_bytes()).hexdigest())
            for raw in (b'{"schema_version":1,"schema_version":1,"keys":[]}',b'x'*(128*1024+1),b'{"schema_version":true,"keys":[]}'):
                external.write_bytes(raw)
                with self.assertRaises(ValueError):m.prepare(root/'bad',model_id=MODEL,catalog_file=external)
                self.assertFalse((root/'bad').exists())
            for kwargs in ({'model_id':'legacy'},{'model_id':MODEL,'key_count':True},{'model_id':MODEL,'probe_repetitions':0},{'model_id':MODEL,'catalog':False}):
                with self.assertRaises(ValueError):m.prepare(root/'bad',**kwargs)
                self.assertFalse((root/'bad').exists())
            cmd=[sys.executable,'-S','-B',str(CASES/'transient_read_catalogs.py'),'--output-dir',str(root/'cli'),'--model-id',MODEL,'--probe-repetitions','64']
            result=subprocess.run(cmd,capture_output=True,timeout=60);self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(json.loads(result.stdout),{'prepared':3})
            cmd=cmd[:cmd.index('--model-id')];cmd[cmd.index('--output-dir')+1]=str(root/'no-model')
            self.assertNotEqual(subprocess.run(cmd,capture_output=True,timeout=60).returncode,0)
            self.assertFalse((root/'no-model').exists())

class RawTransientReadCatalogTests(unittest.TestCase):
    def api(self):
        module=producer()
        self.assertTrue(callable(getattr(module,'make_raw_case',None)), 'raw-key factory is missing')
        self.assertTrue(callable(getattr(module,'zero_hash_catalog',None)), 'symbolic catalog helper is missing')
        return module

    def test_old_complete_rows_and_requests_match_frozen_goldens(self):
        module=producer();rows={}
        def record(label,row):
            rows[label]=dict(row_sha256=hashlib.sha256(module.encoded(row)).hexdigest(),
                request_sha256=hashlib.sha256(module.encoded(row['request'])).hexdigest())
        for k in (1,2,16,448,449,512,896,897,1024):
            for catalog in ('mixed','ordered'):
                keys=module.make_case(catalog,'zero',key_count=k)['parameters']['ordered_keys']
                for level in ('zero','full'):record(f'k{k}/{catalog}/default/{level}',module.make_case(catalog,level,key_count=k))
                for name,values in [('catalog',keys),('low64',[f'0x{int(v,16)>>192:064x}' for v in keys[:-1]]+[keys[-1]])]:
                    for level in ('zero','full'):record(f'k{k}/{catalog}/{name}/custom/{level}',module.make_custom_case(values,level,key_count=k))
                    for r in (2,64,512):record(f'k{k}/{catalog}/{name}/probe/{r}',module.make_probe_case(values,r,key_count=k))
        keys=module.make_case('mixed','zero')['parameters']['ordered_keys']
        for label,row in [('default/make_case',module.make_case()),('default/ordered',module.make_case('ordered')),('default/custom',module.make_custom_case(keys)),('default/probe',module.make_probe_case(keys,64))]:record(label,row)
        self.assertEqual(len(rows),220)
        self.assertEqual(hashlib.sha256(json.dumps(rows,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
            '472ae18cfeba27ddbb7f14f599c367e3a2937c968b1eaf466daa0a6914ba631c')

    def test_raw_prefix_panel_literal_gas_and_direct_cached_vm(self):
        from collections import Counter
        from transient_read_catalogs_vm import execute,run_users
        m=self.api();shared=0x0102030405060708;query=shared<<64;codes={}
        for offset,expected_group,expected_c in ((0,1,65),(1,1,65),(2,1,65),(3,2,66),(4,512,512)):
            keys=m.zero_hash_catalog(shared,offset);self.assertEqual(len(keys),512);self.assertEqual(keys[-1],f'0x{query:064x}')
            self.assertEqual([int(key,16) for key in keys[:-1]],[query | (i<<(8*offset)) for i in range(1,512)])
            def fold(left,right):
                product=left*right;return (product&((1<<64)-1))^(product>>64)
            for key in keys:
                limbs=[(int(key,16)>>(64*i))&((1<<64)-1) for i in range(4)]
                for own_seed in (0,(1<<64)-1):
                    self.assertEqual(fold(limbs[2]^fold(limbs[0]^own_seed,limbs[1]^shared),limbs[3]^shared),0)
            groups=Counter(int(key,16)&0xffffffff for key in keys);self.assertEqual(max(groups.values()),expected_group)
            self.assertEqual((512+7*max(groups.values())+7)//8,expected_c)
            data=b''.join(bytes.fromhex(k[2:]) for k in keys);self.assertEqual(sum(v!=0 for v in data),4862)
            for level,r in (('zero',0),('probe',2),('probe',512),('full',2179)):
                row=m.make_raw_case(keys,level,query_key=query,repetitions=r if level=='probe' else None)
                p=row['parameters'];program=row['request']['program'];self.assertIs(p['raw_keys'],True)
                result=run_users(row)[0];direct=execute(bytes.fromhex(program['code_hex'][2:]),data,p['transaction_gas_limit'],repetitions=r,cache=False,query_key=query,raw_keys=True)
                cached=execute(bytes.fromhex(program['code_hex'][2:]),data,p['transaction_gas_limit'],repetitions=r,query_key=query,raw_keys=True)
                self.assertEqual(direct,cached);self.assertEqual(result['charged_gas'],max(233940+6775*r,330700))
                self.assertEqual(result['gross_gas'],233940+6775*r)
                self.assertEqual((p['calldata_zero_bytes'],p['calldata_nonzero_bytes']),(11522,4862))
                self.assertEqual((p['full_repetitions'],p['transaction_gas_limit']),(2179,14997665))
                self.assertEqual(p['gas_accounting_per_user']['intrinsic_gas'],144880)
                self.assertEqual(p['gas_accounting_per_user']['eip7623_floor_gas'],330700)
                self.assertEqual((result['tload_count'],result['tstore_count']),(2+64*r,512))
                self.assertEqual(result['seed_keys'],list(keys));self.assertEqual(result['transient_state_after_tx'],{})
                self.assertEqual(result['query_values'],[0,1]);self.assertEqual(result['stack'],[])
                self.assertEqual(p['full_plus_one_constraint_failures'],['transaction_fixture_cap'])
                if r in codes:self.assertEqual(codes[r],program['code_hex'])
                else:codes[r]=program['code_hex']

    def test_raw_zero_query_byte_extremes_and_domains(self):
        from transient_read_catalogs_vm import execute,run_users
        m=self.api()
        for shared in (0,1,(1<<64)-1):
            for k in (1,2,512,1024):
                keys=m.zero_hash_catalog(shared,2,key_count=k);query=shared<<64
                row=m.make_raw_case(keys,'zero',query_key=query,key_count=k);p=row['parameters'];data=bytes.fromhex(row['request']['program']['input_hex'][2:]);nz=sum(v!=0 for v in data)
                expected_gross=21484+301*k+12*nz;floor=21000+10*(32*k+3*nz)
                self.assertEqual(run_users(row)[0]['gross_gas'],expected_gross)
                self.assertEqual(p['expected_user_charged_gas'],max(expected_gross,floor))
                self.assertEqual(p['transaction_gas_limit'],expected_gross+6775*p['full_repetitions']+1000)
                self.assertLessEqual(p['transaction_gas_limit'],15000000)
                self.assertGreater(expected_gross+6775*(p['full_repetitions']+1)+1000,15000000)
        zero_keys=m.zero_hash_catalog(0,2);full=m.make_raw_case(zero_keys,'full',query_key=0)
        self.assertEqual(full['parameters']['full_repetitions'],2186)
        self.assertEqual(run_users(full)[0]['counter'],2186)
        with self.assertRaises(ValueError):m.gas_accounting(0,query_key=0)
        with self.assertRaises(ValueError):m.full_repetitions(query_key=0)
        # The former fixed-query/8byte API remains strict, even for otherwise valid raw keys.
        with self.assertRaises(ValueError):m.make_custom_case(zero_keys)
        for query in (True,False,-1,1<<256,0.0,'0x00',None):
            with self.subTest(query=query),self.assertRaises(ValueError):m.make_raw_case(zero_keys,'zero',query_key=query)
        for shared,offset in ((True,0),(-1,0),(1<<64,0),(1.0,0),(0,True),(0,-1),(0,5),(0,1.0)):
            with self.subTest(shared=shared,offset=offset),self.assertRaises(ValueError):m.zero_hash_catalog(shared,offset)
        for k in (True,False,0,1025,1.0):
            with self.assertRaises(ValueError):m.zero_hash_catalog(0,0,key_count=k)
        for values in (list(zero_keys[:-1]),[zero_keys[0]]*512,[*zero_keys[:-2],zero_keys[-1],zero_keys[-2]]):
            with self.assertRaises(ValueError):m.make_raw_case(values,'zero',query_key=0)
        for bad in (1,'0x0','0X'+'0'*64,'0x'+'A'*64):
            values=list(zero_keys);values[0]=bad
            with self.assertRaises(ValueError):m.make_raw_case(values,'zero',query_key=0)
        for level,repetitions in (('probe',True),('probe',0),('probe',2186),('probe',1.0),('zero',1),('full',1),('unknown',None)):
            with self.assertRaises(ValueError):m.make_raw_case(zero_keys,level,query_key=0,repetitions=repetitions)

    def test_raw_oracle_rejects_guard_calldata_metadata_and_feedback_mutants(self):
        from transient_read_catalogs_vm import decode,structure,prove_body,apply_body,run_users
        m=self.api();keys=m.zero_hash_catalog(0,2);row=m.make_raw_case(keys,'probe',query_key=0,repetitions=2)
        for field,value in (('raw_keys','true'),('raw_keys',False),('query_key',False),('query_key','0x1'),('query_key',f'0x{1:064x}'),('calldata_nonzero_bytes',4096),('calldata_zero_bytes',12288),('full_repetitions',2180),('transaction_gas_limit',14995248)):
            bad=copy.deepcopy(row);bad['parameters'][field]=value
            with self.subTest(field=field,value=value),self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);del bad['parameters']['raw_keys']
        with self.assertRaises(AssertionError):run_users(bad)
        bad=copy.deepcopy(row);bad['parameters']['gas_accounting_per_user']['eip7623_floor_gas']=307720
        with self.assertRaises(AssertionError):run_users(bad)
        code=bytes.fromhex(row['request']['program']['code_hex'][2:]);instructions=list(decode(code));shape=structure(instructions,2,query_key=0)
        query_push=next(at for i,(at,op,v) in enumerate(instructions[:-1]) if op==0x7f and instructions[i+1][1]==0x5c)
        bad=copy.deepcopy(row);changed=bytearray(code);changed[query_push+32]=1;bad['request']['program']['code_hex']='0x'+changed.hex();bad['parameters']['runtime_sha256']=hashlib.sha256(changed).hexdigest()
        with self.assertRaises(AssertionError):run_users(bad)
        state=dict.fromkeys((int(k,16) for k in keys),1);memory=bytearray(128);memory[:32]=(512).to_bytes(32,'big');memory[96:128]=(0xa55a).to_bytes(32,'big')
        body=[x for x in instructions if shape['body']<=x[0]<shape['body_end']];proof=prove_body(body,[0],state,memory,0)
        apply_body(proof,[0],state,memory)
        with self.assertRaises(AssertionError):apply_body(proof,[1],state,memory)
        bad_state=dict(state);bad_state[next(k for k in state if k!=0)]=2
        with self.assertRaises(AssertionError):apply_body(proof,[0],bad_state,memory)


if __name__=='__main__':unittest.main()
