#!/usr/bin/env python3
"""Generate32 guarded caller-warmth and leaf/ancestor rollback controls."""
import argparse
from collections import Counter
import inspect
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
from storage_fanout import MODEL
import storage_caught_nesting as common
import storage_dirty_keys as dirty
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-storage-warm-rollback-20261103'
EXIT_MODES = ('leaf', 'ancestor')
KEY_COUNTS = (16, 256)
DEPTHS = (2, 4)
LEVELS = ('zero', 'full')
USERS = 4
SOURCES = ('storage_caught_nesting.py',) + common.SOURCES


def _validate(exit_mode, keys, depth, prewarm):
    if type(exit_mode) is not str or exit_mode not in EXIT_MODES: raise ValueError('exit_mode must be leaf or ancestor')
    original.integer(keys,1,1024,'keys')
    if type(depth) is not int or depth not in DEPTHS: raise ValueError('depth must be2 or4')
    if type(prewarm) is not bool: raise ValueError('prewarm must be boolean')


def _user_count(value):
    original.integer(value,1,4,'user_count')
    return value


def _selected_key_counts(values):
    if values is None: return KEY_COUNTS
    if type(values) not in (list,tuple) or not 1<=len(values)<=4:
        raise ValueError('key_counts must be a list or tuple of1..4 unique integers')
    for value in values: original.integer(value,1,1024,'keys')
    if len(set(values))!=len(values): raise ValueError('key_counts must be unique')
    return tuple(values)


def _emit_reads(asm, keys, label):
    """Guard raw phase then ascending body values using one32-byte counter."""
    asm.push(0); asm.op(0x54); mutation._check(asm, 65537)
    asm.push(0); asm.push(0); asm.op(0x52)
    asm.label(label, keys+1)
    asm.push(0, times=keys+1); asm.op(0x51, keys+1)
    asm.push(keys, times=keys+1); asm.op(0x14, keys+1)
    asm.target(label+'-done', keys+1); asm.op(0x57, keys+1)
    asm.push(0, times=keys); asm.op(0x51, keys); asm.push(65537, times=keys)
    asm.op(1, keys); asm.op(0x54, keys); mutation._check(asm, 1, keys)
    asm.push(0, times=keys); asm.op(0x51, keys); asm.push(1, times=keys); asm.op(1, keys)
    asm.push(0, times=keys); asm.op(0x52, keys); asm.target(label, keys); asm.op(0x56, keys)
    asm.label(label+'-done'); asm.push(0); asm.op(0x51); mutation._check(asm, keys)


def _ordinary(counts):
    costs = {0:0,1:3,0x14:3,0x15:3,0x3d:2,0x51:3,0x52:3,0x54:0,0x56:8,0x57:10,0x5b:1,0xf4:0,0xfd:0,0xfe:0}
    return sum(n*(3 if 0x60 <= int(op[-2:],16) <= 0x7f else costs[int(op[-2:],16)]) for op,n in counts.items())


def _read_metrics(keys):
    asm=_Factory(); _emit_reads(asm,keys,'reads'); asm.label('failure',0); asm.op(0xfe,0)
    # Resolve every emitted jump, then derive fees from executed assembly counts.
    asm.finish()
    return dict(ordinary_gas=_ordinary(asm.counts), counts=asm.counts, storage_reads=keys+1)


def _wrapper(target, expected_status, keys, *, top=False, prewarm=False, revert=False):
    asm=_Factory()
    if top and prewarm: _emit_reads(asm,keys,'prewarm')
    common._emit_call(asm,target,expected_status,call_kind='delegatecall')
    if top: _emit_reads(asm,keys,'postcheck')
    if revert: asm.push(0); asm.push(0); asm.op(0xfd)
    else: asm.op(0)
    asm.label('failure',0); asm.op(0xfe,0)
    return asm.finish(),asm.counts


