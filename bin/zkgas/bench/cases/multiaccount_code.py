#!/usr/bin/env python3
"""Generate36 predeployed multi-account code-loading probes and own-N0 controls."""
import argparse
from collections import Counter
from functools import lru_cache
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
TARGET_COUNTS = (16, 128, 512)
CODE_LENGTHS = (32, 4096, 24576)
BASE_ADDRESS = 0x2000000000000000000000000000000000010000
CHILD_PREFIX = bytes.fromhex('3060005260206000f3')
USERS = 4
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
# Ordinary opcodes only. CALL/DELEGATECALL's100/2600 access charge and memory expansion are
# accounted separately; no value transfer, calldata, refund or deployment here.
STATIC_GAS = {0x00: 0, 0x01: 3, 0x14: 3, 0x15: 3, 0x16: 3, 0x30: 2,
              0x3d: 2, 0x50: 2, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10,
              0x5a: 2, 0x5b: 1, 0xf1: 0, 0xf4: 0, 0xf3: 0, 0xfe: 0}
STATIC_GAS.update({op: 3 for op in range(0x60, 0x90)})


def address(index):
    return f'0x{BASE_ADDRESS + index:040x}'


def child_code(length, variant, index):
    """Identical executed prefix; fixed-width salt lies after the terminal RETURN."""
    original.integer(length, 32, 24576, 'child runtime bytes')
    if length not in CODE_LENGTHS: raise ValueError('unsupported child runtime bytes')
    if type(variant) is not str or variant not in ('shared', 'unique'): raise ValueError('unknown code variant')
    original.integer(index, 0, 511, 'child index')
    salt = (index + 1 if variant == 'unique' else 0).to_bytes(4, 'big')
    return CHILD_PREFIX + bytes(length - len(CHILD_PREFIX) - 4) + salt


def validate_child_call_kind(child_call_kind):
    if type(child_call_kind) is not str or child_call_kind not in ('call', 'delegatecall'):
        raise ValueError('child_call_kind must be call or delegatecall')


def parent_code(targets, calls, *, child_call_kind='call'):
    """A fixed single-site loop, including N0; only PUSH3 bound immediates vary."""
    validate_child_call_kind(child_call_kind)
    original.integer(targets, 1, 512, 'target count')
    if targets not in (1,) + TARGET_COUNTS: raise ValueError('unsupported target count')
    original.integer(calls, 0, 65535, 'calls per user')
    a = _Factory(); a.push(0)
    a.label('loop', calls + 1); a.op(0x80, calls + 1); a.push(calls, times=calls + 1)
    a.op(0x14, calls + 1); a.target('done', calls + 1); a.op(0x57, calls + 1)
    a.op(0x80, calls); a.push(targets - 1, 2, calls); a.op(0x16, calls)
    a.push(BASE_ADDRESS, 20, calls); a.op(0x01, calls); a.push(64, 1, calls); a.op(0x52, calls)
    # Reversed stack operands: output, input, optional CALL value, target, gas.
    operands = (32, 32, 0, 0, 0, 64) if child_call_kind == 'call' else (32, 32, 0, 0, 64)
    for value in operands: a.push(value, 1, calls)
    a.op(0x51, calls); a.op(0x5a, calls); a.op(0xf1 if child_call_kind == 'call' else 0xf4, calls)
    a.op(0x15, calls); a.target('failure', calls); a.op(0x57, calls)
    a.op(0x3d, calls); a.push(32, 1, calls); a.op(0x14, calls)
    a.op(0x15, calls); a.target('failure', calls); a.op(0x57, calls)
    a.push(32, 1, calls); a.op(0x51, calls)
    if child_call_kind == 'call': a.push(64, 1, calls); a.op(0x51, calls)
    else: a.op(0x30, calls)  # Delegate ADDRESS is this parent's context, not the code target.
    a.op(0x14, calls); a.op(0x15, calls); a.target('failure', calls); a.op(0x57, calls)
    a.push(1, 1, calls); a.op(0x01, calls); a.target('loop', calls); a.op(0x56, calls)
    a.label('done'); a.op(0x80); a.push(calls); a.op(0x14); a.op(0x15)
    a.target('failure'); a.op(0x57); a.op(0x50); a.op(0)
    a.label('failure', 0); a.op(0xfe, 0)
    return a.finish(), a.counts


