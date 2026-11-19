"""Opt-in raw access-list vectors and complete old producer-byte compatibility."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

CASES=Path(__file__).resolve().parents[1]/'cases'


def producer():
    sys.path.insert(0,str(CASES))
    spec=importlib.util.spec_from_file_location('raw_access_list_cases',CASES/'access_list_values.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


class AccessListRawKeysTests(unittest.TestCase):
    def test_raw_zero_extreme_order_duplicates_and_identity(self):
        m=producer();keys=[0,(1<<256)-1,1,0];saved=list(keys)
        row=m.make_raw_case(keys);p=row['parameters'];program=row['request']['program']
        raw=b''.join(k.to_bytes(32,'big') for k in keys);digest=hashlib.sha256(raw).hexdigest()
        self.assertEqual(program['access_list'],dict(pattern='raw_keys',count=4,storage_keys=[f'0x{k:064x}' for k in keys]))
        self.assertEqual(p['expected_access_list_counts_per_block'],dict(raw_address_items=1,raw_storage_keys=4,unique_addresses=1,unique_storage_pairs=3))
        self.assertEqual(p['ordered_storage_keys_sha256'],digest);self.assertEqual(p['storage_key_encoding'],'32-byte big-endian concatenation in raw order')
        self.assertIn(digest,row['sample_id']);self.assertIn(digest,row['corpus_id'])
        self.assertEqual(program['code_hex'],'0x60006000f3');self.assertEqual(program['input_hex'],'0x')
        self.assertEqual(p['transaction_gas_limit'],15_000_000);self.assertFalse(p['storage_preloads'])
        self.assertNotIn('storage_state',program);self.assertNotIn('prestate_contracts',program)
        self.assertEqual(keys,saved);frozen=copy.deepcopy(row);keys.append(2);self.assertEqual(row,frozen)
        self.assertEqual(m.make_raw_case(tuple(saved)),row)
        reordered=m.make_raw_case([0,1,(1<<256)-1,0])
        self.assertNotEqual(reordered['sample_id'],row['sample_id']);self.assertEqual(reordered['parameters']['expected_access_list_counts_per_block'],p['expected_access_list_counts_per_block'])

    def test_strict_inputs_bounds_and_multiblock(self):
        m=producer()
        for keys in (None,{},'keys',set([1]),[],[0]*8193,[True],[-1],[1<<256],[1.0],['0x00'],[None]):
            with self.subTest(keys_type=type(keys).__name__),self.assertRaises(ValueError):m.make_raw_case(keys)
        for blocks in (True,False,0,33,-1,1.0,'1',None):
            with self.assertRaises(ValueError):m.make_raw_case([0],blocks)
        for count in (1,8192):
            row=m.make_raw_case([0]*count,32)
            self.assertEqual(row['request']['program']['access_list']['count'],count)
            self.assertEqual(row['parameters']['expected_access_list_counts_per_block']['unique_storage_pairs'],1)
            self.assertEqual(row['request']['program']['block_count'],32)
            self.assertEqual(row['request']['program']['expected_route_counts']['opcode:0xf3'],32)
            self.assertIn('-b32',row['sample_id'])
        with self.assertRaises(ValueError):m.make_case('raw_keys',1)

    def test_old_23_complete_rows_and_requests_remain_exact(self):
        m=producer();digest=hashlib.sha256()
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'cases';self.assertEqual(m.prepare(output),23)
            catalog=json.loads((output/'catalog.json').read_bytes())
            for exported in catalog['cases']:
                raw=(output/exported['request_path']).read_bytes();full=dict(exported)
                del full['request_path'];del full['request_sha256'];full['request']=json.loads(raw)
                for chunk in (m.encoded(full),raw):digest.update(len(chunk).to_bytes(8,'big'));digest.update(chunk)
                self.assertNotIn('storage_keys',full['request']['program'].get('access_list',{}))
            self.assertEqual(digest.hexdigest(),'a235fff409831e0dd1e42d13846637755a81ed49b001391a85b8040f3307e4b0')


if __name__=='__main__':unittest.main()
