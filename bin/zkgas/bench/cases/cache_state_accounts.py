#!/usr/bin/env python3
"""Prepare one bounded cross-transaction account-cache request; never execute it."""
import argparse
from collections import Counter
import hashlib
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import top_level_create

MODEL = 'af30c61656fd456ab290a837b44c181e4ff54e575a949d3579f07df20659a949'
USERS = 512
READS_PER_USER = 8
TRANSACTION_GAS_LIMIT = 50_000
HEADER_GAS_LIMIT = 31_000_000
BLOCK_GAS_BUDGET = 30_000_000
ANCHOR_GAS = 167_355
REFERENCE_DOMAIN = b'cache-state-absent-control\x00'
RECIPIENT_BASE = 0x4300000000000000000000000000000000000000
# The offline constructor's fixed initial accounts plus fee/proposer identities.
# Initial absence still requires the separate authenticated native/fullworld gate.
RESERVED = frozenset(('0x' + '66' * 20, '0x' + '88' * 20, '0x' + '99' * 20,
    '0x' + '33' * 20, '0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a',
    '0x1670000000000000000000000000000000010001',
    '0x0000777735367b36bc9b61c50022d9d0700db4ec'))


def _users(user_count):
    top_level_create.integer(user_count, 1, USERS, 'user count')
    return user_count


def _recipients(user_count):
    return tuple(f'0x{RECIPIENT_BASE + i:040x}' for i in range(1, _users(user_count) + 1))


def _validate_targets(targets, recipients):
    if type(targets) not in (list, tuple) or len(targets) != READS_PER_USER * len(recipients):
        raise ValueError('targets require exactly eight addresses per user')
    excluded = RESERVED.union(recipients)
    for address in targets:
        if type(address) is not str or re.fullmatch(r'0x[0-9a-f]{40}', address) is None:
            raise ValueError('targets require canonical lowercase20-byte addresses')
        if int(address, 16) <= 0xffff or address in excluded:
            raise ValueError('target conflicts with reserved, recipient or initial account')
    if len(set(targets)) != len(targets):
        raise ValueError('targets must be globally unique, including across users')
    return tuple(targets)


def _reference_targets(recipients):
    # Exactly one hash per index, no filtering, collision retry or seed selection.
    targets = ['0x' + hashlib.sha256(REFERENCE_DOMAIN + i.to_bytes(4, 'big')).digest()[:20].hex()
               for i in range(1, READS_PER_USER * len(recipients) + 1)]
    return _validate_targets(targets, recipients)


def _runtime(targets):
    asm = _Factory()
    for address in targets:
        asm.push(int(address, 16), 20); asm.op(0x31)
        asm.target('failure'); asm.op(0x57)
    asm.push(0, 1); asm.push(0, 1); asm.op(0xf3)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(user_count=USERS):
    """Fixed successful path with eight initially cold reads and empty calldata."""
    count = _users(user_count)
    gross = 21_000 + READS_PER_USER * (3 + 2600 + 3 + 10) + 6
    return dict(intrinsic_gas=21_000, calldata_floor_gas=21_000,
        cold_account_gas=READS_PER_USER * 2600, warm_account_gas=0,
        ordinary_opcode_gas=READS_PER_USER * 16 + 6, memory_expansion_gas=0,
        gross_user_gas=gross, charged_user_gas=gross, refund_counter=0, refund_applied=0,
        transaction_gas_limit=TRANSACTION_GAS_LIMIT, header_gas_limit=HEADER_GAS_LIMIT,
        charged_gas_budget=BLOCK_GAS_BUDGET, expected_anchor_gas=ANCHOR_GAS,
        expected_block_charged_gas=ANCHOR_GAS + count * gross,
        last_user_remaining_header_gas=HEADER_GAS_LIMIT - ANCHOR_GAS - (count - 1) * gross)


