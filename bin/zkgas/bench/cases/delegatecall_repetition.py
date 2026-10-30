#!/usr/bin/env python3
"""Generate16 minimal-frame DELEGATECALL repetition observations under frozen pricing."""
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


def make_case(child_terminal='stop',repetitions=1):
    if type(child_terminal) is not str or child_terminal not in RUNTIMES:raise ValueError('unknown child terminal')
    original.integer(repetitions,0,32768,'DELEGATECALL repetitions')
    runtime=RUNTIMES[child_terminal];length=len(runtime);success=child_terminal in ('stop','return0')
    # The tiny initcode returns exactly the runtime embedded in its PUSH. The
    # parent creates the child even at N0; child code/account is never preloaded.
    initcode=bytes([0x5f+length])+runtime+bytes.fromhex('60005260')+bytes([length,0x60,32-length,0xf3])
    asm=_Factory();asm.push(int.from_bytes(initcode,'big'),len(initcode));asm.push(0,1);asm.op(0x52)
    asm.push(len(initcode),1);asm.push(32-len(initcode),1);asm.push(0,1);asm.op(0xf0)
    asm.op(0x80);asm.op(0x15);asm.target('failure');asm.op(0x57)
    # Stack is [child address,index]. Fixed PUSH3 makes N an immediate change,
    # not unrolled parent code. N0 exits before making any DELEGATECALL.
    asm.push(0);asm.label('loop',repetitions+1);asm.op(0x80,repetitions+1)
    asm.push(repetitions,times=repetitions+1);asm.op(0x14,repetitions+1);asm.target('done',repetitions+1);asm.op(0x57,repetitions+1)
    for _ in range(4):asm.push(0,1,repetitions)  # out size/offset and in size/offset.
    asm.op(0x85,repetitions);asm.push(64,1,repetitions);asm.op(0xf4,repetitions)
    asm.push(int(success),1,repetitions);asm.op(0x14,repetitions);asm.op(0x15,repetitions);asm.target('failure',repetitions);asm.op(0x57,repetitions)
    asm.op(0x3d,repetitions);asm.target('failure',repetitions);asm.op(0x57,repetitions)
    asm.push(1,1,repetitions);asm.op(0x01,repetitions);asm.target('loop',repetitions);asm.op(0x56,repetitions)
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
    if repetitions>1:row['repetition_reference_sample_id']=f'delegatecall-repetition-{child_terminal}-n1'
    return row


def prepare(output):
    plans=[make_case(terminal,count) for terminal in RUNTIMES for count in (0,1,1024,32768)]
    ids={r['sample_id'] for r in plans};assert len(plans)==len(ids)==16
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':MODEL,
        'purpose':'High-count minimal DELEGATECALL with exactly one child deployment, fixed64forwarded gas and checked per-call status/empty return data',
        'generator_sha256':digest(Path(__file__).read_bytes()),'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'interpretation':'Only within-terminal repetition comparisons match child/runtime and setup. Expected failure status alone does not establish REVERT execution; exact completed terminal and attempted INVALID counts gate qualification.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True)
    print(json.dumps({'prepared':prepare(parser.parse_args().output_dir)}))
