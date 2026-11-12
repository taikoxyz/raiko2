#!/usr/bin/env python3
"""Emit finite BN254 ECMUL scalar/point fit and reserved validation probes."""
import argparse
import json
from pathlib import Path

from ecmul_scalar_mix import expected_point
from precompile_workload import FIELD_MODULUS, GROUP_ORDER
import precompile_boundaries as guarded
from spawn_memory import digest, encoded, write
import top_level_create as original

SCALARS = {'zero': 0, 'two': 2, 'dense': (3 << 252)-1}
FIT_POINTS = (1, 2)
FIT_REPETITIONS = (0, 1, 8)
RESERVED_POINT = 3
RESERVED_SCALARS = ('two', 'dense')
RESERVED_REPETITIONS = (0, 4)
SELECTIONS = ('fit', 'reserved', 'all')
SOURCE_NAMES = ('precompile_boundaries.py', 'ecmul_scalar_mix.py', 'precompile_workload.py',
                'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def make_case(point_multiple, scalar_case, repetitions):
    original.integer(point_multiple, 1, 3, 'point multiple')
    original.integer(repetitions, 0, guarded.MAX_REPETITIONS, 'repetitions')
    if type(scalar_case) is not str or scalar_case not in SCALARS:
        raise ValueError('scalar_case must be zero, two or dense')
    scalar = SCALARS[scalar_case]
    point = expected_point(point_multiple)
    reduced = scalar % GROUP_ORDER
    generator_scalar = (scalar*point_multiple) % GROUP_ORDER
    result = expected_point(generator_scalar)
    payload = b''.join(value.to_bytes(32, 'big') for value in (*point, scalar))
    output = b''.join(value.to_bytes(32, 'big') for value in result)
    prefix = f'ecmul-tariff-p{point_multiple}-{scalar_case}'
    vector = dict(vector_id=prefix, address='0x'+'00'*19+'07', input_hex='0x'+payload.hex(),
        expected_output_hex='0x'+output.hex(), native_gas=6000,
        description=f'BN254 point {point_multiple}*G with {scalar_case} scalar; independent affine result including infinity',
        provenance=dict(sources=[
            dict(url='https://eips.ethereum.org/EIPS/eip-196', description='BN254 G=(1,2), full-width field/scalar encoding and infinity=(0,0)'),
            dict(url='https://eips.ethereum.org/EIPS/eip-1108', description='ECMUL native gas6000'),
            dict(description='ecmul_scalar_mix.expected_point((scalar*point_multiple)%GROUP_ORDER), affine double-and-add using precompile_workload._point_add',
                 oracle_source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('ecmul_scalar_mix.py', 'precompile_workload.py')})]))
    row = guarded._case_for_vector(vector, repetitions)
    row.update(sample_id=f'{prefix}-n{repetitions}', family='ecmul-tariff',
        pair_group=prefix, baseline_sample_id=prefix+'-n0',
        data_role='fit' if point_multiple in FIT_POINTS else 'reserved',
        actuals_used_for_fitting=False,
        scope='Finite point/scalar workload observations; bitlength+popcount is source-derived work metadata, not a price. Reserved rows are excluded from candidate fitting and actual measurement until candidate sealing.')
    row['parameters'].update(point_multiple=point_multiple, scalar_case=scalar_case,
        input_point=list(point), raw_scalar_hex=f'0x{scalar:064x}', reduced_scalar_hex=f'0x{reduced:064x}',
        output_generator_scalar_hex=f'0x{generator_scalar:064x}',
        scalar_bit_length=reduced.bit_length(), scalar_popcount=reduced.bit_count(),
        scalar_work_units=reduced.bit_length()+reduced.bit_count(),
        work_units_definition='bit_length(scalar mod GROUP_ORDER)+popcount(scalar mod GROUP_ORDER); diagnostic, not actual PGU or tariff',
        expected_output_point=list(result), point_at_infinity=False,
        expected_output_at_infinity=result == (0, 0),
        native_gas_per_call=6000, expected_user_precompile_calls=repetitions)
    return row


def prepare(output_dir, selection='fit'):
    if not isinstance(output_dir, Path):
        raise ValueError('output_dir must be a Path')
    if type(selection) is not str or selection not in SELECTIONS:
        raise ValueError('selection must be fit, reserved or all')
    plans = []
    if selection in ('fit', 'all'):
        plans += [make_case(point, scalar_case, n) for point in FIT_POINTS for scalar_case in SCALARS for n in FIT_REPETITIONS]
    if selection in ('reserved', 'all'):
        plans += [make_case(RESERVED_POINT, scalar_case, n) for scalar_case in RESERVED_SCALARS for n in RESERVED_REPETITIONS]
    expected = {'fit': 18, 'reserved': 4, 'all': 22}[selection]
    if len(plans) != expected or len({row['sample_id'] for row in plans}) != expected:
        raise ValueError('finite ECMUL selection is inconsistent')
    output_dir.mkdir(parents=True, exist_ok=False); (output_dir/'requests').mkdir()
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request')); relative = f"requests/{plan['sample_id']}.json"
        write(output_dir/relative, request)
        rows.append(plan | dict(request_path=relative, request_sha256=digest(request)))
    write(output_dir/'catalog.json', encoded(dict(schema_version=1,
        purpose='Finite BN254 ECMUL point/scalar probes with separate fit and reserved validation selections',
        selection=selection, generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCE_NAMES},
        reserved_policy='Reserved point3 cases are selected before actuals and excluded from fitting; measure their actuals only after the candidate is sealed. Native qualification is separate from actual measurement.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--selection', choices=SELECTIONS, default='fit')
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, selection=args.selection)}))
