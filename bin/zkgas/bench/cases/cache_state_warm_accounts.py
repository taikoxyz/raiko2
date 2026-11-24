#!/usr/bin/env python3
"""Prepare one finite warm-first CacheState request; never execute it."""
import argparse
from collections import Counter
import inspect
import json
from pathlib import Path
import re

import cache_state_accounts as cold
import access_list_hash as owner
import top_level_create
from spawn_memory import digest, encoded, write

MODEL = cold.MODEL
USERS = 86
READS_PER_USER = 128
TRANSACTION_GAS_LIMIT = 400_000


def _users(user_count):
    return top_level_create.integer(user_count, 1, USERS, 'user count')


def _targets(targets, recipients):
    count = READS_PER_USER * len(recipients)
    if targets is None:
        targets = owner.concentrated_owner_addresses(0, count, prefix_bytes=6)
    if type(targets) not in (list, tuple) or len(targets) != count:
        raise ValueError('targets require exactly128 addresses per user')
    excluded = cold.RESERVED.union(recipients)
    for address in targets:
        if type(address) is not str or re.fullmatch(r'0x[0-9a-f]{40}', address) is None:
            raise ValueError('targets require canonical lowercase20-byte addresses')
        if int(address, 16) <= 0xffff or address in excluded:
            raise ValueError('target conflicts with recipient, reserved or initial account')
    if len(set(targets)) != len(targets):
        raise ValueError('targets must be globally unique across users')
    return tuple(targets)


def gas_accounting(user_count=USERS):
    """Source arithmetic; signed AL warmth and actual receipts remain separate gates."""
    count = _users(user_count)
    intrinsic = 21_000 + READS_PER_USER * 2400
    ordinary = READS_PER_USER * 16 + 6
    gross = intrinsic + READS_PER_USER * 100 + ordinary
    return dict(intrinsic_gas=intrinsic, calldata_floor_gas=21_000,
        cold_account_gas=0, warm_account_gas=READS_PER_USER * 100,
        ordinary_opcode_gas=ordinary, memory_expansion_gas=0,
        gross_user_gas=gross, charged_user_gas=gross, refund_counter=0, refund_applied=0,
        transaction_gas_limit=TRANSACTION_GAS_LIMIT, header_gas_limit=cold.HEADER_GAS_LIMIT,
        charged_gas_budget=cold.BLOCK_GAS_BUDGET, expected_anchor_gas=cold.ANCHOR_GAS,
        expected_block_charged_gas=cold.ANCHOR_GAS + count * gross,
        last_user_remaining_header_gas=cold.HEADER_GAS_LIMIT - cold.ANCHOR_GAS - (count - 1) * gross)