def _gas_single(exit_mode, keys, depth, prewarm, rounds):
    leaf_mode='revert' if exit_mode=='leaf' else 'stop'
    leaf=dirty.gas_accounting(leaf_mode,keys,rounds)
    saving=2000*(keys+1) if prewarm else 0
    execution=leaf['gross_gas']-21000-saving
    metrics=_read_metrics(keys); read_ordinary=metrics['ordinary_gas']
    required=execution+2301; requirements=[required]; segments=[]; total=execution
    for level in reversed(range(depth)):
        pre=18+2600; post=46
        if level==0:
            if prewarm: pre+=read_ordinary+2100*(keys+1)+3
            post+=read_ordinary+(100 if prewarm else 2100)*(keys+1)+(0 if prewarm else 3)
        if exit_mode=='ancestor' and level==depth-1: post+=6
        segments.append(dict(level=level,pre_gas=pre,post_gas=post))
        total+=pre+post
        required=pre+max((64*required+62)//63,required+post); requirements.append(required)
    gross=21000+total
    return dict(intrinsic_gas=21000,leaf_mode=leaf_mode,leaf_cold_gas_accounting=leaf,
        leaf_execution_gas=execution,leaf_gross_gas=21000+execution,leaf_prewarm_saving=saving,
        leaf_refund_counter_before_terminal=2800*keys,leaf_effective_refund_counter=2800*keys if exit_mode=='ancestor' else 0,
        ancestor_refund_counter_before_terminal=2800*keys if exit_mode=='ancestor' else 0,
        refund_counter_before_terminal=0,refund_counter=0,refund_applied=0,
        gross_gas=gross,charged_gas=gross,wrapper_ordinary_pre=18,wrapper_cold_target_gas=2600,
        wrapper_ordinary_post=46,top_read_ordinary_gas_per_traversal=read_ordinary,
        top_pre_read_gas=(read_ordinary+2100*(keys+1)) if prewarm else 0,
        top_post_read_gas=read_ordinary+(100 if prewarm else 2100)*(keys+1),
        top_memory_expansion_gas=3,leaf_memory_expansion_gas=18,
        wrapper_segments_leaf_to_root=segments,child_gas_request=12_000_000,
        minimum_frame_envelopes_leaf_to_root=requirements,required_transaction_envelope=21000+required)


def gas_accounting(exit_mode, keys, depth, prewarm, rounds):
    _validate(exit_mode,keys,depth,prewarm); original.integer(rounds,0,65536,'rounds per key')
    pair={mode:_gas_single(mode,keys,depth,prewarm,rounds) for mode in EXIT_MODES}
    gas=pair[exit_mode]
    return gas | dict(transaction_envelope=max(g['required_transaction_envelope'] for g in pair.values()),
        mode_required_transaction_envelopes={mode:g['required_transaction_envelope'] for mode,g in pair.items()})


def fits(keys, depth, prewarm, rounds, *, user_count=4):
    _user_count(user_count)
    _validate('leaf',keys,depth,prewarm); original.integer(rounds,0,65536,'rounds per key')
    for mode in EXIT_MODES:
        gas=gas_accounting(mode,keys,depth,prewarm,rounds); envelope=gas['transaction_envelope']
        code,_=dirty._runtime('revert' if mode=='leaf' else 'stop',keys,rounds)
        if not (user_count*(keys+1)<=8192 and len(code)<=24576 and envelope<=min(15_000_000,16_777_216)
            and max(gas['minimum_frame_envelopes_leaf_to_root'][:-1])<=12_000_000
            and 167355+user_count*gas['charged_gas']<=30_000_000
            and all(31_000_000-167355-j*gas['charged_gas']>=envelope for j in range(user_count))): return False
    return True


def full_rounds(keys, depth, prewarm, *, user_count=4):
    _user_count(user_count)
    _validate('leaf',keys,depth,prewarm); low,high=0,65536
    if not fits(keys,depth,prewarm,0,user_count=user_count): raise ValueError('R0 control must fit selected user count and budgets')
    while low<high:
        middle=(low+high+1)//2
        if fits(keys,depth,prewarm,middle,user_count=user_count): low=middle
        else: high=middle-1
    if low==0: raise ValueError('selected user count and keys have no positive full rounds')
    assert 0<low<65536 and fits(keys,depth,prewarm,low,user_count=user_count) and not fits(keys,depth,prewarm,low+1,user_count=user_count)
    return low


def _make_case(exit_mode, keys, depth, prewarm, rounds, maximum, *, user_count=4):
    _user_count(user_count)
    _validate(exit_mode,keys,depth,prewarm)
    original.integer(rounds,0,65536,'rounds per key'); original.integer(maximum,0,65536,'full rounds')
    leaf_mode='revert' if exit_mode=='leaf' else 'stop'
    row=dirty.make_case(leaf_mode,keys,'zero'); p=row['parameters']; program=row['request']['program']
    gas=gas_accounting(exit_mode,keys,depth,prewarm,rounds)
    envelope=gas_accounting(exit_mode,keys,depth,prewarm,maximum)['transaction_envelope']
    code,counts=dirty._runtime(leaf_mode,keys,rounds); leaf_address=common.address(0)
    owners=[common.address(i,0) for i in range(user_count)]
    state={'0x0':'0x10001'} | {hex(key):'0x1' for key in range(65537,65537+keys)}
    contracts=[dict(address=leaf_address,code_hex='0x'+code.hex(),initial_storage={})]
    roles=[dict(address=leaf_address,role='leaf',chain=None,level=depth,next_target=None,
        code_address=leaf_address,storage_addresses=owners,account_proof_required=True,code_proof_required=True,
        storage_proof_required=False,storage_targets=[],runtime_bytes=len(code),runtime_sha256=digest(code))]
    routes=Counter({op:0 for op in program['expected_route_counts']}); routes.update({op:n*user_count for op,n in counts.items()})
    for level in range(depth):
        for i in range(user_count):
            target=common.address(i,level+1) if level+1<depth else leaf_address
            status=0 if (exit_mode=='leaf' and level==depth-1) or (exit_mode=='ancestor' and level==depth-2) else 1
            reverting=exit_mode=='ancestor' and level==depth-1
            wrapper,wrapper_counts=_wrapper(target,status,keys,top=level==0,prewarm=prewarm,revert=reverting)
            owner=common.address(i,level); storage=state.copy() if level==0 else {}
            contracts.append(dict(address=owner,code_hex='0x'+wrapper.hex(),initial_storage=storage)); routes.update(wrapper_counts)
            roles.append(dict(address=owner,role='top' if level==0 else 'wrapper',chain=i,level=level,next_target=target,
                code_address=owner,storage_address=owners[i],expected_child_status=status,expected_terminal='REVERT' if reverting else 'STOP',
                account_proof_required=True,code_proof_required=True,storage_proof_required=level==0,
                storage_targets=list(storage),runtime_bytes=len(wrapper),runtime_sha256=digest(wrapper)))
    program.update(prestate_contracts=contracts,expected_route_counts=dict(routes))
    program['transactions'].update(count_per_block=user_count,recipients=owners,gas_limit=envelope); row['request']['expected_user_success']=True
    prefix=f'storage-warm-rollback-{exit_mode}-k{keys}-d{depth}-w{int(prewarm)}' + (f'-u{user_count}' if user_count!=4 else '')
    level='zero' if rounds==0 else 'full'
    row.update(sample_id=f'{prefix}-r{rounds}',baseline_sample_id=f'{prefix}-r0',family='storage-warm-rollback',corpus_id=CORPUS,
        role='control' if rounds==0 else 'candidate',data_role='unfitted_storage_warm_rollback_validation',
        completion_gate='Independent source review and all32 native/signatures/receipts/world/authenticated owner roots/canonical witness/quotes/attempted counters precede freeze and actuals.',
        scope='Finite whole-input warmth and rollback composition. Preserve whole-world support and signed envelope differences; no isolated opcode allocation, price fit, global or production bound, generated proof or throughput claim.')
    p.update(mode=leaf_mode,exit_mode=exit_mode,exit_placement=exit_mode,leaf_mode=leaf_mode,prewarm=prewarm,
        call_kind='delegatecall',call_depth=depth,depth=depth,account_count=user_count*depth+1,total_account_count=user_count*depth+1,
        storage_owner_count=user_count,storage_owner_addresses=owners,leaf_owners=[leaf_address],leaf_code_addresses=[leaf_address],
        wrapper_account_count=user_count*depth,wrapper_levels=[[common.address(i,j) for i in range(user_count)] for j in range(depth)],account_roles=roles,
        workload_level=level,rounds_per_key=rounds,full_rounds_per_key=maximum,transaction_gas_limit=envelope,
        common_full_transaction_envelope=envelope,gas_accounting_per_user=gas,
        expected_block_charged_gas=167355+user_count*gas['charged_gas'],near_full=167355+user_count*gas['charged_gas']>=29_700_000,
        header_prefix_remaining_gas=[31_000_000-167355-j*gas['charged_gas'] for j in range(user_count)],
        expected_storage_reads=user_count*(2+6*keys+(1+int(prewarm))*(keys+1)),
        expected_storage_writes=user_count*keys*(2+2*rounds),expected_dirty_storage_writes=2*user_count*keys*rounds,
        expected_main_storage_writes=user_count*keys*(2+2*rounds),expected_storage_writes_per_account=keys*(2+2*rounds),
        expected_round_counter=rounds,expected_marker=keys*rounds,dirty_batches_per_key=rounds//64,dirty_tail_pairs_per_key=rounds%64,
        phase_base_by_account=dict.fromkeys(owners,65536),initial_phase_by_account=dict.fromkeys(owners,65537),
        expected_final_phase_by_account=dict.fromkeys(owners,65537),body_key_schedule_by_account={owner:list(range(65537,65537+keys)) for owner in owners},
        recipient_schedule=owners,start_phase_schedule=[65537]*user_count,final_phase_schedule=[65537]*user_count,
        expected_terminal='STOP',expected_leaf_terminal='REVERT' if exit_mode=='leaf' else 'STOP',expected_user_success=True,
        expected_user_receipt_statuses=[True]*user_count,expected_call_count=user_count*depth,
        expected_wrapper_stops=user_count*(depth-int(exit_mode=='ancestor')),expected_leaf_stops=user_count*int(exit_mode=='ancestor'),
        expected_leaf_reverts=user_count*int(exit_mode=='leaf'),expected_ancestor_reverts=user_count*int(exit_mode=='ancestor'),
        expected_all_frame_terminal_counts={'STOP':user_count*depth,'REVERT':user_count},expected_leaf_refund_counter_before_terminal=2800*keys,
        expected_leaf_effective_refund_counter=2800*keys if exit_mode=='ancestor' else 0,
        expected_ancestor_refund_counter_before_terminal=2800*keys if exit_mode=='ancestor' else 0,
        runtime_bytes=len(code),runtime_sha256=digest(code),runtime_scope='shared leaf only',shared_runtime=False,shared_leaf_runtime=True,
        storage_entry_count_by_account={c['address']:len(c['initial_storage']) for c in contracts},
        top_memory_bytes=32,leaf_memory_bytes=192,top_read_order=[0]+list(range(65537,65537+keys)),
        top_pre_read_cold_flags=[True]*(keys+1) if prewarm else [],top_post_read_cold_flags=[not prewarm]*(keys+1),
        leaf_first_read_cold=not prewarm,
        initial_storage_rule='Four top owners share identical raw phase0=65537 and body65537..65536+K=1 maps; all other accounts have empty storage.',
        wrapper_storage_rule='DELEGATECALL preserves each top storage owner. Code and intermediate accounts have empty maps.',
        per_account_storage_count_scope='Scalar storage counts describe the four top owners only.',
        refund_policy='Leaf preterminal2800K; successful leaf refund survives only until ancestor REVERT. Every top preterminal/effective/global applied refund is0; child intrinsic and applied cap are excluded.',
        envelope_rule='Tleaf=C+2301; Ti=pre_i+max(ceil(64*Tchild/63),Tchild+post_i). Common mode-pair maximum at common maximal full R.',
        range_rule='Optional guarded phase/body caller prewarm precedes delegated leaf; exactly one caught leaf/ancestor REVERT per user; mandatory guarded phase/body postcheck verifies rollback. Attempted events survive checkpoints.')
    # These totals originate in the fixed-F4 dirty template; replace every one.
    p.update(aggregate_state_size=user_count*(keys+1),user_transactions_per_block=user_count,
        all_user_transactions=user_count,all_transactions_including_anchor=user_count+1,
        expected_distinct_account_slot_pairs=user_count*(keys+1),expected_distinct_write_pairs=user_count*keys,
        expected_seed_storage_writes=user_count*keys,expected_cleanup_storage_writes=user_count*keys,
        expected_initial_storage_entry_count=user_count*(keys+1),expected_final_storage_entry_count=user_count*(keys+1))
    if user_count!=4: p['user_count']=user_count
    if user_count!=4 or keys not in KEY_COUNTS:
        p.update(initial_storage_rule=f'{user_count} top owners share identical raw phase0=65537 and body65537..65536+K=1 maps; all other accounts have empty storage.',
            per_account_storage_count_scope=f'Scalar storage counts describe the {user_count} selected top owners only.')
        row['completion_gate']='Selected explicit-key/user inputs require independent byte/state/signature/receipt/root/witness/quote validation before actuals.'
        row['scope']=f'Finite selected F{user_count} whole-input warmth and rollback composition, with all configured chains reachable. No isolated opcode allocation, refit, global/production bound, proof or throughput claim.'
    for field in ('gross_gas','refund_counter_before_terminal','refund_counter','refund_applied','charged_gas'): p['expected_user_'+field]=gas[field]
    return row


def make_case(exit_mode='leaf', keys=16, depth=2, prewarm=False, level='full', *, user_count=4):
    _user_count(user_count)
    _validate(exit_mode,keys,depth,prewarm)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum=full_rounds(keys,depth,prewarm,user_count=user_count)
    return _make_case(exit_mode,keys,depth,prewarm,0 if level=='zero' else maximum,maximum,user_count=user_count)


def helpers():
    return common.helpers() | {'storage_caught_nesting._emit_call':common._emit_call,
        **{'storage_warm_rollback.'+f.__name__:f for f in (_validate,_user_count,_selected_key_counts,_emit_reads,_ordinary,_read_metrics,_wrapper,_gas_single,gas_accounting,fits,full_rounds,_make_case,make_case)}}


def prepare(output: Path, *, model_id=MODEL, user_count=4, key_counts=None):
    _user_count(user_count); selected=_selected_key_counts(key_counts)
    if not isinstance(output,Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    if output.exists() or output.is_symlink(): raise FileExistsError(output)
    rows=[]; payloads=[]
    for mode in EXIT_MODES:
        for keys in selected:
            for depth in DEPTHS:
                for prewarm in (False,True):
                    for level in LEVELS:
                        row=make_case(mode,keys,depth,prewarm,level,user_count=user_count); raw=encoded(row.pop('request')); path=f"requests/{row['sample_id']}.json"
                        payloads.append((path,raw)); rows.append(row|dict(request_path=path,request_sha256=digest(raw)))
    expected=len(EXIT_MODES)*len(selected)*len(DEPTHS)*2*len(LEVELS)
    assert len({r['sample_id'] for r in rows})==len({r['request_sha256'] for r in rows})==expected
    output.mkdir(parents=True,exist_ok=False); (output/'requests').mkdir()
    for path,raw in payloads: write(output/path,raw)
    write(output/'catalog.json',encoded(dict(schema_version=1,corpus_id=CORPUS,frozen_model_id=model_id,
        generator_sha256=digest(Path(__file__).read_bytes()),source_sha256={name:digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name:digest(inspect.getsource(helper).encode()) for name,helper in helpers().items()},
        exit_modes=list(EXIT_MODES),keys_per_account=list(selected),depths=list(DEPTHS),prewarm=[False,True],levels=list(LEVELS),users=user_count,
        holdout_only=True,actuals_used_for_fitting=False,purpose=('32 finite guarded caller warmth and ancestor rollback controls; old32 CALL/F4 identities remain unchanged.' if selected==KEY_COUNTS and user_count==4 else f'{len(rows)} finite explicit-key/user caller warmth and rollback controls; original default32 remains unchanged.'),
        required_comparison_quantities=['actual same-proposal SP1 PGU','node per-block estimates and literal sum','complete proposal estimate'],
        interpretation=('Whole-input evidence only. Parent warmth persists across lower rollback; child warmth/refund/state roll back while attempted work remains. All32 independent qualification gates precede freeze and actuals. No per-operation actual allocation, refit, global, production, proof or throughput claim.' if selected==KEY_COUNTS and user_count==4 else 'Finite explicit-key/user whole-input selection; all selected inputs require independent qualification before actuals. No per-operation allocation, fitting, global/production bound or proof claim.'),cases=rows)))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir',type=Path,required=True); parser.add_argument('--model-id',default=MODEL)
    parser.add_argument('--user-count',type=int,default=4); parser.add_argument('--key-counts',type=int,nargs='+')
    args=parser.parse_args(); print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id,user_count=args.user_count,key_counts=args.key_counts)}))
