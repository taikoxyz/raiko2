#!/usr/bin/env python3
"""Generate the fixed16-row prospective validation for frozen code-preparation pricing."""
import argparse
import json
from pathlib import Path

import code_loading as loading
import top_level_create as original
import top_level_create_followup as followup
from spawn_memory import digest, encoded, write

POLICY = {'revision':'5fd6697f','unique_code_byte_pgu':64,
          'nonempty_preparation_pgu':8192,'prepared_byte_pgu':32}


def factory(outer_revert=False):
    """CALL a factory, require its child CREATE success, then STOP or revert the ancestor."""
    if type(outer_revert) is not bool: raise ValueError('outer_revert must be boolean')
    initcode = bytes.fromhex('6160006000f3')  # Child returns 24576 zero bytes.
    prefix = b'\x65'+initcode+bytes.fromhex('6000526006601a6000f08015')
    terminal = bytes.fromhex('60006000fd') if outer_revert else b'\x00'
    failure_pc = len(prefix)+3+2+len(terminal)
    code = prefix+b'\x61'+failure_pc.to_bytes(2,'big')+b'\x57\x50'+terminal+b'\x5b\xfe'
    row = original.make_case('call','stop',gas_limit=15_000_000)
    row['request']['expected_user_success'] = not outer_revert
    program = row['request']['program']; program['code_hex'] = '0x'+code.hex()
    program['expected_route_counts'].update({'opcode:0x00':int(not outer_revert),
        'opcode:0xf0':1,'opcode:0xf3':1,'opcode:0xfd':int(outer_revert),
        'opcode:0x60':7 if outer_revert else 5,'opcode:0x61':2,'opcode:0x65':1,
        'opcode:0x52':1,'opcode:0x80':1,'opcode:0x15':1,'opcode:0x57':1,'opcode:0x50':1})
    row['sample_id'] = 'factory-outer-revert' if outer_revert else 'factory-outer-stop'
    row['parameters'].update(shape=row['sample_id'],code_bytes=len(code),code_sha256=digest(code),
        expected_user_success=not outer_revert,validation_family='factory',
        expected_code_preparation={'nonempty_events':1,'total_bytes':24576},
        expected_child_account_present=not outer_revert,child_creator='program account, nonce zero')
    return row


def wrap(row, family, known=False):
    prior = row['sample_id'] if known else None
    row['sample_id'] = 'code-preparation-holdout-'+row['sample_id']
    row.update(family='code-preparation-prospective',baseline_sample_id=row['sample_id'],
        holdout_only=not known,data_role='previously_observed_reference_replay' if known else 'prospective_unfitted_candidate',
        reference_role='previously_observed_reference_replay' if known else 'prospective_unfitted_candidate',
        prior_observed_reference_sample_id=prior,
        scope='Frozen-policy prospective validation; actual proposal PGU/node-known pricing, complete quotes and increments are separate. No fitting, per-block actual or capacity claim.')
    row.pop('model_development_probe_only',None)
    p = row['parameters']; p['validation_family'] = family
    if family != 'factory':
        size = p.get('returned_runtime_bytes') or 0
        count = p['count_per_block']*p['block_count'] if family=='creation' and size else 0
        p['expected_code_preparation'] = {'nonempty_events':count,'total_bytes':size*count}
    return row


def prepare(output):
    plans = [wrap(original.make_case('call','stop'),'reference',True),
             wrap(original.make_case('create','return_runtime',runtime_bytes=0,gas_limit=8_000_000),'reference',True)]
    call_reference,create_reference = (r['sample_id'] for r in plans)
    for size in (8192,16384):
        for pattern in loading.PATTERNS:
            row = wrap(loading.make_case(size,pattern),'loading')
            row['baseline_sample_id'] = call_reference
            plans.append(row)
    for row in [followup.make_case(runtime_bytes=2048),followup.make_case(runtime_bytes=12288),
                followup.make_case(count=3,runtime_bytes=24576),
                followup.make_case(shape='filled_runtime',runtime_bytes=8192,fill_byte=0x5b),
                followup.make_case(count=16,runtime_bytes=1,gas_limit=100_000),
                followup.make_case(count=128,runtime_bytes=1,gas_limit=100_000)]:
        row = wrap(row,'creation')
        # A missing matched gas/count control is explicit: evaluate those points
        # absolutely rather than inventing an isolated marginal comparison.
        if row['parameters']['count_per_block']==1: row['baseline_sample_id'] = create_reference
        plans.append(row)
    plans += [wrap(factory(flag),'factory') for flag in (False,True)]
    ids = {r['sample_id'] for r in plans}; assert len(plans)==len(ids)==16
    for row in plans:
        row.pop('same_length_reference_sample_id',None)
        if row['parameters']['validation_family']=='loading' and row['parameters']['pattern']!='zero':
            row['same_length_reference_sample_id'] = 'code-preparation-holdout-'+loading.sample_id(row['parameters']['runtime_bytes'],'zero')
        assert row['baseline_sample_id'] in ids
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir()
    rows=[]
    for row in plans:
        request=encoded(row.pop('request'));relative=f"requests/{row['sample_id']}.json"
        write(output/relative,request);rows.append(dict(row,request_path=relative,request_sha256=digest(request)))
    write(output/'catalog.json',encoded({'schema_version':1,
        'purpose':'Prospective validation of frozen unique-code and repeated deployment preparation prices',
        'frozen_policy':POLICY,'prospective_unfitted_case_count':14,'previously_observed_reference_replay_count':2,
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in
                         ('top_level_create.py','top_level_create_followup.py','code_loading.py','spawn_memory.py')},
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,required=True)
    print(json.dumps({'prepared':prepare(parser.parse_args().output_dir)}))
