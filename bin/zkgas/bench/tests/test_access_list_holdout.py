"""Nine prospective access-list cases and two explicitly reused reference replays."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES=Path(__file__).resolve().parents[1]/'cases'


class AccessListHoldoutTests(unittest.TestCase):
    def load(self):
        source=CASES/'access_list_holdout.py'
        self.assertTrue(source.is_file(),'access-list holdout producer is not implemented')
        sys.path.insert(0,str(CASES))
        spec=importlib.util.spec_from_file_location('access_list_holdout_cases',source)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        return module

    def test_exact_nine_unfitted_candidates_two_reused_references_and_same_block_baselines(self):
        cases=self.load()
        expected={('repeated_address',2,1),('repeated_address',3,1),
                  ('repeated_items',32,1),('repeated_items',512,1),('repeated_items',3072,1),
                  ('repeated_items',512,3),('repeated_key',3072,1),
                  ('distinct_addresses',512,1),('reversed_distinct_keys',256,1)}
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'holdout'
            self.assertEqual(cases.prepare(output),11)
            catalog=json.loads((output/'catalog.json').read_text());rows=catalog['cases']
            self.assertEqual(catalog['prospective_unfitted_case_count'],9)
            self.assertEqual(catalog['previously_observed_reference_replay_count'],2)
            selected={(r['parameters']['pattern'],r['parameters']['count'],r['parameters']['block_count'])
                      for r in rows if r['holdout_only']}
            self.assertEqual(selected,expected)
            references=[r for r in rows if not r['holdout_only']]
            self.assertEqual({(r['parameters']['pattern'],r['parameters']['count'],r['parameters']['block_count'])
                              for r in references},{(None,None,1),(None,None,3)})
            self.assertTrue(all(r['data_role']=='previously_observed_reference_replay' for r in references))
            self.assertTrue(all(r['data_role']=='prospective_unfitted_candidate' for r in rows if r['holdout_only']))
            self.assertEqual(len({r['sample_id'] for r in rows}),11)
            by_id={r['sample_id']:r for r in rows}
            for row in rows:
                self.assertTrue(row['sample_id'].startswith('access-list-holdout-'))
                self.assertNotEqual(row['sample_id'],row.get('prior_observed_reference_sample_id'))
                params=row['parameters'];blocks=params['block_count']
                self.assertEqual(by_id[row['baseline_sample_id']]['parameters']['block_count'],blocks)
                self.assertIsNone(by_id[row['baseline_sample_id']]['parameters']['pattern'])
                request=json.loads((output/row['request_path']).read_text())
                original=cases.base_make_case(params['pattern'],params['count'],blocks)
                self.assertEqual(request,original['request'])
                program=request['program']
                self.assertEqual(program['code_hex'],'0x60006000f3')
                self.assertEqual(program['input_hex'],'0x')
                self.assertEqual(program['expected_route_counts']['opcode:0xf3'],blocks)
                self.assertEqual(params['transaction_gas_limit'],15_000_000)
                if not row['holdout_only']:
                    self.assertEqual(row['prior_observed_reference_sample_id'],original['sample_id'])

    def test_catalog_binds_both_generators_and_requests_and_never_overwrites(self):
        cases=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'holdout';cases.prepare(output)
            catalog=json.loads((output/'catalog.json').read_text())
            self.assertEqual(catalog['generator_sha256'],cases.digest((CASES/'access_list_holdout.py').read_bytes()))
            self.assertEqual(catalog['base_generator_sha256'],cases.digest((CASES/'access_list_values.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'],cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual(catalog['runtime_sha256'],cases.digest(bytes.fromhex('60006000f3')))
            for row in catalog['cases']:
                self.assertEqual(cases.digest((output/row['request_path']).read_bytes()),row['request_sha256'])
            before={p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):cases.prepare(output)
            self.assertEqual(before,{p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_typed_bounds_and_selection_prevent_mislabelled_main_cases(self):
        cases=self.load()
        for args in [('repeated_address',0,1),('repeated_items',8193,1),('repeated_items',True,1),
                     ('repeated_items',512,True),('repeated_items',512,33),('unknown',2,1),
                     ('repeated_address','2',1),('repeated_address',2.0,1),(None,0,1),
                     ('repeated_address',16,1),('distinct_keys',256,1)]:
            with self.assertRaises(ValueError):cases.make_case(*args)
        self.assertEqual(cases.make_case('repeated_items',3072,1)['parameters']['count'],3072)
        self.assertEqual(cases.make_case('repeated_items',512,3)['parameters']['block_count'],3)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'),'set frozen native for all eleven qualifications')
    def test_all_eleven_cases_execute_frozen_native_under_original_gas_envelope(self):
        cases=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);cases.prepare(root/'cases')
            catalog=json.loads((root/'cases/catalog.json').read_text())
            completed=0
            for row in catalog['cases']:
                folder=root/row['sample_id'];folder.mkdir()
                request=root/'cases'/row['request_path']
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(request),
                    '--output',str(folder/'input.json'),'--report',str(folder/'native.json'),
                    '--execution-profile','evm-gas-only'],capture_output=True,timeout=90)
                self.assertEqual(result.returncode,0,result.stderr.decode())
                native=json.loads((folder/'native.json').read_text());params=row['parameters'];blocks=params['block_count']
                self.assertEqual(native['execution_profile'],'evm-gas-only')
                self.assertEqual(native['observed_route_counts'],{'opcode:0x60':2*blocks,'opcode:0xf3':blocks})
                self.assertTrue(all(b[1]['success'] for b in native['receipts_by_block']))
                self.assertTrue(all(b['user_transaction_gas_used']<15_000_000 for b in native['blocks']))
                if params['pattern'] is None:self.assertNotIn('access_list',native)
                else:
                    self.assertEqual(native['access_list']['pattern'],params['pattern'])
                    for item in native['access_list']['blocks']:
                        self.assertEqual(item['transaction_type'],2)
                        for key,value in params['expected_access_list_counts_per_block'].items():
                            self.assertEqual(item[key],value)
                completed+=1
            self.assertEqual(completed,11)


if __name__=='__main__':unittest.main()
