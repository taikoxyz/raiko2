#!/usr/bin/env python3
"""Generate 24 unreachable legacy-bytecode analysis probes with own-N0 controls."""
import argparse
import json
from pathlib import Path
import re

import multiaccount_code as original
from spawn_memory import digest, encoded, write
from top_level_create import integer

MODEL = original.MODEL
TARGET_COUNT = 512
CODE_LENGTHS = (4096, 24576)
PROFILES = ('stop', 'jumpdest', 'mixed', 'push32', 'jumpdest-copy', 'jumpdest-pad32')
LEVELS = ('zero', 'full')


def child_code(length, profile, index):
    """Keep execution identical; align PUSH4 salt and terminal after the tail."""
    integer(length, 4096, 24576, 'child runtime bytes')
    if length not in CODE_LENGTHS:
        raise ValueError('unsupported child runtime bytes')
    if type(profile) is not str or profile not in PROFILES:
        raise ValueError('unknown tail profile')
    integer(index, 0, TARGET_COUNT - 1, 'child index')
    size = length - len(original.CHILD_PREFIX) - 6
    if profile == 'stop':
        tail = bytes(size)
    elif profile == 'mixed':
        tail = (b'\x5b\x01' * ((size + 1) // 2))[:size]
    elif profile == 'push32':
        groups, remainder = divmod(size, 33)
        tail = (b'\x7f' + b'\x5b' * 32) * groups + bytes(remainder)
    else:
        tail = b'\x5b' * size
    terminal = {'jumpdest-copy': b'\x01', 'jumpdest-pad32': b'\x7f'}.get(profile, b'\x00')
    return original.CHILD_PREFIX + tail + b'\x63' + (index + 1).to_bytes(4, 'big') + terminal


def sample_id(length, profile, level):
    return f'code-analysis-tail-a512-l{length}-{profile}-{level}'


def make_case(length=4096, profile='stop', level='full'):
    # Validate before allocating the512-target request; bools are not integers.
    child_code(length, profile, 0)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('unknown workload level')
    row = original.make_case(TARGET_COUNT, length, 'unique', level, child_call_kind='delegatecall')
    for index, contract in enumerate(row['request']['program']['prestate_contracts']):
        contract['code_hex'] = '0x' + child_code(length, profile, index).hex()
    row.update(sample_id=sample_id(length, profile, level), family='unreachable-code-analysis',
               baseline_sample_id=sample_id(length, profile, 'zero'),
               scope='Finite whole-proposal unreachable legacy-bytecode content observations under unchanged pricing. '
                     'Different code changes authenticated state; contrasts are not isolated causal coefficients. '
                     'Deployment and setup are outside the measured block. No universal bytecode bound or proof throughput claim.')
    row['parameters'].update(tail_profile=profile, unreachable_salt_bytes=4,
                             unreachable_suffix_bytes=6, unreachable_salt_opcode='PUSH4',
                             unreachable_terminal_opcode={'jumpdest-copy': 'ADD', 'jumpdest-pad32': 'PUSH32'}.get(profile, 'STOP'))
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be a lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'requests').mkdir()
    rows = []
    for length in CODE_LENGTHS:
        for profile in PROFILES:
            for level in LEVELS:
                row = make_case(length, profile, level)
                raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                write(output / path, raw)
                rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len(rows) == len({r['sample_id'] for r in rows}) == 24
    sources = ('multiaccount_code.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
    write(output / 'catalog.json', encoded({
        'schema_version': 1, 'frozen_model_id': model_id, 'child_call_kind': 'delegatecall',
        'target_count': TARGET_COUNT, 'code_lengths': CODE_LENGTHS, 'profiles': PROFILES, 'levels': LEVELS,
        'purpose': 'Unreachable legacy-bytecode analysis tails,512 unique targets,12 exact own-N0 pairs',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in sources},
        'required_comparison_quantities': ['complete proposal zkGas estimate',
            'node-observable estimate for each block and their literal sum', 'actual same-proposal SP1 PGU'],
        'interpretation': 'Keep literal actual/node/proposal estimates, all controls and failures. '
            'Strictly above2B positive actual-minus-node PGU within30M charged block gas is high priority. '
            'No price fit, limit change, universal bytecode bound or generated-proof/GPU throughput claim.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
