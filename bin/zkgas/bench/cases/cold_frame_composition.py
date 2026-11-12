#!/usr/bin/env python3
"""Generate12 fixed-model child-access inputs near30M charged block gas."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

MODEL='c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
PROFILES=('cold-return','cold-revert','parent-prewarm-revert')
UNROLL=4
ANCHOR_GAS=167355
TARGET_GAS=29990000
BUDGET_GAS=30000000
HEADER_GAS=31000000
GAS_MARGIN=100000
TAG=int.from_bytes(b'cold-frame-child-v1'.ljust(32,b'\0'),'big')
STATIC={0:0,1:3,0x14:3,0x15:3,0x30:2,0x35:3,0x36:2,0x3d:2,
        0x51:3,0x52:3,0x56:8,0x57:10,0x5b:1,0xf4:100,0xf3:0,0xfd:0}


def _profile(profile):
    if type(profile) is not str or profile not in PROFILES:raise ValueError('unknown cold-frame profile')
    return profile=='parent-prewarm-revert',profile=='cold-return'


def _check(asm,value,*,times=1,width=3):
    asm.push(value,width,times);asm.op(0x14,times);asm.op(0x15,times)
    asm.target('failure',times);asm.op(0x57,times)


def _runtime(profile,calls):
    prewarm,success=_profile(profile)
    original.integer(calls,0,65536,'child calls per user')
    if calls%UNROLL:raise ValueError('child calls must be divisible by4')
    batches=calls//UNROLL;asm=_Factory()
    # Only the outer user has empty calldata. Every self-child authenticates32
    # bytes before reading slot2; terminal output comes from the actual read.
    both=calls+1
    asm.op(0x36,both);asm.op(0x15,both);asm.target('parent',both);asm.op(0x57,both)
    asm.op(0x36,calls);_check(asm,32,times=calls)
    asm.push(0,times=calls);asm.op(0x35,calls);_check(asm,TAG,times=calls,width=32)
    asm.push(2,times=calls);asm.op(0x54,calls);asm.push(0,times=calls);asm.op(0x52,calls)
    asm.push(32,times=calls);asm.push(0,times=calls);asm.op(0xf3 if success else 0xfd,calls)
    asm.label('parent')
    if prewarm:
        asm.push(2);asm.op(0x54);_check(asm,0)
    asm.push(TAG,32);asm.push(256);asm.op(0x52)
    asm.push(0);asm.push(64);asm.op(0x52)
    asm.label('loop',batches+1);asm.push(64,times=batches+1);asm.op(0x51,batches+1)
    asm.push(batches,times=batches+1);asm.op(0x14,batches+1);asm.target('done',batches+1);asm.op(0x57,batches+1)
    for _ in range(UNROLL):
        asm.push(32,times=batches);asm.push(128,times=batches)
        asm.push(32,times=batches);asm.push(256,times=batches)
        asm.op(0x30,batches);asm.push(12000000,times=batches);asm.op(0xf4,batches)
        _check(asm,int(success),times=batches)
        asm.op(0x3d,batches);_check(asm,32,times=batches)
        asm.push(128,times=batches);asm.op(0x51,batches);_check(asm,0,times=batches)
    asm.push(64,times=batches);asm.op(0x51,batches);asm.push(1,1,batches);asm.op(1,batches)
    asm.push(64,times=batches);asm.op(0x52,batches);asm.target('loop',batches);asm.op(0x56,batches)
    asm.label('done');asm.push(64);asm.op(0x51);_check(asm,batches)
    # Do not touch target storage inside the parent loop: a parent SLOAD after
    # the first REVERT would warm the key and invalidate the cold-REVERT axis.
    asm.push(2);asm.op(0x54);_check(asm,0);asm.op(0)
    asm.label('failure',0);asm.op(0xfe,0)
    return asm.finish(),dict(asm.counts)


def _gas(profile,calls):
    prewarm,success=_profile(profile);_,counts=_runtime(profile,calls)
    static=sum(count*(3 if 0x60<=int(name[9:],16)<=0x7f else STATIC[int(name[9:],16)])
               for name,count in counts.items() if count and name!='opcode:0x54')
    cold=calls+1 if profile=='cold-revert' else 1
    warm=(calls+1 if prewarm else calls if success else 0)
    assert cold+warm==counts['opcode:0x54']
    # Parent reaches nine memory words through its input tag; each child
    # separately allocates one word for its SLOAD-derived return/revert data.
    memory=27+3*calls;storage=2100*cold+100*warm
    gross=21000+static+memory+storage
    return {'intrinsic_gas':21000,'static_opcode_gas':static,'memory_expansion_gas':memory,
            'storage_read_gas':storage,'cold_storage_reads':cold,'warm_storage_reads':warm,
            'gross_user_gas':gross,'refund_counter':0,'applied_refund':0,'charged_user_gas':gross}


def make_case(profile,packing=4,level='full'):
    prewarm,success=_profile(profile)
    if type(packing) is not int or packing not in (4,16):raise ValueError('packing must be4 or16')
    if type(level) is not str or level not in ('zero','full'):raise ValueError('level must be zero or full')
    base=_gas(profile,0);step=_gas(profile,UNROLL)['charged_user_gas']-base['charged_user_gas']
    full_batches=((TARGET_GAS-ANCHOR_GAS)//packing-base['charged_user_gas'])//step
    full_calls=UNROLL*full_batches;full_gas=_gas(profile,full_calls)
    limit=full_gas['gross_user_gas']+GAS_MARGIN
    assert ANCHOR_GAS+packing*full_gas['charged_user_gas']>=29700000
    assert ANCHOR_GAS+packing*full_gas['charged_user_gas']<=TARGET_GAS
    assert ANCHOR_GAS+packing*_gas(profile,full_calls+UNROLL)['charged_user_gas']>TARGET_GAS
    assert limit<=min(15000000,HEADER_GAS-ANCHOR_GAS-(packing-1)*full_gas['charged_user_gas'])
    calls=full_calls if level=='full' else 0;gas=_gas(profile,calls);code,counts=_runtime(profile,calls)
    row=original.make_case('call','stop',count=packing,gas_limit=limit)
    routes={key:0 for key in row['request']['program']['expected_route_counts']};routes.update({key:value*packing for key,value in counts.items()})
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
    sid=f'cold-frame-{profile}-t{packing}-{level}'
    row.update(sample_id=sid,family='cold-frame-composition',role='control' if level=='zero' else 'candidate',
        baseline_sample_id=f'cold-frame-{profile}-t{packing}-zero',data_role='unfitted_high_occupancy_child_access_validation',
        holdout_only=True,actuals_used_for_fitting=False,
        completion_gate='Concrete ordered slot2 zero reads, exact child calldata/status/32-byte read-derived output, checkpointed storage warmth and transaction resets, parent counter; exact completed maps, successful canonical user receipts and charged gas.',
        scope='Actual same-proposal SP1PGU, literal node per-block estimate/sum and full proposal estimate remain separate. Coldness is not DB/MPT materialization. Near30M charged budget includes Anchor; native header31M and new node cap not selected. No isolated frame price, refit, global bound, production parity, proof-speed or capacity claim.')
    row['parameters']={'profile':profile,'packing':packing,'level':level,'block_count':1,
        'calls_per_user':calls,'full_calls_per_user':full_calls,'all_child_calls':calls*packing,
        'batch_count_per_user':calls//UNROLL,'full_batch_count_per_user':full_batches,'unroll_factor':UNROLL,
        'all_user_transactions':packing,'all_transactions_including_anchor':packing+1,
        'transaction_kind':'call','transaction_gas_limit':limit,'transaction_envelope':'canonical EIP-1559 same gas limit/empty input users',
        'gas_limit_margin_above_full_gross':GAS_MARGIN,'full_gross_user_gas':full_gas['gross_user_gas'],
        'expected_user_gas_used':gas['charged_user_gas'],'expected_all_user_gas_used':packing*gas['charged_user_gas'],
        'expected_block_gas_used':ANCHOR_GAS+packing*gas['charged_user_gas'],'expected_anchor_gas_used':ANCHOR_GAS,
        'gas':gas,'per_batch_gas_increment':step,'charged_block_gas_budget':BUDGET_GAS,'target_block_gas':TARGET_GAS,
        'minimum_full_block_gas':29700000,'native_header_gas_limit':HEADER_GAS,
        'minimum_native_remaining_before_user':HEADER_GAS-ANCHOR_GAS-(packing-1)*gas['charged_user_gas'],
        'selected_full_last_user_remaining_gas':HEADER_GAS-ANCHOR_GAS-(packing-1)*full_gas['charged_user_gas'],
        'forwarded_gas_request':12000000,'eip150_policy':'Self DELEGATECALL request is capped by63/64 available gas; full gross+100000 outer margin and successful native guards qualify child completion.',
        'parent_prewarm':prewarm,'child_terminal':'return' if success else 'revert','expected_child_status':int(success),
        'expected_child_return_bytes':32,'expected_child_output_hex':'0x'+bytes(32).hex(),
        'child_input_tag_hex':f'0x{TAG:064x}','child_input_bytes':32,'target_storage_slot':2,'expected_storage_value':0,
        'storage_writes':0,'refunds':0,'counter_memory_offset':64,'output_memory_offset':128,'input_tag_memory_offset':256,
        'expected_final_counter':calls//UNROLL,'expected_parent_memory_words':9,'expected_child_memory_words':1,
        'cold_reads_per_user':gas['cold_storage_reads'],'warm_reads_per_user':gas['warm_storage_reads'],
        'access_scope':'Slot2 is absent/zero. Only parent-prewarm profile reads before loop; parent never reads between child calls. Successful child warmth persists; child REVERT restores its checkpoint; each new transaction resets warmth. This does not count DB loads or trie materializations.',
        'expected_user_success':True,'expected_exceptional_instructions':0,'outer_calldata_bytes':0,
        'execution_profile':'evm-gas-only','runtime_bytes':len(code),'runtime_sha256':digest(code),
        'code_location':'prestate runtime','preload_created_accounts_or_code':False}
    return row


def prepare(output,*,model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:raise ValueError('model_id must be lowercase SHA256')
    plans=[make_case(profile,packing,level) for profile in PROFILES for packing in (4,16) for level in ('zero','full')]
    ids={row['sample_id'] for row in plans};assert len(plans)==len(ids)==12
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));name=f"requests/{row['sample_id']}.json";write(output/name,raw)
        rows.append(row|{'request_path':name,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':model_id,
        'purpose':'Twelve zero/full repeated child-access inputs near30M charged block gas; cold RETURN, cold REVERT and parent-prewarmed REVERT, four/sixteen users.',
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'actuals_used_for_fitting':False,'required_comparison_quantities':['actual identical-proposal SP1 PGU','node per-block quotes and literal sum, aggregate-ceil retained','full proposal estimate'],
        'priority':'Strictly more than2B positive actual-minus-node PGU within30M charged whole-block gas is high priority; every actual/deficit/control remains visible.',
        'interpretation':'Frozen modelc893/SDK6.3/ELFb76, canonical honest input. Native warmth is inferred only with concrete checkpoint oracle plus exact receipt gas; cold flags do not imply extra DB/MPT work. Keep all six controls and failures. No price, guest, SDK, limit or proof-deadline change.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
