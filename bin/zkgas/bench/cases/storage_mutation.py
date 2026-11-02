#!/usr/bin/env python3
"""Generate 24 committed storage update/delete/insert/mixed validation inputs."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
MODES = ('update', 'delete', 'insert', 'mixed')
STATE_SIZES = (8192, 16384)
LEVELS = ('zero', 'small', 'full')
USERS = 8
SMALL_N = 64
INSERT_BASE = 1048576
ANCHOR_GAS = 167355
HEADER_GAS_LIMIT = 31_000_000
BLOCK_BUDGET = 30_000_000


def _validate(mode, state_size):
    if type(mode) is not str or mode not in MODES:
        raise ValueError('unknown storage mutation mode')
    if type(state_size) is not int or state_size not in STATE_SIZES:
        raise ValueError('state_size must be 8192 or 16384')


def _require(asm, times=1):
    asm.op(0x15, times); asm.target('failure', times); asm.op(0x57, times)


def _check(asm, expected, times=1):
    asm.push(expected, times=times); asm.op(0x14, times); _require(asm, times)


def _key(asm, absent, times):
    asm.push(96, times=times); asm.op(0x51, times)
    if absent:
        asm.push(INSERT_BASE, times=times); asm.op(1, times)


def _runtime(mode, n):
    """One fixed-width loop; multiplicities describe one successful user."""
    asm = _Factory()
    asm.push(0); asm.op(0x54); asm.push(64); asm.op(0x52)
    for offset in (0, 32):
        asm.push(0); asm.push(offset); asm.op(0x52)
    asm.label('loop', n + 1)
    asm.push(0, times=n + 1); asm.op(0x51, n + 1)
    asm.push(n, times=n + 1); asm.op(0x14, n + 1)
    asm.target('done', n + 1); asm.op(0x57, n + 1)
    # The phase is committed by the preceding transaction. Counter i selects
    # exactly phase+i; an absent counterpart uses a separate raw-slot domain.
    asm.push(64, times=n); asm.op(0x51, n)
    asm.push(0, times=n); asm.op(0x51, n); asm.op(1, n)
    asm.push(96, times=n); asm.op(0x52, n)
    operations = ((False, 2),) if mode == 'update' else ((False, 0),) if mode == 'delete' else ((True, 2),) if mode == 'insert' else ((False, 0), (True, 2))
    for absent, value in operations:
        _key(asm, absent, n); asm.op(0x54, n); _check(asm, 0 if absent else 1, n)
        asm.push(value, times=n); _key(asm, absent, n); asm.op(0x55, n)
        _key(asm, absent, n); asm.op(0x54, n)
        asm.push(32, times=n); asm.op(0x51, n); asm.op(1, n)
        asm.push(32, times=n); asm.op(0x52, n)
    asm.push(0, times=n); asm.op(0x51, n); asm.push(1, times=n); asm.op(1, n)
    asm.push(0, times=n); asm.op(0x52, n); asm.target('loop', n); asm.op(0x56, n)
    asm.label('done')
    asm.push(0); asm.op(0x51); _check(asm, n)
    asm.push(32); asm.op(0x51); _check(asm, 0 if mode == 'delete' else 2 * n)
    asm.push(64); asm.op(0x51); asm.push(n); asm.op(1); asm.push(0); asm.op(0x55)
    asm.push(0); asm.op(0x54); asm.push(64); asm.op(0x51); asm.push(n); asm.op(1); asm.op(0x14); _require(asm)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(mode, n):
    """Price actual instruction counts once, then cold/warm storage separately."""
    if type(mode) is not str or mode not in MODES:
        raise ValueError('unknown storage mutation mode')
    original.integer(n, 0, 8192, 'mutations per user')
    _, counts = _runtime(mode, n)
    static_cost = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3,
                   0x54: 0, 0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= int(route[-2:], 16) <= 0x7f else static_cost[int(route[-2:], 16)])
                   for route, count in counts.items())
    words = 4 if n else 3
    memory = 3 * words + words * words // 512
    arms = 2 if mode == 'mixed' else 1
    loads = 2200 + 2200 * n * arms  # phase cold+warm, body first-read+postcheck
    stores = (2900 if n else 100) + n * {'update': 2900, 'delete': 2900, 'insert': 20000, 'mixed': 22900}[mode]
    gross = 21000 + ordinary + memory + loads + stores
    refund = 4800 * n if mode in ('delete', 'mixed') else 0
    applied = min(refund, gross // 5)
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
                storage_read_gas=loads, storage_write_gas=stores, gross_gas=gross,
                refund_counter=refund, refund_applied=applied, charged_gas=gross - applied)


def fits(mode, state_size, n):
    """Test this candidate's own envelope at every ordered header prefix."""
    _validate(mode, state_size); original.integer(n, 0, 8192, 'mutations per user')
    arms = 2 if mode == 'mixed' else 1
    if 1 + USERS * n * arms > 8192:
        return False
    if mode != 'insert' and USERS * n > state_size - 1:
        return False
    gas = gas_accounting(mode, n); envelope = gas['gross_gas'] + 1000
    return (envelope <= 15_000_000 and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
            and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_n(mode, state_size):
    _validate(mode, state_size)
    low, high = 0, 8192
    while low < high:
        middle = (low + high + 1) // 2
        if fits(mode, state_size, middle): low = middle
        else: high = middle - 1
    assert fits(mode, state_size, low) and not fits(mode, state_size, low + 1)
    return low


def make_case(mode='update', state_size=8192, level='full'):
    _validate(mode, state_size)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('level must be zero, small or full')
    maximum = full_n(mode, state_size)
    n = {'zero': 0, 'small': SMALL_N, 'full': maximum}[level]
    assert fits(mode, state_size, n)
    gas = gas_accounting(mode, n); envelope = gas_accounting(mode, maximum)['gross_gas'] + 1000
    code, counts = _runtime(mode, n)
    row = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)
    program = row['request']['program']
    routes = {route: 0 for route in program['expected_route_counts']}
    routes.update({route: count * USERS for route, count in counts.items()})
    proof = [0]; inserts = []; deletes = []
    for key in range(1, USERS * n + 1):
        if mode != 'insert': proof.append(key)
        if mode in ('insert', 'mixed'):
            proof.append(INSERT_BASE + key); inserts.append(INSERT_BASE + key)
        if mode in ('delete', 'mixed'): deletes.append(key)
    program.update(code_hex='0x' + code.hex(), expected_route_counts=routes, storage_state={
        'initial_storage': {hex(key): '0x1' for key in range(state_size)},
        'proof_slots': [hex(key) for key in proof], 'insert_slots': [hex(key) for key in inserts],
        'delete_slots': [hex(key) for key in deletes]})
    sid = f'storage-mutation-{mode}-s{state_size}-n{n}'
    row.update(sample_id=sid, family='storage-mutation', role='control' if n == 0 else 'candidate',
        baseline_sample_id=f'storage-mutation-{mode}-s{state_size}-n0',
        data_role='unfitted_storage_mutation_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Qualify all eight signed CALLs, ordered raw reads/writes/values, per-transaction originals/warmth/refunds, counter/checksum/phase, exact routes and authenticated initial/final storage roots independently before same-proposal actuals.',
        scope='Finite committed batch-storage patterns under frozen c893 pricing; retain own-N0 controls and all failures. Compare whole-proposal actual SP1 PGU, literal node block estimate/sum and complete-proposal quote separately. No refit, per-user actual attribution, universal MPT bound, proof generation or throughput claim.')
    row['parameters'] = dict(mode=mode, state_size=state_size, workload_level=level, mutations_per_user=n,
        full_mutations_per_user=maximum, user_transactions_per_block=USERS, all_user_transactions=USERS,
        block_count=1, all_transactions_including_anchor=USERS + 1, transaction_kind='call',
        transaction_gas_limit=envelope, transaction_envelope_reserve=1000,
        header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
        expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=gas['refund_counter'],
        expected_user_refund_applied=gas['refund_applied'], expected_user_charged_gas=gas['charged_gas'],
        expected_block_charged_gas=ANCHOR_GAS + USERS * gas['charged_gas'], gas_accounting_per_user=gas,
        proof_slot_count=len(proof), expected_distinct_storage_keys=len(proof),
        expected_storage_writes=USERS * (n * (2 if mode == 'mixed' else 1) + 1),
        expected_main_storage_writes=USERS * n * (2 if mode == 'mixed' else 1),
        expected_phase_storage_writes=USERS, insert_slot_count=len(inserts), delete_slot_count=len(deletes),
        expected_initial_storage_entry_count=state_size,
        expected_final_storage_entry_count=state_size + len(inserts) - len(deletes), expected_final_phase=1 + USERS * n,
        phase_slot=0, initial_phase=1, insert_base=INSERT_BASE, expected_counter=n,
        expected_checksum=0 if mode == 'delete' else 2 * n,
        counter_memory_offset=0, checksum_memory_offset=32, phase_memory_offset=64, key_memory_offset=96,
        initial_storage_rule='Exactly raw slots 0..state_size-1 contain 1; every other raw slot is absent.',
        range_rule='User j starts at 1+j*N and visits start+i in ascending order; absent counterparts add insert_base. Slot 0 commits start+N.',
        witness_policy='Canonical mutation-aware target multiproof for real raw slots; no padding or hashed-key injection.',
        expected_user_success=True, expected_exceptional_instructions=0, outer_calldata_bytes=0,
        execution_profile='evm-gas-only', preload_created_accounts_or_code=False,
        runtime_bytes=len(code), runtime_sha256=digest(code))
    return row


def prepare(output: Path, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for mode in MODES:
        for size in STATE_SIZES:
            for level in LEVELS:
                row = make_case(mode, size, level)
                raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                write(output / path, raw)
                rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 24
    write(output / 'catalog.json', encoded(dict(schema_version=1, frozen_model_id=model_id,
        purpose='24 committed storage mutation inputs: four modes, two initialized state sizes and own-N0/small/maximal-full levels.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        modes=list(MODES), state_sizes=list(STATE_SIZES), levels=list(LEVELS), users=USERS,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Frozen pricing validation only. Qualify canonical inputs, signed receipts, opcode routes, concrete ordered semantics and authenticated initial/final roots independently before actuals. Charged budget 30M and header 31M remain separate; preserve failures and both deficit flags.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
