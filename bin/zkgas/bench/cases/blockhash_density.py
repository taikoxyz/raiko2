#!/usr/bin/env python3
"""Generate twelve fixed-body BLOCKHASH density probes under frozen pricing."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL='2fb4bb71092f11cca2217e69b25e94217342ffb46a540a961e7a77a483cb5601'
DISTANCES={'nearest':1,'farthest':256,'expired':257,'current':0,'future':-1}


def make_case(mode='nearest',bulk_queries=64):
    if type(mode) is not str or mode not in DISTANCES:raise ValueError('unknown BLOCKHASH target mode')
    original.integer(bulk_queries,0,262144,'bulk queries')
    if bulk_queries%64:raise ValueError('bulk queries must be divisible by64')
    distance=DISTANCES[mode];nonzero=mode in ('nearest','farthest');batches=bulk_queries//64
    asm=_Factory()
    # EVM SUB consumes the top operand first: NUMBER minus distance. All target
    # offsets and batch counts have fixed PUSH3 width, including N0.
    asm.push(abs(distance));asm.op(0x43);asm.op(0x01 if distance<0 else 0x03)
    asm.op(0x80);asm.op(0x40);asm.op(0x15)
    if not nonzero:asm.op(0x15)
    asm.target('failure');asm.op(0x57)
    # Warmup preserved target; stack now [target,index]. One body has64 sites;
    # N0 skips this same unused body rather than using a different program.
    asm.push(0);asm.label('loop',batches+1);asm.op(0x80,batches+1)
    asm.push(batches,times=batches+1);asm.op(0x14,batches+1);asm.target('done',batches+1);asm.op(0x57,batches+1)
    for _ in range(64):asm.op(0x81,batches);asm.op(0x40,batches);asm.op(0x50,batches)
    asm.push(1,1,batches);asm.op(0x01,batches);asm.target('loop',batches);asm.op(0x56,batches)
    asm.label('done');asm.op(0x50);asm.op(0x50);asm.op(0x00)
    asm.label('failure',0);asm.op(0xfe,0)
    code=asm.finish();row=original.make_case('call','stop',gas_limit=15_000_000)
    routes={key:0 for key in row['request']['program']['expected_route_counts']};routes.update(asm.counts)
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
    sid=f'blockhash-density-{mode}-n{bulk_queries}'
    row.update(sample_id=sid,family='blockhash-density',role='control' if bulk_queries==0 else 'candidate',
        baseline_sample_id=f'blockhash-density-{mode}-n0',data_role='model_development_probe' if mode in ('nearest','farthest','expired') else 'native_only_boundary_control',holdout_only=False,
        completion_gate='one semantic warmup, exact completed BLOCKHASH and all parent opcode counts, zero exceptional instructions and successful user receipt',
        scope='Finite whole-proposal actual SP1 PGU/node-known zkGas search. Full closure and within-mode matched increments remain separate; no isolated opcode actual, universal bound, capacity or throughput claim.')
    row['parameters']={'mode':mode,'target_distance':distance,'target_expression':'B+1' if distance<0 else f'B-{distance}',
        'expected_blockhash_nonzero':nonzero,'bulk_queries':bulk_queries,'semantic_warmup_queries':1,'total_blockhash_queries':bulk_queries+1,
        'unroll_factor':64,'batch_count':batches,'per_bulk_query_semantic_guard':False,'expected_user_success':True,'expected_exceptional_instructions':0,
        'expected_code_preparation':{'nonempty_events':0,'total_bytes':0},'expected_factory_nonce':0,'transaction_kind':'call','count_per_block':1,'block_count':1,
        'transaction_gas_limit':15_000_000,'outer_calldata_bytes':0,'code_location':'prestate runtime','runtime_bytes':len(code),'runtime_sha256':digest(code),
        'preload_created_accounts_or_code':False}
    if bulk_queries>64:row['repetition_reference_sample_id']=f'blockhash-density-{mode}-n64'
    return row


def prepare(output,*,model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:raise ValueError('model_id must be a lowercase SHA256')
    plans=[make_case(mode,n) for mode in ('nearest','farthest','expired') for n in (0,64,4096,262144)]
    ids={r['sample_id'] for r in plans};assert len(ids)==len(plans)==12
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':model_id,'unroll_factor':64,
        'purpose':'Twelve BLOCKHASH density probes: nearest/farthest valid ancestor and expired height, one semantic warmup plus fixed64-query bulk batches',
        'generator_sha256':digest(Path(__file__).read_bytes()),'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'interpretation':'Total BLOCKHASH events are bulk_queries+1 including N0. Within-mode N0/N64 references isolate repetition; cross-mode differences also include semantic checks and target arithmetic. Current/future modes are native-only controls outside this catalog. Model ID is metadata, not a pricing override.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
