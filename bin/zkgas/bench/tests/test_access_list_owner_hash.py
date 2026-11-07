"""Fb20 owner value construction; independent RV64 multiplication oracle only."""
import hashlib
import importlib.util
import sys
from pathlib import Path
import unittest

CASES=Path(__file__).resolve().parents[1]/'cases'
sys.path.insert(0,str(CASES))
M32=(1<<32)-1
M64=(1<<64)-1


def rv64_fold(left,right):
    """MUL/MULHU via 32-bit schoolbook products and explicit carries."""
    columns=[0]*5
    for i in range(2):
        for j in range(2):
            columns[i+j]+=((left>>(32*i))&M32)*((right>>(32*j))&M32)
    for i in range(4):
        columns[i+1]+=columns[i]>>32
        columns[i]&=M32
    assert columns[4]==0
    return (columns[0]|columns[1]<<32)^(columns[2]|columns[3]<<32)


def outer_hash(address,seed,actual_shared):
    raw=bytes.fromhex(address[2:])
    x0=int.from_bytes(raw[:8],'little')
    x1=int.from_bytes(raw[8:16],'little')
    x2=int.from_bytes(raw[16:],'little')
    return rv64_fold(rv64_fold(x0^seed,x1^actual_shared)^x2,actual_shared)


def helper():
    spec=importlib.util.spec_from_file_location('owner_catalog_helper',CASES/'access_list_hash.py')
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    return m


class OwnerCatalogTests(unittest.TestCase):
    def require_helper(self):
        m=helper();self.assertTrue(hasattr(m,'concentrated_owner_addresses'),'owner catalog API is missing');return m

    def test_encoding_order_uniqueness_prefixes_and_conditional_full_hash(self):
        m=self.require_helper()
        for g in (0,M64,0x1122334455667788):
            for prefix in (0,4,6):
                for count in (1,64,8192):
                    values=m.concentrated_owner_addresses(g,count,prefix_bytes=prefix)
                    self.assertEqual(len(values),count);self.assertEqual(len(set(values)),count)
                    for index,value in enumerate(values,1):
                        raw=bytes.fromhex(value[2:]);fixed=(b'\xa5'+bytes(prefix-1)) if prefix else b''
                        self.assertEqual(value,'0x'+raw.hex());self.assertEqual(len(raw),20)
                        self.assertEqual(raw[:8],fixed+index.to_bytes(8-prefix,'little'))
                        self.assertEqual(raw[8:16],g.to_bytes(8,'little'));self.assertEqual(raw[16:],b'\x01\x00\x00\x00')
                    for seed in (0,M64,0x8877665544332211):
                        self.assertEqual({outer_hash(a,seed,g) for a in values},{g})
        self.assertEqual(m.concentrated_owner_addresses(17,2),m.concentrated_owner_addresses(17,2,prefix_bytes=6))

    def test_rv64_schoolbook_oracle_exercises_high_half_and_carry_edges(self):
        for left,right,expected in ((1<<63,2,1),(1<<32,1<<32,1),(M64,M64,M64),(M64,2,M64),(0,M64,0)):
            self.assertEqual(rv64_fold(left,right),expected)
        # Independent full-width oracle qualification, not a producer cost/hash helper.
        import random
        rng=random.Random(48271)
        for _ in range(128):
            left=rng.getrandbits(64);right=rng.getrandbits(64);product=left*right
            self.assertEqual(rv64_fold(left,right),(product&M64)^(product>>64))

    def test_wrong_own_shared_is_not_a_realization_certificate(self):
        m=self.require_helper();values=m.concentrated_owner_addresses(17,8)
        hashes=[outer_hash(a,3,19) for a in values]
        self.assertNotEqual(set(hashes),{19});self.assertGreater(len(set(hashes)),1)
        self.assertEqual({outer_hash(a,3,17) for a in values},{17})
        # Not a claim that every wrong-G/input pair has a particular distribution.

    def test_strict_typed_domain_and_existing_raw_address_composition(self):
        m=self.require_helper()
        for g in (True,False,None,'1',1.0,-1,1<<64):
            with self.assertRaises(ValueError):m.concentrated_owner_addresses(g,1)
        for count in (True,False,None,'1',1.0,0,-1,16385):
            with self.assertRaises(ValueError):m.concentrated_owner_addresses(0,count)
        for prefix in (True,False,None,'6',6.0,-1,1,2,3,5,7,8):
            with self.assertRaises(ValueError):m.concentrated_owner_addresses(0,1,prefix_bytes=prefix)
        with self.assertRaises(TypeError):m.concentrated_owner_addresses(0,1,6)
        from access_list_values import make_raw_addresses_case
        values=m.concentrated_owner_addresses(17,4,prefix_bytes=4)
        row=make_raw_addresses_case(values)
        self.assertEqual(row['request']['program']['access_list'],dict(pattern='raw_addresses',count=4,addresses=values))
        self.assertEqual(row['parameters']['expected_access_list_counts_per_block'],dict(raw_address_items=4,raw_storage_keys=0,unique_addresses=4,unique_storage_pairs=0))
        self.assertEqual(row['parameters']['ordered_addresses_sha256'],hashlib.sha256(b''.join(bytes.fromhex(a[2:]) for a in values)).hexdigest())
        self.assertNotIn('transactions',row['request']['program'])


if __name__=='__main__':unittest.main()
