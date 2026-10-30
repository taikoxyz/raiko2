#!/usr/bin/env python3
"""Nine prospective access-list candidates and two explicitly reused reference replays."""
import argparse
import json
from pathlib import Path

from access_list_values import RUNTIME, make_case as base_make_case
from spawn_memory import digest, encoded, write

CANDIDATES=(('repeated_address',2,1),('repeated_address',3,1),
            ('repeated_items',32,1),('repeated_items',512,1),('repeated_items',3072,1),
            ('repeated_items',512,3),('repeated_key',3072,1),
            ('distinct_addresses',512,1),('reversed_distinct_keys',256,1))
REFERENCES=((None,None,1),(None,None,3))


def make_case(pattern=None,count=None,block_count=1):
    # The shared producer validates types/bounds and owns the signed-input schema.
    row=base_make_case(pattern,count,block_count)
    if (pattern,count,block_count) not in CANDIDATES+REFERENCES:
        raise ValueError('case is outside the fixed prospective holdout/reference selection')
    previous=row['sample_id']
    reference=pattern is None
    row.update(sample_id=previous.replace('access-list-values-','access-list-holdout-',1),
               family='access-list-preexecution-holdout',
               role='control' if reference else 'candidate',
               pair_group=f'access-list-holdout-b{block_count}',
               baseline_sample_id=f'access-list-holdout-empty-b{block_count}',
               holdout_only=not reference,
               data_role='previously_observed_reference_replay' if reference else 'prospective_unfitted_candidate',
               scope='Nine prospective unfitted candidates; two previously observed empty references '
                     'replayed explicitly; comparisons use equal-block-count own references; no new gas budget')
    if reference:
        row['prior_observed_reference_sample_id']=previous
    return row


def prepare(output):
    plans=[make_case(*selection) for selection in REFERENCES+CANDIDATES]
    output.mkdir(parents=True,exist_ok=False)
    (output/'requests').mkdir()
    write(output/'runtime.hex',(RUNTIME.hex()+'\n').encode())
    rows=[]
    for plan in plans:
        request=encoded(plan.pop('request'))
        relative=f"requests/{plan['sample_id']}.json"
        write(output/relative,request)
        rows.append(dict(plan,request_path=relative,request_sha256=digest(request)))
    write(output/'catalog.json',encoded({'schema_version':1,
        'purpose':'nine prospective access-list candidates plus two explicit previously observed reference replays',
        'prospective_unfitted_case_count':9,'previously_observed_reference_replay_count':2,
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'base_generator_sha256':digest(Path(__file__).with_name('access_list_values.py').read_bytes()),
        'shared_generator_sha256':digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'runtime_sha256':digest(RUNTIME),'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',required=True,type=Path)
    args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir)}))