def gas_components(targets, calls, *, child_call_kind='call'):
    _, counts = parent_code(targets, calls, child_call_kind=child_call_kind)
    parent = sum(STATIC_GAS[int(key[-2:], 16)] * n for key, n in counts.items())
    return {'intrinsic': 21000, 'parent_static': parent,
            'parent_memory_expansion': 9 if calls else 0,
            'call_cold': 2600 * min(calls, targets),
            'call_warm': 100 * max(0, calls - targets),
            'child_static': 14 * calls, 'child_memory_expansion': 3 * calls}


@lru_cache(maxsize=8)
def full_bound(targets, *, child_call_kind='call'):
    low, high = 0, 65535
    while low < high:
        mid = (low + high + 1) // 2
        if ANCHOR_GAS + USERS * sum(gas_components(targets, mid, child_call_kind=child_call_kind).values()) <= BLOCK_BUDGET: low = mid
        else: high = mid - 1
    assert low > targets
    return low


def sample_id(targets, length, variant, level, *, child_call_kind='call'):
    validate_child_call_kind(child_call_kind)
    prefix = 'multiaccount-code' if child_call_kind == 'call' else 'multiaccount-delegatecall-code'
    return f'{prefix}-a{targets}-l{length}-{variant}-{level}'


def make_case(targets=16, length=32, variant='shared', level='full', *, calls=None, child_call_kind='call'):
    validate_child_call_kind(child_call_kind)
    original.integer(targets, 1, 512, 'target count')
    if targets not in (1,) + TARGET_COUNTS: raise ValueError('unsupported target count')
    if type(level) is not str or level not in ('zero', 'full'): raise ValueError('unknown workload level')
    # Explicit calls is only a native construction/cache sanity control and is
    # never part of the36-case engineering matrix.
    if calls is not None:
        original.integer(calls, 0, 65535, 'calls per user')
        if targets != 1 or level != 'full': raise ValueError('explicit calls require one-target native control')
    full = full_bound(targets, child_call_kind=child_call_kind); n = (0 if level == 'zero' else full) if calls is None else calls
    contracts = [{'address': address(i), 'code_hex': '0x' + child_code(length, variant, i).hex()} for i in range(targets)]
    code, parent_counts = parent_code(targets, n, child_call_kind=child_call_kind)
    routes = Counter({key: value * USERS for key, value in parent_counts.items()})
    for op, count in {0x30: 1, 0x60: 3, 0x52: 1, 0xf3: 1}.items(): routes[f'opcode:0x{op:02x}'] += USERS * n * count
    gas = gas_components(targets, n, child_call_kind=child_call_kind); user_gas = sum(gas.values())
    envelope = sum(gas_components(targets, full, child_call_kind=child_call_kind).values()) + 1000
    row = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)
    row['request']['program'].update(code_hex='0x' + code.hex(), prestate_contracts=contracts,
                                     expected_route_counts=dict(sorted(routes.items())))
    sid = sample_id(targets, length, variant, level, child_call_kind=child_call_kind)
    if calls is not None: sid += f'-native-calls{n}'
    row.update(sample_id=sid, family='multiaccount-code-loading', role='control' if n == 0 else 'candidate',
        baseline_sample_id=sample_id(targets, length, variant, 'zero', child_call_kind=child_call_kind),
        data_role='prospective_validation_probe', holdout_only=True,
        completion_gate='independent concrete target/order/ADDRESS return/status/length/counter and gas oracle; four signed successful CALL users; minimal authenticated accounts and hash-deduplicated code',
        scope='Finite whole-proposal predeployed-code observations under unchanged pricing. Deployment and setup lifecycle cost are outside this measured block. No isolated byte price, universal bound, capacity or per-transaction actual claim.')
    row['parameters'] = {'target_count': targets, 'runtime_bytes_per_target': length, 'code_variant': variant,
        'workload_level': level, 'calls_per_user': n, 'full_calls_per_user': full, 'unroll_factor': 1,
        'target_base_address': address(0), 'target_derivation': '(counter AND (target_count-1)) ADD base_address',
        'child_executed_prefix_hex': '0x' + CHILD_PREFIX.hex(), 'unreachable_salt_bytes': 4,
        'expected_configured_unique_code_count': targets if variant == 'unique' else 1,
        'expected_witnessed_target_accounts': targets if n else 0,
        'expected_witnessed_target_code_count': (targets if variant == 'unique' else 1) if n else 0,
        'expected_witnessed_target_code_bytes': ((targets if variant == 'unique' else 1) * length) if n else 0,
        'expected_cold_calls_per_user': min(n, targets), 'expected_warm_calls_per_user': max(0, n - targets),
        'warmth_reset_each_user': True, 'expected_ending_counter_per_user': n,
        'parent_runtime_bytes': len(code), 'parent_runtime_sha256': digest(code),
        'expected_completed_user_opcodes': sum(routes.values()), 'expected_user_success': True,
        'expected_exceptional_instructions': 0, 'transaction_kind': 'call', 'count_per_block': USERS,
        'block_count': 1, 'outer_calldata_bytes': 0, 'transaction_gas_limit': envelope,
        'gas_components_per_user': gas, 'expected_user_charged_gas': user_gas,
        'expected_anchor_gas': ANCHOR_GAS, 'expected_block_charged_gas': ANCHOR_GAS + USERS * user_gas,
        'header_gas_limit': HEADER_GAS_LIMIT, 'charged_gas_budget': BLOCK_BUDGET,
        'expected_remaining_header_gas_before_users': [HEADER_GAS_LIMIT - ANCHOR_GAS - i * user_gas for i in range(USERS)],
        'expected_refund': 0, 'calldata_floor_applies': False, 'prestate_contract_nonce': 1,
        'prestate_contract_balance': 0, 'prestate_contract_storage': 'empty',
        'deployment_and_setup_cost': 'outside measured block; not free over contract lifecycle',
        'native_only_control': calls is not None or targets == 1}
    if child_call_kind == 'delegatecall':
        row['parameters'].update(child_call_kind=child_call_kind, child_address_context='parent',
                                 expected_child_address_return='0x' + '66' * 20)
        row['completion_gate'] = ('independent ordered code-target identity/hash from actual bytecode, native reads and raw witness; '
            'child ADDRESS returns parent context, not code-target identity; status/length/counter and gas oracle; '
            'four signed successful CALL users; minimal authenticated accounts and hash-deduplicated code')
    return row


