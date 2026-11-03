#!/usr/bin/env python3
"""Generate16 separate-leaf CALL chains with caught terminal rollback controls."""
import argparse
from collections import Counter
import inspect
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
from storage_fanout import BASE_ADDRESS, MODEL
import storage_dirty_keys as dirty
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-storage-caught-nesting-20261103'
MODES = ('stop', 'revert')
KEY_COUNTS = (16, 1024)
DEPTHS = (1, 4)
LEVELS = ('zero', 'full')
USERS = 4
CHILD_GAS = 12_000_000
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
MAX_DISTINCT_PAIRS = 8192
ENVELOPE_RESERVE = 2301
SOURCES = ('storage_dirty_keys.py', 'storage_fanout.py', 'storage_mutation.py',
           'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')


def _validate(mode, keys, depth):
    if type(mode) is not str or mode not in MODES: raise ValueError('mode must be stop or revert')
    if type(keys) is not int or keys not in KEY_COUNTS: raise ValueError('keys must be16 or1024')
    if type(depth) is not int or depth not in DEPTHS: raise ValueError('depth must be1 or4')


def address(user, level=None):
    """Four leaf owners, with separate disjoint wrapper levels numbered top first."""
    return f'0x{BASE_ADDRESS + user + (0 if level is None else 0x10000 + level * 0x100):040x}'


def _wrapper(target, expected_status):
    asm = _Factory()
    for _ in range(5): asm.push(0)  # out size/offset, in size/offset, value
    asm.push(int(target, 16), width=20); asm.push(CHILD_GAS); asm.op(0xf1)
    mutation._check(asm, expected_status)
    asm.op(0x3d); mutation._check(asm, 0); asm.op(0)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _wrapper_gas():
    """Derive both ordinary regions from emitted assembly; CALL target is cold."""
    code, counts = _wrapper(address(0), 1); pc = 0; pre = post = 0; seen_call = False
    costs = {0: 0, 0x14: 3, 0x15: 3, 0x3d: 2, 0x57: 10, 0x5b: 1, 0xfe: 0}
    derived = Counter()
    while pc < len(code):
        op = code[pc]; pc += 1
        if op == 0x5b: break  # failure label is unreachable
        derived[f'opcode:0x{op:02x}'] += 1
        if op == 0xf1: seen_call = True; continue
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        pc += width; cost = 3 if width else costs[op]
        if seen_call: post += cost
        else: pre += cost
    assert derived == +counts and (pre, post) == (21, 46)
    return pre, 2600, post


