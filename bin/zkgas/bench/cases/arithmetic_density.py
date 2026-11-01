#!/usr/bin/env python3
"""Generate24 fixed-body ADD/MUL observations with qualified ordered uint256 operands."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL='bbce9be0cd9792bfa9c46369203f6441c3fd288787ea357e311eae8ebd48aac4'
MASK=(1<<256)-1
MIXED_A=int(digest(b'zkgas-arithmetic-operand-a-v1'),16)|(1<<255)
MIXED_B=int(digest(b'zkgas-arithmetic-operand-b-v1'),16)|(1<<255)
OPERANDS={'add':{'zero':(0,0),'small':(7,11),'carry':(MASK,1),'mixed256':(MIXED_A,MIXED_B)},
          'mul':{'zero':(0,0),'small':(7,11),'wide':(MASK,MASK-2),'mixed256':(MIXED_A,MIXED_B)}}


def make_case(operation='add',operand_class='small',bulk_operations=4096):
    if type(operation) is not str or operation not in OPERANDS:raise ValueError('operation must be add or mul')
    if type(operand_class) is not str or operand_class not in OPERANDS[operation]:raise ValueError('unknown operand class for this operation')
    original.integer(bulk_operations,0,262144,'bulk operations')
    if bulk_operations%64:raise ValueError('bulk operations must be divisible by64')
    a,b=OPERANDS[operation][operand_class];opcode=1 if operation=='add' else 2;batches=bulk_operations//64
    expected=(a+b if operation=='add' else a*b)&MASK;asm=_Factory()
    # Keep [b,a] so every target event consumes top-first(a,b). Fixed PUSH32
    # widths prevent zero/small operands or results from changing the shape.
    asm.push(b,32);asm.push(a,32);asm.op(0x81);asm.op(0x81);asm.op(opcode)
    asm.push(expected,32);asm.op(0x14);asm.op(0x15);asm.target('failure');asm.op(0x57)
    # Stack remains [b,a,index]. Each pair of DUP3 copies b then a, preserving
    # both original operands and the counter, even after overflowing results.
    asm.push(0);asm.label('loop',batches+1);asm.op(0x80,batches+1)
    asm.push(batches,times=batches+1);asm.op(0x14,batches+1);asm.target('done',batches+1);asm.op(0x57,batches+1)
    for _ in range(64):asm.op(0x82,batches);asm.op(0x82,batches);asm.op(opcode,batches);asm.op(0x50,batches)
    asm.push(1,1,batches);asm.op(0x01,batches);asm.target('loop',batches);asm.op(0x56,batches)
    asm.label('done');asm.op(0x50);asm.op(0x50);asm.op(0x50);asm.op(0x00)
    asm.label('failure',0);asm.op(0xfe,0)
    code=asm.finish();row=original.make_case('call','stop',gas_limit=15_000_000)
    routes={key:0 for key in row['request']['program']['expected_route_counts']};routes.update(asm.counts)
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
    sid=f'arithmetic-density-{operation}-{operand_class}-n{bulk_operations}'
    row.update(sample_id=sid,family='ordinary-arithmetic-density',role='control' if bulk_operations==0 else 'candidate',
        baseline_sample_id=f'arithmetic-density-{operation}-{operand_class}-n0',data_role='model_development_probe',holdout_only=False,
        completion_gate='independent concrete ordered operands and modulo result, semantic guard, exact target/counter and completed opcode counts, successful user receipt and zero exceptions',
        scope='Finite same-proposal actual SP1 PGU / node-known zkGas search. Keep proposal estimate, each node block estimate plus sum, and actual workload separately. Both underestimation flags include controls; matched increments explain gaps but do not replace either flag. No isolated-opcode actual, capacity or universal-bound claim.')
    row['parameters']={'operation':operation,'opcode':f'0x{opcode:02x}','operand_class':operand_class,
        'operand_a_hex':f'0x{a:064x}','operand_b_hex':f'0x{b:064x}','expected_result_hex':f'0x{expected:064x}',
        'operand_order':'EVM top-first pop order(a,b); retained stack[b,a,index]','result_arithmetic':'modulo2^256',
        'bulk_operations':bulk_operations,'semantic_target_operations':1,'target_pair_events':bulk_operations+1,
        'counter_ADD_events':batches,'completed_target_opcode_events':bulk_operations+1+(batches if operation=='add' else 0),
        'unroll_factor':64,'batch_count':batches,'operand_immediate_bytes':32,'per_bulk_operation_guard':False,
        'runtime_bytes':len(code),'runtime_sha256':digest(code),'expected_user_success':True,'expected_exceptional_instructions':0,
        'expected_code_preparation':{'nonempty_events':0,'total_bytes':0},'expected_program_nonce':0,
        'transaction_kind':'call','count_per_block':1,'block_count':1,'transaction_gas_limit':15_000_000,'outer_calldata_bytes':0,
        'code_location':'prestate runtime','preload_created_accounts_or_code':False}
    if bulk_operations>4096:row['repetition_reference_sample_id']=f'arithmetic-density-{operation}-{operand_class}-n4096'
    return row


def prepare(output,*,model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:raise ValueError('model_id must be a lowercase SHA256')
    plans=[make_case(operation,cls,n) for operation in OPERANDS for cls in OPERANDS[operation] for n in (0,4096,262144)]
    ids={r['sample_id'] for r in plans};assert len(ids)==len(plans)==24
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':model_id,'unroll_factor':64,
        'purpose':'Twenty-four dense ADD/MUL observations with unchanged ordered uint256 operand pairs, one semantic guard and fixed64-site body evenN0',
        'generator_sha256':digest(Path(__file__).read_bytes()),'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'mixed_operand_derivation':'SHA256(zkgas-arithmetic-operand-a-v1/b-v1) interpreted big-endian with bit255 set',
        'required_comparison_quantities':['complete proposal zkGas estimate','node-observable estimate for each block and its sum','actual same-proposal SP1 PGU'],
        'interpretation':'Target operand-pair events=N+1 including semantic guard. ADD also executes N/64 counter events with different operands. Retain literal actual/node and both independent underestimation flags, including N0. Same-profile N0/N4096 increments diagnose fixed versus scalable gaps only. Model ID changes metadata, not request bytes or compiled prices. No proposal-aware mining allocation is implemented.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
