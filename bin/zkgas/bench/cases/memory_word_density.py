#!/usr/bin/env python3
"""Generate12 finite compiler-free dense in-bounds word-memory probes."""
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

CORPUS = 'unzen-memory-word-density-20261104'
OPERATIONS = {'store': 0x52, 'load': 0x51}
ARENAS = (256, 1048576)
GROUPS = tuple((op, arena, offset) for op in OPERATIONS for arena in ARENAS for offset in ((0, 1) if op == 'store' else (1,)))
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
A = int.from_bytes(bytes.fromhex('a5' * 32), 'big')
B = int.from_bytes(bytes.fromhex('5a' * 32), 'big')


def _validate(operation, arena_bytes, residue, repetitions=0, tag=0):
    if type(operation) is not str or operation not in OPERATIONS: raise ValueError('unknown memory operation')
    if type(arena_bytes) is not int or arena_bytes not in ARENAS: raise ValueError('arena must be256 or1048576 bytes')
    if type(residue) is not int or residue not in ((0, 1) if operation == 'store' else (1,)): raise ValueError('invalid target residue')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, 1, 'owner tag')


def _assemble(operation, arena_bytes, residue, repetitions, tag):
    _validate(operation, arena_bytes, residue, repetitions, tag); asm = _Factory()
    def store(offset, value):
        asm.push(value, 32); asm.push(offset); asm.op(0x52)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    def require_value(value):
        asm.push(value, 32); asm.op(0x14); mutation._require(asm)
    # Exactly arena_bytes resident EVM bytes, including own-zero controls.
    asm.push(0, 1); asm.push(arena_bytes - 1); asm.op(0x53)
    store(64, 0); store(96, tag)
    if operation == 'store':
        asm.push(A, 32); asm.push(B, 32); asm.push(residue)
        for duplicate, value in ((0x82, A), (0x81, B)):
            asm.op(duplicate); asm.op(0x81); asm.op(0x52)
            load(residue); require_value(value)
    else:
        store(residue, B); asm.push(residue)
        asm.op(0x80); asm.op(0x51); require_value(B)
    visits = repetitions + 1; asm.label('loop', visits)
    load(64, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for site in range(UNROLL):
        opcodes = ((0x82 if site % 2 == 0 else 0x81), 0x81, 0x52) if operation == 'store' else (0x80, 0x51, 0x50)
        for opcode in opcodes: asm.op(opcode, repetitions)
    load(64, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done')
    load(residue); require_value(B); load(arena_bytes - 32); require_value(0)
    for value in ((residue, B, A) if operation == 'store' else (residue,)):
        asm.op(0x80); require_value(value); asm.op(0x50)
    load(64); require_value(repetitions); load(96); require_value(tag)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(operation, arena_bytes, residue, repetitions, tag=0):
    return _assemble(operation, arena_bytes, residue, repetitions, tag)


def gas_accounting(operation, arena_bytes, residue, repetitions):
    """Count emitted instructions with multiplicities and exact first expansion."""
    _validate(operation, arena_bytes, residue, repetitions)
    code, counts = _assemble(operation, arena_bytes, residue, repetitions, 0)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x50: 2, 0x51: 3, 0x52: 3, 0x53: 3,
        0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0x81: 3, 0x82: 3, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, count in counts.items())
    words = arena_bytes // 32; expansion = 3 * words + words * words // 512
    gross = 21000 + ordinary + expansion
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=expansion,
        target_gas=3 * (UNROLL * repetitions + (2 if operation == 'store' else 1)),
        body_gas_per_repetition=UNROLL * (9 if operation == 'store' else 8),
        gross_gas=gross, charged_gas=gross, refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def _constraint_failures(gas):
    envelope = gas['gross_gas'] + 1000
    return [name for name, failed in (
        ('charged_block_budget', ANCHOR_GAS + USERS * gas['charged_gas'] > BLOCK_BUDGET),
        ('transaction_fixture_cap', envelope > TRANSACTION_GAS_CAP),
        ('eip7825_cap', envelope > EIP7825_GAS_CAP),
        ('ordered_header_prefix', any(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] < envelope for j in range(USERS))),
        ('runtime_size', gas['runtime_bytes'] >= 24576)) if failed]