def gas_accounting(mode, keys, depth, rounds):
    _validate(mode, keys, depth); original.integer(rounds, 0, 65536, 'rounds per key')
    leaf = dirty.gas_accounting(mode, keys, rounds)
    pre, cold, post = _wrapper_gas(); gross = leaf['gross_gas'] + depth * (pre + cold + post)
    refund = leaf['refund_counter']; applied = min(refund, gross // 5)
    required = leaf['gross_gas'] - 21000 + ENVELOPE_RESERVE; required_by_depth = [required]
    for _ in range(depth):
        required = pre + cold + max((64 * required + 62) // 63, required + post)
        required_by_depth.append(required)
    return dict(intrinsic_gas=21000, leaf_gross_gas=leaf['gross_gas'], leaf_execution_gas=leaf['gross_gas']-21000,
        leaf_gas_accounting=leaf, wrapper_ordinary_pre=pre, wrapper_cold_target_gas=cold,
        wrapper_ordinary_post=post, wrapper_gross_gas=pre+cold+post,
        wrapper_total_gas=depth*(pre+cold+post), gross_gas=gross,
        refund_counter_before_terminal=refund,
        leaf_refund_counter_before_terminal=leaf['refund_counter_before_terminal'], refund_counter=refund,
        refund_applied=applied, charged_gas=gross-applied, child_gas_request=CHILD_GAS,
        minimum_frame_envelopes_leaf_to_root=required_by_depth, transaction_envelope=21000+required)


def fits(mode, keys, depth, rounds):
    gas = gas_accounting(mode, keys, depth, rounds); envelope = gas['transaction_envelope']
    code, _ = dirty._runtime(mode, keys, rounds)
    return (USERS*(keys+1) <= MAX_DISTINCT_PAIRS and len(code) <= 24576
        and envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and max(gas['minimum_frame_envelopes_leaf_to_root'][:-1]) <= CHILD_GAS
        and ANCHOR_GAS+USERS*gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT-ANCHOR_GAS-j*gas['charged_gas'] >= envelope for j in range(USERS)))


def full_rounds(mode, keys, depth):
    _validate(mode, keys, depth); low, high = 0, 65536
    while low < high:
        middle = (low+high+1)//2
        if fits(mode, keys, depth, middle): low = middle
        else: high = middle-1
    assert 0 < low < 65536 and fits(mode, keys, depth, low) and not fits(mode, keys, depth, low+1)
    return low


def _make_case(mode, keys, depth, rounds, maximum):
    """Internal explicit-R construction also supports execution of rejected R+1."""
    _validate(mode, keys, depth)
    original.integer(rounds, 0, 65536, 'rounds per key'); original.integer(maximum, 0, 65536, 'full rounds')
    # Preserve the original maps and invariant metadata, replacing every R-dependent field.
    row = dirty.make_case(mode, keys, 'zero'); p = row['parameters']; program = row['request']['program']
    gas = gas_accounting(mode, keys, depth, rounds)
    envelope = gas_accounting(mode, keys, depth, maximum)['transaction_envelope']
    code, leaf_counts = dirty._runtime(mode, keys, rounds); routes = Counter({op:0 for op in program['expected_route_counts']})
    routes.update({op:n*USERS for op,n in leaf_counts.items()})
    contracts = program['prestate_contracts']
    for c in contracts: c['code_hex'] = '0x'+code.hex()
    roles = [dict(address=c['address'], role='leaf', chain=i, level=depth, next_target=None,
                  account_proof_required=True, code_proof_required=True, storage_proof_required=True,
                  runtime_bytes=len(code), runtime_sha256=digest(code),
                  storage_targets=[hex(key) for key in range((i+1)*65536+1,(i+1)*65536+1+keys)]+['0x0']) for i,c in enumerate(contracts)]
    for level in range(depth):
        for i in range(USERS):
            target = address(i, level+1) if level+1 < depth else address(i)
            expected = 1 if level+1 < depth or mode == 'stop' else 0
            wrapper, counts = _wrapper(target, expected); routes.update(counts)
            owner = address(i, level)
            contracts.append(dict(address=owner, code_hex='0x'+wrapper.hex(), initial_storage={}))
            roles.append(dict(address=owner, role='wrapper', chain=i, level=level, next_target=target,
                expected_child_status=expected, account_proof_required=True, code_proof_required=True,
                storage_proof_required=False, storage_targets=[], runtime_bytes=len(wrapper), runtime_sha256=digest(wrapper)))
    program['expected_route_counts'] = dict(routes)
    recipients = [address(i, 0) for i in range(USERS)]
    program['transactions'].update(recipients=recipients, gas_limit=envelope)
    row['request']['expected_user_success'] = True
    level = 'zero' if rounds == 0 else 'full'; prefix = f'storage-caught-nesting-{mode}-k{keys}-d{depth}'
    row.update(sample_id=f'{prefix}-r{rounds}', family='storage-caught-nesting', corpus_id=CORPUS,
        role='control' if rounds == 0 else 'candidate', baseline_sample_id=f'{prefix}-r0',
        data_role='unfitted_storage_caught_nesting_validation',
        completion_gate='Independent source review and all16 native/signature/receipt/whole-world/authenticated roots/canonical mutation-witness/original-real-quote/all-frame-counter gates precede freeze and actuals.',
        scope='Finite caught nested CALL composition under frozen c893 pricing. Separate account/code/global support differs from exact old direct references; no isolated frame-cost, per-operation actual allocation, refit, global or production bound, proof or throughput claim.')
    p.update(call_depth=depth, account_count=USERS*(depth+1), total_account_count=USERS*(depth+1),
        storage_owner_count=USERS, storage_owner_addresses=[address(i) for i in range(USERS)],
        depth=depth, wrapper_account_count=USERS*depth,
        leaf_owners=[address(i) for i in range(USERS)],
        wrapper_levels=[[address(i,j) for i in range(USERS)] for j in range(depth)], account_roles=roles,
        workload_level=level, rounds_per_key=rounds, full_rounds_per_key=maximum,
        transaction_gas_limit=envelope, gas_accounting_per_user=gas,
        expected_block_charged_gas=ANCHOR_GAS+USERS*gas['charged_gas'],
        near_full=ANCHOR_GAS+USERS*gas['charged_gas'] >= 29_700_000,
        header_prefix_remaining_gas=[HEADER_GAS_LIMIT-ANCHOR_GAS-j*gas['charged_gas'] for j in range(USERS)],
        expected_storage_writes=USERS*keys*(2+2*rounds), expected_dirty_storage_writes=2*USERS*keys*rounds,
        expected_main_storage_writes=USERS*keys*(2+2*rounds), expected_storage_writes_per_account=keys*(2+2*rounds),
        expected_round_counter=rounds, expected_marker=keys*rounds,
        dirty_batches_per_key=rounds//64, dirty_tail_pairs_per_key=rounds%64,
        recipient_schedule=recipients, expected_terminal='STOP', expected_leaf_terminal='STOP' if mode=='stop' else 'REVERT',
        expected_user_success=True, expected_user_receipt_statuses=[True]*USERS,
        expected_call_count=USERS*depth, expected_wrapper_stops=USERS*depth,
        expected_leaf_stops=USERS*int(mode=='stop'), expected_leaf_reverts=USERS*int(mode=='revert'),
        expected_leaf_refund_counter_before_terminal=gas['leaf_refund_counter_before_terminal'],
        expected_all_frame_terminal_counts={'STOP':USERS*(depth+int(mode=='stop')), 'REVERT':USERS*int(mode=='revert')},
        runtime_bytes=len(code), runtime_sha256=digest(code), runtime_scope='shared leaf only',
        shared_runtime=False, shared_leaf_runtime=True,
        storage_entry_count_by_account={c['address']:len(c['initial_storage']) for c in contracts},
        per_account_storage_count_scope='Legacy scalar per-account counts refer only to the four storage owners; wrappers have zero entries.',
        wrapper_storage_rule='Explicit empty maps; account and code proofs required, no selected storage targets.',
        refund_policy='Remove child intrinsic; propagate successful leaf refund, discard leaf REVERT refund; cap once at transaction gross//5.',
        child_gas_request=CHILD_GAS, envelope_rule='T0=C+2301; Tj=2621+max(ceil(64*Tprev/63),Tprev+46); tx=21000+Tdepth',
        range_rule='Four successful users visit four disjoint guarded CALL chains. Original leaf seed/dirty/cleanup bytes and storage owners unchanged; caught leaf REVERT returns0, ancestors STOP.',
        direct_reference_sample_id=dirty.make_case(mode, keys, level)['sample_id'])
    for field in ('gross_gas','refund_counter_before_terminal','refund_counter','refund_applied','charged_gas'):
        p['expected_user_'+field] = gas[field]
    return row


def make_case(mode='stop', keys=16, depth=1, level='full'):
    _validate(mode, keys, depth)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum = full_rounds(mode, keys, depth)
    return _make_case(mode, keys, depth, 0 if level == 'zero' else maximum, maximum)


def helpers():
    return dirty.helpers() | {'storage_caught_nesting._wrapper':_wrapper,
        'storage_caught_nesting._wrapper_gas':_wrapper_gas, 'storage_caught_nesting.address':address,
        'storage_caught_nesting._validate':_validate, 'storage_caught_nesting._make_case':_make_case,
        'storage_caught_nesting.gas_accounting':gas_accounting, 'storage_caught_nesting.fits':fits,
        'storage_caught_nesting.full_rounds':full_rounds, 'storage_caught_nesting.make_case':make_case,
        'storage_dirty_keys.make_case':dirty.make_case}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows=[]; references=[]
    for mode in MODES:
        for keys in KEY_COUNTS:
            for level in LEVELS:
                direct = dirty.make_case(mode, keys, level)
                references.append(dict(sample_id=direct['sample_id'], mode=mode, keys_per_account=keys,
                    workload_level=level, request_sha256=digest(encoded(direct['request'])),
                    runtime_sha256=direct['parameters']['runtime_sha256'], new_measurement=False))
            for depth in DEPTHS:
                for level in LEVELS:
                    row=make_case(mode, keys, depth, level); raw=encoded(row.pop('request'))
                    path=f"requests/{row['sample_id']}.json"; write(output/path,raw)
                    rows.append(row|dict(request_path=path,request_sha256=digest(raw)))
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 16
    write(output/'catalog.json',encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name:digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name:digest(inspect.getsource(helper).encode()) for name,helper in helpers().items()},
        purpose='16 new finite separate-leaf caught nested CALL inputs; eight exact old direct references are not rerun or recounted.',
        modes=list(MODES), keys_per_account=list(KEY_COUNTS), depths=list(DEPTHS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, direct_references=references,
        required_comparison_quantities=['actual same-proposal SP1 PGU','node per-block estimates and literal sum','complete proposal estimate'],
        interpretation='Whole-input evidence only. Distinct parent account/code/global proof support prevents isolated frame attribution. All16 native/signature/receipts/world/roots/witness/quotes/attempted counters before freeze and actuals; preserve failures. No prices/refit/global/production/proof/throughput claim.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,required=True); parser.add_argument('--model-id',default=MODEL)
    args=parser.parse_args(); print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
