"""Guardless density keeps child completion gates and the frozen checked request contract."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_delegatecall_repetition import execute

CASES=Path(__file__).resolve().parents[1]/'cases'
CHECKED_REQUEST_SHA256={
    'delegatecall-repetition-stop-n0':'0b490984509e96a0bdd80906ffd549999cad4502544943a0167ed70c52379ee5',
    'delegatecall-repetition-stop-n1':'8228218320dad72ca1058d1e482c4f881b9dd197b23ed9afe8024701fefa9170',
    'delegatecall-repetition-stop-n1024':'ff4cf3d99c54880a4936cb7ee6d7088ccdfb2dab172c7bade761fd92f07c914c',
    'delegatecall-repetition-stop-n32768':'77e7c8657d5e1fb00a0093e8053b3159abf6dd26ab50030cf81acb6a9794a70f',
    'delegatecall-repetition-return0-n0':'6c1b6c4efc7653d0fdcb48bb445f6d1151da331cdad7f6cf483aa3706c995611',
    'delegatecall-repetition-return0-n1':'fbb81cc4014fce622d58d1c339f8902273bce35ba3057c51485f8f3b53e3acbd',
    'delegatecall-repetition-return0-n1024':'2e72d8e4c81ca9b9ea7610d287a30f217d19632cb196362675df4ae67cd0092e',
    'delegatecall-repetition-return0-n32768':'f90066655e63108ac433b3948f3e56006a4830a43d66022e6e205d38a8e3053a',
    'delegatecall-repetition-revert0-n0':'3f63bb84e44a1602be99e68f7f9c290f8d4c2f6b31f19074a3a7192ccb026981',
    'delegatecall-repetition-revert0-n1':'1b93be5aac3648599411c695c99d6b85cb15575f9c27f438ed31d20c8a619a0b',
    'delegatecall-repetition-revert0-n1024':'e902cfef394ad4526bd34241fcb8a98e702f014a9098f3eed33b579873208494',
    'delegatecall-repetition-revert0-n32768':'5d259e4b0abd6629f248390dcc2737c74ac6b238496e5c9eab3fd2a8fd2a15c4',
    'delegatecall-repetition-invalid-n0':'5e4a75bdb5d0e4edac4887bec4653982d2b26d5a8c68f880190363ce8615610e',
    'delegatecall-repetition-invalid-n1':'157a7cde2e419859b97c29c3251939024dfafe4e067f7bb680f481598ab47ad8',
    'delegatecall-repetition-invalid-n1024':'7efe91784894d758013a0c5c3ff3b19f478a5f363b16003ca9aa38b5d427ef88',
    'delegatecall-repetition-invalid-n32768':'d878c8ab09c7341488ac86a2526e8e577ad5fb15647fe41f1f403000c10b6837',
}


DENSITY_REQUEST_SHA256={'delegatecall-density-stop-n0': '5bf6ea8812108b1434ceeb0b6781389ca35c272edf5218c77ad839429f51cbb9', 'delegatecall-density-stop-n64': '4912f2cc33c4c148955c0c3acff747b034c0e256c8295f133a58ee7268dce6d2', 'delegatecall-density-stop-n1024': 'a1455fc53e05060cb9601639873b1d273ecaee6fdb086b0950efd34efc9e53a3', 'delegatecall-density-stop-n32768': 'd05ae975aa91ab9be82560d4508a31f133bd6667885c5f7f911fe362539a2d56', 'delegatecall-density-return0-n0': '6d468d8c7480c15795daee4884e8a90ca5aa01dd7cc9fc631af712ee8eda9dab', 'delegatecall-density-return0-n64': '05481837ecd601d3bd73703ed72ef9b077b8e517fe613347be3d8034399e49e6', 'delegatecall-density-return0-n1024': 'ce91355123c24de7bf73bc4b18ebf1f8f41b7596fa75bfd171ac5dcb76d00851', 'delegatecall-density-return0-n32768': '1c0edf0bea91942b73a2dba84af4b66e9713b236d870e8366321efd2987edae8'}

def instructions(code):
    pc=0;rows=[]
    while pc<len(code):
        offset=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code);data=code[pc:pc+width];pc+=width;rows.append((offset,op,data))
    return rows


class DelegatecallDensityTests(unittest.TestCase):
    def load(self):
        sys.path.insert(0,str(CASES));path=CASES/'delegatecall_repetition.py'
        spec=importlib.util.spec_from_file_location('density_cases',path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m

    def test_frozen_checked_requests_unchanged_default_and_explicit(self):
        m=self.load();frozen=os.environ.get('CHECKED_DELEGATECALL_CASES')
        if frozen:
            root=Path(frozen);catalog=json.loads((root/'catalog.json').read_text());old={r['sample_id']:root/r['request_path'] for r in catalog['cases']}
        for terminal in ('stop','return0','revert0','invalid'):
            for n in (0,1,1024,32768):
                default=m.make_case(terminal,n);explicit=m.make_case(terminal,n,profile='checked');raw=m.encoded(default['request'])
                self.assertEqual(raw,m.encoded(explicit['request']));self.assertEqual(m.digest(raw),CHECKED_REQUEST_SHA256[default['sample_id']])
                if frozen:self.assertEqual(raw,old[default['sample_id']].read_bytes())

    def test_eight_matrix_and_profile_references_hashes_no_overwrite(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            out=Path(temporary)/'density';self.assertEqual(m.prepare(out,profile='density'),8)
            catalog=json.loads((out/'catalog.json').read_text());rows=catalog['cases'];by={r['sample_id']:r for r in rows}
            self.assertEqual(len(by),8);self.assertEqual(catalog['producer_profile'],'density');self.assertEqual(catalog['unroll_factor'],64)
            self.assertEqual({(r['parameters']['child_terminal'],r['parameters']['repetitions']) for r in rows},{(t,n) for t in ('stop','return0') for n in (0,64,1024,32768)})
            self.assertEqual(catalog['generator_sha256'],m.digest((CASES/'delegatecall_repetition.py').read_bytes()))
            for row in rows:
                p=row['parameters'];raw=(out/row['request_path']).read_bytes();request=json.loads(raw)
                self.assertEqual(m.digest(raw),row['request_sha256']);self.assertTrue(request['expected_user_success']);self.assertNotIn(row['sample_id'],CHECKED_REQUEST_SHA256)
                self.assertEqual(request['program']['transactions'],{'kind':'call','count_per_block':1,'gas_limit':15_000_000});self.assertEqual(request['program']['block_count'],1);self.assertEqual(request['program']['input_hex'],'0x')
                self.assertEqual(p['producer_profile'],'density');self.assertEqual(p['unroll_factor'],64);self.assertEqual(p['batch_count']*64,p['repetitions'])
                self.assertFalse(p['per_call_status_guard']);self.assertFalse(p['per_call_return_data_guard']);self.assertEqual(p['forwarded_gas'],64)
                self.assertEqual(p['expected_child_creations'],1);self.assertEqual(p['expected_factory_nonce'],1)
                base=by[row['baseline_sample_id']]['parameters'];self.assertEqual(base['child_terminal'],p['child_terminal']);self.assertEqual(base['repetitions'],0)
                if p['repetitions']>64:self.assertEqual(by[row['repetition_reference_sample_id']]['parameters']['repetitions'],64)
            before={p.relative_to(out):p.read_bytes() for p in out.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(out,profile='density')
            self.assertEqual(before,{p.relative_to(out):p.read_bytes() for p in out.rglob('*') if p.is_file()})
            checked=Path(temporary)/'checked';self.assertEqual(m.prepare(checked),16)
            for row in json.loads((checked/'catalog.json').read_text())['cases']:self.assertEqual(m.digest((checked/row['request_path']).read_bytes()),CHECKED_REQUEST_SHA256[row['sample_id']])

    def test_independent_static_body_and_dynamic_child_completion(self):
        m=self.load()
        for terminal in ('stop','return0'):
            lengths=[];codes=[]
            for count in (0,64,1024,32768):
                row=m.make_case(terminal,count,profile='density');code=bytes.fromhex(row['request']['program']['code_hex'][2:]);decoded=instructions(code);lengths.append(len(code));codes.append(code)
                self.assertEqual(sum(op==0xf4 for _,op,_ in decoded),64,'N0 must retain the same unused64-call body')
                self.assertEqual(sum(op==0x3d for _,op,_ in decoded),0,'no per-call RETURNDATASIZE check')
                self.assertEqual(sum(op==0x57 for _,op,_ in decoded),2,'only creation guard and batch loop check')
                result=execute(code);self.assertTrue(result['success']);self.assertEqual(len(result['children']),1);self.assertEqual(len(result['calls']),count)
                self.assertEqual(result['calls'],[(True,0,64)]*count);self.assertEqual(result['exceptional'],0)
                for key,value in row['request']['program']['expected_route_counts'].items():self.assertEqual(result['completed'].get(key,0),value,(terminal,count,key))
                self.assertEqual(result['completed'].get('opcode:0x57',0),count//64+2)
            self.assertEqual(len(set(lengths)),1)
            differences=[i for i,(a,b) in enumerate(zip(codes[0],codes[-1])) if a!=b]
            self.assertTrue(differences);self.assertLessEqual(len(differences),3,'only fixed-width batch immediate may change')

    def test_profiles_and_density_typed_bounds(self):
        m=self.load()
        for terminal,n,profile in [('revert0',64,'density'),('invalid',64,'density'),('stop',True,'density'),('stop',-64,'density'),('stop',1,'density'),('stop',63,'density'),('stop',32769,'density'),('stop',65600,'density'),('stop',64,True),('stop',64,'unknown'),('stop',64,None)]:
            with self.subTest(terminal=terminal,n=n,profile=profile),self.assertRaises(ValueError):m.make_case(terminal,n,profile=profile)
        with tempfile.TemporaryDirectory() as temporary:
            for profile in (True,None,'unknown'):
                with self.assertRaises(ValueError):m.prepare(Path(temporary)/'invalid',profile=profile)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native/quote for density qualification')
    def test_actual_eight_and_missing_guard_negative_controls(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases',profile='density')
            def run(request,name):
                folder=root/name;folder.mkdir();source=folder/'request.json';source.write_bytes(m.encoded(request))
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(source),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                return result,folder
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                request=json.loads((root/'cases'/row['request_path']).read_text());result,folder=run(request,row['sample_id']);self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],1);self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0)
                for key,value in request['program']['expected_route_counts'].items():self.assertEqual(native['observed_route_counts'].get(key,0),value)
                quoted=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','delegatecall-density-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(quoted.returncode,0,quoted.stderr.decode());q=json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(q['model_id'],m.MODEL);self.assertEqual(q['guest_input_sha256'],native['guest_input_sha256']);self.assertEqual(q['public_output'],native['public_output'])
                self.assertEqual(q['blocks'][0]['node']['pricing']['code_preparation'],row['parameters']['expected_code_preparation']);self.assertEqual(q['blocks'][0]['prove']['code_preparation'],row['parameters']['expected_code_preparation'])
            for mutation in ('short-batch','empty-target','parent-oog','return-zero-gas','stop-zero-gas'):
                row=m.make_case('return0' if mutation=='return-zero-gas' else 'stop',64,profile='density');request=row['request'];code=bytearray.fromhex(request['program']['code_hex'][2:]);decoded=instructions(code)
                if mutation=='short-batch':
                    at=next(at for at,op,data in decoded if op==0x62 and int.from_bytes(data,'big')==1);code[at+1:at+4]=bytes(3)
                elif mutation=='empty-target':code[next(at for at,op,_ in decoded if op==0x85)]=0x33  # CALLER is an empty-code EOA.
                elif mutation=='parent-oog':request['program']['transactions']['gas_limit']=21000
                else:
                    for at,op,data in decoded:
                        if op==0x60 and data==b'\x40':code[at+1]=0
                request['program']['code_hex']='0x'+code.hex();result,folder=run(request,'control-'+mutation)
                if mutation=='stop-zero-gas':self.assertEqual(result.returncode,0,result.stderr.decode())
                else:self.assertNotEqual(result.returncode,0);self.assertFalse((folder/'input.bin').exists());self.assertFalse((folder/'native.json').exists())



class DelegatecallPush0ProspectiveTests(unittest.TestCase):
    """Fresh programs validate a frozen tariff; no actual-workload result is fitted here."""
    load=DelegatecallDensityTests.load

    def test_push0_six_catalog_and_exact_programs(self):
        m=self.load();model='a'*64
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'push0'
            self.assertEqual(m.prepare(root,profile='density',zero_arguments='push0',model_id=model),6)
            catalog=json.loads((root/'catalog.json').read_text());self.assertEqual(catalog['frozen_model_id'],model)
            self.assertEqual({(r['parameters']['child_terminal'],r['parameters']['repetitions']) for r in catalog['cases']},{(t,n) for t in ('stop','return0') for n in (0,4096,65536)})
            ids={r['sample_id'] for r in catalog['cases']}
            for row in catalog['cases']:
                p=row['parameters'];request=json.loads((root/row['request_path']).read_text());code=bytes.fromhex(request['program']['code_hex'][2:]);decoded=instructions(code)
                self.assertTrue(row['holdout_only']);self.assertEqual(row['data_role'],'prospective_frozen_policy_validation');self.assertIn('push0',row['sample_id'])
                self.assertIn(row['baseline_sample_id'],ids);self.assertEqual(p['argument_zero_opcode'],'PUSH0')
                self.assertEqual(sum(op==0x5f for _,op,_ in decoded),256);self.assertEqual(sum(op==0xf4 for _,op,_ in decoded),64)
                result=execute(code);self.assertTrue(result['success']);self.assertEqual(len(result['calls']),p['repetitions']);self.assertEqual(result['exceptional'],0)
                for key,value in request['program']['expected_route_counts'].items():self.assertEqual(result['completed'].get(key,0),value)
                if p['repetitions']==65536:self.assertIn(row['repetition_reference_sample_id'],ids)

    def test_push0_options_and_checked_limit(self):
        m=self.load()
        for kwargs in ({'zero_arguments':True},{'zero_arguments':'unknown'},{'zero_arguments':'push0'}, {'profile':'density','zero_arguments':'push0','repetitions':65600}):
            with self.subTest(kwargs=kwargs),self.assertRaises(ValueError):m.make_case(**kwargs)
        with self.assertRaises(ValueError):m.make_case('stop',65536)
        with tempfile.TemporaryDirectory() as temporary:
            for bad in ('','A'*64,'g'*64,None,True):
                with self.subTest(model=bad),self.assertRaises(ValueError):m.prepare(Path(temporary)/'bad',profile='density',zero_arguments='push0',model_id=bad)

    def test_previous_density_eight_request_bytes_unchanged(self):
        m=self.load()
        for terminal in ('stop','return0'):
            for count in (0,64,1024,32768):
                row=m.make_case(terminal,count,profile='density')
                self.assertEqual(m.digest(m.encoded(row['request'])),DENSITY_REQUEST_SHA256[row['sample_id']])

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN') and os.environ.get('ZKGAS_QUOTE_BIN'),'set frozen native/quote for prospective qualification')
    def test_six_actual_native_and_quote(self):
        m=self.load();model=os.environ.get('ZKGAS_EXPECTED_MODEL_ID',m.MODEL)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);m.prepare(root/'cases',profile='density',zero_arguments='push0',model_id=model)
            for row in json.loads((root/'cases/catalog.json').read_text())['cases']:
                folder=root/row['sample_id'];folder.mkdir()
                command=[os.environ['NATIVE_FIXTURE_BIN'],'--input',str(root/'cases'/row['request_path']),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only']
                result=subprocess.run(command,capture_output=True,timeout=180);self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());self.assertEqual(native['transaction_count'],2);self.assertLess(native['user_transaction_gas_used_total'],15_000_000)
                self.assertTrue(all(r['success'] for r in native['receipts_by_block'][0]));self.assertEqual(native['observations_by_block'][0]['exceptional_instructions'],0)
                self.assertEqual(native['observed_route_counts'].get('opcode:0xf4',0),row['parameters']['repetitions']);self.assertEqual(native['transactions']['blocks'][0]['users'][0]['poststate']['nonce'],1)
                quoted=subprocess.run([os.environ['ZKGAS_QUOTE_BIN'],'--input-bincode',str(folder/'input.bin'),'--output',str(folder/'quote.jsonl'),'--execution-profile','evm-gas-only','--corpus-id','delegatecall-push0-qualification','--sample-id',row['sample_id']],capture_output=True,timeout=180)
                self.assertEqual(quoted.returncode,0,quoted.stderr.decode());quote=json.loads((folder/'quote.jsonl').read_text())['quote']
                self.assertEqual(quote['model_id'],model);self.assertEqual(quote['guest_input_sha256'],native['guest_input_sha256']);self.assertEqual(quote['public_output'],native['public_output'])
                self.assertEqual(quote['blocks'][0]['node']['pricing']['code_preparation'],row['parameters']['expected_code_preparation'])


if __name__=='__main__':unittest.main()
