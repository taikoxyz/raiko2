#!/usr/bin/env python3
"""Generate 16 finite few-long-user root-REVERT inputs with own N0 controls."""
import argparse
import inspect
import json
from pathlib import Path
import re

from spawn_memory import digest, encoded, write
from storage_fanout import BASE_ADDRESS
import storage_root_journal as journal
import top_level_create as original

MODEL = journal.MODEL
CORPUS = 'unzen-storage-long-revert-20261103'
MODES = ('update-revert', 'restore-revert')
USER_COUNTS = (2, 4)
STATE_SIZES = (8192, 16384)
LEVELS = ('zero', 'full')
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
MAX_DISTINCT_PAIRS = 8192
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
ENVELOPE_RESERVE = 2301
SOURCES = ('storage_root_journal.py', 'storage_root_diversity.py', 'storage_fanout.py',
           'storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def _validate(mode, users, state_size):
    if type(mode) is not str or mode not in MODES:
        raise ValueError('mode must be update-revert or restore-revert')
    if type(users) is not int or users not in USER_COUNTS:
        raise ValueError('users must be 2 or 4')
    if type(state_size) is not int or state_size not in STATE_SIZES:
        raise ValueError('state_size must be 8192 or 16384')


def fits(mode, users, state_size, n) -> bool:
    """Use this candidate's own envelope for every ordered admission prefix."""
    _validate(mode, users, state_size)
    original.integer(n, 0, MAX_DISTINCT_PAIRS, 'mutations per user')
    if users * (n + 1) > MAX_DISTINCT_PAIRS or n > state_size // users - 1:
        return False
    gas = journal.gas_accounting(mode, n); envelope = gas['gross_gas'] + ENVELOPE_RESERVE
    return (envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + users * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(users)))


def full_n(mode, users, state_size) -> int:
    _validate(mode, users, state_size)
    low, high = 0, MAX_DISTINCT_PAIRS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(mode, users, state_size, middle): low = middle
        else: high = middle - 1
    assert fits(mode, users, state_size, low) and not fits(mode, users, state_size, low + 1)
    return low


