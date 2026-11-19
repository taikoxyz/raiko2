#!/usr/bin/env python3
"""Generate equivalent-bytecode proposal-size controls for node/full/actual comparisons."""
import argparse
from pathlib import Path
import re

import normal_dispatch_holdout as base
from spawn_memory import digest, encoded, write
from top_level_create import integer

PATTERNS=('dup16-pop','swap16-swap16')


def make_case(pattern='dup16-pop',bulk_operations=196608,block_count=4):
    if type(pattern) is not str or pattern not in PATTERNS:
        raise ValueError('block scaling accepts context-independent DUP16/POP or paired SWAP16')
    integer(block_count,1,32,'block count')
    row=base.make_case(pattern,bulk_operations);program=row['request']['program']
    program['block_count']=block_count
    program['expected_route_counts']={k:v*block_count for k,v in program['expected_route_counts'].items()}
    old_id=row['sample_id'];sid=f'block-scaling-{pattern}-n{bulk_operations}-b{block_count}'
    row.update(sample_id=sid,family='ordinary-opcode-block-scaling',
        baseline_sample_id=f'block-scaling-{pattern}-n0-b{block_count}',
        known_single_block_reference=block_count==1,reference_sample_id=old_id,
        data_role='known_single_block_reference' if block_count==1 else 'unfitted_block_count_validation',
        holdout_only=block_count!=1,actuals_used_for_fitting=False,
        completion_gate='identical runtime per block; all canonical users included in order, successful receipts, aggregate opcode map,17 seed guards and memory counter at each execution',
        scope='Separate proposal-once remainder from per-block/operation growth. Keep whole-proposal actual against node per-block sum and complete quote. No per-block actual attribution, startup allocation, refit or capacity claim.')
    row.pop('repetition_reference_sample_id',None)
    p=row['parameters'];per_block=p['expected_completed_user_opcodes']
    p.update(block_count=block_count,parameter_scope='Inherited runtime, counter, seed-guard and bulk counts describe one user execution per block; expected_completed_user_opcodes and all_* counts are proposal totals',
        expected_completed_user_opcodes_per_block=per_block,
        expected_completed_user_opcodes=per_block*block_count,all_user_transaction_count=block_count,
        all_transaction_count_including_anchor=2*block_count)
    return row


def prepare(output,*,model_id=base.MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    plans=[make_case(p,n,b) for p in PATTERNS for n in (0,196608) for b in (1,4,16,32)]
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|dict(request_path=path,request_sha256=digest(raw)))
    write(output/'catalog.json',encoded(dict(schema_version=1,frozen_model_id=model_id,
        purpose='Sixteen proposal-size controls, four known single-block references and twelve new multi-block inputs',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name:digest(Path(__file__).with_name(name).read_bytes()) for name in
            ('normal_dispatch_holdout.py','stack_density.py','initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        required_comparison_quantities=['proposal zkGas estimate','node per-block estimates and their literal sum','actual identical proposal SP1 PGU'],
        interpretation='Candidate64 already frozen. Reuse four exact b1 inputs/actuals as known references; new b4/16/32 inputs need separate native qualification and actual measurements. Preserve failures, all N0 controls, both rounding conventions and both underpricing flags. Do not fit from this panel or assign proposal actual to blocks.',cases=rows)))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=base.MODEL);args=parser.parse_args()
    print(encoded(dict(prepared=prepare(args.output_dir,model_id=args.model_id))).decode(),end='')
