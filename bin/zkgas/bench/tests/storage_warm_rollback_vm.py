"""Bounded actual-byte DELEGATECALL oracle with per-frame journal checkpoints.

Code/storage identities, warm account/slot sets, storage and refunds are separate.
Reverting a frame restores its entry checkpoint; attempted counters/events remain.
Only the finite panel subset is supported; this is not a general EVM/trie engine.
"""
from collections import Counter

from test_storage_mutation import decode, execute

WRAPPER_STEP_CAP = 20_000


def run_users(row, *, limit=4):
    program=row['request']['program']; contracts={c['address']:c for c in program['prestate_contracts']}
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
    return records,states
