#!/usr/bin/env python3
"""Emit caught ECMUL failures with exact forwarded-gas and successful parent guards."""
import argparse
import json
from pathlib import Path

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

ALLOWANCES = (6000, 200000)
REPETITIONS = (0, 1, 8)
MAX_ALLOWANCE = 1_000_000
MAX_REPETITIONS = 4096
TRANSACTION_GAS_CAP = 15_000_000
TAIL_GUARD_GAS = 1000
SENTINEL = 0xa55a
OUTPUT_POISONS = (int('a55a'*16, 16), int('5aa5'*16, 16))
PAYLOAD = b''.join(value.to_bytes(32, 'big') for value in (1, 1, 0))


def _runtime(allowance, repetitions):
    original.integer(allowance, 1, MAX_ALLOWANCE, 'forwarded gas allowance')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    asm = _Factory()

    def require(times=1):
        asm.op(0x15, times); asm.target('failure', times); asm.op(0x57, times)

    def check(value, times=1, width=3):
        asm.push(value, width, times); asm.op(0x14, times); require(times)

    def store(offset, value, times=1, width=3):
        asm.push(value, width, times); asm.push(offset, times=times); asm.op(0x52, times)

    def check_output(times=1):
        for index, value in enumerate(OUTPUT_POISONS):
            asm.push(128+32*index, times=times); asm.op(0x51, times); check(value, times, 32)

    asm.op(0x36); check(len(PAYLOAD))
    asm.push(len(PAYLOAD)); asm.push(0); asm.push(0); asm.op(0x37)
    store(96, SENTINEL); store(192, SENTINEL)
    for index, value in enumerate(OUTPUT_POISONS):
        store(128+32*index, value, width=32)
    asm.push(0)
    visits = repetitions+1
    asm.label('loop', visits); asm.op(0x80, visits); asm.push(repetitions, times=visits)
    asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for index, value in enumerate(OUTPUT_POISONS):
        store(128+32*index, value, repetitions, 32)
    for value in (64, 128, len(PAYLOAD), 0):
        asm.push(value, times=repetitions)
    asm.push(7, 20, repetitions); asm.push(allowance, times=repetitions); asm.op(0xfa, repetitions)
    check(0, repetitions)  # Child must fail while the parent keeps executing.
    asm.op(0x3d, repetitions); check(0, repetitions)
    check_output(repetitions)
    asm.push(1, times=repetitions); asm.op(0x01, repetitions); asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); check(repetitions)  # Consume and independently guard the final stack counter.
    check_output()
    for offset in (96, 192):
        asm.push(offset); asm.op(0x51); check(SENTINEL)
    asm.op(0x00); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), dict(asm.counts)


