"""Tiny independent byte/gas checks; never generate the86-user campaign."""
from collections import Counter
import copy
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))


def producer():
    path = CASES / 'cache_state_warm_accounts.py'
    assert path.is_file(), 'warm producer API is missing'
    spec = importlib.util.spec_from_file_location('warm_accounts', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def execute(code, access_list, balances=None):
    # Literal decoder, separate from producer assembler and accounting helpers.
    warm = {int(item['address'], 16) for item in access_list}
    gas = 21000 + sum(2400 + 1900*len(item['storage_keys']) for item in access_list)
    stack=[]; reads=[]; counts=Counter(); pc=0
    boundaries=set(); scan=0
    while scan < len(code):
        boundaries.add(scan); op=code[scan]; scan += 1 + (op-0x5f if 0x60<=op<=0x7f else 0)
    for _ in range(800):
        assert pc in boundaries
        op=code[pc]; pc+=1; counts[f'opcode:0x{op:02x}']+=1
        if 0x60<=op<=0x7f:
            width=op-0x5f; stack.append(int.from_bytes(code[pc:pc+width],'big'));pc+=width;gas+=3
        elif op==0x31:
            address=stack.pop();gas+=100 if address in warm else 2600;warm.add(address)
            reads.append('0x'+address.to_bytes(20,'big').hex());stack.append((balances or {}).get(address,0))
        elif op==0x57:
            target,condition=stack.pop(),stack.pop();gas+=10
            assert target in boundaries and code[target:target+2]==b'\x5b\xfe'
            if condition: pc=target
        elif op==0x5b:gas+=1
        elif op==0xfe:raise AssertionError('nonzero BALANCE guard')
        elif op==0xf3:
            assert stack.pop()==0 and stack.pop()==0 and not stack
            return gas,reads,dict(counts)
        else:raise AssertionError('unexpected opcode')
    raise AssertionError('execution bound')


class WarmAccountsTests(unittest.TestCase):
    def test_tiny_three_users_warm_bytecode_lists_order_and_ids(self):
        m=producer();row=m.make_case(user_count=3);p=row['request']['program'];addresses=row['parameters']['ordered_targets']
        self.assertNotIn('access_list',p);self.assertEqual(p['block_count'],1)
        self.assertEqual(p['transactions']['count_per_block'],3);self.assertEqual(p['transactions']['gas_limit'],400000)
        counts=Counter()
        for i,(contract,descriptor) in enumerate(zip(p['prestate_contracts'],p['transactions']['access_lists'])):
            part=addresses[128*i:128*(i+1)];self.assertEqual(descriptor,dict(pattern='raw_addresses',count=128,addresses=part))
            code=bytes.fromhex(contract['code_hex'][2:]);self.assertEqual(len(code),3335)
            gas,reads,routes=execute(code,[dict(address=a,storage_keys=[]) for a in part]);counts.update(routes)
            self.assertEqual((gas,reads),(343054,part));self.assertEqual(contract['address'],p['transactions']['recipients'][i])
            with self.assertRaisesRegex(AssertionError,'nonzero BALANCE'):
                execute(code,[dict(address=a,storage_keys=[]) for a in part],{int(part[67],16):1})
            missing=execute(code,[dict(address=a,storage_keys=[]) for a in part[:-1]])[0]
            self.assertEqual(missing,343154) # one intrinsic item removed (-2400), one cold read (+2500)
        self.assertEqual({k:v for k,v in p['expected_route_counts'].items() if v},dict(counts))
        digest=hashlib.sha256(b''.join(bytes.fromhex(a[2:]) for a in addresses)).hexdigest()
        self.assertIn(digest,row['sample_id']);self.assertIn(digest,row['corpus_id'])
        self.assertEqual(len(set(addresses)),384)
        self.assertEqual(m.gas_accounting()['expected_block_charged_gas'],29669999)
        self.assertEqual(m.gas_accounting()['last_user_remaining_header_gas'],1673055)

    def test_raw_copy_strict_domains_and_reference_recipe(self):
        m=producer();row=m.make_case(user_count=1);targets=row['parameters']['ordered_targets']
        self.assertEqual(targets,['0x'+(b'\xa5'+bytes(5)+i.to_bytes(2,'little')+bytes(8)+b'\x01'+bytes(3)).hex() for i in range(1,129)])
        self.assertEqual(row['role'],'control')
        self.assertEqual(row['parameters']['reference_shared0'],0)
        self.assertFalse(row['parameters']['fixed_control_is_observed_seed'])
        self.assertEqual(row['parameters']['catalog_source'],'fixed-g0-compressible-owner-control')
        raw=m.make_case(targets=targets,user_count=1);saved=copy.deepcopy(raw);targets.reverse();self.assertEqual(raw,saved)
        self.assertNotEqual(m.make_case(targets=targets,user_count=1)['sample_id'],saved['sample_id'])
        for count in (True,0,87,1.0,'1',None):
            with self.assertRaises(ValueError):m.make_case(user_count=count)
        original=row['parameters']['ordered_targets']
        for bad in ([],original[:-1],original+[original[0]],set(original),original[0],None):
            if bad is None:continue
            with self.assertRaises(ValueError):m.make_case(targets=bad,user_count=1)
        for bad in (True,0,'0x00','0x'+'66'*20,'0x'+'00'*20,row['request']['program']['prestate_contracts'][0]['address'],original[1]):
            targets=list(original);targets[0]=bad
            with self.assertRaises(ValueError):m.make_case(targets=targets,user_count=1)

    def test_tiny_prepare_exclusive_and_source_identity(self):
        m=producer()
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)/'catalog';self.assertEqual(m.prepare(output,user_count=1),1)
            import json
            catalog=json.loads((output/'catalog.json').read_bytes());case=catalog['cases'][0]
            self.assertEqual(hashlib.sha256((output/case['request_path']).read_bytes()).hexdigest(),case['request_sha256'])
            self.assertIn('cases/cache_state_accounts.py',catalog['source_sha256'])
            self.assertIn('cases/access_list_hash.py',catalog['source_sha256'])
            sys.path.insert(0, str(CASES.parent))
            import workload
            self.assertEqual(workload.load_catalog(output/'catalog.json'), catalog['cases'])
            with self.assertRaises(FileExistsError):m.prepare(output,user_count=1)

    def test_default_calls_owner_helper_once_before_slicing(self):
        from unittest.mock import patch
        m=producer(); original=m.owner.concentrated_owner_addresses
        with patch.object(m.owner,'concentrated_owner_addresses',wraps=original) as spy:
            row=m.make_case(user_count=2)
        spy.assert_called_once_with(0,256,prefix_bytes=6)
        self.assertNotEqual(row['request']['program']['transactions']['access_lists'][0]['addresses'],
            row['request']['program']['transactions']['access_lists'][1]['addresses'])

    def test_owner_only_bound_preserves_generic_domain(self):
        import access_list_hash as h
        # Pure layout/domain boundary only; not an actual candidate or canonical input.
        self.assertEqual(len(h.concentrated_owner_addresses(0,16384)),16384)
        for bad in (True,0,16385,1.0):
            with self.assertRaises(ValueError):h.concentrated_owner_addresses(0,bad)
        for fn in (h.concentrated_keys,h.concentrated_short_prefix_keys):
            with self.assertRaises(ValueError):fn(0,8193)
        with self.assertRaises(ValueError):h.dispersed_keys(8193)

if __name__=='__main__':unittest.main()
