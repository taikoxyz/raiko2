#!/usr/bin/env python3
"""Generate a finite, compiler-frozen CALL-output memory expansion catalog."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

SOURCE = Path(__file__).with_name('SpawnMemory.sol')
KINDS = ('delegatecall', 'call', 'callcode', 'staticcall')
OPCODES = (0xf4, 0xf1, 0xf2, 0xfa)


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data):
    with path.open('xb') as stream:
        stream.write(data)


def make_case(runtime, kind, length, count):
    if (type(kind) is not int or kind not in range(4) or
            type(length) is not int or not 0 <= length <= 1048576 or length % 32 or
            type(count) is not int or not 1 <= count <= 1024):
        raise ValueError('invalid bounded spawn-memory parameters')
    code = bytes.fromhex(runtime)
    if not 1 <= len(code) <= 24576:
        raise ValueError('runtime must contain 1..24576 bytes')
    expected = {'opcode:0xf3': count + 1, 'opcode:0x00': count,
                'opcode:0xfd': 0, 'opcode:0x3e': 0, 'opcode:0xf4': count}
    route = f'opcode:0x{OPCODES[kind]:02x}'
    expected[route] = expected.get(route, 0) + count
    request = {'schema_version': 1, 'expected_user_success': True, 'program': {
        'code_hex': '0x' + code.hex(),
        'input_hex': '0x' + b''.join(v.to_bytes(32, 'big') for v in (0, count, length, kind)).hex(),
        'expected_route_counts': expected, 'block_count': 1}}
    sid = f'spawn-memory-{KINDS[kind]}-l{length}-n{count}-b1'
    return {'sample_id': sid, 'family': 'spawn-output-memory',
            'role': 'control' if length == 0 else 'candidate',
            'pair_group': f'spawn-memory-{KINDS[kind]}-n{count}-b1',
            'parameters': {'call_kind': KINDS[kind], 'output_length': length,
                           'count': count, 'block_count': 1},
            'expected_spawn_count': 2 * count,
            'memory_semantics': 'fresh middle-frame memory; leaf STOP; no explicit output copy',
            'completion_gate': 'exact successful calls, STOP and RETURN counts; root returns count',
            'request': request}


def compile_source(solc, source=SOURCE, contract='SpawnMemory'):
    compiler = Path(solc).resolve(strict=True)
    version = subprocess.run([str(compiler), '--version'], capture_output=True, check=True,
                             timeout=30).stdout.decode()
    if 'Version: 0.8.21+' not in version:
        raise ValueError('fixture requires solc0.8.21')
    standard = {'language': 'Solidity', 'sources': {source.name: {'content': source.read_text()}},
                'settings': {'optimizer': {'enabled': True, 'runs': 200}, 'viaIR': True,
                             'evmVersion': 'shanghai', 'metadata': {'bytecodeHash': 'none'},
                             'outputSelection': {'*': {'*': ['evm.deployedBytecode.object']}}}}
    result = subprocess.run([str(compiler), '--standard-json'], input=encoded(standard),
                            capture_output=True, check=True, timeout=30)
    output = json.loads(result.stdout)
    errors = [item for item in output.get('errors', []) if item['severity'] == 'error']
    if errors:
        raise ValueError(errors)
    runtime = output['contracts'][source.name][contract]['evm']['deployedBytecode']['object']
    return runtime, {'compiler_sha256': digest(compiler.read_bytes()), 'version': version,
                     'source_sha256': digest(source.read_bytes()), 'standard_input': standard,
                     'standard_output': output, 'stderr': result.stderr.decode()}


def prepare(output, solc, *, source=SOURCE, contract='SpawnMemory', case_factory=make_case,
            generator=Path(__file__), purpose='CALL-output memory selector ownership'):
    runtime, compilation = compile_source(solc, source, contract)
    # Separate resource-limited candidates from matched zero-output controls. A
    # native rejection remains a result, never a reason to drop the candidate.
    selections = ((0, 1), (65536, 1), (0, 64), (65536, 64), (65536, 512),
                  (0, 512), (262144, 64), (0, 16), (524288, 16), (0, 4), (1048576, 4))
    cases = [case_factory(runtime, kind, length, count)
             for kind in range(4) for length, count in selections]
    output.mkdir(parents=True, exist_ok=False)
    (output / 'requests').mkdir()
    write(output / source.name, source.read_bytes())
    write(output / 'compiler.json', encoded(compilation))
    write(output / 'runtime.hex', (runtime + '\n').encode())
    rows = []
    for case in cases:
        request = encoded(case.pop('request'))
        relative = f"requests/{case['sample_id']}.json"
        write(output / relative, request)
        rows.append(dict(case, request_path=relative, request_sha256=digest(request)))
    catalog = {'schema_version': 1, 'purpose': purpose,
               'generator_sha256': digest(generator.read_bytes()),
               'shared_generator_sha256': digest(Path(__file__).read_bytes()),
               'source_sha256': compilation['source_sha256'],
               'compiler_artifact_sha256': digest((output / 'compiler.json').read_bytes()),
               'runtime_sha256': digest(bytes.fromhex(runtime)), 'cases': rows}
    write(output / 'catalog.json', encoded(catalog))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--solc', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, args.solc)}))
