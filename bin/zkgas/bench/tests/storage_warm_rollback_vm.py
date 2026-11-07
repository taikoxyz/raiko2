"""Bounded actual-byte DELEGATECALL oracle with per-frame journal checkpoints.

Code/storage identities, warm account/slot sets, storage and refunds are separate.
Reverting a frame restores its entry checkpoint; attempted counters/events remain.
Only the finite panel subset is supported; this is not a general EVM/trie engine.
"""
from collections import Counter

from test_storage_mutation import decode, execute

WRAPPER_STEP_CAP = 65_536


def run_users(row, *, limit=4, enforce_bounds=True):
    program=row['request']['program']; p=row['parameters']; tx=program['transactions']
    users=p.get('user_count',4); keys=p['keys_per_account']; depth=p['depth']; warm=p['prewarm']; rounds=p['rounds_per_key']; mode=p['exit_mode']
    assert type(users) is int and 1<=users<=4, 'user_count metadata domain'
    assert type(keys) is int and 1<=keys<=1024 and type(depth) is int and depth in (2,4), 'key/depth metadata domain'
    assert type(warm) is bool and type(rounds) is int and 0<=rounds<=65536 and mode in ('leaf','ancestor'), 'workload metadata domain'
    assert type(limit) is int and 1<=limit<=4, 'explicit partial execution limit'
    assert type(enforce_bounds) is bool, 'bounds-check mode'
    assert type(tx['count_per_block']) is int and tx['count_per_block']==users, 'transaction count metadata'
    recipients=tx['recipients']; assert type(recipients) is list and len(recipients)==len(set(recipients))==users, 'recipient count/identity'
    assert set(row['request'])=={'schema_version','expected_user_success','program'} and row['request']['schema_version']==1 and row['request']['expected_user_success'] is True, 'request domain'
    assert set(program)=={'code_hex','input_hex','block_count','expected_route_counts','transactions','prestate_contracts'}, 'program domain'
    assert tx==dict(kind='call',count_per_block=users,gas_limit=p['transaction_gas_limit'],recipients=recipients), 'transaction descriptor schedule'
    assert program['code_hex']=='0x00' and program['input_hex']=='0x' and program['block_count']==1, 'base runtime/input/block domain'
    assert type(tx['gas_limit']) is int and 0<tx['gas_limit']<=15000000 and tx['gas_limit']<=16777216, 'transaction gas caps'
    expected=dict(aggregate_state_size=users*(keys+1),user_transactions_per_block=users,all_user_transactions=users,
        all_transactions_including_anchor=users+1,account_count=users*depth+1,total_account_count=users*depth+1,
        storage_owner_count=users,wrapper_account_count=users*depth,expected_distinct_account_slot_pairs=users*(keys+1),
        expected_distinct_write_pairs=users*keys,expected_seed_storage_writes=users*keys,expected_cleanup_storage_writes=users*keys,
        expected_initial_storage_entry_count=users*(keys+1),expected_final_storage_entry_count=users*(keys+1),
        expected_storage_reads=users*(2+6*keys+(1+int(warm))*(keys+1)),expected_storage_writes=users*keys*(2+2*rounds),
        expected_dirty_storage_writes=2*users*keys*rounds,expected_main_storage_writes=users*keys*(2+2*rounds),
        expected_storage_writes_per_account=keys*(2+2*rounds),expected_call_count=users*depth,
        expected_wrapper_stops=users*(depth-int(mode=='ancestor')),expected_leaf_stops=users*int(mode=='ancestor'),
        expected_leaf_reverts=users*int(mode=='leaf'),expected_ancestor_reverts=users*int(mode=='ancestor'),
        expected_user_refund_counter_before_terminal=0,expected_user_refund_counter=0,expected_user_refund_applied=0)
    for name,value in expected.items(): assert type(p[name]) is int and p[name]==value, 'count metadata: '+name
    assert p['recipient_schedule']==p['storage_owner_addresses']==recipients, 'owner/recipient schedule metadata'
    assert p['expected_user_receipt_statuses']==[True]*users, 'receipt status metadata'
    assert p['start_phase_schedule']==p['final_phase_schedule']==[65537]*users, 'phase schedule metadata'
    assert p['expected_all_frame_terminal_counts']==dict(STOP=users*depth,REVERT=users), 'terminal counts metadata'
    contracts={c['address']:c for c in program['prestate_contracts']}
    assert len(contracts)==users*depth+1, 'only selected reachable configured accounts'
    assert set(contracts)=={a['address'] for a in p['account_roles']}, 'account roles metadata'
    assert p['wrapper_levels'][0]==recipients and len(p['wrapper_levels'])==depth and all(len(level)==users for level in p['wrapper_levels']), 'wrapper levels metadata'
    assert set(contracts)==set(sum(p['wrapper_levels'],[]))|set(p['leaf_code_addresses']), 'configured graph metadata'
    assert len(p['leaf_code_addresses'])==1 and p['leaf_code_addresses']==p['leaf_owners'], 'shared leaf metadata'
    state={0:65537}|dict.fromkeys(range(65537,65537+keys),1)
    for owner,contract in contracts.items():
        assert set(contract)=={'address','code_hex','initial_storage'}, 'contract descriptor domain'
        actual={int(k,16):int(v,16) for k,v in contract['initial_storage'].items()}
        assert actual==(state if owner in recipients else {}), 'selected-owner initial storage'
    assert p['storage_entry_count_by_account']=={a:(keys+1 if a in recipients else 0) for a in contracts}, 'storage counts by owner metadata'
    assert len(contracts)==len(program['prestate_contracts']), 'duplicate account'
    states={owner:{int(k,16):int(v,16) for k,v in c['initial_storage'].items()} for owner,c in contracts.items()}
    records=[]

    for recipient in program['transactions']['recipients'][:limit]:
        warm_accounts={recipient}; warm_slots=set(); refund=0

        def frame(code_owner, storage_owner, budget, depth, active):
            nonlocal refund
            assert depth<=4, 'frame depth bound'
            assert code_owner in contracts, 'unknown DELEGATECALL target'
            assert code_owner not in active, 'cyclic DELEGATECALL chain'
            active=active|{code_owner}
            state_checkpoint={owner:state.copy() for owner,state in states.items()}
            account_checkpoint=warm_accounts.copy(); slot_checkpoint=warm_slots.copy(); refund_checkpoint=refund
            code=bytes.fromhex(contracts[code_owner]['code_hex'][2:])

            def rollback():
                nonlocal refund
                for owner,state in state_checkpoint.items(): states[owner].clear(); states[owner].update(state)
                warm_accounts.clear(); warm_accounts.update(account_checkpoint)
                warm_slots.clear(); warm_slots.update(slot_checkpoint); refund=refund_checkpoint

            common=dict(owner=code_owner,code_address=code_owner,storage_address=storage_owner,budget=budget,
                warm_accounts_before=account_checkpoint,warm_slots_before=slot_checkpoint)
            if not any(op==0xf4 for _,op,_ in decode(code)):
                initial_keys={key for owner,key in warm_slots if owner==storage_owner}
                result=execute(code,states[storage_owner],budget+21000,initial_warm_keys=initial_keys)
                assert result['terminal'] in (0,0xfd), 'leaf guard failed'
                for event in result['reads']+result['writes']: warm_slots.add((storage_owner,event[0]))
                refund+=result['refund']
                before=result.get('refund_before_revert',result['refund'])
                if result['terminal']==0xfd: rollback()
                leaf=common|dict(result=result,refund_before_terminal=before,
                    warm_accounts_after=warm_accounts.copy(),warm_slots_after=warm_slots.copy())
                return dict(terminal=result['terminal'],gas=result['gas']-21000,refund=refund-refund_checkpoint,
                    counts=result['counts'],stack=result['stack'],returndata=b''),[leaf]

            stack=[]; memory=bytearray(); counts=Counter(); reads=[]; pc=0; used=0
            pre_gas=post_gas=0; called=False; returndata=b''; descendants=[]; call=None; cold_fee=0
            boundaries={at for at,op,_ in decode(code) if op==0x5b}
            for _ in range(WRAPPER_STEP_CAP):
                assert 0<=pc<len(code), 'wrapper PC out of bounds'
                op=code[pc]; pc+=1; counts[f'opcode:0x{op:02x}']+=1; cost=0
                if 0x60<=op<=0x7f:
                    width=op-0x5f; assert pc+width<=len(code), 'truncated wrapper PUSH'
                    stack.append(int.from_bytes(code[pc:pc+width],'big')); pc+=width; cost=3
                elif op==1: stack.append((stack.pop()+stack.pop())% (1<<256)); cost=3
                elif op==0x14: stack.append(int(stack.pop()==stack.pop())); cost=3
                elif op==0x15: stack.append(int(stack.pop()==0)); cost=3
                elif op in (0x51,0x52):
                    offset=stack.pop(); assert offset==0, 'top memory must use one counter word'
                    if not memory: memory.extend(bytes(32)); cost+=3
                    cost+=3
                    if op==0x51: stack.append(int.from_bytes(memory,'big'))
                    else: memory[:]=stack.pop().to_bytes(32,'big')
                elif op==0x54:
                    key=stack.pop(); pair=(storage_owner,key); cold=pair not in warm_slots; warm_slots.add(pair)
                    value=states[storage_owner].get(key,0); reads.append((key,value,cold)); stack.append(value)
                    cost=2100 if cold else 100
                elif op in (0x56,0x57):
                    target=stack.pop(); condition=op==0x56 or stack.pop()!=0; cost=8 if op==0x56 else 10
                    if condition:
                        assert target in boundaries, 'invalid wrapper jump'; pc=target
                elif op==0x5b: cost=1
                elif op==0x3d: stack.append(len(returndata)); cost=2
                elif op==0xf4:
                    assert not called, 'multiple calls in wrapper'
                    requested,target,inoff,insize,outoff,outsize=[stack.pop() for _ in range(6)]
                    assert inoff==insize==outoff==outsize==0, 'DELEGATECALL must use empty ranges'
                    target=f'0x{target:040x}'; cold=target not in warm_accounts; warm_accounts.add(target)
                    cold_fee=2600 if cold else 100; used+=cold_fee; pre_gas+=cold_fee
                    available=budget-used; assert available>=0, 'DELEGATECALL out of gas'
                    forwarded=min(requested,available-available//64)
                    child,descendants=frame(target,storage_owner,forwarded,depth+1,active)
                    assert child['gas']<=forwarded, 'child out of gas'
                    used+=child['gas']; counts.update(child['counts']); status=int(child['terminal']==0)
                    stack.append(status); returndata=child['returndata']; called=True
                    call=dict(kind='delegatecall',target=target,requested=requested,available=available,forwarded=forwarded,
                        child_used=child['gas'],child_remaining=forwarded-child['gas'],parent_remaining_after_child=budget-used,child_status=status)
                elif op in (0,0xfd):
                    assert called, 'wrapper must call'
                    if op==0xfd:
                        offset,length=stack.pop(),stack.pop(); assert offset==length==0, 'nonempty wrapper REVERT'
                    assert stack==[], 'wrapper guard operands remain'
                    assert used<=budget, 'wrapper out of gas'
                    before=refund-refund_checkpoint
                    if op==0xfd: rollback()
                    result=dict(terminal=op,gas=used,refund=refund-refund_checkpoint,counts=counts,stack=stack,returndata=returndata,memory=memory)
                    current=common|dict(result=result,call=call,reads=reads,pre_gas=pre_gas,post_gas=post_gas,
                        cold_target_fee=cold_fee,refund_before_terminal=before,
                        warm_accounts_after=warm_accounts.copy(),warm_slots_after=warm_slots.copy())
                    return result,[current]+descendants
                elif op==0xfe: raise AssertionError('wrapper guard failed')
                else: raise AssertionError(f'unsupported wrapper opcode {op:02x}')
                used+=cost
                if called: post_gas+=cost
                else: pre_gas+=cost
                assert used<=budget, 'wrapper out of gas'
            raise AssertionError('wrapper instruction bound exceeded')

        result,frames=frame(recipient,recipient,program['transactions']['gas_limit']-21000,0,set())
        result=result.copy(); result['gas']+=21000
        result['applied']=min(result['refund'],result['gas']//5); result['charged']=result['gas']-result['applied']
        records.append(dict(recipient=recipient,result=result,frames=frames,state=states[recipient].copy(),
            warm_accounts=warm_accounts.copy(),warm_slots=warm_slots.copy()))
    assert records, 'at least one explicit selected user must execute'
    gross=records[0]['result']['gas']; charged=records[0]['result']['charged']
    for record in records:
        assert record['result']['gas']==gross and record['result']['charged']==charged, 'uniform user gas'
        assert record['result']['refund']==record['result']['applied']==0, 'rollback removes effective refund'
        frames=record['frames']; assert len(frames)==depth+1, 'concrete frame depth'
        required=frames[-1]['result']['gas']-21000+2301; requirements=[required]
        for frame_record in reversed(frames[:-1]):
            required=frame_record['pre_gas']+max((64*required+62)//63,required+frame_record['post_gas']); requirements.append(required)
        assert p['gas_accounting_per_user']['minimum_frame_envelopes_leaf_to_root']==requirements, 'executed EIP150 envelope metadata'
        assert p['gas_accounting_per_user']['required_transaction_envelope']==21000+required, 'required envelope metadata'
    total=167355+users*charged; prefixes=[31000000-167355-i*charged for i in range(users)]
    for name,value in dict(expected_user_gross_gas=gross,expected_user_charged_gas=charged,expected_block_charged_gas=total).items():
        assert type(p[name]) is int and p[name]==value, 'gas metadata: '+name
    assert p['gas_accounting_per_user']['gross_gas']==p['gas_accounting_per_user']['charged_gas']==gross, 'per-user gas metadata'
    assert p['header_prefix_remaining_gas']==prefixes, 'header prefix metadata'
    if enforce_bounds:
        assert all(value>=tx['gas_limit'] for value in prefixes) and total<=30000000, 'block/header prefix budgets'
    if len(records)==users:
        counts=Counter()
        for record in records: counts.update(record['result']['counts'])
        assert counts==+Counter(program['expected_route_counts']), 'concrete aggregate route counts'
        assert {frame_record['owner'] for record in records for frame_record in record['frames']}==set(contracts), 'all configured code accounts reached'
    return records,states
