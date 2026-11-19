#!/usr/bin/env python3
"""Bounded same-owner caught transient rollback recipe; no execution on import."""
from collections import Counter

from initcode_preparation import _Factory
from spawn_memory import digest
import storage_caught_nesting as caught
import storage_mutation as mutation
import top_level_create as original
import transient_read_catalogs as transient

PARENT = '0x' + '65' * 20
LEAF = '0x' + '67' * 20
INPUT_OFFSET = 128
USER_SOURCE_CAP = 12_500_000
USER_ENVELOPE = USER_SOURCE_CAP + 1000
ORDINARY_COSTS = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x36: 2, 0x37: 3,
                  0x3d: 2, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10,
                  0x5b: 1, 0x5c: 0, 0x5d: 0, 0xf4: 0, 0xfe: 0}


def _parent(key_count, caught_calls, query):
    """Copy calldata outside counters once; catch each delegate REVERT explicitly."""
    asm = _Factory()
    asm.op(0x36); mutation._check(asm, 32 * key_count)
    asm.push(32 * key_count); asm.push(0); asm.push(INPUT_OFFSET); asm.op(0x37)
    asm.push(0); asm.op(0x5c); mutation._check(asm, 0)
    asm.push(1); asm.push(0); asm.op(0x5d)
    asm.push(0); asm.push(0); asm.op(0x52)
    visits = caught_calls + 1
    asm.label('loop', visits)
    asm.push(0, times=visits); asm.op(0x51, visits)
    asm.push(caught_calls, times=visits); asm.op(0x14, visits)
    asm.target('done', visits); asm.op(0x57, visits)
    caught._emit_call(asm, LEAF, 0, call_kind='delegatecall',
                      input_offset=INPUT_OFFSET, input_size=32 * key_count, times=caught_calls)
    asm.push(query, 32, caught_calls); asm.op(0x5c, caught_calls); mutation._check(asm, 0, caught_calls)
    asm.push(0, times=caught_calls); asm.op(0x5c, caught_calls); mutation._check(asm, 1, caught_calls)
    asm.push(0, times=caught_calls); asm.op(0x51, caught_calls)
    asm.push(1, times=caught_calls); asm.op(1, caught_calls)
    asm.push(0, times=caught_calls); asm.op(0x52, caught_calls)
    asm.target('loop', caught_calls); asm.op(0x56, caught_calls)
    asm.label('done'); asm.push(0); asm.op(0x51); mutation._check(asm, caught_calls)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _ordinary(counts):
    return sum(n * (3 if 0x60 <= (op := int(route[-2:], 16)) <= 0x7f else ORDINARY_COSTS[op])
               for route, n in counts.items())


