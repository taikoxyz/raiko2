#!/usr/bin/env python3
"""Emit finite, compiler-free guarded precompile ProgramRequests; never execute them."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

VECTOR_FILE = Path(__file__).with_name('precompile_boundary_vectors.json')
BASE_VECTOR_IDS = ('ecrecover-valid', 'ripemd160-32', 'bn254-add-double', 'blake2f-12',
    'kzg-valid', 'bls-g1-add-double', 'bls-g2-add-double', 'bls-g2-msm-two',
    'bls-pairing-k1', 'bls-map-fp', 'bls-map-fp2', 'p256-valid')
ADDRESSES = (1, 3, 6, 9, 10, 11, 13, 14, 15, 16, 17, 256)
MAX_REPETITIONS = 4096
TRANSACTION_GAS_CAP = 15_000_000
SENTINEL = 0xa55a


def _hex(value, name):
    if type(value) is not str or not re.fullmatch(r'0x(?:[0-9a-f]{2})+', value):
        raise ValueError(f'{name} must be nonempty lowercase byte hex')
    return bytes.fromhex(value[2:])


def vectors():
    """Read portable retained vectors; the producer has no crypto package dependency."""
    data = json.loads(VECTOR_FILE.read_bytes())
    if data.get('schema_version') != 1 or type(data.get('vectors')) is not list:
        raise ValueError('unsupported vector bank')
    result = {}
    for row in data['vectors']:
        key = row['vector_id']
        if type(key) is not str or not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', key) or key in result:
            raise ValueError('vector IDs must be unique portable names')
        address = _hex(row['address'], 'address')
        if len(address) != 20 or int.from_bytes(address, 'big') not in ADDRESSES:
            raise ValueError('unsupported full-width precompile address')
        payload = _hex(row['input_hex'], 'input')
        output = _hex(row['expected_output_hex'], 'output')
        if not 1 <= len(payload) <= 8192 or not 32 <= len(output) <= 256 or len(output) % 32:
            raise ValueError('vector exceeds input/output bounds')
        original.integer(row['native_gas'], 1, 1_000_000, 'native gas')
        if not row.get('provenance', {}).get('sources'):
            raise ValueError('vector must retain provenance')
        result[key] = row
    return result


def _vector(vector_id):
    bank = vectors()
    if type(vector_id) is not str or vector_id not in bank:
        raise ValueError('unknown finite precompile vector')
    return bank[vector_id]


def _runtime(vector, repetitions):
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    payload = _hex(vector['input_hex'], 'input')
    output = _hex(vector['expected_output_hex'], 'output')
    # The gap and terminal word protect both sides of the separate output range.
    gap = 32*((len(payload)+31)//32)
    out = gap+32
    end = out+len(output)
    asm = _Factory()

    def require(times=1):
        asm.op(0x15, times); asm.target('failure', times); asm.op(0x57, times)

    def check(value, times=1, width=3):
        asm.push(value, width, times); asm.op(0x14, times); require(times)

    def store(offset, value, times=1):
        asm.push(value, times=times); asm.push(offset, times=times); asm.op(0x52, times)

    asm.op(0x36); check(len(payload))
    asm.push(len(payload)); asm.push(0); asm.push(0); asm.op(0x37)
    store(gap, SENTINEL); store(end, SENTINEL)  # Preallocates output even for N=0.
    asm.push(0)
    visits = repetitions+1
    asm.label('loop', visits); asm.op(0x80, visits); asm.push(repetitions, times=visits)
    asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for index in range(0, len(output), 32):
        # Every expected bit starts inverted, including the false-pairing zero word.
        poison = int.from_bytes(output[index:index+32], 'big') ^ ((1 << 256)-1)
        asm.push(poison, 32, repetitions); asm.push(out+index, times=repetitions); asm.op(0x52, repetitions)
    # STATICCALL stack: gas, full 20-byte address, input offset/length, output offset/length.
    for value in (len(output), out, len(payload), 0):
        asm.push(value, times=repetitions)
    asm.push(int(vector['address'], 16), 20, repetitions)
    asm.push(vector['native_gas'], times=repetitions); asm.op(0xfa, repetitions)
    check(1, repetitions); asm.op(0x3d, repetitions); check(len(output), repetitions)
    for index in range(0, len(output), 32):
        asm.push(out+index, times=repetitions); asm.op(0x51, repetitions)
        check(int.from_bytes(output[index:index+32], 'big'), repetitions, 32)
    asm.push(1, times=repetitions); asm.op(0x01, repetitions); asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); asm.op(0x50)
    for offset in (gap, end):
        asm.push(offset); asm.op(0x51); check(SENTINEL)
    asm.op(0x00); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), dict(asm.counts), dict(input_memory_offset=0,
        output_memory_offset=out, protected_memory_offsets=[gap, end], peak_memory_bytes=end+32)


def gas_accounting(vector_id, repetitions):
    """Exact successful Osaka gas, including EIP7623; assumes active warm precompiles."""
    vector = _vector(vector_id)
    code, counts, layout = _runtime(vector, repetitions)
    payload = _hex(vector['input_hex'], 'input')
    fixed = {0x00: 0, 0x01: 3, 0x14: 3, 0x15: 3, 0x36: 2, 0x37: 3,
             0x3d: 2, 0x50: 2, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10,
             0x5b: 1, 0x80: 3, 0xfa: 0, 0xfe: 0}
    ordinary = sum(n*(3 if 0x60 <= int(key[-2:], 16) <= 0x7f else fixed[int(key[-2:], 16)])
                   for key, n in counts.items())
    input_words = (len(payload)+31)//32
    memory_words = layout['peak_memory_bytes']//32
    memory = 3*memory_words+memory_words*memory_words//512
    copy = 3*input_words
    zero = payload.count(0); nonzero = len(payload)-zero
    intrinsic = 21000+4*zero+16*nonzero
    floor = 21000+10*(zero+4*nonzero)
    native = repetitions*vector['native_gas']; access = repetitions*100
    gross = intrinsic+ordinary+copy+memory+native+access
    reserve = max(1000, (vector['native_gas']+62)//63)
    return dict(intrinsic_gas=intrinsic, eip7623_floor_gas=floor,
        ordinary_opcode_gas=ordinary, calldata_copy_word_gas=copy,
        memory_expansion_gas=memory, memory_words=memory_words,
        native_gas_per_call=vector['native_gas'], warm_access_gas_per_call=100,
        precompile_calls=repetitions, precompile_native_gas=native, call_access_gas=access,
        gross_gas=gross, charged_gas=max(gross, floor), eip7623_floor_active=gross < floor,
        refund_counter=0, refund_applied=0, transaction_envelope_reserve=reserve,
        transaction_gas_limit=max(gross, floor)+reserve, runtime_bytes=len(code))


def make_case(vector_id, repetitions):
    vector = _vector(vector_id)
    code, counts, layout = _runtime(vector, repetitions)
    gas = gas_accounting(vector_id, repetitions)
    if gas['transaction_gas_limit'] > TRANSACTION_GAS_CAP or len(code) > 24576:
        raise ValueError('case exceeds finite transaction gas/runtime budget')
    row = original.make_case('call', 'stop', gas_limit=gas['transaction_gas_limit'])
    routes = {key: 0 for key in row['request']['program']['expected_route_counts']}
    routes.update(counts)
    row['request']['program'].update(code_hex='0x'+code.hex(), input_hex=vector['input_hex'], expected_route_counts=routes)
    prefix = f'precompile-boundary-{vector_id}'
    row.update(sample_id=f'{prefix}-n{repetitions}', family='precompile-boundaries',
        pair_group=prefix, baseline_sample_id=prefix+'-n0',
        role='control' if repetitions == 0 else 'candidate',
        completion_gate='successful parent; exact STATICCALL and successful-path opcode counts; each call checks status, returndata size and every output word; protected memory remains intact',
        scope='Finite current-program workload observations, not gas-saturated maxima or pricing changes')
    payload = _hex(vector['input_hex'], 'input'); output = _hex(vector['expected_output_hex'], 'output')
    row['parameters'] = dict(vector_id=vector_id, repetitions=repetitions,
        precompile_address=vector['address'], input_hex=vector['input_hex'], input_bytes=len(payload),
        expected_output_hex=vector['expected_output_hex'], output_bytes=len(output),
        input_sha256=digest(payload), expected_output_sha256=digest(output),
        expected_precompile_calls=repetitions, requested_gas_per_call=vector['native_gas'],
        source_derived_native_gas_per_call=vector['native_gas'], warm_access_gas_per_call=100,
        transaction_gas_limit=gas['transaction_gas_limit'], transaction_envelope_reserve=gas['transaction_envelope_reserve'],
        gas_accounting_per_user=gas, expected_user_success=True, expected_terminal='STOP',
        block_count=1, count_per_block=1, transaction_kind='call', outer_calldata_bytes=len(payload),
        execution_profile='evm-gas-only', runtime_sha256=digest(code), runtime_bytes=len(code),
        vector_description=vector['description'], provenance=vector['provenance'], **layout)
    return row


def prepare(output_dir, *, vector_ids=BASE_VECTOR_IDS, repetitions=(0, 1, 8)):
    if not isinstance(output_dir, Path):
        raise ValueError('output_dir must be a Path')
    if type(vector_ids) not in (tuple, list) or not vector_ids or any(type(v) is not str for v in vector_ids):
        raise ValueError('vector_ids must be a nonempty finite list/tuple')
    if len(set(vector_ids)) != len(vector_ids):
        raise ValueError('duplicate vector IDs')
    if type(repetitions) not in (tuple, list) or not repetitions:
        raise ValueError('repetitions must be a nonempty finite list/tuple')
    for n in repetitions:
        original.integer(n, 0, MAX_REPETITIONS, 'repetitions')
    if len(set(repetitions)) != len(repetitions) or 0 not in repetitions or len(vector_ids)*len(repetitions) > 48:
        raise ValueError('selection needs unique counts including zero and at most48 cases')
    plans = [make_case(vector_id, n) for vector_id in vector_ids for n in repetitions]
    output_dir.mkdir(parents=True, exist_ok=False); (output_dir/'requests').mkdir()
    rows = []
    for plan in plans:
        request = encoded(plan.pop('request')); path = f"requests/{plan['sample_id']}.json"
        write(output_dir/path, request)
        rows.append(plan | dict(request_path=path, request_sha256=digest(request)))
    write(output_dir/'catalog.json', encoded(dict(schema_version=1,
        purpose='Finite guarded remaining-precompile workload boundary probes',
        generator_sha256=digest(Path(__file__).read_bytes()), vector_bank_sha256=digest(VECTOR_FILE.read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes())
            for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--vectors', nargs='+', default=BASE_VECTOR_IDS)
    parser.add_argument('--repetitions', nargs='+', type=int, default=(0, 1, 8))
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, vector_ids=args.vectors, repetitions=args.repetitions)}))
