#!/usr/bin/env python3
"""Generate 24 finite initialized-storage fanout inputs with own-N0 controls."""
import argparse
import inspect
import json
from pathlib import Path
import re

from spawn_memory import digest, encoded, write
import storage_mutation as mutation
import top_level_create as original

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
CORPUS = 'unzen-storage-fanout-20261102'
ACCOUNT_COUNTS = (1, 8, 64)
STATE_SIZES = (8192, 16384)
MODES = ('update', 'delete')
LEVELS = ('zero', 'full')
USERS = 64
BASE_ADDRESS = 0x3000000000000000000000000000000000010000
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
MAX_DISTINCT_PAIRS = 8192
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
ENVELOPE_RESERVE = 1000


def _validate(accounts, state_size, mode):
    if type(accounts) is not int or accounts not in ACCOUNT_COUNTS:
        raise ValueError('accounts must be 1, 8 or 64')
    if type(state_size) is not int or state_size not in STATE_SIZES:
        raise ValueError('state_size must be 8192 or 16384')
    if type(mode) is not str or mode not in MODES:
        raise ValueError('mode must be update or delete')


def fits(accounts, state_size, mode, n):
    """Check this candidate's own envelope and every ordered admission prefix."""
    _validate(accounts, state_size, mode)
    original.integer(n, 0, MAX_DISTINCT_PAIRS, 'mutations per user')
    if USERS * n + accounts > MAX_DISTINCT_PAIRS:
        return False
    if (USERS // accounts) * n > state_size // accounts - 1:
        return False
    gas = mutation.gas_accounting(mode, n)
    envelope = gas['gross_gas'] + ENVELOPE_RESERVE
    return (envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope
                for j in range(USERS)))


def full_n(accounts, state_size, mode):
    _validate(accounts, state_size, mode)
    low, high = 0, MAX_DISTINCT_PAIRS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(accounts, state_size, mode, middle): low = middle
        else: high = middle - 1
    assert fits(accounts, state_size, mode, low) and not fits(accounts, state_size, mode, low + 1)
    return low