def gas_accounting(allowance, repetitions):
    """Successful parent gas; each failed precompile burns its full forwarded allowance."""
    code, counts = _runtime(allowance, repetitions)
    costs = {0x00: 0, 0x01: 3, 0x14: 3, 0x15: 3, 0x36: 2, 0x37: 3,
             0x3d: 2, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1,
             0x80: 3, 0xfa: 0, 0xfe: 0}
    ordinary = sum(n*(3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)])
                   for op, n in counts.items())
    zero = PAYLOAD.count(0); nonzero = len(PAYLOAD)-zero
    intrinsic = 21000+4*zero+16*nonzero
    floor = 21000+10*(zero+4*nonzero)
    memory = 3*7+7*7//512
    copy = 3*((len(PAYLOAD)+31)//32)
    failed = allowance*repetitions; access = 100*repetitions
    gross = intrinsic+ordinary+memory+copy+failed+access
    reserve = (allowance+62)//63+TAIL_GUARD_GAS
    return dict(intrinsic_gas=intrinsic, eip7623_floor_gas=floor, ordinary_opcode_gas=ordinary,
        calldata_copy_word_gas=copy, memory_words=7, memory_expansion_gas=memory,
        failed_gas_per_call=allowance, precompile_failure_gas=failed, precompile_failed_calls=repetitions,
        warm_access_gas_per_call=100, call_access_gas=access, gross_gas=gross,
        charged_gas=max(gross, floor), eip7623_floor_active=gross < floor,
        refund_counter=0, refund_applied=0, eip150_forwarding_reserve=(allowance+62)//63,
        tail_guard_gas=TAIL_GUARD_GAS, transaction_envelope_reserve=reserve,
        transaction_gas_limit=max(gross, floor)+reserve, runtime_bytes=len(code))


def make_case(allowance, repetitions):
    code, counts = _runtime(allowance, repetitions)
    gas = gas_accounting(allowance, repetitions)
    if gas['transaction_gas_limit'] > TRANSACTION_GAS_CAP or len(code) > 24576:
        raise ValueError('case exceeds finite transaction gas/runtime budget')
    row = original.make_case('call', 'stop', gas_limit=gas['transaction_gas_limit'])
    routes = {key: 0 for key in row['request']['program']['expected_route_counts']}; routes.update(counts)
    row['request']['program'].update(code_hex='0x'+code.hex(), input_hex='0x'+PAYLOAD.hex(), expected_route_counts=routes)
    prefix = f'ecmul-failure-a{allowance}'
    row.update(sample_id=f'{prefix}-n{repetitions}', family='ecmul-failures', pair_group=prefix,
        baseline_sample_id=prefix+'-n0', role='control' if repetitions == 0 else 'candidate',
        data_role='failure_validation',
        completion_gate='parent success and exact loop/STOP/opcode counts; each STATICCALL fails with empty returndata, unchanged poisoned output, intact protected words and final counter',
        scope='Caught off-curve ECMUL failures and explicit forwarded-gas consumption; separate from successful scalar work, no tariff or actual-PGU claim')
    row['parameters'] = dict(allowance=allowance, repetitions=repetitions,
        precompile_address='0x'+'00'*19+'07', input_hex='0x'+PAYLOAD.hex(), input_bytes=len(PAYLOAD),
        input_point=[1, 1], raw_scalar_hex='0x'+'00'*32, reduced_scalar_hex='0x'+'00'*32,
        scalar_work_units=0, payload_sha256=digest(PAYLOAD),
        input_validity='off_curve', field_encoding='canonical 32-byte coordinates/scalar; 1^2 != 1^3+3 modulo the BN254 base field',
        expected_failure_class='insufficient_precompile_gas' if allowance < 6000 else 'off_curve_point',
        expected_subcall_success=False, expected_user_success=True,
        expected_precompile_failed_calls=repetitions, expected_precompile_successful_calls=0,
        expected_returndata_bytes=0, output_capacity_bytes=64,
        input_memory_offset=0, output_memory_offset=128, output_poison_words_hex=[f'0x{value:064x}' for value in OUTPUT_POISONS],
        expected_output_memory_unchanged=True, protected_memory_offsets=[96, 192], protected_memory_word=SENTINEL,
        peak_memory_bytes=224, expected_loop_counter=repetitions, counter_location='stack; explicitly checked at completion',
        requested_gas_per_call=allowance, expected_failed_gas_per_call=allowance,
        warm_access_gas_per_call=100, transaction_gas_limit=gas['transaction_gas_limit'],
        transaction_envelope_reserve=gas['transaction_envelope_reserve'], gas_accounting_per_user=gas,
        transaction_kind='call', count_per_block=1, block_count=1, expected_terminal='STOP',
        execution_profile='evm-gas-only', runtime_sha256=digest(code), runtime_bytes=len(code),
        provenance=dict(sources=[
            dict(url='https://eips.ethereum.org/EIPS/eip-196', description='Canonical point(1,1) is off curve, unlike empty input which pads to infinity; invalid point fails and consumes supplied gas'),
            dict(url='https://eips.ethereum.org/EIPS/eip-1108', description='ECMUL base gas6000; lower supplied gas fails before point validation'),
            dict(description='Existing native constructor independently qualifies actual signed calldata, caught failure guards, opcode counts and burned forwarded gas')]))
    return row


def prepare(output_dir):
    if not isinstance(output_dir, Path):
        raise ValueError('output_dir must be a Path')
    plans = [make_case(allowance, n) for allowance in ALLOWANCES for n in REPETITIONS]
    output_dir.mkdir(parents=True, exist_ok=False); (output_dir/'requests').mkdir()
    rows = []
    for plan in plans:
        raw = encoded(plan.pop('request')); relative = f"requests/{plan['sample_id']}.json"
        write(output_dir/relative, raw)
        rows.append(plan | dict(request_path=relative, request_sha256=digest(raw)))
    write(output_dir/'catalog.json', encoded(dict(schema_version=1,
        purpose='Six finite caught ECMUL failure probes with independently guarded full forwarded-gas consumption',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes())
            for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    print(json.dumps({'prepared': prepare(parser.parse_args().output_dir)}))
