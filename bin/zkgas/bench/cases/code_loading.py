#!/usr/bin/env python3
"""Generate ten initial-runtime loading probes that execute only their leading STOP."""
import argparse
import json
from pathlib import Path

import top_level_create as original
from spawn_memory import digest, encoded, write

PATTERNS = ('zero', 'jumpdest', 'push32')


def sample_id(length, pattern):
    return f'code-loading-stop-l{length}-{pattern}-n1-b1-g100000'


def make_case(length=1, pattern='zero'):
    original.integer(length, 1, 24576, 'runtime byte length')
    if type(pattern) is not str or pattern not in PATTERNS:
        raise ValueError('unknown bounded runtime pattern')
    chunk = {'zero':b'\x00', 'jumpdest':b'\x5b', 'push32':b'\x7f'+b'\x5b'*32}[pattern]
    code = b'\x00'+(chunk*((length-1+len(chunk)-1)//len(chunk)))[:length-1]
    row = original.make_case('call', 'stop')
    prior = row['sample_id']; row['request']['program']['code_hex'] = '0x'+code.hex()
    row.update(sample_id=sample_id(length, pattern), family='initial-code-loading',
               role='control' if length == 1 else 'candidate',
               baseline_sample_id=sample_id(1, 'zero'),
               reference_role='previously_observed_reference_replay' if length == 1 else 'new_model_development_case',
               prior_observed_reference_sample_id=prior if length == 1 else None,
               model_development_probe_only=True,
               scope='Finite whole-proposal initial-runtime loading observations with a single executed STOP; input calldata is empty. Same-length content differences and length-versus-one-byte controls are separate diagnostics. No per-block actual, deployment, capacity or universal-bound claim.')
    row['parameters'].update(runtime_bytes=length, runtime_sha256=digest(code), pattern=pattern,
                             code_bytes=len(code), code_sha256=digest(code),
                             bytes_location='prestate_runtime', input_calldata_bytes=0,
                             expected_created_account_present=False)
    if pattern != 'zero': row['same_length_reference_sample_id'] = sample_id(length, 'zero')
    return row


def prepare(output):
    plans = [make_case()]+[make_case(length, pattern) for length in (32,4096,24576) for pattern in PATTERNS]
    assert len(plans) == len({p['sample_id'] for p in plans}) == 10
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir()
    rows = []
    for row in plans:
        request = encoded(row.pop('request')); relative = f"requests/{row['sample_id']}.json"
        write(output/relative, request); rows.append(dict(row, request_path=relative, request_sha256=digest(request)))
    write(output/'catalog.json', encoded({'schema_version':1,
        'purpose':'Initial runtime loading and code analysis, excluding executed suffix work and creation',
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'base_generator_sha256':digest(Path(__file__).with_name('top_level_create.py').read_bytes()),
        'shared_generator_sha256':digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'previously_observed_reference_replay_count':1, 'new_model_development_case_count':9,
        'cases':rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    print(json.dumps({'prepared':prepare(parser.parse_args().output_dir)}))
