#!/usr/bin/env python3
"""Generate38 deployment size/content/repetition and call-return development probes."""
import argparse
import json
from pathlib import Path

import top_level_create as original
from spawn_memory import digest, encoded, write

LENGTHS = (0, 1, 31, 33, 1024, 4095, 4096, 4097, 8192, 16384, 24575, 24576)
FILL_BYTES = (0, 0x5b, 0x7f)


def filled_initcode(length, seed):
    """Build runtime in memory, keeping transaction payload small across contents."""
    original.integer(length, 1, 24576, 'filled runtime bytes')
    if type(seed) is not int or seed not in FILL_BYTES:
        raise ValueError('fill byte must be0x00,0x5b or0x7f')
    def push3(value): return b'\x62'+value.to_bytes(3, 'big')
    code = b'\x60'+bytes([seed])+push3(0)+b'\x53'
    filled, copies = 1, 0
    while filled < length:
        count = min(filled, length-filled)
        code += push3(count)+push3(0)+push3(filled)+b'\x5e'
        filled += count; copies += 1
    return code+push3(length)+push3(0)+b'\xf3', copies


def make_case(kind='create', shape='return_runtime', count=1, block_count=1, *,
              runtime_bytes=0, gas_limit=8_000_000, fill_byte=None):
    if type(kind) is not str or kind not in ('call', 'create'):
        raise ValueError('transaction kind must be call or create')
    if type(shape) is not str or shape not in ('return_runtime', 'filled_runtime'):
        raise ValueError('unknown creation followup shape')
    original.integer(runtime_bytes, 0, 24577 if kind == 'call' else 24576, 'returned runtime bytes')
    original.integer(count, 1, original.MAX_COUNT, 'count per block')
    original.integer(block_count, 1, 32, 'block count')
    original.integer(gas_limit, 21000, 15_000_000, 'transaction gas limit')
    if shape == 'filled_runtime':
        if kind != 'create': raise ValueError('filled runtime is a creation probe')
        code, copies = filled_initcode(runtime_bytes, fill_byte)
    else:
        if fill_byte is not None: raise ValueError('direct RETURN does not accept a fill byte')
        copies = 0; code = b'\x61'+runtime_bytes.to_bytes(2, 'big')+bytes.fromhex('6000f3')
    # Reuse the canonical original descriptor, routes and envelope. CALL return
    # data has no deployment-size restriction, unlike successful creation.
    row = original.make_case('create', 'return_runtime', count, block_count,
                             runtime_bytes=runtime_bytes, gas_limit=gas_limit)
    request = row['request']; request['expected_user_success'] = True
    program = request['program']; program['code_hex'] = '0x'+code.hex()
    descriptor = program['transactions']; descriptor['kind'] = kind
    runtime = bytes([0 if fill_byte is None else fill_byte])*runtime_bytes
    if kind == 'create': descriptor['expected_runtime_hex'] = '0x'+runtime.hex()
    else: descriptor.pop('expected_runtime_hex', None)
    total = count*block_count
    routes = program['expected_route_counts']
    if shape == 'filled_runtime':
        routes.update({'opcode:0x60':total, 'opcode:0x61':0,
                       'opcode:0x62':total*(3*copies+3), 'opcode:0x53':total,
                       'opcode:0x5e':total*copies})
    sid = f'top-create-followup-{kind}-{shape}-r{runtime_bytes}-n{count}-b{block_count}-g{gas_limit}'
    if fill_byte is not None: sid += f'-v{fill_byte:02x}'
    known = (kind == 'create' and shape == 'return_runtime' and count == 1 and
             gas_limit == 8_000_000 and (runtime_bytes, block_count) in ((0, 1), (24576, 1), (0, 3)))
    prior = original.make_case('create', 'return_runtime', block_count=block_count,
                               runtime_bytes=runtime_bytes, gas_limit=gas_limit)['sample_id'] if known else None
    row.update(sample_id=sid, family='top-level-creation-followup', baseline_sample_id=sid,
               role='control' if runtime_bytes == 0 else 'candidate',
               reference_role='previously_observed_reference_replay' if known else 'new_model_development_case',
               prior_observed_reference_sample_id=prior, holdout_only=False,
               scope='Finite model-development followup: deployment size, repeated identical code, content and ordinary call-return controls; three known reference replays; not prospective holdout, per-tx actual or capacity evidence.')
    row['parameters'].update(transaction_kind=kind, shape=shape, code_bytes=len(code), code_sha256=digest(code),
                             fill_byte=fill_byte, mcopy_count=copies, returned_bytes_sha256=digest(runtime),
                             expected_user_success=True, expected_created_account_present=kind == 'create',
                             expected_exceptional_instructions=0, outer_calldata_bytes=0 if kind == 'call' else len(code),
                             code_location='prestate runtime' if kind == 'call' else 'signed creation input')
    return row