def make_case(accounts=1, state_size=8192, mode='update', level='full'):
    _validate(accounts, state_size, mode)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('level must be zero or full')
    maximum = full_n(accounts, state_size, mode)
    n = 0 if level == 'zero' else maximum
    gas = mutation.gas_accounting(mode, n)
    envelope = mutation.gas_accounting(mode, maximum)['gross_gas'] + ENVELOPE_RESERVE
    code, counts = mutation._runtime(mode, n)
    # The existing producer supplies only the schema. Its creation metadata is
    # deliberately discarded, and its unused primary runtime stays one STOP.
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']
    program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({route: count * USERS for route, count in counts.items()})
    addresses = [f'0x{BASE_ADDRESS + i:040x}' for i in range(accounts)]
    recipients = [addresses[j % accounts] for j in range(USERS)]
    slots = state_size // accounts; visits = USERS // accounts
    program['expected_route_counts'] = routes
    program['transactions']['recipients'] = recipients
    program['prestate_contracts'] = [dict(address=address, code_hex='0x' + code.hex(),
        initial_storage={hex(key): '0x1' for key in range(slots)}) for address in addresses]
    sid = f'storage-fanout-a{accounts}-s{state_size}-{mode}-n{n}'
    return dict(sample_id=sid, family='storage-fanout', corpus_id=CORPUS,
        role='control' if n == 0 else 'candidate',
        baseline_sample_id=f'storage-fanout-a{accounts}-s{state_size}-{mode}-n0',
        data_role='unfitted_storage_fanout_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all24 native, real-quote, authenticated initial/final roots, signed recipient order and concrete raw-slot semantics gates precede actuals. Native success alone is not the semantic oracle.',
        scope='Finite initialized-storage fanout under frozen c893 pricing; preserve own-N0 controls and failures. Compare whole-proposal actual SP1 PGU, literal per-block node estimates/sum and complete proposal quote separately. No refit, universal MPT bound, per-user actual attribution, generated proof or throughput claim.',
        parameters=dict(account_count=accounts, aggregate_state_size=state_size, state_size_per_account=slots,
            mode=mode, workload_level=level, mutations_per_user=n, full_mutations_per_user=maximum,
            user_transactions_per_block=USERS, all_user_transactions=USERS, visits_per_account=visits,
            block_count=1, all_transactions_including_anchor=USERS + 1, transaction_kind='call',
            transaction_gas_limit=envelope, transaction_envelope_reserve=ENVELOPE_RESERVE,
            header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=gas['refund_counter'],
            expected_user_refund_applied=gas['refund_applied'], expected_user_charged_gas=gas['charged_gas'],
            expected_block_charged_gas=ANCHOR_GAS + USERS * gas['charged_gas'], gas_accounting_per_user=gas,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            expected_distinct_account_slot_pairs=USERS * n + accounts,
            expected_storage_writes=USERS * (n + 1), expected_main_storage_writes=USERS * n,
            expected_phase_storage_writes=USERS, expected_main_storage_writes_per_account=visits * n,
            expected_storage_writes_per_account=visits * (n + 1),
            expected_initial_storage_entry_count=state_size,
            expected_final_storage_entry_count=state_size - (USERS * n if mode == 'delete' else 0),
            expected_initial_storage_entry_count_per_account=slots,
            expected_final_storage_entry_count_per_account=slots - (visits * n if mode == 'delete' else 0),
            expected_distinct_storage_keys_per_account=visits * n + 1,
            expected_final_phase_per_account=1 + visits * n,
            recipient_schedule=recipients.copy(), start_phase_schedule=[1 + (j // accounts) * n for j in range(USERS)],
            final_phase_schedule=[1 + (j // accounts + 1) * n for j in range(USERS)],
            phase_slot=0, initial_phase=1, expected_counter=n, expected_checksum=0 if mode == 'delete' else 2 * n,
            counter_memory_offset=0, checksum_memory_offset=32, phase_memory_offset=64, key_memory_offset=96,
            initial_storage_rule='Each account has exactly raw slots 0..S/A-1 equal to1; all other raw slots are absent. Slot0 is the phase.',
            range_rule='User j calls account j%A; start=1+floor(j/A)*N. Visit start+i ascending and commit start+N to raw slot0 of that recipient.',
            witness_policy='Canonical mutation-aware raw-slot witnesses per recipient; aggregate limit counts distinct (account,raw-slot) pairs. No padding or hashed-key injection.',
            expected_user_success=True, expected_exceptional_instructions=0, outer_calldata_bytes=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False,
            runtime_bytes=len(code), runtime_sha256=digest(code), shared_runtime=True,
            unused_primary_code_hex='0x00'), request=request)


def prepare(output: Path, *, model_id=MODEL) -> int:
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'requests').mkdir()
    rows = []
    for accounts in ACCOUNT_COUNTS:
        for size in STATE_SIZES:
            for mode in MODES:
                for level in LEVELS:
                    row = make_case(accounts, size, mode, level)
                    raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                    write(output / path, raw)
                    rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 24
    sources = ('storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='24 initialized-storage fanout inputs: accounts1/8/64, aggregate state8192/16384, update/delete and own-N0/maximal-full pairs.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in sources},
        helper_source_sha256={name: digest(inspect.getsource(getattr(mutation, name)).encode()) for name in ('_runtime', 'gas_accounting')},
        account_counts=list(ACCOUNT_COUNTS), aggregate_state_sizes=list(STATE_SIZES), modes=list(MODES),
        levels=list(LEVELS), users=USERS, holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Frozen pricing validation only. Independent source and all24 native, real-quote, authenticated initial/final roots and signed recipient-order gates precede actuals. Native success is insufficient. Preserve failures and both deficit flags; 30M charged budget and31M header remain separate.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
