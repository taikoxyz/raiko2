#!/usr/bin/env python3
"""Generate24 balanced stack-pair density probes with a memory-resident loop counter."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL='bbce9be0cd9792bfa9c46369203f6441c3fd288787ea357e311eae8ebd48aac4'
PATTERNS={'push0-pop':(0x5f,0x50),'dup1-pop':(0x80,0x50),'dup3-pop':(0x82,0x50),
          'dup8-pop':(0x87,0x50),'dup16-pop':(0x8f,0x50),
          'swap1-swap1':(0x90,0x90),'swap8-swap8':(0x97,0x97),'swap16-swap16':(0x9f,0x9f)}
SEEDS=tuple(int(digest(f'zkgas-stack-seed-v1-{i}'.encode()),16)|(1<<255) for i in range(17))
assert len(set(SEEDS))==17


def make_case(pattern='dup1-pop',bulk_pairs=4096):
    if type(pattern) is not str or pattern not in PATTERNS:raise ValueError('unknown bounded stack pair')
    original.integer(bulk_pairs,0,262144,'bulk pairs')
    if bulk_pairs%64:raise ValueError('bulk pairs must be divisible by64')
    batches=bulk_pairs//64;first,second=PATTERNS[pattern];asm=_Factory()
    # Initialize the only memory word before seeding the actual17-word stack.
    asm.push(0);asm.push(0);asm.op(0x52)
    for value in SEEDS:asm.push(value,32)
    asm.label('loop',batches+1);asm.push(0,times=batches+1);asm.op(0x51,batches+1)
    asm.push(batches,times=batches+1);asm.op(0x14,batches+1);asm.target('done',batches+1);asm.op(0x57,batches+1)
    # The loop condition consumes its temporary values. Body stack is exactly
    # the17 original words: DUP1 never observes a changing counter.
    for _ in range(64):asm.op(first,batches);asm.op(second,batches)
    asm.push(0,times=batches);asm.op(0x51,batches);asm.push(1,1,batches);asm.op(0x01,batches)
    asm.push(0,times=batches);asm.op(0x52,batches);asm.target('loop',batches);asm.op(0x56,batches)
    asm.label('done')
    # Consume and compare every seed, from the actual top down. A balanced
    # wrong-depth DUP still needs the independent accessed-value oracle.
    for value in reversed(SEEDS):
        asm.push(value,32);asm.op(0x14);asm.op(0x15);asm.target('failure');asm.op(0x57)
    asm.op(0x00);asm.label('failure',0);asm.op(0xfe,0)
    code=asm.finish();row=original.make_case('call','stop',gas_limit=15_000_000)
    routes={key:0 for key in row['request']['program']['expected_route_counts']};routes.update(asm.counts)
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
    sid=f'stack-density-{pattern}-n{bulk_pairs}'
    row.update(sample_id=sid,family='balanced-stack-density',role='control' if bulk_pairs==0 else 'candidate',
        baseline_sample_id=f'stack-density-{pattern}-n0',data_role='model_development_probe',holdout_only=False,
        completion_gate='independent decoded actual stack access/value/order, memory counter and every ending seed; exact completed opcodes, successful user receipt and zero exceptions',
        scope='Finite balanced-pair observations. Preserve complete proposal estimate, node-observable estimate for each block and sum, and actual same-proposal SP1 PGU; underestimation judgments are independent and include N0. Matched increments explain gaps only. Balanced pairs do not identify isolated POP/DUP/SWAP coefficients or a universal bound.')
    row['parameters']={'pattern':pattern,'pair_opcode_hex':[f'0x{first:02x}',f'0x{second:02x}'],'bulk_pairs':bulk_pairs,
        'body_instruction_events':2*bulk_pairs,'unroll_factor':64,'batch_count':batches,'body_stack_words':17,
        'stack_words_bottom_to_top':[f'0x{v:064x}' for v in SEEDS],'ending_word_guards':17,'counter_memory_offset':0,
        'counter_memory_bytes':32,'expected_counter_final':batches,'counter_ADD_events':batches,
        'counter_MLOAD_events':2*batches+1,'counter_MSTORE_events':batches+1,'counter_present_on_body_stack':False,
        'runtime_bytes':len(code),'runtime_sha256':digest(code),'expected_user_success':True,'expected_exceptional_instructions':0,
        'expected_code_preparation':{'nonempty_events':0,'total_bytes':0},'expected_program_nonce':0,
        'transaction_kind':'call','count_per_block':1,'block_count':1,'transaction_gas_limit':15_000_000,
        'outer_calldata_bytes':0,'code_location':'prestate runtime','preload_created_accounts_or_code':False}
    if bulk_pairs>4096:row['repetition_reference_sample_id']=f'stack-density-{pattern}-n4096'
    return row


def prepare(output,*,model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:raise ValueError('model_id must be a lowercase SHA256')
    plans=[make_case(pattern,n) for pattern in PATTERNS for n in (0,4096,262144)];ids={r['sample_id'] for r in plans};assert len(plans)==len(ids)==24
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':model_id,'unroll_factor':64,
        'purpose':'Twenty-four balanced PUSH0/POP, DUP/POP and double-SWAP density observations; counter stays in memory word0, all17 stack words checked',
        'generator_sha256':digest(Path(__file__).read_bytes()),'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'seed_derivation':'SHA256(zkgas-stack-seed-v1-{i}) interpreted big-endian with bit255 set, i0..16 bottom-to-top',
        'required_comparison_quantities':['complete proposal zkGas estimate','node-observable estimate for each block and their sum','actual same-proposal SP1 PGU'],
        'interpretation':'Keep literal actual/node ranking, N0 and both independent underestimation flags. Same-pattern N0/N4096 increments are explanatory. Each pair and memory-counter loop is composite; do not fabricate isolated POP/DUP/SWAP fits. Model ID is metadata, not compiled price selection. No proposal-to-mining allocation, tariff or guest change.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
