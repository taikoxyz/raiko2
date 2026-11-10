"""Independent literal execution and optional real native qualification of ordinary probes."""
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CASES = Path(__file__).resolve().parents[1] / 'cases'


def execute(code, *, corrupt_copy=False, copy_overrun=False, call_status=1, return_size=0, codesize=None):
    """Only the emitted EVM subset; derive gas/counts from actual instruction execution."""
    stack, memory, counts, logs, pc, gas, calls = [], bytearray(352), Counter(), [], 0, 33, 0
    def pop(): return stack.pop()
    for _ in range(4_000_000):
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; stack.append(int.from_bytes(code[pc:pc+width], 'big')); pc += width; gas += 3
        elif 0x80 <= op <= 0x8f: stack.append(stack[-(op-0x7f)]); gas += 3
        elif op == 1: stack.append((pop()+pop()) % 2**256); gas += 3
        elif op == 0x14: stack.append(int(pop()==pop())); gas += 3
        elif op == 0x15: stack.append(int(pop()==0)); gas += 3
        elif op == 0x50: pop(); gas += 2
        elif op == 0x51: stack.append(int.from_bytes(memory[pop():][:32], 'big')); gas += 3
        elif op == 0x52:
            offset, value = pop(), pop(); memory[offset:offset+32] = value.to_bytes(32, 'big'); gas += 3
        elif op in (0x56, 0x57):
            destination = pop(); take = op == 0x56 or pop() != 0; gas += 8 if op == 0x56 else 10
            if take: assert code[destination] == 0x5b; pc = destination
        elif op == 0x5b: gas += 1
        elif op == 0x38: stack.append(len(code) if codesize is None else codesize); gas += 2
        elif op == 0x39:
            destination, source, length = pop(), pop(), pop(); gas += 3+3*((length+31)//32)
            memory[destination:destination+length] = (bytes(length) if corrupt_copy else code[source:source+length])
            if copy_overrun: memory[destination+length] = 255
        elif op == 0xa3:
            offset, length = pop(), pop(); topics = tuple(pop() for _ in range(3))
            logs.append((topics, bytes(memory[offset:offset+length]))); gas += 1500+8*length
        elif op == 0xfa:
            forwarded, address, input_offset, input_length, output_offset, output_length = [pop() for _ in range(6)]
            assert (forwarded, address, input_offset, input_length, output_offset, output_length) == (1000, int('3000000000000000000000000000000000010001',16), 0,0,0,0)
            gas += 2600 if calls == 0 else 100; calls += 1
            counts['opcode:0x00'] += 1; stack.append(call_status)
        elif op == 0x3d: stack.append(return_size); gas += 2
        elif op in (0, 0xfe): return dict(terminal=op, counts=counts, logs=logs, memory=memory, stack=stack, gas=21000+gas, calls=calls)
        else: raise AssertionError(f'unexpected opcode {op:02x}')
    raise AssertionError('loop did not complete')


class OrdinaryTariffsTests(unittest.TestCase):
    def load(self):
        source = CASES / 'ordinary_tariffs.py'
        self.assertTrue(source.is_file(), 'ordinary tariff producer is not implemented')
        sys.path.insert(0, str(CASES)); spec = importlib.util.spec_from_file_location('ordinary_cases', source)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

    def test_executed_results_counts_and_gas_not_labels(self):
        m = self.load()
        for operation, lengths in [('codesize',(0,)), ('codecopy',(0,32,64,96,97)), ('log3',(0,32,256)), ('staticcall',(0,))]:
            for length in lengths:
                for lane in ('target','scaffold'):
                    sizes = []
                    for n in (0,1,3):
                        row = m.make_case(operation,repetitions=n,length=length,lane=lane); p=row['parameters']; program=row['request']['program']
                        code=bytes.fromhex(program['prestate_contracts'][0]['code_hex'][2:]); result=execute(code); sizes.append(len(code))
                        self.assertEqual(result['terminal'],0); self.assertEqual(result['stack'],[])
                        self.assertEqual(result['gas'],p['expected_user_charged_gas'])
                        for key,count in program['expected_route_counts'].items(): self.assertEqual(result['counts'].get(key,0),count,(operation,lane,n,key))
                        if operation=='log3':
                            self.assertEqual(result['logs'],[((i,0x22,0x33),(bytes(range(1,256))+b'\x01')[:length]) for i in range(n)] if lane=='target' else [])
                        if operation=='codecopy':
                            self.assertEqual(result['memory'][:128],bytes(range(1,length+1))+bytes(128-length) if n and lane=='target' else bytes(128))
                            self.assertEqual(result['memory'][128:160],bytes(32))
                        if operation=='staticcall': self.assertEqual(result['calls'],1+(n if lane=='target' else 0))
                    self.assertEqual(set(sizes),{1024})

    def test_wrong_opcode_and_result_mutations_fail_semantic_gates(self):
        m=self.load()
        def code(operation, **kwargs): return bytes.fromhex(m.make_case(operation,repetitions=2,**kwargs)['request']['program']['prestate_contracts'][0]['code_hex'][2:])
        self.assertEqual(execute(code('codesize'),codesize=1023)['terminal'],0xfe)
        self.assertEqual(execute(code('codecopy',length=96),corrupt_copy=True)['terminal'],0xfe)
        for length in (0,32,64,96,97):
            self.assertEqual(execute(code('codecopy',length=length),copy_overrun=True)['terminal'],0xfe)
        damaged=bytearray(code('codecopy',length=96)); damaged[896]=0
        self.assertEqual(execute(bytes(damaged))['terminal'],0xfe)
        swapped=bytearray(code('codecopy',length=96))
        operands=swapped.index(bytes.fromhex('62000060620003806200000039'))
        swapped[operands+5:operands+8]=bytes(3)
        swapped[operands+9:operands+12]=(896).to_bytes(3,'big')
        self.assertEqual(execute(bytes(swapped))['terminal'],0xfe)
        for kw in (dict(call_status=0),dict(return_size=1)):
            self.assertEqual(execute(code('staticcall'),**kw)['terminal'],0xfe)
        c=bytearray(code('staticcall')); c[c.index(0xfa)]=0xf1
        with self.assertRaises(AssertionError): execute(bytes(c))

    def test_invalid_types_bounds_and_gas_cap(self):
        m=self.load()
        for kwargs in [dict(operation=True),dict(operation='call'),dict(repetitions=True),dict(repetitions=-1),dict(repetitions=65537),dict(length=True),dict(length=-1),dict(length=98),dict(operation='codesize',length=1),dict(operation='staticcall',length=32),dict(operation='log3',length=97),dict(lane=[]),dict(lane='control'),dict(operation='log3',length=256,repetitions=65536)]:
            args=dict(operation='codecopy',repetitions=1,length=32); args.update(kwargs)
            with self.subTest(kwargs=kwargs),self.assertRaises(ValueError): m.make_case(**args)

    def test_catalog_frozen_roles_sources_references_exclusive_output(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'catalog'; self.assertEqual(m.prepare(root),94)
            catalog=json.loads((root/'catalog.json').read_text()); rows=catalog['cases']; ids={r['sample_id'] for r in rows}
            self.assertEqual(len(ids),94); self.assertEqual(catalog['frozen_model_id'],m.MODEL)
            for name,sha in catalog['source_sha256'].items(): self.assertEqual(m.digest((CASES/name).read_bytes()),sha)
            for row in rows:
                raw=(root/row['request_path']).read_bytes(); self.assertEqual(m.digest(raw),row['request_sha256'])
                self.assertIn(row['baseline_sample_id'],ids); self.assertIn(row['paired_sample_id'],ids)
                self.assertEqual(row['holdout_only'],row['data_role']=='reserved_validation')
                self.assertFalse(row['actuals_used_for_fitting']); p=row['parameters']
                self.assertLessEqual(p['expected_user_charged_gas'],15000000)
                self.assertLessEqual(p['expected_block_charged_gas'],30000000)
                if p['operation']=='codecopy': self.assertEqual(p['codecopy_raw_gas'],{0:3,32:6,64:9,96:12,97:15}[p['length']])
            before=(root/'catalog.json').read_bytes()
            with self.assertRaises(FileExistsError): m.prepare(root)
            self.assertEqual((root/'catalog.json').read_bytes(),before)
            with self.assertRaises(ValueError): m.prepare(Path(tmp)/'bad',model_id='invalid')
            self.assertFalse((Path(tmp)/'bad').exists())

    @unittest.skipUnless(os.environ.get('NATIVE_FIXTURE_BIN'),'set frozen native for actual minimized-witness qualification')
    def test_real_native_gas_routes_minimized_witness_and_success(self):
        m=self.load()
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            for operation,length,n in [('codesize',0,0),('codesize',0,65536),('codecopy',0,3),('codecopy',32,3),('codecopy',64,3),('codecopy',96,65536),('codecopy',97,3),('log3',0,3),('log3',32,3),('log3',256,2048),('staticcall',0,0),('staticcall',0,16384)]:
                for lane in ('target','scaffold'):
                    row=m.make_case(operation,repetitions=n,length=length,lane=lane); folder=root/row['sample_id']; folder.mkdir()
                    (folder/'request.json').write_bytes(m.encoded(row['request']))
                    result=subprocess.run([os.environ['NATIVE_FIXTURE_BIN'],'--input',str(folder/'request.json'),'--output',str(folder/'input.json'),'--output-bincode',str(folder/'input.bin'),'--report',str(folder/'native.json'),'--execution-profile','evm-gas-only'],capture_output=True,timeout=180)
                    self.assertEqual(result.returncode,0,result.stderr.decode()); report=json.loads((folder/'native.json').read_text())
                    self.assertEqual(report['transaction_count'],2); self.assertTrue(all(r['success'] for r in report['receipts_by_block'][0]))
                    self.assertEqual(report['user_transaction_gas_used_total'],row['parameters']['expected_user_charged_gas'])
                    self.assertEqual(report['observations_by_block'][0]['exceptional_instructions'],0)
                    for key,count in row['request']['program']['expected_route_counts'].items():
                        self.assertEqual(report['observed_route_counts'].get(key,0),count)
                        self.assertEqual(report['attempted_route_counts'].get(key,0),count)
                    self.assertLessEqual(report['prestate_contracts']['minimized_witness_node_count'],report['prestate_contracts']['full_witness_node_count'])


if __name__=='__main__': unittest.main()
