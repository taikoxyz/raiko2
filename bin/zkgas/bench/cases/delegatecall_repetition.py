#!/usr/bin/env python3
"""Generate checked or fixed-unroll DELEGATECALL observations under frozen pricing."""
import argparse
from collections import Counter
import json
from pathlib import Path

from initcode_preparation import _Factory
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL='a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e'
RUNTIMES={'stop':b'\x00','return0':bytes.fromhex('60006000f3'),
          'revert0':bytes.fromhex('60006000fd'),'invalid':b'\xfe'}


def _profile(value):
    if type(value) is not str or value not in ('checked','density'):raise ValueError('profile must be checked or density')
    return value


def make_case(child_terminal='stop',repetitions=1,*,profile='checked'):
    _profile(profile)
    if type(child_terminal) is not str or child_terminal not in RUNTIMES:raise ValueError('unknown child terminal')
    original.integer(repetitions,0,32768,'DELEGATECALL repetitions')
    if profile=='density' and (child_terminal not in ('stop','return0') or repetitions%64):
        raise ValueError('density requires STOP/RETURN0 and total calls divisible by64')
    unroll=64 if profile=='density' else 1
    batches=repetitions//unroll
    runtime=RUNTIMES[child_terminal];length=len(runtime);success=child_terminal in ('stop','return0')
    # The tiny initcode returns exactly the runtime embedded in its PUSH. The
    # parent creates the child even at N0; no extra child-account/code witness
    # entries are added (STOP may already serve a legitimate system account).
    initcode=bytes([0x5f+length])+runtime+bytes.fromhex('60005260')+bytes([length,0x60,32-length,0xf3])
    asm=_Factory();asm.push(int.from_bytes(initcode,'big'),len(initcode));asm.push(0,1);asm.op(0x52)
    asm.push(len(initcode),1);asm.push(32-len(initcode),1);asm.push(0,1);asm.op(0xf0)
    asm.op(0x80);asm.op(0x15);asm.target('failure');asm.op(0x57)
    # Stack is [child address,index]. Fixed PUSH3 makes N an immediate change,
    # not unrolled parent code. N0 exits before making any DELEGATECALL.
    asm.push(0);asm.label('loop',batches+1);asm.op(0x80,batches+1)
    asm.push(batches,times=batches+1);asm.op(0x14,batches+1);asm.target('done',batches+1);asm.op(0x57,batches+1)
    for _ in range(unroll):
        for _ in range(4):asm.push(0,1,batches)  # out size/offset and in size/offset.
        asm.op(0x85,batches);asm.push(64,1,batches);asm.op(0xf4,batches)
        if profile=='checked':
            asm.push(int(success),1,batches);asm.op(0x14,batches);asm.op(0x15,batches);asm.target('failure',batches);asm.op(0x57,batches)
            asm.op(0x3d,batches);asm.target('failure',batches);asm.op(0x57,batches)
        else:asm.op(0x50,batches)  # Discard status; native child-terminal gates prove completion.
    asm.push(1,1,batches);asm.op(0x01,batches);asm.target('loop',batches);asm.op(0x56,batches)
    asm.label('done');asm.op(0x50);asm.op(0x50);asm.op(0x00)
    asm.label('failure',0);asm.op(0xfe,0)
    code=asm.finish();routes=Counter(asm.counts)
    # One real constructor frame always completes these instructions.
    routes[f'opcode:0x{0x5f+length:02x}']+=1;routes['opcode:0x60']+=3
    routes['opcode:0x52']+=1;routes['opcode:0xf3']+=1
    if child_terminal=='stop':routes['opcode:0x00']+=repetitions
    elif child_terminal in ('return0','revert0'):
        routes['opcode:0x60']+=2*repetitions
        routes['opcode:0xf3' if child_terminal=='return0' else 'opcode:0xfd']+=repetitions
    # INVALID is attempted and exceptional, never a completed opcode. Native
    # qualification checks its attempted count and exceptional count separately.
    row=original.make_case('call','stop',gas_limit=15_000_000)
    expected={key:0 for key in row['request']['program']['expected_route_counts']};expected.update(routes)
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=expected)
    sid=f'delegatecall-repetition-{child_terminal}-n{repetitions}'
    row.update(sample_id=sid,family='minimal-delegatecall-repetition',role='control' if repetitions==0 or not success else 'candidate',
               baseline_sample_id=f'delegatecall-repetition-{child_terminal}-n0',data_role='model_development_probe' if success else 'high_price_failure_control',holdout_only=False,
               completion_gate='one successful child CREATE, exact completed and attempted child/parent opcode counts, per-call status/empty-return guards, successful user receipt and factory nonce1',
               scope='Finite whole-proposal actual PGU/node-known zkGas search. Compare repetition only within one terminal/runtime; complete quotes and matched increments are separate. No tariff change, per-frame actual, capacity or throughput claim.')
    row['parameters']={'child_terminal':child_terminal,'repetitions':repetitions,'forwarded_gas':64,'expected_child_success':success,'expected_return_data_bytes':0,
        'child_runtime_hex':'0x'+runtime.hex(),'child_runtime_sha256':digest(runtime),'child_runtime_bytes':length,'child_initcode_sha256':digest(initcode),
        'expected_child_creations':1,'expected_factory_nonce':1,'expected_parent_success':True,'expected_exceptional_instructions':repetitions if child_terminal=='invalid' else 0,
        'expected_code_preparation':{'nonempty_events':1,'total_bytes':length},'factory_runtime_bytes':len(code),'factory_runtime_sha256':digest(code),
        'transaction_kind':'call','count_per_block':1,'block_count':1,'transaction_gas_limit':15_000_000,'outer_calldata_bytes':0,
        'preload_created_accounts_or_code':False,'call_input_bytes':0,'call_output_bytes':0}
    if profile=='density':
        row['request']['program']['expected_route_counts']['opcode:0x3d']=0
        row.update(sample_id=f'delegatecall-density-{child_terminal}-n{repetitions}',family='minimal-delegatecall-density',
            baseline_sample_id=f'delegatecall-density-{child_terminal}-n0',
            completion_gate='one real child CREATE, exact completed child/parent opcodes, zero exceptional instructions, successful user receipt and factory nonce1; no per-call EVM status/return-data guard')
        row['parameters'].update(producer_profile='density',unroll_factor=unroll,batch_count=batches,
            per_call_status_guard=False,per_call_return_data_guard=False)
    reference_count=64 if profile=='density' else 1
    if repetitions>reference_count:
        prefix='delegatecall-density' if profile=='density' else 'delegatecall-repetition'
        row['repetition_reference_sample_id']=f'{prefix}-{child_terminal}-n{reference_count}'
    return row


