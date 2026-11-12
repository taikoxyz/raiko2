#!/usr/bin/env python3
"""Shared-calldata role dispatch with genuine persistent phase and journal rollback."""
from collections import Counter
from functools import lru_cache
import argparse
import json
from pathlib import Path
import re

from spawn_memory import digest, encoded, write

from initcode_preparation import _Factory
from stack_density import SEEDS
import storage_hash_composition as journal
import top_level_create as original

MODEL=journal.MODEL
PHASE_SLOT=4096
PHASE_MEMORY=320
NEXT_PHASE_MEMORY=352
TAG=int.from_bytes(b'order-composition-child-v1'.ljust(32,b'\0'),'big')
ORDERS=('alternating','densityfirst','journalfirst')
KERNELS=('dirty','mixed')
DENSITY_UNROLL=2048
JOURNAL_UNROLL=64
STATIC_GAS={0:0,1:3,2:5,0x10:3,0x14:3,0x15:3,0x20:36,0x30:2,0x35:3,0x36:2,0x3d:2,0x50:2,0x51:3,0x52:3,0x56:8,0x57:10,0x5b:1,0x80:3,0x9f:3,0xf4:100,0xfd:0}


class _Assembly(_Factory):
    def __init__(self):
        super().__init__();self.weights=(1,1);self.role_counts={'density':Counter(),'journal':Counter()}

    def scope(self,density,journal):self.weights=(density,journal)

    def op(self,opcode,times=1):
        super().op(opcode,0)
        for role,weight in zip(('density','journal'),self.weights):self.role_counts[role][f'opcode:0x{opcode:02x}']+=times*weight


class _KernelScope:
    """Prefix legacy kernel labels while keeping one shared failure destination."""
    def __init__(self,asm):self.asm=asm
    def __getattr__(self,name):return getattr(self.asm,name)
    def target(self,name,times=1):self.asm.target(name if name=='failure' else 'journal_'+name,times)
    def label(self,name,times=1):self.asm.label(name if name=='failure' else 'journal_'+name,times)


