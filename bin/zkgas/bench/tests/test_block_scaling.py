"""Proposal-size controls must preserve bytecode and count all included blocks."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'
sys.path.insert(0, str(CASES))
from normal_dispatch_holdout import make_case as reference
from spawn_memory import digest, encoded


def producer():
    path = CASES / 'block_scaling.py'
    assert path.is_file(), 'block scaling producer missing'
    spec = importlib.util.spec_from_file_location('block_scaling', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


class BlockScalingTests(unittest.TestCase):
    def test_matrix_byte_identity_counts_and_matching_controls(self):
        m = producer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'cases'; self.assertEqual(m.prepare(root), 16)
            catalog = json.loads((root/'catalog.json').read_text()); rows = catalog['cases']; by = {r['sample_id']: r for r in rows}
            self.assertEqual(len(by), 16)
            self.assertEqual({(r['parameters']['pattern'],r['parameters']['bulk_operations'],r['parameters']['block_count']) for r in rows}, {(p,n,b) for p in ('dup16-pop','swap16-swap16') for n in (0,196608) for b in (1,4,16,32)})
            for row in rows:
                p=row['parameters']; raw=(root/row['request_path']).read_bytes(); req=json.loads(raw)
                original=reference(p['pattern'],p['bulk_operations'])['request']; blocks=p['block_count']
                self.assertEqual(digest(raw),row['request_sha256'])
                self.assertEqual(req['program']['code_hex'],original['program']['code_hex'])
                self.assertEqual(req['program']['transactions'],original['program']['transactions'])
                self.assertEqual(req['program']['block_count'],blocks)
                self.assertEqual(req['program']['expected_route_counts'],{k:v*blocks for k,v in original['program']['expected_route_counts'].items()})
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['block_count'],blocks)
                self.assertEqual(by[row['baseline_sample_id']]['parameters']['bulk_operations'],0)
                self.assertEqual(p['expected_completed_user_opcodes'],p['expected_completed_user_opcodes_per_block']*blocks)
                self.assertIn('one user execution per block',p['parameter_scope'])
                self.assertEqual(row['known_single_block_reference'],blocks==1)
                self.assertFalse(row['actuals_used_for_fitting'])
                if blocks==1:self.assertEqual(raw,encoded(original))
            before={p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            with self.assertRaises(FileExistsError):m.prepare(root)
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_invalid_domain_and_contextual_guard_not_silently_repeated(self):
        m=producer()
        for args in [('number-pop',0,4),('timestamp-pop',0,4),('dup16-pop',0,True),('dup16-pop',0,0),('dup16-pop',0,33),('dup16-pop',0,1.0),('dup16-pop',1,4)]:
            with self.subTest(args=args),self.assertRaises(ValueError):m.make_case(*args)

    @unittest.skipUnless(os.getenv('NATIVE_FIXTURE_BIN'),'requires frozen native constructor')
    def test_native_multi_block_counts_and_short_total_rejection(self):
        m=producer()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for pattern,n,blocks in [('dup16-pop',0,4),('swap16-swap16',8192,4),('dup16-pop',0,32)]:
                row=m.make_case(pattern,n,blocks); request=root/'request.json';request.write_bytes(encoded(row['request']))
                report=root/'native.json'; output=root/'input.json'; bincode=root/'input.bin'
                command=[os.environ['NATIVE_FIXTURE_BIN'],'--input',str(request),'--output',str(output),'--output-bincode',str(bincode),'--report',str(report),'--execution-profile','evm-gas-only']
                result=subprocess.run(command,capture_output=True,text=True,timeout=90);self.assertEqual(result.returncode,0,result.stderr)
                native=json.loads(report.read_text());self.assertEqual(native['observed_route_counts'],{k:v for k,v in row['request']['program']['expected_route_counts'].items() if v})
                self.assertEqual(native['block_count'],blocks);self.assertEqual(native['transaction_count'],2*blocks)
                self.assertEqual(len(native['receipts_by_block']),blocks)
                self.assertTrue(all(len(rs)==2 and all(r['success'] for r in rs) for rs in native['receipts_by_block']))
                # A per-block rather than proposal-wide declared total must fail construction.
                for path in [report,output,bincode]:path.unlink()
                bad=json.loads(request.read_text());bad['program']['expected_route_counts']['opcode:0x00']-=1;request.write_bytes(encoded(bad))
                rejected=subprocess.run(command,capture_output=True,text=True,timeout=90);self.assertNotEqual(rejected.returncode,0)
                self.assertFalse(any(p.exists() for p in [report,output,bincode]))


if __name__=='__main__':unittest.main()
