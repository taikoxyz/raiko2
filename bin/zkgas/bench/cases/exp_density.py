#!/usr/bin/env python3
"""Generate10 finite compiler-free fixed-operand EXP density probes."""
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

CORPUS = 'unzen-exp-density-20261104'
EXPONENTS = {'dense2': (1 << 16) - 1, 'sparse24': 1 << 191, 'dense24': (1 << 192) - 1, 'dense25': (1 << 200) - 1, 'dense32': (1 << 256) - 1}
BASE = int('a5' * 32, 16)
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


def _validate(exponent_class, repetitions=0, tag=0):
    if type(exponent_class) is not str or exponent_class not in EXPONENTS: raise ValueError('unknown exponent class')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, 1, 'owner tag')


def _operands(exponent_class):
    _validate(exponent_class)
    exponent = EXPONENTS[exponent_class]
    return BASE, exponent, pow(BASE, exponent, 1 << 256)


def _assemble(exponent_class, repetitions, tag):
    _validate(exponent_class, repetitions, tag)
    base, exponent, result = _operands(exponent_class); asm = _Factory()
    def store(offset, value, times=1):
        asm.push(value, 32, times); asm.push(offset, times=times); asm.op(0x52, times)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    def require_value(value):
        asm.push(value, 32); asm.op(0x14); mutation._require(asm)
    store(0, 0); store(32, tag)
    asm.push(exponent, 32); asm.push(base, 32)
    asm.op(0x81); asm.op(0x81); asm.op(0x0a); require_value(result)
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for _ in range(UNROLL):
        for opcode in (0x81, 0x81, 0x0a, 0x50): asm.op(opcode, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done')
    for value in (base, exponent):
        asm.op(0x80); require_value(value); asm.op(0x50)
    load(0); require_value(repetitions); load(32); require_value(tag)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(exponent_class, repetitions, tag=0):
    return _assemble(exponent_class, repetitions, tag)


def gas_accounting(exponent_class, repetitions):
    """Size gas using fixed-size code and multiplicities; never replay the body."""
    _validate(exponent_class, repetitions)
    code, counts = _assemble(exponent_class, repetitions, 0)
    target_cost = 10 + 50 * ((EXPONENTS[exponent_class].bit_length() + 7) // 8)
    costs = {0: 0, 1: 3, 0x0a: target_cost, 0x14: 3, 0x15: 3, 0x50: 2,
        0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0x81: 3, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, count in counts.items())
    gross = 21000 + ordinary + 6
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=6,
        target_gas=target_cost * (UNROLL * repetitions + 1), target_gas_per_event=target_cost,
        body_gas_per_repetition=UNROLL * (target_cost + 8),
        gross_gas=gross, charged_gas=gross, refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def fits(exponent_class, repetitions):
    gas = gas_accounting(exponent_class, repetitions); envelope = gas['gross_gas'] + 1000
    return (gas['runtime_bytes'] < 24576 and envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(exponent_class):
    _validate(exponent_class)
    if not fits(exponent_class, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(exponent_class, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(exponent_class, low) and not fits(exponent_class, low + 1)
    return low


def make_case(exponent_class='dense24', level='full'):
    _validate(exponent_class)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum = full_repetitions(exponent_class); repetitions = 0 if level == 'zero' else maximum
    gas = gas_accounting(exponent_class, repetitions); envelope = gas_accounting(exponent_class, maximum)['gross_gas'] + 1000
    runtimes = [_runtime(exponent_class, repetitions, tag) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]; assert runtimes[1][1] == counts
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    base, exponent, result = _operands(exponent_class)
    prefix = f'exp-density-{exponent_class}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    next_gas = gas_accounting(exponent_class, maximum + 1); next_envelope = next_gas['gross_gas'] + 1000
    next_failures = [name for name, failed in (
        ('charged_block_budget', ANCHOR_GAS + USERS * next_gas['charged_gas'] > BLOCK_BUDGET),
        ('transaction_fixture_cap', next_envelope > TRANSACTION_GAS_CAP),
        ('eip7825_cap', next_envelope > EIP7825_GAS_CAP),
        ('ordered_header_prefix', any(HEADER_GAS_LIMIT - ANCHOR_GAS - j * next_gas['charged_gas'] < next_envelope for j in range(USERS))),
        ('runtime_size', next_gas['runtime_bytes'] >= 24576)) if failed]
    assert next_failures
    return dict(sample_id=f'{prefix}-n{UNROLL * repetitions}', family='exp-density', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_exp_density_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent, source, native/signature/receipt/code/account/empty storage/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal panel; fixed odd base and exponent classes are not global worst inputs. Source h+w wrapping multiplications are not actual PGU or the node pricing selector. Native gas/status/routes do not export operand stacks. No per-op PGU or price claim.',
        parameters=dict(exponent_class=exponent_class, workload_level=level,
            base_hex=f'0x{base:064x}', exponent_hex=f'0x{exponent:064x}', expected_result_hex=f'0x{result:064x}',
            exponent_bit_length=exponent.bit_length(), popcount=exponent.bit_count(), exponent_bytes=(exponent.bit_length() + 7) // 8,
            source_wrapping_mul_count=exponent.bit_length() + exponent.bit_count(),
            source_work_scope='Selected ruint wrapping_pow h squarings plus popcount result multiplications; not actual PGU or node selector.',
            unroll_factor=UNROLL, repetitions=repetitions, full_repetitions=maximum, bulk_operations=UNROLL * repetitions,
            full_plus_one_constraint_failures=next_failures,
            fullness_scope='Maximal uniform64-site batches for these two users, not unrestricted packing; near_full is derived at99 percent.',
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
    return {'cases/initcode_preparation.py': initcode_preparation, 'cases/spawn_memory.py': spawn_memory,
        'cases/storage_fanout.py': storage_fanout, 'cases/storage_mutation.py': mutation, 'cases/top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'exp_density.{fn.__name__}': fn for fn in (_validate, _operands, _assemble, _runtime, gas_accounting, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for exponent_class in EXPONENTS:
        for level in LEVELS:
            row = make_case(exponent_class, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
            write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 10
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Finite10 fixed-operand EXP density/popcount own-zero and maximal uniform64-batch full inputs; maximality is not unrestricted packing.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        exponent_classes=list(EXPONENTS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://ethereum.github.io/yellowpaper/paper.pdf'],
        native_observability='Gas/status/routes and full/minimized execution_result binding; no direct native stack/operand export.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