def _table(order,packing):
    if type(order) is not str or order not in ORDERS:raise ValueError('unknown order')
    if type(packing) is not int or packing not in (2,4,16):raise ValueError('packing must be2,4 or16')
    roles=([1,2]*(packing//2) if order=='alternating' else [1]*(packing//2)+[2]*(packing//2) if order=='densityfirst' else [2]*(packing//2)+[1]*(packing//2))
    return b''.join(role.to_bytes(32,'big') for role in roles)


def _runtime(kernel,packing,density_batches,journal_batches):
    if type(kernel) is not str or kernel not in KERNELS:raise ValueError('unknown journal kernel')
    if type(packing) is not int or packing not in (2,4,16):raise ValueError('packing must be2,4 or16')
    original.integer(density_batches,0,65536,'density batches');original.integer(journal_batches,0,65536,'journal batches')
    rounds=journal_batches*JOURNAL_UNROLL;asm=_Assembly()
    # The outer table is128/512bytes (64 in the two-user builder probe); child
    # input is exactly32bytes. Journal users execute this prefix in both frames.
    asm.scope(1,2);asm.op(0x36);asm.push(32);asm.op(0x14);asm.target('child');asm.op(0x57)
    asm.scope(1,1);asm.op(0x36);journal._check(asm,packing*32)
    asm.push(PHASE_SLOT);asm.op(0x54);asm.push(PHASE_MEMORY);asm.op(0x52)
    asm.push(packing);asm.push(PHASE_MEMORY);asm.op(0x51);asm.op(0x10);asm.op(0x15);asm.target('failure');asm.op(0x57)
    asm.push(PHASE_MEMORY);asm.op(0x51);asm.push(1);asm.op(1);asm.op(0x80);asm.push(NEXT_PHASE_MEMORY);asm.op(0x52);asm.push(PHASE_SLOT);asm.op(0x55)
    asm.push(PHASE_MEMORY);asm.op(0x51);asm.push(32);asm.op(2);asm.op(0x35)
    asm.op(0x80);asm.push(1);asm.op(0x14);asm.target('density');asm.op(0x57)
    asm.scope(0,1);journal._check(asm,2)
    journal._load_slot(asm,0);journal._store(asm,5)
    asm.push(TAG,32);asm.push(256);asm.op(0x52);asm.push(0);asm.push(128);asm.op(0x52)
    asm.push(32);asm.push(128);asm.push(32);asm.push(256);asm.op(0x30);asm.push(12000000);asm.op(0xf4)
    journal._check(asm,0);asm.op(0x3d);journal._check(asm,32)
    asm.push(128);asm.op(0x51);journal._check(asm,rounds);journal._load_slot(asm,5);journal._store(asm,0);journal._load_slot(asm,0)
    asm.target('end');asm.op(0x56)
    asm.scope(1,0);asm.label('density');asm.op(0x50)
    asm.push(0);asm.push(0);asm.op(0x52)
    for value in SEEDS:asm.push(value,32)
    asm.label('density_loop',density_batches+1);asm.push(0,times=density_batches+1);asm.op(0x51,density_batches+1);asm.push(density_batches,times=density_batches+1);asm.op(0x14,density_batches+1);asm.target('density_done',density_batches+1);asm.op(0x57,density_batches+1)
    for _ in range(DENSITY_UNROLL):asm.op(0x9f,density_batches);asm.op(0x9f,density_batches)
    asm.push(0,times=density_batches);asm.op(0x51,density_batches);asm.push(1,1,density_batches);asm.op(1,density_batches);asm.push(0,times=density_batches);asm.op(0x52,density_batches);asm.target('density_loop',density_batches);asm.op(0x56,density_batches)
    asm.label('density_done');asm.push(0);asm.op(0x51);journal._check(asm,density_batches)
    for value in reversed(SEEDS):journal._check(asm,value,width=32)
    asm.target('end');asm.op(0x56)
    asm.scope(1,1);asm.label('end');asm.push(PHASE_SLOT);asm.op(0x54);asm.push(NEXT_PHASE_MEMORY);asm.op(0x51);asm.op(0x14);asm.op(0x15);asm.target('failure');asm.op(0x57);asm.op(0)
    asm.scope(0,1);asm.label('child');asm.push(0);asm.op(0x35);journal._check(asm,TAG,width=32)
    journal._kernel(_KernelScope(asm),kernel,rounds);asm.push(32);asm.push(128);asm.op(0xfd)
    asm.scope(0,0);asm.label('failure',0);asm.op(0xfe,0)
    return asm.finish(),{role:dict(counts) for role,counts in asm.role_counts.items()}


def _gas_user(kernel,packing,density_batches,journal_batches,role,index):
    original.integer(index,0,packing-1,'user index')
    if type(role) is not int or role not in (1,2):raise ValueError('role must be1 or2')
    _,by_role=_runtime(kernel,packing,density_batches,journal_batches)
    return _gas_role(packing,by_role['density' if role==1 else 'journal'],role,index)


def _gas_role(packing,counts,role,index):
    static=sum(count*(3 if 0x60<=int(name[9:],16)<=0x7f else STATIC_GAS[int(name[9:],16)]) for name,count in counts.items() if count and int(name[9:],16) not in (0x54,0x55))
    # Each32byte role word has31zero bytes and one nonzero1/2 byte.
    tokens=35*packing;intrinsic=21000+4*tokens;floor=21000+10*tokens
    phase_write=20000 if index==0 else 2900
    reads=2200+(2500 if role==2 else 0)
    writes=phase_write+(20000+100*(counts['opcode:0x55']-2) if role==2 else 0)
    memory=36+(15 if role==2 else 0)
    gross=intrinsic+static+reads+writes+memory;refund_counter=19900 if role==2 else 0;refund=min(refund_counter,gross//5);after_refund=gross-refund
    return {'intrinsic_gas':intrinsic,'calldata_tokens':tokens,'calldata_floor_candidate_gas':floor,'calldata_floor_nonbinding':after_refund>=floor,
        'static_opcode_gas':static,'storage_read_gas':reads,'storage_write_gas':writes,'phase_write_gas':phase_write,'phase_original':index,
        'memory_expansion_gas':memory,'gross_user_gas':gross,'refund_counter':refund_counter,'refund_cap_gas':gross//5,'applied_refund':refund,
        'charged_before_floor_gas':after_refund,'charged_user_gas':max(after_refund,floor)}


@lru_cache(maxsize=4)
def _full_plan(kernel,packing):
    _,base=_runtime(kernel,packing,0,0);_,d_one=_runtime(kernel,packing,1,0);_,j_one=_runtime(kernel,packing,0,1)
    d0=_gas_role(packing,base['density'],1,1);j0=_gas_role(packing,base['journal'],2,1)
    d_step=_gas_role(packing,d_one['density'],1,1)['gross_user_gas']-d0['gross_user_gas']
    j_step=_gas_role(packing,j_one['journal'],2,1)['gross_user_gas']-j0['gross_user_gas']
    # Full journal refunds are saturated. All noninitial phase writes cost2900;
    # reserve the one additional17100 before splitting equal user gas budgets.
    per_user=(29990000-167355-17100)//packing
    db=(per_user-d0['charged_user_gas'])//d_step
    jb=(per_user+19900-j0['gross_user_gas'])//j_step
    _,full=_runtime(kernel,packing,db,jb)
    max_gross=max(_gas_role(packing,full[role],value,0)['gross_user_gas'] for role,value in [('density',1),('journal',2)])
    envelope=((max_gross+2301)*64+62)//63
    assert db>0 and jb>0 and _gas_role(packing,full['journal'],2,1)['applied_refund']==19900
    return db,jb,envelope


def make_case(kernel='dirty',packing=4,order='alternating',level='full'):
    if type(kernel) is not str or kernel not in KERNELS:raise ValueError('unknown journal kernel')
    if type(packing) is not int or packing not in (4,16):raise ValueError('packing must be4 or16')
    if type(order) is not str or order not in ORDERS:raise ValueError('unknown order')
    if type(level) is not str or level not in ('zero','full'):raise ValueError('level must be zero or full')
    full_d,full_j,gas_limit=_full_plan(kernel,packing)
    db,jb=(full_d,full_j) if level=='full' else (0,0)
    code,by_role=_runtime(kernel,packing,db,jb);data=_table(order,packing)
    role_ids=[int.from_bytes(data[offset:offset+32],'big') for offset in range(0,len(data),32)]
    gas=[_gas_role(packing,by_role['density' if role==1 else 'journal'],role,index) for index,role in enumerate(role_ids)]
    charged=[x['charged_user_gas'] for x in gas];gross=[x['gross_user_gas'] for x in gas]
    remaining=[];available=31000000-167355
    for value in charged:remaining.append(available);available-=value
    block_gas=167355+sum(charged)
    assert all(x['calldata_floor_nonbinding'] for x in gas),'calldata floor requires separately qualified fork binding'
    assert gas_limit<=min(remaining) and gas_limit<=15000000
    if level=='full':assert 29700000<=block_gas<=30000000
    row=original.make_case('call','stop',count=packing,gas_limit=gas_limit)
    routes={name:0 for name in row['request']['program']['expected_route_counts']}
    for counts in by_role.values():
        for name,value in counts.items():routes[name]=routes.get(name,0)+value*(packing//2)
    row['request']['program'].update(code_hex='0x'+code.hex(),input_hex='0x'+data.hex(),expected_route_counts=routes)
    sid=f'order-composition-{kernel}-p{packing}-{order}-{level}'
    row.update(sample_id=sid,family='order-composition',role='control' if level=='zero' else 'candidate',
        baseline_sample_id=f'order-composition-{kernel}-p{packing}-{order}-zero',
        data_role='unfitted_order_composition_validation',holdout_only=True,actuals_used_for_fitting=False,
        completion_gate='All signed users share the exact role table/target/envelope; persistent phase0..packing with per-transaction original snapshots; ordered density/journal roles;17 actual SWAP seeds/final words; slot2 writes/rollback; tagged child status0/returndata32/derived marker; exact per-index gross/refund/floor/charged receipts and all-user opcounts.',
        scope='All phase, dispatch, shared calldata and authenticated state-root work remain in absolute same-proposal actual/node/full comparisons. OwnN0 increments are diagnostics only. One block, shared account,31M native header and30M charged budget; no selected node limit, free cursor, per-user actual allocation, price refit, capacity or universal-bound claim.')
    row['parameters']={'kernel':kernel,'packing':packing,'order':order,'level':level,'block_count':1,
        'role_ids_by_index':role_ids,'roles_by_index':['density' if x==1 else 'journal' for x in role_ids],
        'density_users':packing//2,'journal_users':packing//2,'all_user_transactions':packing,'all_transactions_including_anchor':packing+1,
        'density_batches':db,'journal_batches':jb,'full_density_batches':full_d,'full_journal_batches':full_j,
        'density_unroll':DENSITY_UNROLL,'journal_unroll':JOURNAL_UNROLL,
        'density_pairs_per_user':DENSITY_UNROLL*db,'journal_rounds_per_user':JOURNAL_UNROLL*jb,
        'all_density_pairs_per_proposal':DENSITY_UNROLL*db*(packing//2),'all_journal_rounds_per_proposal':JOURNAL_UNROLL*jb*(packing//2),
        'phase_slot':PHASE_SLOT,'phase_memory_offset':PHASE_MEMORY,'next_phase_memory_offset':NEXT_PHASE_MEMORY,
        'expected_phase_original_by_index':list(range(packing)),'expected_phase_final':packing,'phase_final_zero':False,
        'phase_scope':'Persistent contract storage: advance exactly once outside child; initial original0 set costs20000; each later original=i→i+1 is a clean nonzero2900 write. Never subtract phase work from absolute comparisons.',
        'journal_slot':2,'journal_original_by_user':0,'parent_seed':5,'kernel_dirty_values':[7,11],'parent_final_journal_storage':0,
        'child_input_tag_hex':f'0x{TAG:064x}','child_input_bytes':32,'expected_child_status':0,'expected_child_return_bytes':32,
        'expected_child_marker':JOURNAL_UNROLL*jb,'requested_child_gas':12000000,
        'role_table_hex':'0x'+data.hex(),'role_table_word_bytes':32,'outer_calldata_bytes':len(data),
        'parameter_scope':'Role/kernel work counts and frame values are per appropriate user; all_* and request expected_route_counts aggregate the sole proposal. Gas arrays preserve ordered user indexes; none are per-user actualPGU.',
        'expected_user_route_counts_by_role':by_role,'gas_by_index':gas,
        'expected_user_gas_used_by_index':charged,'expected_user_gross_gas_by_index':gross,
        'expected_block_gas_used':block_gas,'expected_anchor_gas_used':167355,
        'transaction_gas_limit':gas_limit,'full_envelope_same_across_orders':True,
        'native_remaining_gas_before_users':remaining,'native_header_gas_limit':31000000,'charged_block_gas_budget':30000000,
        'target_charged_block_gas':29990000,'minimum_full_charged_block_gas':29700000,
        'envelope_policy':'Shared across all orders and ownN0; ceil((largest full gross with either role first+2301)*64/63), preserving EIP150 and SSTORE sentry headroom. Verify each31M-header prefix; not30M-header admission.',
        'calldata_floor_scope':'Record21000+10*tokens alongside21000+4*tokens intrinsic. All predictions exceed the candidate floor before floor application; no fork-activation inference or floor discount.',
        'expected_user_success':True,'expected_exceptional_instructions':0,'transaction_kind':'call','execution_profile':'evm-gas-only',
        'runtime_bytes':len(code),'runtime_sha256':digest(code),'preload_created_accounts_or_code':False}
    return row


def prepare(output,*,model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}',model_id) is None:raise ValueError('model_id must be lowercase SHA256')
    plans=[make_case(k,p,o,l) for k in KERNELS for p in (4,16) for o in ORDERS for l in ('zero','full')]
    ids={row['sample_id'] for row in plans};assert len(plans)==len(ids)==24
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir();rows=[]
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw);rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'frozen_model_id':model_id,
        'purpose':'Twenty-four shared-account order-composition inputs; paired SWAP16 and dirty/mixed caught REVERT,4/16users,three orders and ownN0.',
        'generator_sha256':digest(Path(__file__).read_bytes()),
        'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in ('storage_hash_composition.py','stack_density.py','initcode_preparation.py','top_level_create.py','spawn_memory.py')},
        'actuals_used_for_fitting':False,'required_comparison_quantities':['actual normalized identical-proposal SP1PGU','node estimate per block and literal sum with aggregate diagnostic','complete proposal estimate'],
        'interpretation':'The cursor/table/phase-original/dispatch/witness work is real and remains in absolute metrics; no retrospective allocation or refit. All12ownN0 controls and failures remain. Full inputs require29.7M..30M charged gas including Anchor with31M native header. No arbitrary signed calldata/accounts, nodeL, per-user actuals, proof-time or global-bound claim.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',default=MODEL);args=parser.parse_args()
    print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