def _gas(data, counts, leaf_execution, calls):
    """Charge outer calldata/floor once; a child never pays intrinsic or floor gas."""
    words = (INPUT_OFFSET + len(data) + 31) // 32
    memory = 3 * words + words * words // 512
    nz = len(data) - data.count(0); z = len(data) - nz
    intrinsic = 21000 + 4 * z + 16 * nz
    floor = 21000 + 10 * (z + 4 * nz)
    ordinary = _ordinary(counts)
    loads = 100 * counts['opcode:0x5c']; stores = 100 * counts['opcode:0x5d']
    copy = 3 * ((len(data) + 31) // 32)
    target = 2600 + 100 * (calls - 1)
    gross = intrinsic + ordinary + memory + copy + loads + stores + target + calls * leaf_execution
    charged = max(gross, floor)
    # Even the last call receives enough gas: bound pre-call spent by the entire
    # successful transaction, hence this reserve is deliberately stronger than needed.
    remaining = USER_ENVELOPE - gross
    guaranteed_forwarded = min(caught.CHILD_GAS, remaining - remaining // 64)
    if charged > USER_SOURCE_CAP or guaranteed_forwarded <= leaf_execution:
        raise ValueError('fixed user source budget or EIP150 forwarding fails')
    return dict(intrinsic_gas=intrinsic, eip7623_floor_gas=floor,
                ordinary_parent_gas=ordinary, parent_memory_gas=memory, calldata_copy_gas=copy,
                parent_transient_load_gas=loads, parent_transient_store_gas=stores,
                call_target_gas=target, leaf_execution_gas=leaf_execution,
                all_leaf_execution_gas=calls * leaf_execution, gross_gas=gross,
                charged_gas=charged, refund_counter=0, refund_applied=0,
                minimum_guaranteed_child_gas=guaranteed_forwarded)


def make_case(*, raw_key_layout, key_count=512, caught_calls=128, user_count=2):
    """Explicit finite geometry; no count, seed, address or gas-budget search."""
    k = original.integer(key_count, 1, 512, 'key_count')
    calls = original.integer(caught_calls, 1, 128, 'caught_calls')
    users = original.integer(user_count, 1, 2, 'user_count')
    if type(raw_key_layout) is not dict or set(raw_key_layout) != {'L1', 'L3'}:
        raise ValueError('raw_key_layout requires exactly L1 and L3')
    word = original.integer(raw_key_layout['L1'], 0, (1 << 64) - 1, 'L1')
    original.integer(raw_key_layout['L3'], 0, 0, 'L3')
    keys = [f'0x{(i << 24) | (word << 64):064x}' for i in range(1, k + 1)]
    data = b''.join(bytes.fromhex(key[2:]) for key in keys); query = int(keys[-1], 16)
    leaf, leaf_counts = transient._runtime(0, key_count=k, query_key=query, terminal='revert')
    leaf_gas = transient.gas_accounting(0, key_count=k, calldata=data, query_key=query, terminal='revert')
    parent, counts = _parent(k, calls, query)
    gas = _gas(data, counts, leaf_gas['gross_gas'] - leaf_gas['intrinsic_gas'], calls)
    prefixes = [30_000_000 - i * gas['charged_gas'] for i in range(users)]
    if (max(len(parent), len(leaf)) > 24576 or len(data) > 131072
            or USER_ENVELOPE > min(15_000_000, 16_777_216)
            or 2_000_000 + users * USER_ENVELOPE > 30_000_000
            or any(prefix < USER_ENVELOPE for prefix in prefixes)):
        raise ValueError('source proposal/header/code/transaction bounds fail')
    routes = Counter(counts)
    routes.update({op: n * calls for op, n in leaf_counts.items()})
    request = dict(schema_version=1, expected_user_success=True, program=dict(
        code_hex='0x00', input_hex='0x' + data.hex(), block_count=2,
        expected_route_counts={op: n * users for op, n in routes.items()},
        transactions=dict(kind='call', counts_by_block=[0, users], gas_limit=USER_ENVELOPE,
                          recipients=[PARENT] * users),
        prestate_contracts=[dict(address=PARENT, code_hex='0x' + parent.hex(), initial_storage={}),
                            dict(address=LEAF, code_hex='0x' + leaf.hex(), initial_storage={})]))
    sid = f'transient-caught-k{k}-{digest(data)}-c{calls}-u{users}'
    return dict(sample_id=sid, baseline_sample_id=sid, family='transient-caught-rollback',
        corpus_id='unzen-transient-caught-rollback', role='candidate',
        data_role='opened_engineering_recipe', holdout_only=False, actuals_used_for_fitting=False,
        scope='Whole-input actual/node comparison; no observed transient placement, isolated unwind cost or global bound.',
        completion_gate='Independent byte/gas/checkpoint/signature/world/witness/current quote gates before actual.',
        parameters=dict(key_count=k, caught_calls=calls, raw_key_layout={'L1': word, 'L3': 0},
            ordinal_shift=24, raw_key_rule='(i<<24)|(L1<<64), i1..K; sentinel0 disjoint; L2=L3=0',
            ordered_keys=keys, query_key=keys[-1], calldata_sha256=digest(data),
            outer_calldata_bytes=len(data), calldata_zero_bytes=data.count(0),
            calldata_nonzero_bytes=len(data)-data.count(0), calldata_memory_offset=INPUT_OFFSET,
            parent_owner=PARENT, leaf_code_address=LEAF, transient_sentinel_key='0x0',
            transient_sentinel_value=1, all_user_transactions=users, user_transactions_by_block=[0, users],
            all_transactions_including_anchor=users+2, transaction_nonce=0, recipient_schedule=[PARENT]*users,
            transaction_gas_limit=USER_ENVELOPE, expected_user_gross_gas=gas['gross_gas'],
            expected_user_charged_gas=gas['charged_gas'], gas_accounting_per_user=gas,
            expected_transient_reads_per_user=1+4*calls, expected_transient_writes_per_user=1+k*calls,
            peak_transient_entries_per_user=k+1, persistent_reads=0, persistent_writes=0,
            expected_final_transient_entries=0, expected_parent_transient_before_tx_end={'0x0': '0x1'},
            expected_user_receipt_statuses=[True]*users, expected_child_status=0,
            expected_leaf_terminal='REVERT', expected_leaf_return_bytes=0, expected_terminal='STOP',
            expected_user_refund_counter=0, expected_user_refund_applied=0,
            parent_runtime_sha256=digest(parent), leaf_runtime_sha256=digest(leaf),
            parent_runtime_bytes=len(parent), leaf_runtime_bytes=len(leaf),
            source_user_charged_gas_cap=USER_SOURCE_CAP, anchor_gas_limit_upper_bound=1_000_000,
            source_proposal_charged_gas_upper_bound=2_000_000+users*gas['charged_gas'],
            source_proposal_envelope_upper_bound=2_000_000+users*USER_ENVELOPE,
            block_charged_gas_upper_bounds=[1_000_000, 1_000_000+users*gas['charged_gas']],
            header_prefix_remaining_gas=prefixes, execution_profile='evm-gas-only'), request=request)
