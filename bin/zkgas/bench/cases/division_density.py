#!/usr/bin/env python3
"""Generate32 finite compiler-free integer division operand-density probes."""
import argparse
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-division-density-20261104'
OPERATIONS = {'div': 4, 'sdiv': 5, 'mod': 6, 'smod': 7}
DIVISORS = {'limb1': 3, 'limb2': (1 << 64) + 1, 'limb3': (1 << 128) + 1, 'limb4': (1 << 192) + 1}
LEVELS = ('zero', 'full')
MAX_REPETITIONS = 1_000_000
UNROLL = 64
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNERS = tuple(f'0x{BASE_ADDRESS + tag:040x}' for tag in range(USERS))
MASK = (1 << 256) - 1


def _validate(operation, divisor_class, repetitions=0, tag=0):
    if type(operation) is not str or operation not in OPERATIONS: raise ValueError('unknown division operation')
    if type(divisor_class) is not str or divisor_class not in DIVISORS: raise ValueError('unknown divisor class')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, 1, 'owner tag')


def _operands(operation, divisor_class):
    _validate(operation, divisor_class)
    signed = operation in ('sdiv', 'smod'); numerator = -(2**255 - 1) if signed else MASK
    divisor = DIVISORS[divisor_class]
    quotient = abs(numerator) // divisor
    if numerator < 0: quotient = -quotient
    result = quotient if operation in ('div', 'sdiv') else numerator - quotient * divisor
    return numerator & MASK, divisor, result & MASK


def _assemble(operation, divisor_class, repetitions, tag):
    _validate(operation, divisor_class, repetitions, tag)
    numerator, divisor, result = _operands(operation, divisor_class); asm = _Factory()
    def store(offset, value, times=1):
        asm.push(value, 32, times); asm.push(offset, times=times); asm.op(0x52, times)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    def require_value(value):
        asm.push(value, 32); asm.op(0x14); mutation._require(asm)
    store(0, 0); store(32, tag)
    asm.push(divisor, 32); asm.push(numerator, 32)
    asm.op(0x81); asm.op(0x81); asm.op(OPERATIONS[operation]); require_value(result)
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for _ in range(UNROLL):
        for opcode in (0x81, 0x81, OPERATIONS[operation], 0x50): asm.op(opcode, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done')
    asm.op(0x80); require_value(numerator); asm.op(0x50)
    asm.op(0x80); require_value(divisor); asm.op(0x50)
    load(0); require_value(repetitions); load(32); require_value(tag)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(operation, divisor_class, repetitions, tag=0):
    return _assemble(operation, divisor_class, repetitions, tag)


def gas_accounting(operation, divisor_class, repetitions):
    """Size gas using fixed-size code and multiplicities; never replay the body."""
    _validate(operation, divisor_class, repetitions)
    code, counts = _assemble(operation, divisor_class, repetitions, 0)
    costs = {0: 0, 1: 3, 4: 5, 5: 5, 6: 5, 7: 5, 0x14: 3, 0x15: 3, 0x50: 2,
        0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0x81: 3, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, count in counts.items())
    gross = 21000 + ordinary + 6
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=6,
        target_gas=5 * (UNROLL * repetitions + 1), body_gas_per_repetition=UNROLL * 13,
        gross_gas=gross, charged_gas=gross, refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def fits(operation, divisor_class, repetitions):
    gas = gas_accounting(operation, divisor_class, repetitions); envelope = gas['gross_gas'] + 1000
    return (gas['runtime_bytes'] < 24576 and envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(operation, divisor_class):
    _validate(operation, divisor_class)
    if not fits(operation, divisor_class, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(operation, divisor_class, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(operation, divisor_class, low) and not fits(operation, divisor_class, low + 1)
    return low


def make_case(operation='div', divisor_class='limb1', level='full'):
    _validate(operation, divisor_class)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum = full_repetitions(operation, divisor_class); repetitions = 0 if level == 'zero' else maximum
    gas = gas_accounting(operation, divisor_class, repetitions); envelope = gas_accounting(operation, divisor_class, maximum)['gross_gas'] + 1000
    runtimes = [_runtime(operation, divisor_class, repetitions, tag) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]; assert runtimes[1][1] == counts
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    numerator, divisor, result = _operands(operation, divisor_class)
    prefix = f'division-density-{operation}-{divisor_class}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{repetitions}', family='division-density', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_division_density_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent, source, native/signature/receipt/code/account/empty storage/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal panel; operands do not maximize every division correction branch. Native gas/status/routes do not export operand stacks. No per-op PGU or price claim.',
        parameters=dict(operation=operation, divisor_class=divisor_class, workload_level=level,
            operand_a_hex=f'0x{numerator:064x}', operand_b_hex=f'0x{divisor:064x}', expected_result_hex=f'0x{result:064x}',
            unroll_factor=UNROLL, repetitions=repetitions, full_repetitions=maximum, bulk_operations=UNROLL * repetitions,
            target_operations_per_user=UNROLL * repetitions + 1, owner_tags=[0, 1], expected_counter=repetitions,
            counter_memory_offset=0, protected_memory_offset=32, memory_words=2, outer_calldata_bytes=0,
            account_count=USERS, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0, expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            user_transactions_per_block=USERS, all_user_transactions=USERS, all_transactions_including_anchor=USERS + 1,
            block_count=1, visits_per_account=1, transaction_kind='call', transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET,
            expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_user_charged_gas=gas['charged_gas'],
            expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            recipient_schedule=list(OWNERS), expected_terminal='STOP', expected_user_success=True,
            expected_user_receipt_statuses=[True] * USERS, expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=False,
            runtime_bytes=len(codes[0]), runtime_sha256=[digest(code) for code in codes], unused_primary_code_hex='0x00',
            initial_storage_rule='Two distinct authenticated code owners with empty persistent storage; one signer with contiguous nonces0/1.',
            range_rule='Warmup once even R0; fixed64 pure sites per outer iteration; retained operands, counter and owner tag checked before STOP.'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'spawn_memory.py': spawn_memory,
        'storage_fanout.py': storage_fanout, 'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'division_density.{fn.__name__}': fn for fn in (_validate, _operands, _assemble, _runtime, gas_accounting, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for operation in OPERATIONS:
        for divisor_class in DIVISORS:
            for level in LEVELS:
                row = make_case(operation, divisor_class, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 32
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Finite32 integer division operand/limb density own-zero and maximal uniform full inputs.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        operations=list(OPERATIONS), divisor_classes=list(DIVISORS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://ethereum.github.io/yellowpaper/paper.pdf'],
        native_observability='Gas/status/routes and full/minimized execution_result binding; no direct native stack/operand export.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