def fits(operation, arena_bytes, residue, repetitions):
    return not _constraint_failures(gas_accounting(operation, arena_bytes, residue, repetitions))


def full_repetitions(operation, arena_bytes, residue):
    _validate(operation, arena_bytes, residue)
    if not fits(operation, arena_bytes, residue, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(operation, arena_bytes, residue, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(operation, arena_bytes, residue, low) and not fits(operation, arena_bytes, residue, low + 1)
    return low


def make_case(operation='store', arena_bytes=256, residue=1, level='full'):
    _validate(operation, arena_bytes, residue)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum = full_repetitions(operation, arena_bytes, residue); repetitions = 0 if level == 'zero' else maximum
    gas = gas_accounting(operation, arena_bytes, residue, repetitions); envelope = gas_accounting(operation, arena_bytes, residue, maximum)['gross_gas'] + 1000
    runtimes = [_runtime(operation, arena_bytes, residue, repetitions, tag) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]; assert runtimes[1][1] == counts
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    prefix = f'memory-word-{operation}-a{arena_bytes}-o{residue}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    warmup = 2 if operation == 'store' else 1; target_count = UNROLL * repetitions + warmup
    return dict(sample_id=f'{prefix}-n{UNROLL * repetitions}', family='memory-word-density', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_memory_word_density_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent decoded stack/memory/intent/gas, source, native/signature/receipt/code/account/empty storage/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal panel; relative offset is input observable, actual per-occurrence pointers/branches and per-op PGU are not exported. No global or pricing claim.',
        parameters=dict(operation=operation, arena_bytes=arena_bytes, residue=residue, target_offset_bytes=residue, workload_level=level,
            value_a_hex=f'0x{A:064x}', value_b_hex=f'0x{B:064x}',
            unroll_factor=UNROLL, repetitions=repetitions, full_repetitions=maximum, bulk_operations=UNROLL * repetitions,
            full_plus_one_constraint_failures=_constraint_failures(gas_accounting(operation, arena_bytes, residue, maximum + 1)),
            fullness_scope='Maximal uniform64-site batches for these two users, not unrestricted packing; near_full derived at99 percent.',
            target_operations_per_user=target_count, warmup_target_operations_per_user=warmup, body_target_operations_per_user=UNROLL * repetitions,
            non_target_same_opcode_operations_per_user=counts[f'opcode:0x{OPERATIONS[operation]:02x}'] - target_count,
            owner_tags=[0, 1], expected_counter=repetitions, counter_memory_offset=64, protected_memory_offset=96,
            memory_words=arena_bytes // 32, peak_memory_bytes=arena_bytes, preallocation_mstore8_events_per_user=1, outer_calldata_bytes=0,
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
            range_rule='Exact preallocation; alternating A/B stores ending B or repeated B loads; original retained operands, target, last word, counter and tag guarded before empty-stack STOP.'), request=request)


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation, 'cases/spawn_memory.py': spawn_memory,
        'cases/storage_fanout.py': storage_fanout, 'cases/storage_mutation.py': mutation, 'cases/top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'memory_word_density.{fn.__name__}': fn for fn in (_validate, _assemble, _runtime, gas_accounting, _constraint_failures, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for operation, arena_bytes, residue in GROUPS:
        for level in LEVELS:
            row = make_case(operation, arena_bytes, residue, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
            write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 12
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Finite12 dense in-bounds word-memory own-zero and maximal uniform64-batch full inputs.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        groups=[dict(operation=op, arena_bytes=arena, residue=residue) for op, arena, residue in GROUPS], levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://ethereum.github.io/yellowpaper/paper.pdf'],
        native_observability='Gas/status/routes and full/minimized execution_result binding; no direct native stack/operand export.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
