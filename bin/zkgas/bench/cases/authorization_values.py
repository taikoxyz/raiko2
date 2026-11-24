#!/usr/bin/env python3
"""Generate a finite EIP-7702 authorization-recovery/nonce-pattern catalog."""
import argparse
import json
from pathlib import Path

from spawn_memory import digest, encoded, write

RUNTIME = bytes.fromhex('60006000f3')
PATTERNS = ('sequential', 'repeated', 'distinct')
MAX_COUNT = 1024


def make_case(pattern=None, count=None):
    if pattern is None:
        if count is not None:
            raise ValueError('EIP-1559 reference has no authorization count')
    elif (type(pattern) is not str or pattern not in PATTERNS or type(count) is not int or
          not 1 <= count <= MAX_COUNT):
        raise ValueError('authorization pattern/count must be bounded and nonempty')
    reference = pattern is None
    program = {'code_hex': '0x'+RUNTIME.hex(), 'input_hex': '0x', 'block_count': 1,
               'expected_route_counts': {'opcode:0xf3': 1, 'opcode:0xfd': 0, 'opcode:0x00': 0,
                                         'opcode:0xfa': 0, 'opcode:0xf1': 0, 'opcode:0xf2': 0,
                                         'opcode:0xf4': 0, 'opcode:0xf0': 0, 'opcode:0xf5': 0}}
    if not reference:
        program['authorizations'] = {'pattern': pattern, 'count': count}
    sid = 'authorization-values-eip1559-reference-b1' if reference else f'authorization-values-{pattern}-k{count}-b1'
    return {'sample_id': sid, 'family': 'authorization-list-validation',
            'role': 'control' if reference or count == 1 else 'candidate',
            'pair_group': 'authorization-values-shared-single-entry-baseline-b1',
            'value_boundary_probe_only': True,
            'parameters': {'transaction_type': 'eip1559' if reference else 'eip7702',
                           'pattern': pattern, 'count': count,
                           'expected_applied_delegations': 0 if reference else (1 if pattern == 'repeated' else count),
                           'expected_authority_count': 0 if reference else (count if pattern == 'distinct' else 1),
                           'block_count': 1, 'outer_calldata_bytes': 0},
            'completion_gate': 'successful canonical inclusion; exact root RETURN; native report '
                               'gates actual authority nonce and delegation code hash',
            'scope': 'finite offline authorization-list validation probe; one count-one baseline '
                     'is shared across pattern comparisons; no empty type-4 control',
            'request': {'schema_version': 1, 'expected_user_success': True, 'program': program}}


def prepare(output):
    selections = [(None, None), ('sequential', 1)]
    selections += [(pattern, count) for pattern in ('sequential', 'repeated') for count in (4, 16, 64, 256)]
    selections += [('distinct', count) for count in (4, 16)]
    plans = [make_case(*selection) for selection in selections]
    output.mkdir(parents=True, exist_ok=False)
    (output/'requests').mkdir()
    write(output/'runtime.hex', (RUNTIME.hex()+'\n').encode())
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output/relative, request)
        rows.append(dict(plan, request_path=relative, request_sha256=digest(request)))
    write(output/'catalog.json', encoded({'schema_version': 1,
        'purpose': 'finite EIP-7702 recovery/nonce patterns with shared single-entry baseline and EIP-1559 reference',
        'value_boundary_probe_only': True,
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'shared_generator_sha256': digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'runtime_sha256': digest(RUNTIME), 'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir)}))
