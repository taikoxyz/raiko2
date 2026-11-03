#!/usr/bin/env python3
"""Generate 36 heterogeneous storage restore/root-rollback inputs with own N0."""
import argparse
import inspect
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import storage_mutation as mutation
import storage_root_diversity as diversity

MODEL = diversity.MODEL
CORPUS = 'unzen-storage-root-journal-20261103'
MODES = ('restore-stop', 'update-revert', 'restore-revert')
ACCOUNT_COUNTS = (8, 64)
STATE_SIZES = (8192, 16384)
LEVELS = ('zero', 'small', 'full')
USERS = 64
SMALL_N = 32
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
MAX_DISTINCT_PAIRS = 8192
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
ENVELOPE_RESERVE = 2301
SOURCES = ('storage_root_diversity.py', 'storage_fanout.py', 'storage_mutation.py',
           'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def _mode(mode):
    if type(mode) is not str or mode not in MODES:
        raise ValueError('mode must be restore-stop, update-revert or restore-revert')


def _validate(mode, accounts, state_size):
    _mode(mode)
    if type(accounts) is not int or accounts not in ACCOUNT_COUNTS:
        raise ValueError('accounts must be 8 or 64')
    if type(state_size) is not int or state_size not in STATE_SIZES:
        raise ValueError('state_size must be 8192 or 16384')


def _runtime(mode, n):
    """One fixed-width loop, with actual successful or root-reverted route counts."""
    asm = _Factory(); restore = mode != 'update-revert'
    asm.push(0); asm.op(0x54); asm.push(64); asm.op(0x52)
    for offset in (0, 32):
        asm.push(0); asm.push(offset); asm.op(0x52)
    asm.label('loop', n + 1)
    asm.push(0, times=n + 1); asm.op(0x51, n + 1)
    asm.push(n, times=n + 1); asm.op(0x14, n + 1)
    asm.target('done', n + 1); asm.op(0x57, n + 1)
    asm.push(64, times=n); asm.op(0x51, n)
    asm.push(0, times=n); asm.op(0x51, n); asm.op(1, n)
    asm.push(96, times=n); asm.op(0x52, n)
    mutation._key(asm, False, n); asm.op(0x54, n); mutation._check(asm, 1, n)
    for value in ((2, 1) if restore else (2,)):
        asm.push(value, times=n); mutation._key(asm, False, n); asm.op(0x55, n)
        mutation._key(asm, False, n); asm.op(0x54, n)
        asm.push(32, times=n); asm.op(0x51, n); asm.op(1, n)
        asm.push(32, times=n); asm.op(0x52, n)
    asm.push(0, times=n); asm.op(0x51, n); asm.push(1, times=n); asm.op(1, n)
    asm.push(0, times=n); asm.op(0x52, n); asm.target('loop', n); asm.op(0x56, n)
    asm.label('done')
    asm.push(0); asm.op(0x51); mutation._check(asm, n)
    asm.push(32); asm.op(0x51); mutation._check(asm, (3 if restore else 2) * n)
    asm.push(64); asm.op(0x51); asm.push(n); asm.op(1); asm.push(0); asm.op(0x55)
    asm.push(0); asm.op(0x54); asm.push(64); asm.op(0x51); asm.push(n); asm.op(1); asm.op(0x14); mutation._require(asm)
    if mode == 'restore-stop': asm.op(0)
    else: asm.push(0); asm.push(0); asm.op(0xfd)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(mode, n):
    _mode(mode); mutation.original.integer(n, 0, MAX_DISTINCT_PAIRS, 'mutations per user')
    _, counts = _runtime(mode, n)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3,
             0x54: 0, 0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= int(route[-2:], 16) <= 0x7f else costs[int(route[-2:], 16)]) for route, count in counts.items())
    restore = mode != 'update-revert'; words = 4 if n else 3
    memory = 3 * words + words * words // 512
    reads = 2200 + n * (2300 if restore else 2200)
    writes = (2900 if n else 100) + n * (3000 if restore else 2900)
    gross = 21000 + ordinary + memory + reads + writes
    before = 2800 * n if restore else 0
    refund = before if mode == 'restore-stop' else 0; applied = min(refund, gross // 5)
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        storage_read_gas=reads, storage_write_gas=writes, gross_gas=gross,
        refund_counter_before_terminal=before, refund_counter=refund, refund_applied=applied, charged_gas=gross - applied)