def prepare(output,*,profile='checked'):
    _profile(profile)
    terminals=('stop','return0') if profile=='density' else RUNTIMES
    counts=(0,64,1024,32768) if profile=='density' else (0,1,1024,32768)
    plans=[make_case(terminal,count,profile=profile) for terminal in terminals for count in counts]
    ids={r['sample_id'] for r in plans};assert len(plans)==len(ids)==(8 if profile=='density' else 16)
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    catalog={'schema_version':1,'frozen_model_id':MODEL,
        'purpose':'High-count minimal DELEGATECALL with exactly one child deployment, fixed64forwarded gas and checked per-call status/empty return data',
        'generator_sha256':digest(Path(__file__).read_bytes()),'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'interpretation':'Only within-terminal repetition comparisons match child/runtime and setup. Expected failure status alone does not establish REVERT execution; exact completed terminal and attempted INVALID counts gate qualification.',
        'cases':rows}
    if profile=='density':
        catalog.update(producer_profile='density',unroll_factor=64,
            purpose='Fixed64-call unroll density with one real child deployment evenN0, fixed64forwarded gas, no per-call status or return-data guard',
            interpretation='Exact native child-terminal and exceptional maps prove every call completed; parent success alone is insufficient. Compare only within-terminal N0/N64 increments; no assumed startup subtraction or pricing conclusion.')
    write(output/'catalog.json',encoded(catalog))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True)
    parser.add_argument('--profile',choices=('checked','density'),default='checked');args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,profile=args.profile)}))