def prepare(output):
    plans = [make_case(runtime_bytes=size) for size in LENGTHS]
    plans += [make_case(block_count=3)]
    plans += [make_case(count=n, block_count=b, runtime_bytes=24576) for n, b in ((2, 1), (4, 1), (1, 3), (2, 3))]
    plans += [make_case(count=n, block_count=b) for n, b in ((2, 1), (4, 1), (2, 3))]
    plans += [make_case(count=n, runtime_bytes=256, gas_limit=300_000) for n in (1, 16, 128)]
    plans += [make_case(count=n, runtime_bytes=1024, gas_limit=500_000) for n in (1, 16, 64)]
    plans += [make_case(shape='filled_runtime', runtime_bytes=size, fill_byte=seed)
              for size in (4096, 24576) for seed in FILL_BYTES]
    plans += [make_case('call', runtime_bytes=size) for size in (0, 4096, 24576, 24577)]
    plans += [make_case('call', count=4, runtime_bytes=size) for size in (0, 24576)]
    def key(row):
        p = row['parameters']
        return (p['transaction_kind'], p['shape'], p['returned_runtime_bytes'], p['count_per_block'],
                p['block_count'], p['transaction_gas_limit'], p['fill_byte'])
    by_key = {key(row):row for row in plans}
    assert len(plans) == len(by_key) == 38
    for row in plans:
        kind, shape, size, count, blocks, gas, fill = key(row)
        if shape == 'filled_runtime':
            target = (kind, 'return_runtime', size, 1, blocks, gas, None) if fill == 0 else (kind, shape, size, 1, blocks, gas, 0)
        elif kind == 'call':
            target = (kind, shape, 0, count if size else 1, blocks, gas, None)
        elif gas != 8_000_000:
            target = (kind, shape, size, 1, blocks, gas, None)
        elif size == 24576 and count > 1:
            target = (kind, shape, 0, count, blocks, gas, None)
            row['repeat_reference_sample_id'] = by_key[(kind, shape, size, 1, blocks, gas, None)]['sample_id']
        else:
            target = (kind, shape, 0, 1, blocks, gas, None)
        row['baseline_sample_id'] = by_key[target]['sample_id']
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir()
    rows = []
    for row in plans:
        request = encoded(row.pop('request')); relative = f"requests/{row['sample_id']}.json"
        write(output/relative, request); rows.append(dict(row, request_path=relative, request_sha256=digest(request)))
    write(output/'catalog.json', encoded({'schema_version':1,
        'purpose':'Model-development followup separating deployed bytes, repeated same code, runtime content and ordinary return data',
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'base_generator_sha256':digest(Path(__file__).with_name('top_level_create.py').read_bytes()),
        'shared_generator_sha256':digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'previously_observed_reference_replay_count':3, 'new_model_development_case_count':35,
        'prospective_holdout':False, 'cases':rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    print(json.dumps({'prepared':prepare(parser.parse_args().output_dir)}))