def make_case(mode='update-revert', users=2, state_size=8192, level='full') -> dict:
    _validate(mode, users, state_size)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('level must be zero or full')
    maximum = full_n(mode, users, state_size); n = 0 if level == 'zero' else maximum
    gas = journal.gas_accounting(mode, n)
    envelope = journal.gas_accounting(mode, maximum)['gross_gas'] + ENVELOPE_RESERVE
    code, counts = journal._runtime(mode, n)
    # Discard upstream experiment metadata; retain only the authoritative schema.
    request = original.make_case('call', 'stop', count=users, gas_limit=envelope)['request']
    request['expected_user_success'] = False
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({route: count * users for route, count in counts.items()})
    program['expected_route_counts'] = routes
    recipients = [f'0x{BASE_ADDRESS + i:040x}' for i in range(users)]
    program['transactions']['recipients'] = recipients
    slots = state_size // users
    bases = {owner: (i + 1) * 65536 for i, owner in enumerate(recipients)}
    initial = {owner: base + 1 for owner, base in bases.items()}
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(),
        initial_storage={'0x0': hex(base + 1)} | {hex(key): '0x1' for key in range(base + 1, base + slots)})
        for owner, base in bases.items()]
    restore = mode == 'restore-revert'; arms = 2 if restore else 1
    starts = list(initial.values()); final = initial.copy()
    block_gas = ANCHOR_GAS + users * gas['charged_gas']
    prefix = f'storage-long-revert-{mode}-u{users}-s{state_size}'
    return dict(sample_id=f'{prefix}-n{n}', family='storage-long-revert', corpus_id=CORPUS,
        role='control' if n == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_storage_long_revert_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all16 native, signed recipient/receipt, authenticated original/final per-account roots, canonical mutation-witness and original real-quote gates precede actuals. Require U distinct initial/final roots and exact root restoration on REVERT. Native success alone is insufficient.',
        scope='Finite few-long-user root rollback under frozen c893 pricing. Preserve own-N0 controls and failures; compare whole-proposal actual SP1 PGU, literal node block estimates/sum and complete proposal quote separately. No refit, per-operation actual allocation, universal trie bound, generated proof or throughput claim.',
        parameters=dict(mode=mode, account_count=users, aggregate_state_size=state_size,
            state_size_per_account=slots, workload_level=level, mutations_per_user=n, full_mutations_per_user=maximum,
            user_transactions_per_block=users, all_user_transactions=users, visits_per_account=1,
            block_count=1, all_transactions_including_anchor=users + 1, transaction_kind='call',
            transaction_gas_limit=envelope, transaction_envelope_reserve=ENVELOPE_RESERVE,
            header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=gas['refund_counter'],
            expected_user_refund_counter_before_terminal=gas['refund_counter_before_terminal'],
            expected_user_refund_applied=gas['refund_applied'], expected_user_charged_gas=gas['charged_gas'],
            gas_accounting_per_user=gas, expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(users)],
            expected_distinct_account_slot_pairs=users * (n + 1),
            expected_distinct_storage_keys_per_account=n + 1,
            expected_distinct_body_keys_per_account=n,
            expected_body_key_visits_per_account=n,
            expected_repeated_body_key_visits_per_account=0,
            expected_storage_reads=users * ((3 if restore else 2) * n + 2),
            expected_storage_writes=users * (arms * n + 1), expected_main_storage_writes=users * arms * n,
            expected_phase_storage_writes=users, expected_main_storage_writes_per_account=arms * n,
            expected_storage_writes_per_account=arms * n + 1,
            expected_initial_storage_entry_count=state_size, expected_final_storage_entry_count=state_size,
            expected_initial_storage_entry_count_per_account=slots, expected_final_storage_entry_count_per_account=slots,
            expected_net_main_storage_changes=0, phase_slot=0, phase_base_by_account=bases,
            initial_phase_by_account=initial, expected_final_phase_by_account=final,
            recipient_schedule=recipients.copy(), start_phase_schedule=starts,
            attempted_final_phase_schedule=[start + n for start in starts],
            expected_counter=n, expected_checksum=(3 if restore else 2) * n,
            counter_memory_offset=0, checksum_memory_offset=32, phase_memory_offset=64, key_memory_offset=96,
            initial_storage_rule='Disjoint keys: B_i=(i+1)*65536; raw slot0=B_i+1; raw slots B_i+1..B_i+K-1=1; all others absent. Identical own-N0/full maps.',
            range_rule='User j calls distinct account j once; body visits B_j+1..B_j+N; phase+N is attempted/read-checked; root REVERT restores the complete original map including phase.',
            witness_policy='Canonical conservative mutation-aware raw-slot witnesses; distinct (account,rawslot) pairs count attempted accesses even on root rollback. No padding or hashed-key injection.',
            expected_terminal='REVERT', expected_user_success=False,
            expected_user_receipt_statuses=[False] * users, expected_anchor_success=True,
            expected_exceptional_instructions=0, outer_calldata_bytes=0, execution_profile='evm-gas-only',
            preload_created_accounts_or_code=False, runtime_bytes=len(code), runtime_sha256=digest(code),
            shared_runtime=True, unused_primary_code_hex='0x00'), request=request)


def helpers():
    return {'storage_root_journal._runtime': journal._runtime,
        'storage_root_journal.gas_accounting': journal.gas_accounting,
        '_Factory': journal._Factory, 'storage_mutation._require': journal.mutation._require,
        'storage_mutation._check': journal.mutation._check, 'storage_mutation._key': journal.mutation._key,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        'storage_long_revert.full_n': full_n, 'storage_long_revert.fits': fits,
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL) -> int:
    if not isinstance(output, Path):
        raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for mode in MODES:
        for users in USER_COUNTS:
            for size in STATE_SIZES:
                for level in LEVELS:
                    row = make_case(mode, users, size, level)
                    raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                    write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 16
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='16 long root-REVERT inputs: update/restore, users=accounts2/4, aggregate state8192/16384, own-N0/maximal-full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        modes=list(MODES), user_counts=list(USER_COUNTS), account_counts=list(USER_COUNTS),
        aggregate_state_sizes=list(STATE_SIZES), levels=list(LEVELS),
        holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Finite frozen-pricing composition validation. Independent review and all16 native/signature/authenticated-root/canonical-witness/original-real-quote gates precede freeze and actuals. Included failed user receipts are expected; whole-world fees/nonces/Anchor change. Preserve controls and failures. No refit, per-operation actual allocation or global bound.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
