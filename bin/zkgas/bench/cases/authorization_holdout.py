#!/usr/bin/env python3
"""Generate separate compact-signature EIP-7702 pricing-validation holdouts."""
import argparse
import json
from pathlib import Path

from spawn_memory import digest, encoded, write

RUNTIME = bytes.fromhex('60006000f3')
MAX_COUNT = 1024


def make_case(count, block_count=1):
    if (type(count) is not int or not 1 <= count <= MAX_COUNT or
            type(block_count) is not int or not 1 <= block_count <= 32):
        raise ValueError('compact authorization count/block count exceeds typed local bounds')
    sid = f'authorization-holdout-compact-k{count}-b{block_count}'
    program = {'code_hex':'0x'+RUNTIME.hex(), 'input_hex':'0x', 'block_count':block_count,
               'authorizations':{'pattern':'compact','count':count},
               'expected_route_counts':{'opcode:0xf3':block_count, 'opcode:0xfd':0, 'opcode:0x00':0,
                                        'opcode:0xfa':0, 'opcode:0xf1':0, 'opcode:0xf2':0,
                                        'opcode:0xf4':0, 'opcode:0xf0':0, 'opcode:0xf5':0}}
    return {'sample_id':sid, 'family':'authorization-compact-holdout',
            'role':'control' if count == 1 else 'candidate',
            'pair_group':f'authorization-compact-holdout-b{block_count}',
            'holdout_only':True,
            'parameters':{'transaction_type':'eip7702', 'pattern':'compact', 'count':count,
                           'block_count':block_count, 'outer_calldata_bytes':0,
                           'signature':{'r':'0x1','s':'0x1','y_parity':0}, 'authorization_nonce':0,
                           'authority_semantics':'canonical recovery of the fixed authorization signature; no private fixture key used',
                           'expected_signed_entries_total':count*block_count,
                           'expected_applied_delegations_by_block':[1]+[0]*(block_count-1),
                           'expected_authority_nonce_by_block':[1]*block_count,
                           'expected_authority_count':1, 'canonical_authorization_rlp_bytes':30},
            'completion_gate':'canonical type-4/signature recovery and inclusion; all signed entries retained; '
                              'actual authority nonce/code hash; root RETURN per block',
            'scope':'separate compact-signature validation holdout; repeated fixed-nonce entries remain '
                    'in every list even when stale; distinct from the frozen main cohort baseline',
            'request':{'schema_version':1, 'expected_user_success':True, 'program':program}}


def prepare(output):
    selections = [(1,1),(16,1),(128,1),(512,1),(16,3)]
    plans = [make_case(*selection) for selection in selections]
    output.mkdir(parents=True,exist_ok=False)
    (output/'requests').mkdir()
    write(output/'runtime.hex',(RUNTIME.hex()+'\n').encode())
    rows = []
    for plan in plans:
        data = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output/relative,data)
        rows.append(dict(plan,request_path=relative,request_sha256=digest(data)))
    write(output/'catalog.json',encoded({'schema_version':1,
        'purpose':'separate compact EIP-7702 signed-entry pricing-validation holdout',
        'holdout_only':True,
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'shared_generator_sha256':digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'runtime_sha256':digest(RUNTIME), 'cases':rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir)}))
