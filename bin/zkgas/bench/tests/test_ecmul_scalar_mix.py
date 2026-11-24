"""Compiler-free scalar vectors, literal call semantics and successful gas envelopes."""
from collections import Counter
import copy
import hashlib
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

CASES=Path(__file__).resolve().parents[1]/'cases'


def producer():
    path=CASES/'ecmul_scalar_mix.py'
    assert path.is_file(),'ECMUL scalar mixture producer is missing'
    sys.path.insert(0,str(CASES))
    spec=importlib.util.spec_from_file_location('ecmul_scalar_mix',path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


class EcmulScalarMixTests(unittest.TestCase):
    def test_six_reference_rows_and_same_runtime(self):
        m=producer();from ecmul_scalar_mix_vm import run_users
        by_r={};scalars={};results={}
        for recipe in ('low','high','mixed'):
            for level,r in (('zero',0),('probe',1)):
                row=m.make_case(recipe,level,repetitions=r if level=='probe' else None)
                p=row['parameters'];program=row['request']['program'];result=run_users(row)[0]
                by_r.setdefault(r,program['code_hex']);self.assertEqual(by_r[r],program['code_hex'])
                self.assertEqual(len(bytes.fromhex(program['input_hex'][2:])),31*96)
                self.assertEqual((result['terminal'],result['stack'],result['counter']),(0,[],r))
                self.assertEqual(result['completed_calls'],31*r);self.assertEqual(result['counts'],+Counter(program['expected_route_counts']))
                self.assertEqual(p['source_derived_work_units'],[0 if s==0 else 507 for s in m.scalars_for(recipe)])
                self.assertEqual(result['memory_expansion_gas'],24);self.assertEqual(result['peak_memory_bytes'],256)
                self.assertEqual(result['gross_gas'],p['expected_user_gross_gas']);self.assertEqual(result['charged_gas'],p['expected_user_charged_gas'])
                self.assertEqual(result['refund_counter'],0)
                for i,call in enumerate(result['calls']):
                    self.assertEqual(call['entry_index'],i%31);self.assertEqual(call['native_gas'],6000)
                    self.assertEqual(call['forwarded_gas'],6000);self.assertEqual(call['returned_gas'],0)
                    self.assertGreaterEqual(call['available_gas']-call['available_gas']//64,6000)
                    self.assertEqual(call['return_size'],64)
                self.assertNotIn('prestate_contracts',program);self.assertNotIn('storage_state',program)
                self.assertEqual(p['expected_persistent_storage_reads'],0);self.assertEqual(p['expected_persistent_storage_writes'],0)
                scalars[recipe]=p['scalars_hex'];results[recipe,r]=result
        self.assertEqual(scalars['mixed'],scalars['low'][:12]+scalars['high'][:19])
        self.assertEqual(m.make_case(),m.make_case('mixed','full'))

    def test_scalar_reduction_zero_and_independent_points(self):
        m=producer();from ecmul_scalar_mix_vm import run_users,validate_payload
        import precompile_workload as old
        old_scalars=dict(old.SCALARS)
        values=[0,1,m.GROUP_ORDER,m.GROUP_ORDER+1,(1<<256)-1,m.HIGH_SCALAR]
        with patch.object(old,'compile_source',side_effect=AssertionError('compiler forbidden')),patch.object(old,'prepare',side_effect=AssertionError('export forbidden')):
            row=m.make_custom_case(values,'probe',repetitions=1)
        result=run_users(row)[0];entries=validate_payload(bytes.fromhex(row['request']['program']['input_hex'][2:]),len(values))
        self.assertEqual(m.expected_point(0),(0,0));self.assertEqual(m.expected_point(m.GROUP_ORDER),(0,0))
        self.assertEqual(m.expected_point(1),(1,2));self.assertEqual(m.expected_point(m.GROUP_ORDER+1),(1,2))
        self.assertEqual(result['calls'][0]['output_hex'],'0x'+bytes(64).hex())
        self.assertEqual(result['calls'][2]['output_hex'],'0x'+bytes(64).hex())
        self.assertEqual(entries[-1]['work_units'],507);self.assertEqual(entries[2]['reduced_scalar'],0)
        # Fixed external vectors, independent of both factory and literal VM.
        self.assertEqual(result['calls'][4]['output_hex'],'0x2f588cffe99db877a4434b598ab28f81e0522910ea52b45f0adaa772b2d5d35212f42fa8fd34fb1b33d8c6a718b6590198389b26fc9d8808d971f8b009777a97')
        self.assertEqual(result['calls'][5]['output_hex'],'0x11c74c606a7b392ddce12ac348fa17186286210ad790c04b144c4d0a2a5fd0f22c1dad4a17dc0f61bde172d9dc4021d342387693086dee8bcd17e82df75a601b')
        self.assertEqual(entries[4]['work_units'],396)
        self.assertEqual(old.SCALARS,old_scalars)
        with self.assertRaises(ValueError):old.expected_output(0)

    def test_full_reference_plus_one_and_bounded_sequences(self):
        m=producer();from ecmul_scalar_mix_vm import execute,run_users
        for sequence in ([0],[m.HIGH_SCALAR],[0,m.HIGH_SCALAR]*32):
            maximum=m.full_repetitions(sequence);self.assertTrue(m.fits(sequence,maximum));self.assertFalse(m.fits(sequence,maximum+1))
            row=m.make_custom_case(sequence,'full');p=row['parameters'];result=run_users(row)[0]
            self.assertEqual(result['completed_calls'],len(sequence)*maximum)
            data=m.payload(sequence);code,counts=m._runtime(len(sequence),maximum+1)
            too_large=execute(code,data,40_000_000,sequence_length=len(sequence),repetitions=maximum+1)
            gas=m.gas_accounting(sequence,maximum+1)
            self.assertEqual(too_large['gross_gas'],gas['gross_gas']);self.assertEqual(too_large['counts'],+Counter(counts))
            self.assertGreater(max(gas['gross_gas'],gas['eip7623_floor_gas'])+1000,15_000_000)
            self.assertLessEqual(len(code),24576)
            self.assertEqual(p['transaction_gas_limit'],max(result['gross_gas'],p['gas_accounting_per_user']['eip7623_floor_gas'])+1000)

    def test_strict_input_domains(self):
        m=producer()
        for sequence in (None,{},'zero',[],[0]*65,[True],[-1],[1<<256],[1.0],[None]):
            with self.subTest(sequence=sequence),self.assertRaises(ValueError):m.make_custom_case(sequence,'zero')
        for recipe in (None,True,'LOW',0):
            with self.assertRaises(ValueError):m.make_case(recipe)
        for r in (None,True,-1,1.0,1_000_001):
            with self.assertRaises(ValueError):m._runtime(1,r)
        for k in (True,0,65,1.0):
            with self.assertRaises(ValueError):m._runtime(k,0)
        for args in (([0],'invalid'),([0],True)):
            with self.assertRaises(ValueError):m.make_custom_case(*args)
        with self.assertRaises(ValueError):m.make_custom_case([0],'zero',repetitions=0)
        with self.assertRaises(ValueError):m.make_custom_case([0],'probe',repetitions=0)

    def test_payload_and_typed_metadata_mutants(self):
        m=producer();from ecmul_scalar_mix_vm import run_users,validate_payload
        row=m.make_custom_case([0,m.HIGH_SCALAR],'probe',repetitions=1)
        data=bytes.fromhex(row['request']['program']['input_hex'][2:])
        for wrong in (data[:-1],data+bytes(32),data[:32]+bytes.fromhex('00'*31+'01')+data[64:],data[:64]+m.FIELD_MODULUS.to_bytes(32,'big')+data[96:]):
            with self.assertRaises(AssertionError):validate_payload(wrong,2)
        for field,value in (('sequence_length',True),('repetitions',2),('expected_precompile_calls',1),('expected_user_gross_gas',0),('transaction_gas_limit',21000),('full_repetitions',1),('source_derived_native_gas_per_call',5999)):
            mutant=copy.deepcopy(row);mutant['parameters'][field]=value
            with self.assertRaises(AssertionError,msg=field):run_users(mutant)
        mutant=copy.deepcopy(row);mutant['request']['program']['expected_route_counts']['opcode:0xfa']-=1
        with self.assertRaises(AssertionError):run_users(mutant)
        mutant=copy.deepcopy(row);wrong=bytearray(data);wrong[63]=1;mutant['request']['program']['input_hex']='0x'+wrong.hex();mutant['parameters']['calldata_sha256']=hashlib.sha256(wrong).hexdigest()
        with self.assertRaises(AssertionError):run_users(mutant)
        for field,value in (('schema_version',True),):
            mutant=copy.deepcopy(row);mutant['request'][field]=value
            with self.assertRaises(AssertionError):run_users(mutant)
        for field,value in (('count_per_block',True),('gas_limit',True),('kind','create')):
            mutant=copy.deepcopy(row);mutant['request']['program']['transactions'][field]=value
            with self.assertRaises(AssertionError):run_users(mutant)
        for field,value in (('input_hex','zz'+data.hex()),('code_hex','zz'+row['request']['program']['code_hex'][2:]),('prestate_contracts',[])):
            mutant=copy.deepcopy(row);mutant['request']['program'][field]=value
            with self.assertRaises(AssertionError):run_users(mutant)
        repeat=m.make_custom_case([0,m.HIGH_SCALAR,0],'probe',repetitions=2)
        self.assertEqual([c['entry_index'] for c in run_users(repeat)[0]['calls']],[0,1,2,0,1,2])

    def test_call_failures_zero_output_and_byte_guards(self):
        m=producer();from ecmul_scalar_mix_vm import execute,decode,structure
        for sequence in ([0],[m.HIGH_SCALAR]):
            row=m.make_custom_case(sequence,'probe',repetitions=1);p=row['parameters'];data=m.payload(sequence);code=bytes.fromhex(row['request']['program']['code_hex'][2:])
            for failure in ({'status':0},{'return_size':0},{'return_size':63},{'return_size':65},{'output':bytes(63)+b'\x01'},{'write_size':0}):
                with self.subTest(sequence=sequence,failure=failure),self.assertRaises(AssertionError):execute(code,data,p['transaction_gas_limit'],sequence_length=1,repetitions=1,override=failure)
            shape=structure(list(decode(code)),1,1)
            for pc,value in ((shape['calls'][0]['target_push'],6),(shape['calls'][0]['gas_push'],5999),(shape['calls'][0]['output_size_push'],0),(shape['calls'][0]['clear_x_value'],1)):
                wrong=bytearray(code);wrong[pc+1:pc+4]=value.to_bytes(3,'big')
                with self.assertRaises(AssertionError):execute(bytes(wrong),data,p['transaction_gas_limit'],sequence_length=1,repetitions=1)
            with self.assertRaises(AssertionError):execute(code,data,21000,sequence_length=1,repetitions=1)


if __name__=='__main__':unittest.main()
