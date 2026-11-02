#!/usr/bin/env python3
"""Generate12 fixed-model journal/control inputs near30M charged block gas."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import storage_hash_composition as old
import top_level_create as original

MODEL = old.MODEL
PROFILES = ('dirty-direct', 'dirty-revert', 'mixed-revert')
PACKING = (4, 16)
ANCHOR_GAS = 167355
TARGET_GAS = 29990000
BUDGET_GAS = 30000000
HEADER_GAS = 31000000
RESTORE_ZERO_REFUND = 20000-100
SSTORE_SENTRY_CUSHION = 2301
STATIC_GAS = {0:0, 1:3, 2:5, 0x14:3, 0x15:3, 0x20:36, 0x30:2, 0x35:3, 0x36:2,
              0x3d:2, 0x50:2, 0x51:3, 0x52:3, 0x56:8, 0x57:10, 0x5b:1, 0xf4:100, 0xfd:0}


def _profile(profile):
    if type(profile) is not str or profile not in PROFILES: raise ValueError('unknown journal profile')
    return profile.split('-')


def _runtime(profile, rounds):
    """Reuse the unchanged semantic kernel; the old public round limit stays intact."""
    kernel, layout = _profile(profile)
    original.integer(rounds, 0, 65536, 'rounds per user')
    if rounds % 64: raise ValueError('rounds must be divisible by64')
    asm = _Factory()
    if layout == 'revert':
        asm.op(0x36, 2); asm.op(0x15, 2); asm.target('parent', 2); asm.op(0x57, 2)
        asm.op(0x36); old._check(asm, 32); asm.push(0); asm.op(0x35); old._check(asm, old.TAG, width=32)
        asm.target('kernel'); asm.op(0x56); asm.label('parent')
    old._load_slot(asm, 0); old._store(asm, 5)
    if layout == 'direct':
        old._kernel(asm, kernel, rounds); old._cleanup(asm, rounds)
    else:
        asm.push(old.TAG, 32); asm.push(256); asm.op(0x52)
        asm.push(0); asm.push(128); asm.op(0x52)
        asm.push(32); asm.push(128); asm.push(32); asm.push(256); asm.op(0x30); asm.push(12000000); asm.op(0xf4)
        old._check(asm, 0); asm.op(0x3d); old._check(asm, 32); old._cleanup(asm, rounds)
        asm.label('kernel'); old._kernel(asm, kernel, rounds); asm.push(32); asm.push(128); asm.op(0xfd)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), dict(asm.counts)


def _memory_cost(words):
    return 3*words+words*words//512


def _gas(profile, rounds):
    """Gross EVM gas from executed opcodes, actual storage branches and frame memory."""
    _, layout = _profile(profile); _, counts = _runtime(profile, rounds)
    static = 0
    for name, count in counts.items():
        op = int(name[9:], 16)
        if count and op not in (0x54, 0x55):
            static += count*(3 if 0x60 <= op <= 0x7f else STATIC_GAS[op])
    # The initial slot2 read warms it. Original0→seed5 costs20000; all later
    # writes are dirty and cost100. Child rollback restores parent5 and its
    # refund checkpoint; only the parent's final original-zero clear earns19900.
    reads = 2100+100*(counts['opcode:0x54']-1)
    writes = 20000+100*(counts['opcode:0x55']-1)
    memory_words = [5] if layout == 'direct' else [9, 5]
    memory = sum(_memory_cost(words) for words in memory_words)
    gross = 21000+static+reads+writes+memory
    refund = min(gross//5, RESTORE_ZERO_REFUND)
    return {'intrinsic_gas':21000, 'static_opcode_gas':static, 'storage_read_gas':reads,
            'storage_write_gas':writes, 'memory_words_by_frame':memory_words,
            'memory_expansion_gas':memory, 'gross_user_gas':gross,
            'refund_counter':RESTORE_ZERO_REFUND, 'refund_cap_gas':gross//5,
            'applied_refund':refund, 'charged_user_gas':gross-refund}


def make_case(profile, packing=4, level='full'):
    kernel, layout = _profile(profile)
    if type(packing) is not int or packing not in PACKING: raise ValueError('packing must be4 or16')
    if type(level) is not str or level not in ('zero', 'full'): raise ValueError('level must be zero or full')
    base = _gas(profile, 0); step = _gas(profile, 64)['gross_user_gas']-base['gross_user_gas']
    budget_user = (TARGET_GAS-ANCHOR_GAS)//packing
    batches = (budget_user+RESTORE_ZERO_REFUND-base['gross_user_gas'])//step
    full_rounds = batches*64; full_gas = _gas(profile, full_rounds)
    assert full_gas['applied_refund'] == RESTORE_ZERO_REFUND
    assert ANCHOR_GAS+packing*full_gas['charged_user_gas'] <= TARGET_GAS
    assert ANCHOR_GAS+packing*_gas(profile, full_rounds+64)['charged_user_gas'] > TARGET_GAS
    # Reserve2301 above consumed gross for the EIP-2200 SSTORE sentry,
    # then cover EIP-150 even when the12M requested child gas is capped.
    # It is an admission envelope, not consumed or charged gas. Both controls
    # and full cases select this identical full-workload envelope.
    gross = full_gas['gross_user_gas']
    gas_limit = ((gross+SSTORE_SENTRY_CUSHION)*64+62)//63 if layout == 'revert' else gross+SSTORE_SENTRY_CUSHION
    rounds = 0 if level == 'zero' else full_rounds
    gas = _gas(profile, rounds)
    gas.update(anchor_charged_gas=ANCHOR_GAS, charged_block_gas=ANCHOR_GAS+packing*gas['charged_user_gas'],
               minimum_native_remaining_before_user=HEADER_GAS-ANCHOR_GAS-(packing-1)*gas['charged_user_gas'],
               batch_increment_gross_gas=step)
    assert gas_limit <= gas['minimum_native_remaining_before_user'] and gas_limit <= 15000000
    code, counts = _runtime(profile, rounds)
    row = original.make_case('call', 'stop', count=packing, gas_limit=gas_limit)
    program = row['request']['program']; routes = {name:0 for name in program['expected_route_counts']}
    routes.update({name:count*packing for name,count in counts.items()})
    program.update(code_hex='0x'+code.hex(), expected_route_counts=routes)
    sid = f'gas-budget-journal-{profile}-t{packing}-{level}'
    row.update(sample_id=sid, family='gas-budget-journal', role='control' if level == 'zero' else 'candidate',
        baseline_sample_id=f'gas-budget-journal-{profile}-t{packing}-zero',
        data_role='unfitted_high_occupancy_composition_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Concrete slot2 original0/parent5/kernel7↔11/restore5/clear0; zero32 EthereumKeccak; ordered reads/writes and frames; derived loop marker; caught REVERT status0/returndata32; all-user exact opcode counts; canonical EIP-1559 users and successful receipts; exact charged/gross/refund/envelope checks.',
        scope='One identical-proposal actual SP1PGU, complete proposal estimate and node per-block estimate/literal sum stay separate. Full target29.99M charged block gas includes Anchor; native header remains31M. No30M-header admission proof, startup allocation, price fit, extrapolated actual, capacity or proof-time claim. More than2B missingPGU is a finite observed priority criterion, not a result or deadline.')
    row['parameters'] = {'profile':profile, 'kernel':kernel, 'layout':layout, 'packing':packing, 'level':level,
        'rounds_per_user':rounds, 'batch_count_per_user':rounds//64, 'unroll_factor':64,
        'all_rounds_per_block':rounds*packing, 'all_rounds_per_proposal':rounds*packing,
        'block_count':1, 'all_user_transactions':packing, 'all_transactions_including_anchor':packing+1,
        'parameter_scope':'Storage, guards, loop counters and gas describe one user; expected_route_counts and all_rounds aggregate every user in the sole block.',
        'transaction_kind':'call', 'transaction_envelope':'EIP-1559 explicit canonical user descriptor',
        'transaction_gas_limit':gas_limit, 'full_gross_user_gas':gross,
        'selected_envelope_headroom_above_full_gross':gas_limit-gross,
        'sstore_sentry_cushion_gas':SSTORE_SENTRY_CUSHION,
        'forwarded_gas':12000000 if layout == 'revert' else None,
        'eip150_policy':'Child requested12M is capped by63/64 of caller available gas; selected user envelope covers ceil((full gross+2301)*64/63), preserving SSTORE sentry cushion and parent cleanup.' if layout == 'revert' else 'No child call.',
        'charged_block_gas_budget':BUDGET_GAS, 'charged_block_gas_target':TARGET_GAS,
        'full_occupancy_min_gas':29700000, 'native_header_gas_limit':HEADER_GAS,
        'anchor_gas_scope':'167355 is this canonical fixture Anchor receipt expectation, not a general protocol tariff.',
        'gas':gas, 'storage_slot':2, 'original_storage_per_transaction':0, 'parent_seed':5,
        'kernel_dirty_values':[7,11], 'kernel_exit':5, 'parent_final_storage':0,
        'all_other_storage_expected_zero':True, 'hash_offset':0, 'hash_length':32,
        'hash_input_hex':'0x'+bytes(32).hex(), 'hash_expected_hex':f'0x{old.ZERO_HASH:064x}',
        'counter_memory_offset':64, 'marker_memory_offset':128, 'input_tag_memory_offset':256,
        'expected_marker_per_user':rounds, 'expected_child_status':0 if layout == 'revert' else None,
        'expected_child_return_bytes':32 if layout == 'revert' else 0,
        'storage_scope':'Parent prewarms slot2 and self; no cold-first-access coverage. Child revert restores the parent storage/refund checkpoint.',
        'outer_calldata_bytes':0, 'execution_profile':'evm-gas-only', 'expected_user_success':True,
        'runtime_bytes':len(code), 'runtime_sha256':digest(code), 'preload_created_accounts_or_code':False}
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    plans = [make_case(profile, packing, level) for profile in PROFILES for packing in PACKING for level in ('zero','full')]
    ids = {row['sample_id'] for row in plans}; assert len(plans) == len(ids) == 12
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request')); path=f"requests/{row['sample_id']}.json"; write(output/path,raw)
        rows.append(row | {'request_path':path, 'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1, 'frozen_model_id':model_id,
        'purpose':'Twelve zero/full matched dirty-direct, dirty-revert and mixed-revert inputs near30M charged block gas, at4/16 user packing.',
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in ('storage_hash_composition.py','initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'required_comparison_quantities':['actual identical-proposal SP1 PGU','complete proposal estimate','node per-block estimates and their literal sum, with aggregate ceiling retained'],
        'interpretation':'Keep all six ownN0 controls, failures and both deficit flags. Full required charged block gas29.7M..30M, including fixture Anchor; header31M is unchanged and every selected user envelope fits remaining native gas. No sampled actual,2B priority result, new tariff or proof-time claim is produced by this catalog.',
        'cases':rows}))
    return len(rows)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
