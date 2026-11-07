#!/usr/bin/env python3
"""Finite automatic returndata-copy triplets with preallocated caller memory."""
import argparse
import json
from pathlib import Path

from spawn_memory import KINDS, OPCODES, compile_source, digest, encoded, write

SOURCE = Path(__file__).with_name('ReturnCopy.sol')
MODES = ('zero', 'half', 'full')


def make_case(runtime, kind, length, count, mode, target='self', child_revert=False):
    if (type(kind) is not int or kind not in range(4) or type(length) is not int or
            not 0 <= length <= 1048576 or type(count) is not int or not 1 <= count <= 1024 or
            mode not in MODES or target not in ('self', 'identity') or type(child_revert) is not bool or
            (target == 'identity' and child_revert)):
        raise ValueError('invalid bounded automatic-return-copy parameters')
    code = bytes.fromhex(runtime)
    if not 1 <= len(code) <= 24576:
        raise ValueError('runtime must contain 1..24576 bytes')
    capacity = {'zero': 0, 'half': length // 2, 'full': length}[mode]
    expected = {f'opcode:0x{opcode:02x}': count if number == kind else 0
                for number, opcode in enumerate(OPCODES)}
    expected.update({'opcode:0x00': 0, 'opcode:0x3e': 0,
                     'opcode:0xf3': 1 + (count if target == 'self' and not child_revert else 0),
                     'opcode:0xfd': count if child_revert else 0})
    words = (0, count, length, capacity, int(target == 'identity'), kind, int(child_revert), 0)
    request = {'schema_version': 1, 'expected_user_success': True, 'program': {
        'code_hex': '0x' + code.hex(), 'input_hex': '0x' + b''.join(v.to_bytes(32, 'big') for v in words).hex(),
        'expected_route_counts': expected, 'block_count': 1}}
    terminal = 'revert' if child_revert else 'return'
    group = f'return-copy-{target}-{terminal}-{KINDS[kind]}-l{length}-n{count}-b1'
    return {'sample_id': group + '-' + mode, 'family': 'preallocated-automatic-return-copy',
            'role': 'control' if mode == 'zero' else 'candidate', 'pair_group': group,
            'parameters': {'call_kind': KINDS[kind], 'target_kind': target,
                           'child_revert': child_revert, 'return_length': length,
                           'count': count, 'output_capacity': capacity, 'copy_mode': mode,
                           'outer_calldata_bytes': 256, 'block_count': 1},
            'expected_spawn_count': count,
            'expected_identity_calls': count if target == 'identity' else 0,
            'memory_semantics': 'one parent allocation for distinct input/output areas; reused each iteration',
            'completion_gate': 'status, returndata length, alternating copied first byte, copy-boundary bytes, '
                               'uncopied sentinels, prefix/suffix guards, exact native terminal/opcode counts',
            'request': request}


def selections(profile='boundary'):
    if profile == 'repetition':
        return [(0, 65536, count, 'self', reverts)
                for count in (256, 512) for reverts in (False, True)]
    if profile != 'boundary':
        raise ValueError('unknown return-copy selection profile')
    # Deliberately finite rather than a Cartesian product. IDENTITY is a priced negative control.
    rows = [(kind, 65536, 16, 'self', reverts) for kind in range(4) for reverts in (False, True)]
    rows += [(0, length, 2, 'self', False) for length in (0, 1, 31, 32, 33)]
    rows += [(0, length, 2, 'self', True) for length in (1, 33)]
    rows += [(0, length, count, 'self', reverts)
             for length, count in ((65536, 64), (262144, 4), (262144, 16), (1048576, 1), (1048576, 2))
             for reverts in (False, True)]
    rows += [(0, 65536, count, 'identity', False) for count in (1, 32)]
    rows += [(0, length, 512, 'identity', False) for length in (0, 1, 32)]
    return rows


def prepare(output, solc, profile='boundary'):
    selected = selections(profile)
    runtime, compilation = compile_source(solc, SOURCE, 'ReturnCopy')
    plans = [make_case(runtime, kind, length, count, mode, target, reverts)
             for kind, length, count, target, reverts in selected for mode in MODES]
    output.mkdir(parents=True, exist_ok=False)
    (output / 'requests').mkdir()
    write(output / SOURCE.name, SOURCE.read_bytes())
    write(output / 'compiler.json', encoded(compilation))
    write(output / 'runtime.hex', (runtime + '\n').encode())
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output / relative, request)
        rows.append(dict(plan, request_path=relative, request_sha256=digest(request)))
    write(output / 'catalog.json', encoded({'schema_version': 1,
        'purpose': 'actual returned-data automatic copy isolated from parent memory growth',
        'selection_profile': profile,
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'shared_generator_sha256': digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'source_sha256': compilation['source_sha256'],
        'compiler_artifact_sha256': digest((output / 'compiler.json').read_bytes()),
        'runtime_sha256': digest(bytes.fromhex(runtime)), 'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--solc', required=True, type=Path)
    parser.add_argument('--profile', choices=('boundary', 'repetition'), default='boundary')
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, args.solc, args.profile)}))