def make_case(*, targets=None, user_count=USERS):
    """One fixed public G0 control or explicit vector; generation certifies no own G/placement."""
    user_count = _users(user_count)
    recipients = cold._recipients(user_count)
    addresses = _targets(targets, recipients)
    row = top_level_create.make_case('call', 'stop', user_count, gas_limit=TRANSACTION_GAS_LIMIT)
    program = row['request']['program']; contracts = []; lists = []; counts = Counter()
    for index, recipient in enumerate(recipients):
        part = addresses[index * READS_PER_USER:(index + 1) * READS_PER_USER]
        code, routes = cold._runtime(part)
        counts.update(routes)
        contracts.append(dict(address=recipient, code_hex='0x' + code.hex()))
        lists.append(dict(pattern='raw_addresses', count=READS_PER_USER, addresses=list(part)))
    program['expected_route_counts'] = dict(counts)
    program['transactions'].update(recipients=list(recipients), access_lists=lists)
    program['prestate_contracts'] = contracts
    identity = digest(b''.join(bytes.fromhex(address[2:]) for address in addresses))
    sid = f'cache-state-warm-accounts-u{user_count}-k128-{identity}'
    row.update(sample_id=sid, corpus_id='unzen-' + sid, baseline_sample_id=sid,
        family='cache-state-warm-accounts', role='control' if targets is None else 'candidate',
        pair_group=f'cache-state-warm-accounts-u{user_count}-k128-b1',
        data_role='prospective_cache_state_probe', holdout_only=False, actuals_used_for_fitting=False,
        completion_gate='Authenticate every signed AL/recipient/nonce/order, absent target/world roots, successful guards, routes, receipts and gas before canonical actual.',
        scope='Finite warm-first cross-TX cache composition; AL warmth is not physical population or placement. No seed stability, whole pressure, missing tariff, global bound or SDK parity claim.')
    row['parameters'] = dict(transaction_kind='call', block_count=1, count_per_block=user_count,
        reads_per_user=READS_PER_USER, cold_reads_per_user=0, warm_reads_per_user=READS_PER_USER,
        expected_total_balance_reads=len(addresses), expected_caller_nonce_after=user_count,
        ordered_recipients=list(recipients), ordered_targets=list(addresses),
        ordered_targets_sha256=identity, address_encoding='20-byte network-order concatenation in raw order',
        catalog_source='fixed-g0-compressible-owner-control' if targets is None else 'explicit-raw-addresses',
        reference_shared0=0 if targets is None else None,
        reference_prefix_bytes=6 if targets is None else None,
        target_index_encoding='1..128*users as unsigned16-bit little-endian in first8-byte chunk' if targets is None else None,
        fixed_control_is_observed_seed=False,
        counterfactual_control='Fixed public G0 is neither randomized nor certified dispersed. New-input own G and blob fit require independent native/profile gates; changed AL values change signed payload.',
        configured_recipient_count=user_count, configured_recipient_code_bytes=3335 * user_count,
        recipient_runtime_bytes=3335, target_balance=0, target_initial_state='must be authenticated absent',
        access_list=True, access_list_mode='per_transaction', raw_address_items=len(addresses),
        raw_storage_keys=0, unique_addresses=len(addresses), unique_storage_pairs=0,
        workload_address_union=len(addresses) + user_count, workload_union_is_physical_population=False,
        pressure_hypothesis_realized=None, computed_catalog_is_observed_placement=False,
        expected_user_success=True, expected_return_bytes=0, outer_calldata_bytes=0,
        expected_exceptional_instructions=0, expected_stack_at_return=[],
        prestate_contract_nonce=1, prestate_contract_balance=0, prestate_contract_storage='empty',
        deployment_cost_scope='Predeployment is outside this measured block, not free over its lifecycle.',
        same_shape_comparison='Changed runtime immediates and AL values change request data; authenticate signed payload and roots independently.',
        **gas_accounting(user_count))
    return row


def source_modules():
    return {'cases/cache_state_accounts.py': cold, 'cases/access_list_hash.py': owner, **cold.source_modules()}


def helpers():
    return {**cold.helpers(), 'access_list_hash.concentrated_owner_addresses': owner.concentrated_owner_addresses,
        **{'cache_state_warm_accounts.' + fn.__name__: fn for fn in
        (_users, _targets, gas_accounting, make_case)}}


def prepare(output: Path, *, targets=None, user_count=USERS, model_id=MODEL):
    """Write one standard catalog/request to a fresh directory without executing it."""
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id requires lowercase SHA256')
    row = make_case(targets=targets, user_count=user_count)
    raw = encoded(row.pop('request')); relative = 'requests/' + row['sample_id'] + '.json'
    row.update(request_path=relative, request_sha256=digest(raw))
    catalog = dict(schema_version=1, corpus_id=row['corpus_id'], frozen_model_id=model_id,
        purpose='One finite warm-first CacheState hypothesis; data generation is not qualification or execution.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(fn).encode()) for name, fn in helpers().items()}, cases=[row])
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir()
    write(output/relative, raw); write(output/'catalog.json', encoded(catalog))
    return 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    parser.add_argument('--user-count', type=int, default=USERS)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id, user_count=args.user_count)}))