def make_case(*, targets=None, user_count=USERS):
    """One reference or explicit raw Address20 vector, always eight reads per user.

    Smaller user counts are engineering controls. Supplied targets are data, not
    an observed own-cache seed, placement or hash-concentration certificate.
    """
    recipients = _recipients(user_count)
    addresses = _reference_targets(recipients) if targets is None else _validate_targets(targets, recipients)
    contracts = []; routes = Counter()
    for index, recipient in enumerate(recipients):
        code, counts = _runtime(addresses[8 * index:8 * (index + 1)])
        contracts.append(dict(address=recipient, code_hex='0x' + code.hex()))
        routes.update(counts)
    row = top_level_create.make_case('call', 'stop', count=user_count, gas_limit=TRANSACTION_GAS_LIMIT)
    program = row['request']['program']
    # Preserve the existing selected zero terminal/call routes, then add all
    # executed user opcodes; the constructor separately observes Anchor execution.
    program['expected_route_counts'] = dict.fromkeys(program['expected_route_counts'], 0)
    program['expected_route_counts'].update(dict(routes))
    program['transactions']['recipients'] = list(recipients)
    program['prestate_contracts'] = contracts
    raw = b''.join(bytes.fromhex(address[2:]) for address in addresses)
    identity = digest(raw); sid = f'cache-state-accounts-u{user_count}-k8-{identity}'
    row.update(sample_id=sid, corpus_id='unzen-' + sid, baseline_sample_id=sid,
        family='cache-state-accounts', role='control' if targets is None else 'candidate',
        pair_group=f'cache-state-accounts-u{user_count}-k8-b1',
        data_role='prospective_cache_state_probe', holdout_only=False, actuals_used_for_fitting=False,
        completion_gate='All signed users/recipient code/order/nonces, authenticated target absence, exact routes, zero-balance guards, successful receipts, gas and canonical roots before actuals.',
        scope='Finite whole-block CacheState hypothesis. Per-TX journal and cross-TX cache differ; declared addresses are not measured physical population. No seed stability, dynamic placement/count, missing tariff, global-bound or SDK parity claim.')
    row['parameters'] = dict(transaction_kind='call', block_count=1, count_per_block=user_count,
        reads_per_user=READS_PER_USER, cold_reads_per_user=READS_PER_USER, warm_reads_per_user=0,
        expected_total_balance_reads=len(addresses), expected_caller_nonce_after=user_count,
        ordered_recipients=list(recipients), ordered_targets=list(addresses),
        ordered_targets_sha256=identity, address_encoding='20-byte network-order concatenation in raw order',
        catalog_source='sha256-domain-index-reference' if targets is None else 'explicit-raw-addresses',
        reference_domain_hex=REFERENCE_DOMAIN.hex() if targets is None else None,
        target_index_encoding='1..8*users as unsigned32-bit big-endian' if targets is None else None,
        configured_recipient_count=user_count, configured_recipient_code_bytes=215 * user_count,
        recipient_runtime_bytes=215, target_balance=0, target_initial_state='must be authenticated absent',
        workload_address_union=len(addresses) + user_count, workload_union_is_physical_population=False,
        pressure_hypothesis_realized=None, computed_catalog_is_observed_placement=False,
        expected_user_success=True, expected_exceptional_instructions=0, expected_return_bytes=0,
        expected_stack_at_return=[], outer_calldata_bytes=0, access_list=False,
        prestate_contract_nonce=1, prestate_contract_balance=0, prestate_contract_storage='empty',
        deployment_cost_scope='Predeployment is outside this measured block, not free over its lifecycle.',
        same_shape_comparison='Recipient/signer/nonces/user envelopes remain fixed; changed prestate runtime need not change user signed bytes or Kona payload. Authenticate each actual input independently.',
        **gas_accounting(user_count))
    return row


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation,
        'cases/spawn_memory.py': spawn_memory, 'cases/top_level_create.py': top_level_create}


def helpers():
    return {'initcode_preparation._Factory': _Factory,
        **{'cache_state_accounts.' + fn.__name__: fn for fn in
           (_users, _recipients, _validate_targets, _reference_targets, _runtime, gas_accounting, make_case)}}


def prepare(output: Path, *, targets=None, user_count=USERS, model_id=MODEL):
    """Write exactly one standard request/catalog to a fresh directory, without execution."""
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id requires lowercase SHA256')
    row = make_case(targets=targets, user_count=user_count)
    raw = encoded(row.pop('request')); relative = 'requests/' + row['sample_id'] + '.json'
    row.update(request_path=relative, request_sha256=digest(raw))
    catalog = dict(schema_version=1, corpus_id=row['corpus_id'], frozen_model_id=model_id,
        purpose='One finite cross-TX CacheState probe; generation is not qualification or execution.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(fn).encode()) for name, fn in helpers().items()},
        cases=[row])
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