def prepare(output, *, model_id=MODEL, child_call_kind='call'):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be a lowercase SHA256')
    validate_child_call_kind(child_call_kind)
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for targets in TARGET_COUNTS:
        for length in CODE_LENGTHS:
            for variant in ('shared', 'unique'):
                for level in ('zero', 'full'):
                    row = make_case(targets, length, variant, level, child_call_kind=child_call_kind)
                    raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                    write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len(rows) == len({r['sample_id'] for r in rows}) == 36
    write(output / 'catalog.json', encoded({'schema_version': 1, 'frozen_model_id': model_id,
        **({'child_call_kind': child_call_kind} if child_call_kind == 'delegatecall' else {}),
        'purpose': 'Multi-account predeployed shared/unique runtime code loading with18 exact own-N0 pairs',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        'required_comparison_quantities': ['complete proposal zkGas estimate', 'node-observable estimate for each block and their literal sum', 'actual same-proposal SP1 PGU'],
        'interpretation': 'Preserve all controls and literal actual/node/full estimates. Strictly above2B missing node PGU is high priority. No quiet refit, deployment, limit change or lifecycle-cost omission.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('--output-dir', type=Path, required=True)
    p.add_argument('--model-id', default=MODEL); p.add_argument('--child-call-kind', choices=('call', 'delegatecall'), default='call'); args = p.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id, child_call_kind=args.child_call_kind)}))
