#!/usr/bin/env python3
"""Generate a finite compiler-frozen BLS G1 MSM scalar/batch boundary catalog."""
import argparse
import hashlib
import json
from pathlib import Path

from spawn_memory import compile_source, digest, encoded, write

SOURCE = Path(__file__).with_name('BlsMsmValues.sol')
FIELD_MODULUS = int('1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab', 16)
GROUP_ORDER = int('73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001', 16)
GENERATOR = (int('17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb', 16),
             int('08b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1', 16))
BATCH_SIZES = (1, 4, 16)
MAX_COUNT = 2048
SCALARS = {'one': 1, 'binary-dense': (7 << 252)-1,
           'alternating': int.from_bytes(b'\x55'*32, 'big'),
           'random': int.from_bytes(hashlib.sha256(b'bls-g1-msm-scalar-1').digest(), 'big') & ((1 << 255)-1)}
SOURCE_DERIVED_GAS = {1: 12000, 4: 38256, 16: 129984}


def point_add(left, right):
    """Affine addition on y²=x³+4; None encodes infinity."""
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


def point_mul(scalar, point=GENERATOR):
    """Unreduced binary double-and-add, also supporting independent r*G checks."""
    if type(scalar) is not int or scalar < 0:
        raise ValueError('curve scalar must be a nonnegative integer')
    result = None
    while scalar:
        if scalar & 1:
            result = point_add(result, point)
        point = point_add(point, point)
        scalar >>= 1
    return result


def encode_point(point):
    return b'\0'*128 if point is None else b''.join(value.to_bytes(64, 'big') for value in point)


def scalars_for(k, pattern):
    if (type(k) is not int or k not in BATCH_SIZES or type(pattern) is not str or
            pattern not in (*SCALARS, 'mixed-random')):
        raise ValueError('unknown bounded BLS batch/scalar pattern')
    if pattern == 'mixed-random':
        return [(int.from_bytes(hashlib.sha256(f'bls-g1-msm-scalar-{i}'.encode()).digest(), 'big') &
                 ((1 << 255)-1)) % GROUP_ORDER for i in range(1, k+1)]
    return [SCALARS[pattern]]*k


def expected_output(k, pattern):
    # P_i=i*G, so linearity yields one independent aggregate-scalar multiplication.
    # Tests separately sum s_i*P_i to check the complete MSM result and ordering.
    scalar = sum(i*value for i, value in enumerate(scalars_for(k, pattern), 1)) % GROUP_ORDER
    return encode_point(point_mul(scalar))


def make_case(runtime, k, pattern, count):
    if type(count) is not int or not 0 <= count <= MAX_COUNT:
        raise ValueError('BLS MSM count must be an integer in 0..2048')
    scalars = scalars_for(k, pattern)
    code = bytes.fromhex(runtime)
    if not 1 <= len(code) <= 24576:
        raise ValueError('runtime must contain 1..24576 bytes')
    raw_input = b''.join(encode_point(point_mul(i))+scalar.to_bytes(32, 'big')
                         for i, scalar in enumerate(scalars, 1))
    expected = expected_output(k, pattern)
    calldata = count.to_bytes(32, 'big')+k.to_bytes(32, 'big')+raw_input+expected
    counts = {'opcode:0xfa': count, 'opcode:0xf3': 1, 'opcode:0xfd': 0,
              'opcode:0x00': 0, 'opcode:0xf1': 0, 'opcode:0xf2': 0,
              'opcode:0xf4': 0, 'opcode:0xf0': 0, 'opcode:0xf5': 0, 'opcode:0x3e': 0}
    request = {'schema_version': 1, 'expected_user_success': True, 'program': {
        'code_hex': '0x'+code.hex(), 'input_hex': '0x'+calldata.hex(),
        'expected_route_counts': counts, 'block_count': 1}}
    group = f'bls-msm-values-k{k}-{pattern}-b1'
    return {'sample_id': f'{group}-n{count}', 'family': 'bls-g1-msm-value-boundaries',
            'role': 'control' if count == 0 else 'candidate', 'pair_group': group,
            'value_boundary_probe_only': True,
            'parameters': {'precompile_address': '0x0c', 'operation': 'bls-g1-msm',
                           'k': k, 'scalar_pattern': pattern,
                           'scalars_hex': ['0x'+f'{value:064x}' for value in scalars],
                           'point_semantics': 'distinct P_i=i*G for one-based input index i',
                           'expected_output_hex': '0x'+expected.hex(), 'count': count,
                           'outer_calldata_bytes': len(calldata), 'block_count': 1,
                           'source_derived_native_gas_per_call': SOURCE_DERIVED_GAS[k]},
            'expected_precompile_calls': count,
            'memory_semantics': 'input copied once; separate preallocated 128-byte output; '
                                'all four output words cleared before every STATICCALL',
            'completion_gate': 'success, exact 128-byte returndata, all four independently derived '
                               'output words; exact STATICCALL and root RETURN counts',
            'scope': 'finite unsaturated value/batching probe; source-derived precompile gas '
                     'is not measured whole-transaction gas or a whole-node quote',
            'request': request}


def prepare(output, solc):
    runtime, compilation = compile_source(solc, SOURCE, 'BlsMsmValues')
    selections = [(k, pattern, 1) for k in BATCH_SIZES for pattern in SCALARS]
    selections += [(k, 'random', 0) for k in BATCH_SIZES]
    selections += [(1, 'binary-dense', 4), (1, 'random', 4), (16, 'mixed-random', 1)]
    plans = [make_case(runtime, *selection) for selection in selections]
    output.mkdir(parents=True, exist_ok=False)
    (output/'requests').mkdir()
    write(output/SOURCE.name, SOURCE.read_bytes())
    write(output/'compiler.json', encoded(compilation))
    write(output/'runtime.hex', (runtime+'\n').encode())
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output/relative, request)
        rows.append(dict(plan, request_path=relative, request_sha256=digest(request)))
    write(output/'catalog.json', encoded({'schema_version': 1,
        'purpose': 'finite BLS G1 MSM scalar/batching boundaries with repetition controls',
        'value_boundary_probe_only': True,
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'shared_generator_sha256': digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'source_sha256': compilation['source_sha256'],
        'compiler_artifact_sha256': digest((output/'compiler.json').read_bytes()),
        'runtime_sha256': digest(bytes.fromhex(runtime)), 'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--solc', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, args.solc)}))
