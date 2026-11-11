"""Finite signed-calldata byte/content/count probes with a constant STOP body."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))


class SignedPayloadTests(unittest.TestCase):
    def load(self):
        return importlib.import_module('signed_payload')

    def test_14_cases_unique_requests_hashes_and_same_count_empty_baselines(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cases'
            self.assertEqual(m.prepare(output),14)
            catalog = json.loads((output/'catalog.json').read_bytes()); rows = catalog['cases']
            self.assertEqual(len({row['sample_id'] for row in rows}),14)
            self.assertEqual({(row['parameters']['calldata_bytes_per_user'],row['parameters']['pattern'],row['parameters']['count_per_block']) for row in rows},
                {(length,pattern,count) for length in (0,32,4096,65536) for pattern in ('zero','sequence') if length or pattern == 'zero' for count in (1,8)})
            by_id = {row['sample_id']: row for row in rows}; requests = set()
            for row in rows:
                p = row['parameters']; raw = (output/row['request_path']).read_bytes(); requests.add(raw)
                self.assertEqual(row['request_sha256'],hashlib.sha256(raw).hexdigest())
                baseline = by_id[row['baseline_sample_id']]['parameters']
                self.assertEqual(baseline['count_per_block'],p['count_per_block'])
                self.assertEqual(baseline['calldata_bytes_per_user'],0)
            self.assertEqual(len(requests),14)
            before = {p:p.read_bytes() for p in output.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(output)
            self.assertEqual(before,{p:p.read_bytes() for p in output.rglob('*') if p.is_file()})

    def test_payload_bytes_exact_floor_gas_and_body_counts(self):
        m = self.load()
        for length in (0,1,32,256,4096,65536):
            for pattern in ('zero','sequence'):
                if length == 0 and pattern != 'zero':continue
                for count in (1,8):
                    row = m.make_case(length,pattern,count); p = row['parameters']; program = row['request']['program']
                    expected = bytes(length) if pattern == 'zero' else bytes((73*i+19)%256 for i in range(length))
                    zero = expected.count(0); nonzero = length-zero
                    self.assertEqual(bytes.fromhex(program['input_hex'][2:]),expected)
                    self.assertEqual(p['payload_sha256'],hashlib.sha256(expected).hexdigest())
                    self.assertEqual(p['calldata_zero_bytes_per_user'],zero)
                    self.assertEqual(p['calldata_nonzero_bytes_per_user'],nonzero)
                    self.assertEqual(p['all_user_calldata_bytes'],length*count)
                    intrinsic = 21000+4*zero+16*nonzero; floor = 21000+10*(zero+4*nonzero)
                    self.assertEqual(p['expected_user_intrinsic_gas'],intrinsic)
                    self.assertEqual(p['expected_user_floor_gas'],floor)
                    self.assertEqual(p['expected_user_charged_gas'],max(intrinsic,floor))
                    self.assertEqual(p['expected_all_user_charged_gas'],count*max(intrinsic,floor))
                    self.assertEqual(p['expected_block_charged_gas'],m.ANCHOR_GAS+count*max(intrinsic,floor))
                    self.assertEqual(program['code_hex'],'0x00')
                    self.assertEqual(program['transactions'],{'kind':'call','count_per_block':count,'gas_limit':3_000_000})
                    self.assertEqual(program['block_count'],1)
                    self.assertTrue(row['request']['expected_user_success'])
                    self.assertEqual({key:value for key,value in program['expected_route_counts'].items() if value},{'opcode:0x00':count})
                    self.assertFalse({'storage_state','authorizations','access_list','prestate_contracts'} & program.keys())
                    self.assertEqual(p['execution_profile'],'evm-gas-only')
                    self.assertIsNone(p['canonical_signed_envelope_bytes'])
                    self.assertIn('encode_2718_len',p['canonical_signed_envelope_byte_source'])
                    self.assertFalse(p['calldata_bytes_equal_canonical_signed_envelope_bytes'])
        self.assertEqual(m.make_case(65536,'sequence',8)['parameters']['expected_user_charged_gas'],2_634_760)

    def test_invalid_typed_bounds_rejected_before_output_creation(self):
        m = self.load()
        bad = [(True,'zero',1),(-1,'zero',1),(65537,'zero',1),(None,'zero',1),('32','zero',1),(32.0,'zero',1),
               (0,'sequence',1),(1,True,1),(1,None,1),(1,[] ,1),(1,'unknown',1),
               (1,'zero',True),(1,'zero',0),(1,'zero',9),(1,'zero','1'),(1,'zero',1.0)]
        for args in bad:
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)
        for bad_path in (None,True,'catalog'):
            with self.assertRaises(ValueError):m.prepare(bad_path)

    def test_stdlib_only_cli_and_no_overwrite(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)/'cli'; command = [sys.executable,'-S',str(CASES/'signed_payload.py'),'--output-dir',str(output)]
            result = subprocess.run(command,capture_output=True,timeout=30)
            self.assertEqual(result.returncode,0,result.stderr.decode())
            self.assertEqual(json.loads(result.stdout),{'prepared':14})
            self.assertNotEqual(subprocess.run(command,capture_output=True,timeout=30).returncode,0)

    @unittest.skipUnless(os.environ.get('SIGNED_PAYLOAD_NATIVE_BIN'),'set native binary for signed payload qualification')
    def test_native_all_cases_status_gas_envelopes_and_exact_stop_counts(self):
        m = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); m.prepare(root/'cases')
            for row in json.loads((root/'cases/catalog.json').read_bytes())['cases']:
                folder = root/row['sample_id']; folder.mkdir(); p = row['parameters']
                result = subprocess.run([os.environ['SIGNED_PAYLOAD_NATIVE_BIN'],'--input',str(root/'cases'/row['request_path']),
                    '--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),
                    '--execution-profile','evm-gas-only'],capture_output=True,timeout=60)
                self.assertEqual(result.returncode,0,f"{row['sample_id']}: {result.stderr.decode()}")
                report = json.loads((folder/'native.json').read_bytes()); receipts = report['receipts_by_block'][0]
                self.assertEqual(len(receipts),p['count_per_block']+1)
                self.assertTrue(receipts[0]['success']); self.assertEqual(receipts[0]['gas_used'],m.ANCHOR_GAS)
                for receipt in receipts[1:]:
                    self.assertTrue(receipt['success']); self.assertEqual(receipt['gas_used'],p['expected_user_charged_gas'])
                self.assertEqual(report['user_transaction_gas_used_total'],p['expected_all_user_charged_gas'])
                self.assertEqual(report['blocks'][0]['block_gas_used'],p['expected_block_charged_gas'])
                self.assertEqual({key:value for key,value in report['observed_route_counts'].items() if value},{'opcode:0x00':p['count_per_block']})
                users = report['transactions']['blocks'][0]['users']
                self.assertEqual([user['nonce'] for user in users],list(range(p['count_per_block'])))
                self.assertTrue(all(user['gas_limit'] == 3_000_000 and user['create_address'] is None and user['to'] is not None for user in users))
                guest = json.loads((folder/'input.json').read_bytes())
                signed = guest['witnesses'][0]['block']['body']['transactions']
                self.assertEqual(len(signed),p['count_per_block']+1)
                expected_input = json.loads((root/'cases'/row['request_path']).read_bytes())['program']['input_hex']
                for index, signed_user in enumerate(signed[1:]):
                    transaction = signed_user['transaction']['Eip1559']
                    self.assertEqual(transaction['input'],expected_input)
                    self.assertEqual(transaction['nonce'],index)
                    self.assertEqual(transaction['gas_limit'],3_000_000)
                    self.assertIn('signature',signed_user)


if __name__ == '__main__':unittest.main()