def fits(mode, accounts, state_size, n):
    """Candidate envelope, initialized ranges, distinct pairs and every prefix."""
    _validate(mode, accounts, state_size)
    mutation.original.integer(n, 0, MAX_DISTINCT_PAIRS, 'mutations per user')
    stop = mode == 'restore-stop'; visits = USERS // accounts; slots = state_size // accounts
    if (USERS * n + accounts if stop else accounts * (n + 1)) > MAX_DISTINCT_PAIRS:
        return False
    if (visits * n if stop else n) > slots - 1:
        return False
    gas = gas_accounting(mode, n); envelope = gas['gross_gas'] + ENVELOPE_RESERVE
    return (envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_n(mode, accounts, state_size):
    _validate(mode, accounts, state_size)
    low, high = 0, MAX_DISTINCT_PAIRS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(mode, accounts, state_size, middle): low = middle
        else: high = middle - 1
    assert fits(mode, accounts, state_size, low) and not fits(mode, accounts, state_size, low + 1)
    return low


def make_case(mode='restore-stop', accounts=8, state_size=8192, level='full'):
    _validate(mode, accounts, state_size)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('level must be zero, small or full')
    maximum = full_n(mode, accounts, state_size)
    n = {'zero': 0, 'small': SMALL_N, 'full': maximum}[level]
    assert fits(mode, accounts, state_size, n)
    gas = gas_accounting(mode, n); envelope = gas_accounting(mode, maximum)['gross_gas'] + ENVELOPE_RESERVE
    code, counts = _runtime(mode, n)
    # Reuse only fresh authoritative request schema, declarations and disjoint maps.
    request = diversity.make_case('disjoint-keys', accounts, state_size, 'update', 'zero')['request']
    stop = mode == 'restore-stop'; restore = mode != 'update-revert'; arms = 2 if restore else 1
    request['expected_user_success'] = stop
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({route: count * USERS for route, count in counts.items()})
    program['expected_route_counts'] = routes; program['transactions']['gas_limit'] = envelope
    contracts = program['prestate_contracts']
    for contract in contracts: contract['code_hex'] = '0x' + code.hex()
    recipients = program['transactions']['recipients']; slots = state_size // accounts; visits = USERS // accounts
    bases = {contract['address']: (i + 1) * 65536 for i, contract in enumerate(contracts)}
    initial = {owner: base + 1 for owner, base in bases.items()}
    final = {owner: phase + (visits * n if stop else 0) for owner, phase in initial.items()}
    starts = [initial[owner] + ((j // accounts) * n if stop else 0) for j, owner in enumerate(recipients)]
    block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    prefix = f'storage-root-journal-{mode}-a{accounts}-s{state_size}'
    return dict(sample_id=f'{prefix}-n{n}', family='storage-root-journal', corpus_id=CORPUS,
        role='control' if n == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_storage_root_journal_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all36 native, signed recipient/receipt, authenticated original/final per-account roots, canonical mutation-witness and original real-quote gates precede actuals. Require A distinct initial/final roots and exact root restoration on REVERT. Native success alone is insufficient.',
        scope='Finite heterogeneous dirty restore/root rollback under frozen c893 pricing. Preserve own-N0 controls and failures; compare whole-proposal actual SP1 PGU, literal node block estimates/sum and complete proposal quote separately. No refit, per-operation actual allocation, universal trie bound, generated proof or throughput claim.',
        parameters=dict(mode=mode, account_count=accounts, aggregate_state_size=state_size,
            state_size_per_account=slots, workload_level=level, mutations_per_user=n, full_mutations_per_user=maximum,
            user_transactions_per_block=USERS, all_user_transactions=USERS, visits_per_account=visits,
            block_count=1, all_transactions_including_anchor=USERS + 1, transaction_kind='call',
            transaction_gas_limit=envelope, transaction_envelope_reserve=ENVELOPE_RESERVE,
            header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=gas['refund_counter'],
            expected_user_refund_counter_before_terminal=gas['refund_counter_before_terminal'],
            expected_user_refund_applied=gas['refund_applied'], expected_user_charged_gas=gas['charged_gas'],
            gas_accounting_per_user=gas, expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            expected_distinct_account_slot_pairs=USERS * n + accounts if stop else accounts * (n + 1),
            expected_distinct_storage_keys_per_account=(visits * n if stop else n) + 1,
            expected_distinct_body_keys_per_account=visits * n if stop else n,
            expected_body_key_visits_per_account=visits * n,
            expected_repeated_body_key_visits_per_account=0 if stop else (visits - 1) * n,
            expected_storage_reads=USERS * ((3 if restore else 2) * n + 2),
            expected_storage_writes=USERS * (arms * n + 1), expected_main_storage_writes=USERS * arms * n,
            expected_phase_storage_writes=USERS, expected_main_storage_writes_per_account=visits * arms * n,
            expected_storage_writes_per_account=visits * (arms * n + 1),
            expected_initial_storage_entry_count=state_size, expected_final_storage_entry_count=state_size,
            expected_initial_storage_entry_count_per_account=slots, expected_final_storage_entry_count_per_account=slots,
            expected_net_main_storage_changes=0, phase_slot=0, phase_base_by_account=bases,
            initial_phase_by_account=initial, expected_final_phase_by_account=final,
            recipient_schedule=recipients.copy(), start_phase_schedule=starts,
            attempted_final_phase_schedule=[start + n for start in starts],
            expected_counter=n, expected_checksum=(3 if restore else 2) * n,
            counter_memory_offset=0, checksum_memory_offset=32, phase_memory_offset=64, key_memory_offset=96,
            initial_storage_rule='Disjoint keys: B_i=(i+1)*65536; raw slot0=B_i+1; raw slots B_i+1..B_i+K-1=1; all others absent. Identical own-N0/small/full maps.',
            range_rule=('User j calls account j%A; start=B_i+1+floor(j/A)*N; body keys start..start+N-1; main values restore1, phase commits start+N.' if stop else
                'User j calls account j%A; every visit repeats B_i+1..B_i+N; phase+N is attempted/read-checked; root REVERT restores the entire original map including phase.'),
            witness_policy='Canonical conservative mutation-aware raw-slot witnesses; distinct (account,rawslot) pairs count attempted accesses even on root rollback. No padding or hashed-key injection.',
            expected_terminal='STOP' if stop else 'REVERT', expected_user_success=stop,
            expected_exceptional_instructions=0, outer_calldata_bytes=0, execution_profile='evm-gas-only',
            preload_created_accounts_or_code=False, runtime_bytes=len(code), runtime_sha256=digest(code),
            shared_runtime=True, unused_primary_code_hex='0x00'), request=request)


def helpers():
    return {'storage_root_diversity.make_case': diversity.make_case, '_Factory': _Factory,
        'storage_mutation._require': mutation._require, 'storage_mutation._check': mutation._check,
        'storage_mutation._key': mutation._key, 'storage_root_journal._runtime': _runtime,
        'storage_root_journal.gas_accounting': gas_accounting, 'storage_root_journal.full_n': full_n}


def prepare(output: Path, *, model_id=MODEL) -> int:
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for mode in MODES:
        for accounts in ACCOUNT_COUNTS:
            for size in STATE_SIZES:
                for level in LEVELS:
                    row = make_case(mode, accounts, size, level)
                    raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                    write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 36
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='36 heterogeneous restore/root-rollback inputs: modes3, accounts8/64, aggregate state8192/16384, own-N0/small32/maximal-full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        modes=list(MODES), account_counts=list(ACCOUNT_COUNTS), aggregate_state_sizes=list(STATE_SIZES), levels=list(LEVELS),
        users=USERS, holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Finite frozen-pricing validation. Independent source review and all36 native/signature/root/witness/original-real-quote gates precede actuals. Expected user REVERT is legitimate included failure. Preserve own controls and deficits; no refit or global bound.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
