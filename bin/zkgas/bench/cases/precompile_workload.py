#!/usr/bin/env python3
"""Generate a small, fixed-input BN254 ECMUL proving-workload capacity probe."""
import argparse
import json
from pathlib import Path

from spawn_memory import compile_source, digest, encoded, write

SOURCE = Path(__file__).with_name('PrecompileWorkload.sol')
FIELD_MODULUS = 21888242871839275222246405745257275088696311157297823662689037894645226208583
GROUP_ORDER = 21888242871839275222246405745257275088548364400416034343698204186575808495617
COUNTS = (0, 1, 4, 16, 32)
MAX_COUNT = 4096
SCALARS = {'r-minus-two': GROUP_ORDER-2, 'dense254': (3 << 252)-1, 'dense253': (1 << 253)-1}


def _point_add(left, right):
    if left is None:
        return right
    if right is None:
        return left
    x, y = left
    u, v = right
    if x == u and (y+v) % FIELD_MODULUS == 0:
        return None
    slope = ((3*x*x * pow(2*y, -1, FIELD_MODULUS)) if left == right else
             ((v-y) * pow(u-x, -1, FIELD_MODULUS))) % FIELD_MODULUS
    result_x = (slope*slope-x-u) % FIELD_MODULUS
    return result_x, (slope*(x-result_x)-y) % FIELD_MODULUS


def expected_output(scalar_case='r-minus-two'):
    """Compute scalar*G through affine double-and-add, independent of the precompile."""
    if type(scalar_case) is not str or scalar_case not in SCALARS:
        raise ValueError('unknown bounded precompile scalar case')
    scalar, point, result = SCALARS[scalar_case], (1, 2), None
    while scalar:
        if scalar & 1:
            result = _point_add(result, point)
        point = _point_add(point, point)
        scalar >>= 1
    if result is None:
        raise ValueError('capacity probe requires a non-infinity expected result')
    return result


def make_case(runtime, count, scalar_case='r-minus-two'):
    if type(count) is not int or not 0 <= count <= MAX_COUNT:
        raise ValueError('precompile workload count must be an integer in 0..4096')
    code = bytes.fromhex(runtime)
    if not 1 <= len(code) <= 24576:
        raise ValueError('runtime must contain 1..24576 bytes')
    point = expected_output(scalar_case)
    scalar = SCALARS[scalar_case]
    expected = {'opcode:0xfa': count, 'opcode:0xf3': 1, 'opcode:0xfd': 0,
                'opcode:0x00': 0, 'opcode:0x3e': 0, 'opcode:0xf1': 0,
                'opcode:0xf2': 0, 'opcode:0xf4': 0}
    request = {'schema_version': 1, 'expected_user_success': True, 'program': {
        'code_hex': '0x' + code.hex(),
        'input_hex': '0x' + b''.join(value.to_bytes(32, 'big')
                                   for value in (count, scalar, *point)).hex(),
        'expected_route_counts': expected, 'block_count': 1}}
    group = f'precompile-workload-bn254-ecmul-{scalar_case}-b1'
    return {'sample_id': f'{group}-n{count}', 'family': 'precompile-workload-capacity-probe',
            'role': 'control' if count == 0 else 'candidate', 'pair_group': group,
            'capacity_probe_only': True,
            'parameters': {'precompile_address': '0x07', 'operation': 'bn254-ecmul',
                           'input_point': ['0x' + f'{v:064x}' for v in (1, 2)],
                           'scalar_case': scalar_case, 'scalar_hex': '0x' + f'{scalar:064x}',
                           'scalar_bit_length': scalar.bit_length(), 'scalar_popcount': scalar.bit_count(),
                           'expected_output_hex': '0x' + ''.join(f'{v:064x}' for v in point),
                           'count': count, 'outer_calldata_bytes': 128, 'block_count': 1},
            'expected_precompile_calls': count,
            'memory_semantics': 'fixed distinct 96-byte input and 64-byte output areas; '
                                'output cleared before every STATICCALL',
            'completion_gate': 'success, exact 64-byte returndata, both independently derived coordinates; '
                               'native STATICCALL count and one root RETURN',
            'scope': 'initial proving-workload lower-bound capacity probe; not gas-saturated',
            'request': request}


def prepare(output, solc):
    runtime, compilation = compile_source(solc, SOURCE, 'PrecompileWorkload')
    # Qualify controls and all single-call scalar axes before increasing repetition.
    plans = [make_case(runtime, count, scalar_case) for count in COUNTS for scalar_case in SCALARS
             if scalar_case != 'dense253' or count == 1]
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
        'purpose': 'small fixed-input BN254 ECMUL proving-workload capacity probe',
        'capacity_probe_only': True,
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
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, args.solc)}))
