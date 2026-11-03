"""Bounded CALL-frame oracle; unchanged single-frame oracle executes leaf bytes.

Only the wrapper's finite instruction subset is interpreted here. Leaf intrinsic
is offset on entry and removed on exit; no child refund cap affects parent gas.
This is a literal panel oracle, not a general EVM or an authenticated trie engine.
"""
from collections import Counter

from test_storage_mutation import decode, execute


def run_users(row, *, limit=4):
    program=row['request']['program']; contracts={c['address']:c for c in program['prestate_contracts']}
    states={owner:{int(k,16):int(v,16) for k,v in c['initial_storage'].items()} for owner,c in contracts.items()}
    records=[]

    def frame(owner, budget, depth, active):
        assert depth <= 4, 'frame depth bound'
        assert owner in contracts, 'unknown CALL target'
        assert owner not in active, 'cyclic CALL chain'
        active=active|{owner}; code=bytes.fromhex(contracts[owner]['code_hex'][2:])
        is_wrapper=any(op==0xf1 for _,op,_ in decode(code))
        if not is_wrapper:
            result=execute(code,states[owner],budget+21000)
            assert result['terminal'] in (0,0xfd), 'leaf guard failed'
            leaf=dict(owner=owner,result=result,budget=budget)
            return dict(terminal=result['terminal'],gas=result['gas']-21000,refund=result['refund'],
                        counts=result['counts'],stack=result['stack'],returndata=b''), [leaf]
        assert states[owner]=={}, 'wrapper must own empty storage'
        stack=[]; counts=Counter(); pc=0; used=0; ordinary_pre=ordinary_post=0
        called=False; returndata=b''; descendants=[]; call=None
        boundaries={at for at,op,_ in decode(code) if op==0x5b}
        for _ in range(64):
            assert 0 <= pc < len(code), 'wrapper PC out of bounds'
            op=code[pc]; pc+=1; counts[f'opcode:0x{op:02x}']+=1; cost=0
            if 0x60<=op<=0x7f:
                width=op-0x5f; assert pc+width<=len(code), 'truncated wrapper PUSH'
                stack.append(int.from_bytes(code[pc:pc+width],'big')); pc+=width; cost=3
            elif op==0xf1:
                assert not called, 'multiple calls in wrapper'
                requested,target,value,inoff,insize,outoff,outsize=[stack.pop() for _ in range(7)]
                assert value==inoff==insize==outoff==outsize==0, 'CALL must have zero value and empty ranges'
                target=f'0x{target:040x}'; assert target not in active, 'CALL target must be cold and disjoint'
                used+=2600; available=budget-used; assert available>=0, 'CALL out of gas'
                forwarded=min(requested,available-available//64)
                child,descendants=frame(target,forwarded,depth+1,active)
                assert child['gas']<=forwarded, 'child out of gas'
                used+=child['gas']; counts.update(child['counts']); status=int(child['terminal']==0)
                stack.append(status); returndata=child['returndata']; called=True
                call=dict(target=target,requested=requested,available=available,forwarded=forwarded,
                    child_used=child['gas'],child_remaining=forwarded-child['gas'],
                    parent_remaining_after_child=budget-used,child_status=status)
            elif op==0x14: stack.append(int(stack.pop()==stack.pop())); cost=3
            elif op==0x15: stack.append(int(stack.pop()==0)); cost=3
            elif op==0x3d: stack.append(len(returndata)); cost=2
            elif op==0x57:
                target,condition=stack.pop(),stack.pop(); cost=10
                if condition:
                    assert target in boundaries, 'invalid wrapper jump'; pc=target
            elif op==0x5b: cost=1
            elif op==0:
                assert called and stack==[], 'wrapper must call and consume both guards'
                result=dict(terminal=0,gas=used,refund=child['refund'],counts=counts,stack=stack,returndata=returndata)
                current=dict(owner=owner,result=result,call=call,ordinary_pre=ordinary_pre,
                    ordinary_post=ordinary_post,cold_target_fee=2600)
                return result,[current]+descendants
            elif op==0xfe: raise AssertionError('wrapper guard failed')
            else: raise AssertionError(f'unsupported wrapper opcode {op:02x}')
            used+=cost
            if called: ordinary_post+=cost
            else: ordinary_pre+=cost
            assert used<=budget, 'wrapper out of gas'
        raise AssertionError('wrapper instruction bound exceeded')

    for recipient in program['transactions']['recipients'][:limit]:
        result,frames=frame(recipient,program['transactions']['gas_limit']-21000,0,set())
        result=result.copy(); result['gas']+=21000
        result['applied']=min(result['refund'],result['gas']//5); result['charged']=result['gas']-result['applied']
        records.append(dict(recipient=recipient,result=result,frames=frames,state=states[frames[-1]['owner']].copy()))
    return records,states
