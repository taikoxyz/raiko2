"""Finite access-list vectors, raw counts, controls and native inclusion gates."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1]/'cases'


class AccessListValuesTests(unittest.TestCase):
    def load(self):
        source = CASES/'access_list_values.py'
        self.assertTrue(source.is_file(), 'access-list producer is not implemented')
        sys.path.insert(0, str(CASES))
        spec = importlib.util.spec_from_file_location('access_list_cases', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_fixed_23_case_matrix_shared_baselines_hashes_and_no_overwrite(self):
        cases = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(cases.prepare(output), 23)
            catalog = json.loads((output/'catalog.json').read_text())
            rows = catalog['cases']
            expected = {(None,None,1),(None,None,3),('repeated_address',1,1),('repeated_key',1,1),
                        ('repeated_key',2048,1),('reversed_distinct_keys',4096,1),
                        ('repeated_address',4096,3),('repeated_key',4096,3)}
            expected |= {(p,k,1) for p in ('repeated_address','distinct_addresses',
                         'repeated_key','distinct_keys') for k in (16,256,4096)}
            expected |= {('repeated_items',k,1) for k in (16,256,2048)}
            self.assertEqual(len(rows), 23)
            self.assertEqual({(r['parameters']['pattern'],r['parameters']['count'],
                              r['parameters']['block_count']) for r in rows},expected)
            self.assertEqual(len({r['sample_id'] for r in rows}),23)
            self.assertEqual(sum(r['parameters']['count']==1 for r in rows),2)
            self.assertEqual(catalog['generator_sha256'],cases.digest((CASES/'access_list_values.py').read_bytes()))
            self.assertEqual(catalog['shared_generator_sha256'],cases.digest((CASES/'spawn_memory.py').read_bytes()))
            self.assertEqual(catalog['runtime_sha256'],cases.digest(bytes.fromhex('60006000f3')))
            by_id={r['sample_id']:r for r in rows}
            requests=[]
            for row in rows:
                data=(output/row['request_path']).read_bytes();requests.append(data)
                self.assertEqual(row['request_sha256'],cases.digest(data))
                request=json.loads(data);program=request['program'];params=row['parameters']
                self.assertEqual(program['code_hex'],'0x60006000f3')
                self.assertEqual(program['input_hex'],'0x')
                self.assertEqual(program['expected_route_counts']['opcode:0xf3'],params['block_count'])
                self.assertNotIn('authorizations',program)
                self.assertNotIn('storage_state',program)
                self.assertEqual(params['transaction_gas_limit'],15_000_000)
                self.assertEqual(by_id[row['baseline_sample_id']]['parameters']['block_count'],params['block_count'])
                if params['pattern'] is None:self.assertNotIn('access_list',program)
                else:self.assertEqual(program['access_list'],dict(pattern=params['pattern'],count=params['count']))
            self.assertEqual(len(set(requests)),23)
            saved={p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):cases.prepare(output)
            self.assertEqual(saved,{p.relative_to(output):p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_typed_bound_and_raw_versus_unique_counts(self):
        cases=self.load()
        for pattern,count,blocks in [(None,0,1),('unknown',1,1),(True,1,1),
            ('repeated_key',0,1),('repeated_key',8193,1),('repeated_key',True,1),
            ('repeated_key','1',1),('repeated_key',1,True),('repeated_key',1,0),('repeated_key',1,33)]:
            with self.assertRaises(ValueError):cases.make_case(pattern,count,blocks)
        expected={'repeated_address':(4,0,1,0),'distinct_addresses':(4,0,4,0),
                  'repeated_key':(1,4,1,1),'distinct_keys':(1,4,1,4),
                  'repeated_items':(4,4,1,1),'reversed_distinct_keys':(1,4,1,4)}
        for pattern,counts in expected.items():
            row=cases.make_case(pattern,4)
            selectors=row['parameters']['expected_access_list_counts_per_block']
            self.assertEqual(tuple(selectors[k] for k in ('raw_address_items','raw_storage_keys',
                             'unique_addresses','unique_storage_pairs')),counts)
            self.assertEqual(cases.make_case(pattern,8192)['parameters']['count'],8192)

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'),'set new native fixture for access-list qualification')
    def test_all_23_real_native_vectors_preserve_counts_and_root_return(self):
        cases=self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);catalog=root/'cases';cases.prepare(catalog)
            for row in json.loads((catalog/'catalog.json').read_text())['cases']:
                folder=root/row['sample_id'];folder.mkdir()
                result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(catalog/row['request_path']),
                    '--execution-profile','evm-gas-only','--output',str(folder/'guest.json'),
                    '--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json')],
                    capture_output=True,timeout=120)
                self.assertEqual(result.returncode,0,f"{row['sample_id']}: {result.stderr.decode()}")
                report=json.loads((folder/'native.json').read_text());params=row['parameters']
                self.assertEqual(report['observed_route_counts']['opcode:0xf3'],params['block_count'])
                self.assertTrue(all(r[0]['success'] and r[1]['success'] for r in report['receipts_by_block']))
                self.assertTrue(all(r[1]['gas_used']<=15_000_000 for r in report['receipts_by_block']))
                if params['pattern'] is None:self.assertNotIn('access_list',report)
                else:
                    self.assertEqual(report['access_list']['pattern'],params['pattern'])
                    self.assertEqual(report['access_list']['count_per_block'],params['count'])
                    self.assertEqual(len(report['access_list']['blocks']),params['block_count'])
                    for index,block in enumerate(report['access_list']['blocks']):
                        self.assertEqual(block['block_index'],index);self.assertEqual(block['transaction_type'],2)
                        self.assertEqual({k:block[k] for k in params['expected_access_list_counts_per_block']},
                                         params['expected_access_list_counts_per_block'])


if __name__ == '__main__':
    unittest.main()
