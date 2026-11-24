#!/usr/bin/env python3
"""Generate a finite compiler-frozen MODEXP input-value boundary catalog."""
import argparse
import json
from pathlib import Path

from spawn_memory import compile_source, digest, encoded, write

SOURCE = Path(__file__).with_name('ModexpValues.sol')
WIDTHS = (128, 256, 512)
MODULUS_CASES = ('odd', 'power-two', 'even-crt')
EXPONENTS = (0, 1, 3)
MAX_COUNT = 65536
SOURCE_DERIVED_GAS = {128: 512, 256: 2048, 512: 8192}


def modulus_for(width, modulus_case):
    if (type(width) is not int or width not in WIDTHS or
            type(modulus_case) is not str or modulus_case not in MODULUS_CASES):
        raise ValueError('unknown bounded MODEXP width/modulus case')
    bits = 8*width
    if modulus_case == 'odd':
        return (1 << bits)-189
    if modulus_case == 'power-two':
        return 1 << (bits-1)
    return 3 << (bits-2)


def expected_output(width, modulus_case, exponent):
    modulus = modulus_for(width, modulus_case)
    if type(exponent) is not int or exponent not in EXPONENTS:
        raise ValueError('MODEXP exponent must be 0, 1 or 3')
    return pow((1 << (8*width))-1, exponent, modulus).to_bytes(width, 'big')


def make_case(runtime, width, modulus_case, exponent, count):
    if type(count) is not int or not 0 <= count <= MAX_COUNT:
        raise ValueError('MODEXP count must be an integer in 0..65536')
    result = expected_output(width, modulus_case, exponent)
    code = bytes.fromhex(runtime)
    if not 1 <= len(code) <= 24576:
        raise ValueError('runtime must contain 1..24576 bytes')
    raw_input = (b''.join(value.to_bytes(32, 'big') for value in (width, 1, width)) +
                 b'\xff'*width + bytes([exponent]) +
                 modulus_for(width, modulus_case).to_bytes(width, 'big'))
    calldata = (count.to_bytes(32, 'big') + width.to_bytes(32, 'big') + raw_input + result)
    counts = {'opcode:0xfa': count, 'opcode:0xf3': 1, 'opcode:0xfd': 0,
              'opcode:0x00': 0, 'opcode:0xf1': 0, 'opcode:0xf2': 0,
              'opcode:0xf4': 0, 'opcode:0xf0': 0, 'opcode:0xf5': 0, 'opcode:0x3e': 0}
    request = {'schema_version': 1, 'expected_user_success': True, 'program': {
        'code_hex': '0x'+code.hex(), 'input_hex': '0x'+calldata.hex(),
        'expected_route_counts': counts, 'block_count': 1}}
    group = f'modexp-values-l{width}-{modulus_case}-e{exponent}-b1'
    return {'sample_id': f'{group}-n{count}', 'family': 'modexp-value-boundaries',
            'role': 'control' if count == 0 or exponent == 0 else 'candidate',
            'pair_group': group, 'value_boundary_probe_only': True,
            'parameters': {'precompile_address': '0x05', 'operation': 'modexp',
                           'width': width, 'base_hex': '0x'+('ff'*width),
                           'exponent': exponent, 'exponent_bytes': 1,
                           'modulus_case': modulus_case,
                           'modulus_hex': '0x'+modulus_for(width, modulus_case).to_bytes(width, 'big').hex(),
                           'expected_output_hex': '0x'+result.hex(), 'count': count,
                           'outer_calldata_bytes': len(calldata), 'block_count': 1,
                           'source_derived_native_gas_per_call': SOURCE_DERIVED_GAS[width]},
            'expected_precompile_calls': count,
            'memory_semantics': 'raw input copied once; separate preallocated output; '
                                'every output word cleared before every STATICCALL',
            'completion_gate': 'success, exact width returndata, every output word compared '
                               'with independent Python pow; exact STATICCALL and root RETURN counts',
            'scope': 'finite unsaturated input-value probe; source-derived per-call gas is not '
                     'measured whole-transaction gas or a whole-node quote',
            'request': request}


def prepare(output, solc):
    runtime, compilation = compile_source(solc, SOURCE, 'ModexpValues')
    selections = [(width, shape, exponent, 1) for width in WIDTHS
                  for shape in MODULUS_CASES for exponent in EXPONENTS]
    selections += [(width, 'even-crt', 3, 0) for width in WIDTHS]
    selections += [(512, 'even-crt', 3, count) for count in (4, 16)]
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
        'purpose': 'finite MODEXP input-value boundaries with matched repetition controls',
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
