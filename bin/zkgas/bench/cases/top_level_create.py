#!/usr/bin/env python3
"""Generate 44 finite top-level-creation probes using the existing fixture constructor."""
import argparse
import json
from pathlib import Path

from spawn_memory import digest, encoded, write

MAX_COUNT = 2048
MAX_INITCODE = 49152
PREFIXES = {'stop_zero': b'\x00', 'stop_jumpdest': b'\x00',
            'stop_push32': b'\x00', 'return_zero': bytes.fromhex('60006000f3'),
            'revert_zero': bytes.fromhex('60006000fd'), 'invalid_zero': b'\xfe'}


def integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f'{name} must be an integer in {low}..{high}')
    return value


def make_case(kind, shape, count=1, block_count=1, *, length=None,
              runtime_bytes=None, gas_limit=100_000):
    if type(shape) is not str:
        raise ValueError('shape must be a string')
    if type(kind) is not str or kind not in ('call', 'create'):
        raise ValueError('transaction kind must be call or create')
    integer(count, 1, MAX_COUNT, 'count per block')
    integer(block_count, 1, 32, 'block count')
    integer(gas_limit, 21000, 15_000_000, 'transaction gas limit')
    if kind == 'call' and shape != 'stop':
        raise ValueError('call controls use one-byte STOP runtime')
    success, runtime, terminal = True, b'', 'stop'
    if shape in ('empty', 'stop'):
        if length is not None or runtime_bytes is not None:
            raise ValueError('empty/stop shapes have fixed code and no runtime length argument')
        code = b'' if shape == 'empty' else b'\x00'
    elif shape in PREFIXES:
        if runtime_bytes is not None:
            raise ValueError('tail shapes do not accept a deployed runtime length')
        prefix = PREFIXES[shape]
        integer(length, len(prefix), MAX_INITCODE, 'initcode length')
        chunk = (b'\x5b' if shape == 'stop_jumpdest' else
                 b'\x7f'+b'\x5b'*32 if shape == 'stop_push32' else b'\x00')
        remaining = length-len(prefix)
        code = prefix+(chunk*((remaining+len(chunk)-1)//len(chunk)))[:remaining]
        terminal = ('return' if shape == 'return_zero' else
                    'revert' if shape == 'revert_zero' else
                    'invalid' if shape == 'invalid_zero' else 'stop')
        success = terminal not in ('revert', 'invalid')
    elif shape == 'return_runtime':
        if length is not None:
            raise ValueError('runtime-return shape has fixed six-byte initcode')
        integer(runtime_bytes, 0, 65535, 'returned runtime bytes')
        code = b'\x61'+runtime_bytes.to_bytes(2, 'big')+bytes.fromhex('6000f3')
        terminal, success = 'return', runtime_bytes <= 24576
        runtime = bytes(runtime_bytes) if success else None
    else:
        raise ValueError('unknown top-level creation shape')
    total = count*block_count
    routes = {f'opcode:0x{opcode:02x}': 0 for opcode in
              (0x00, 0x5b, 0x7f, 0xf0, 0xf5, 0xf1, 0xf2, 0xf4, 0xfa, 0xf3, 0xfd, 0xfe)}
    if terminal in ('stop', 'return', 'revert'):
        routes['opcode:0x'+{'stop': '00', 'return': 'f3', 'revert': 'fd'}[terminal]] = total
    if terminal in ('return', 'revert'):
        routes['opcode:0x60'] = total if shape == 'return_runtime' else 2*total
    if shape == 'return_runtime': routes['opcode:0x61'] = total
    descriptor = dict(kind=kind, count_per_block=count, gas_limit=gas_limit)
    if kind == 'create' and success:
        descriptor['expected_runtime_hex'] = '0x'+runtime.hex()
    program = dict(code_hex='0x'+code.hex(), input_hex='0x', block_count=block_count,
                   expected_route_counts=routes, transactions=descriptor)
    suffix = (f'-l{length}' if length is not None else '')
    suffix += (f'-r{runtime_bytes}' if runtime_bytes is not None else '')
    sid = f'top-create-{kind}-{shape}{suffix}-n{count}-b{block_count}-g{gas_limit}'
    return {'sample_id': sid, 'family': 'top-level-creation',
            'role': 'control' if count == 1 and (length is None or length <= 5) and
                    (runtime_bytes is None or runtime_bytes == 0) else 'candidate',
            'baseline_sample_id': sid,
            'parameters': {'transaction_kind': kind, 'shape': shape,
                'count_per_block': count, 'block_count': block_count,
                'transaction_gas_limit': gas_limit, 'code_bytes': len(code),
                'code_sha256': digest(code), 'requested_tail_length': length,
                'returned_runtime_bytes': runtime_bytes, 'expected_user_success': success,
                'expected_created_account_present': kind == 'create' and success,
                'expected_exceptional_instructions': total if terminal == 'invalid' else 0,
                'outer_calldata_bytes': 0 if kind == 'call' else len(code),
                'code_location': 'prestate runtime' if kind == 'call' else 'signed creation input',
                'preload_created_accounts_or_code': False},
            'completion_gate': 'all canonical signed users included in exact order; receipt status, '
                'all-user opcode counts and actual created account/runtime match',
            'scope': 'finite same-block-count slopes and initcode boundary controls; '
                '15M maximum per-tx and single-blob constructor bounds remain; no capacity claim',
            'request': dict(schema_version=1, expected_user_success=success, program=program)}


def baseline(row):
    p = row['parameters']; kind, shape = p['transaction_kind'], p['shape']
    length, runtime = p['requested_tail_length'], p['returned_runtime_bytes']
    if shape in ('stop_jumpdest', 'stop_push32'):
        shape = 'stop_zero'
    elif shape in PREFIXES:
        length = len(PREFIXES[shape])
    if shape == 'return_runtime': runtime = 0
    return make_case(kind, shape, 1, p['block_count'], length=length,
                     runtime_bytes=runtime, gas_limit=p['transaction_gas_limit'])['sample_id']


def prepare(output):
    plans = []
    for kind, shape in [('call', 'stop'), ('create', 'empty')]:
        plans += [make_case(kind, shape, n) for n in (1, 16, 128, 512)]
        plans += [make_case(kind, shape, n, 3) for n in (1, 16)]
    plans += [make_case('create', 'stop', n) for n in (1, 16, 128, 512)]
    plans += [make_case('create', 'return_zero', n, length=5) for n in (1, 16, 128, 512)]
    plans += [make_case('create', 'stop_zero', length=size, gas_limit=3_000_000)
              for size in (1, 32, 4096, 16384, 49152)]
    plans += [make_case('create', shape, length=size, gas_limit=3_000_000)
              for shape in ('stop_jumpdest', 'stop_push32') for size in (4096, 49152)]
    plans += [make_case('create', 'return_zero', length=size, gas_limit=3_000_000)
              for size in (5, 4096, 49152)]
    plans += [make_case('create', 'revert_zero', length=size, gas_limit=3_000_000)
              for size in (5, 4096, 49152)]
    plans += [make_case('create', 'invalid_zero', length=size, gas_limit=3_000_000)
              for size in (1, 49152)]
    plans += [make_case('create', 'return_runtime', runtime_bytes=size, gas_limit=8_000_000)
              for size in (0, 32, 256, 24576, 24577)]
    plans += [make_case('create', 'return_runtime', block_count=3,
                        runtime_bytes=size, gas_limit=8_000_000) for size in (0, 32)]
    assert len(plans) == 44
    ids = {row['sample_id'] for row in plans}
    assert len(ids) == 44
    for row in plans:
        row['baseline_sample_id'] = baseline(row)
        assert row['baseline_sample_id'] in ids
    output.mkdir(parents=True, exist_ok=False)
    (output/'requests').mkdir()
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output/relative, request)
        rows.append(dict(plan, request_path=relative, request_sha256=digest(request)))
    write(output/'catalog.json', encoded({'schema_version': 1,
        'purpose': 'top-level creation count, initcode analysis and deployment boundaries',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'shared_generator_sha256': digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    print(json.dumps({'prepared': prepare(parser.parse_args().output_dir)}))
