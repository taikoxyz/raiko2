"""Independent literal byte oracle for direct-write transient read catalogs.

No producer assembler/gas imports. A pure-read body proof executes every decoded
site, retains the whole transient mapping and stack, and proves no memory access.
Live complete-state preconditions still run on each oracle-only cached iteration.
Real native/SP1 execution never uses this cache and performs every instruction.
"""
from collections import Counter
import hashlib
import json
import re

QUERY=0x1122334455667788 << 192
PRIMARY='0x'+'66'*20
SENTINEL=0xa55a
MASK=(1<<256)-1


def decode(code):
    pc=0
    while pc<len(code):
        at=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code),'truncated PUSH'
        yield at,op,int.from_bytes(code[pc:pc+width],'big');pc+=width


def structure(instructions,repetitions,*,key_count=512,query_key=QUERY):
    """Check literal full-program intent, including unreachable R0 body/guards."""
    assert type(query_key) is int and 0<=query_key<=MASK,'query domain'
    index=0;failures=[]
    def take(opcode,value=0):
        nonlocal index
        assert index<len(instructions),'missing instruction'
        at,op,immediate=instructions[index];index+=1
        assert op==opcode and (value is None or immediate==value),'byte intent/guard shape'
        return at,immediate
    def push(value,width=3):return take(0x5f+width,value)
    def load(offset):push(offset);return take(0x51)[0]
    def guard(value):
        push(value,32);take(0x14);take(0x15);failures.append(take(0x61,None)[1]);take(0x57)
    take(0x36);guard(32*key_count)
    push(query_key,32);initial_read=take(0x5c)[0];guard(0)
    for offset,value in ((0,0),(64,0),(96,SENTINEL)):push(value);push(offset);take(0x52)
    seed,_=take(0x5b);load(0);push(key_count);take(0x14);_,seed_done_target=take(0x61,None);take(0x57)
    seed_value=push(1)[0];load(0);push(5);take(0x1b);seed_load=take(0x35)[0];seed_write=take(0x5d)[0]
    load(0);push(1);take(1);push(0);take(0x52);take(0x61,seed);take(0x56)
    seed_done,_=take(0x5b);assert seed_done==seed_done_target,'seed target'
    push(32*(key_count-1));query_load=take(0x35)[0];take(0x80);seeded_read=take(0x5c)[0];guard(1)
    loop,_=take(0x5b);load(64);push(repetitions,32);take(0x14);_,done_target=take(0x61,None);take(0x57)
    body=instructions[index][0];body_pcs=[]
    for _ in range(64):take(0x80);body_pcs.append(take(0x5c)[0]);take(0x50)
    body_end=instructions[index][0]
    load(64);push(1);take(1);push(64);take(0x52);take(0x61,loop);take(0x56)
    done,_=take(0x5b);assert done_target==done,'done target'
    take(0x80);guard(query_key);take(0x50)
    for offset,value in ((0,key_count),(64,repetitions),(96,SENTINEL)):load(offset);guard(value)
    take(0);failure,_=take(0x5b);take(0xfe)
    assert index==len(instructions) and failures==[failure]*7,'failure targets/extra bytes'
    return dict(seed=seed,seed_value=seed_value,seed_load=seed_load,seed_write=seed_write,
        initial_read=initial_read,query_load=query_load,seeded_read=seeded_read,loop=loop,
        body=body,body_end=body_end,body_pcs=body_pcs)


def _entry(stack,transient,memory,query,expected,*,key_count):
    assert list(stack)==[query] and type(query) is int and 0<=query<=MASK,'complete query stack precondition'
    assert type(transient) is dict and query in transient and len(transient)==key_count and tuple(sorted(transient.items()))==expected,'complete transient-map feedback'
    assert all(type(k) is int and type(v) is int and v==1 for k,v in transient.items()),'transient key/value types'
    assert type(memory) is bytearray and len(memory)==128,'complete memory footprint'
    assert int.from_bytes(memory[:32],'big')==key_count and memory[32:64]==bytes(32) and int.from_bytes(memory[96:128],'big')==SENTINEL,'protected seed/memory/sentinel'


def prove_body(instructions,entry_stack,transient,memory,query,*,key_count=512):
    expected=tuple(sorted(transient.items()));_entry(entry_stack,transient,memory,query,expected,key_count=key_count)
    assert len(instructions)==192,'body exact64-site length'
    stack=list(entry_stack);state=dict(transient);before_memory=bytes(memory);counts=Counter();sites=[];gas=0
    for index,(at,op,immediate) in enumerate(instructions):
        assert immediate==0 and op==(0x80,0x5c,0x50)[index%3],'decoded body read intent'
        counts[f'opcode:0x{op:02x}']+=1
        if op==0x80:stack.append(stack[-1]);gas+=3
        elif op==0x5c:
            key=stack.pop();assert key==query and state[key]==1,'body key/value';stack.append(state[key]);gas+=100;sites.append(at)
        else:assert stack.pop()==1,'discarded read value';gas+=2
    assert stack==list(entry_stack)==[query] and state==transient and bytes(memory)==before_memory and len(sites)==64,'complete pure-read transition'
    return dict(key_count=key_count,query=query,transient=expected,counts=counts,gas=gas,sites=tuple(sites),reads=64,writes=0)


def apply_body(proof,stack,transient,memory):
    _entry(stack,transient,memory,proof['query'],proof['transient'],key_count=proof['key_count'])
    # The authenticated body has no memory or write opcode; unchanged memory holds
    # for every current loop counter. Outer flow independently checks that counter.


def _keys(data,*,key_count,query_key=QUERY,raw_keys=False):
    assert type(key_count) is int and 1<=key_count<=1024,'key_count domain'
    assert type(query_key) is int and 0<=query_key<=MASK and type(raw_keys) is bool,'query/raw policy domain'
    assert type(data) is bytes and len(data)==32*key_count,'calldata length'
    keys=[int.from_bytes(data[i:i+32],'big') for i in range(0,len(data),32)]
    assert len(set(keys))==key_count and keys[-1]==query_key,'unique catalog/query last'
    if not raw_keys:
        assert query_key==QUERY and all(sum(b!=0 for b in k.to_bytes(32,'big'))==8 for k in keys),'strict query/calldata byte cost'
    return keys


def execute(code,data,envelope,*,repetitions,cache=True,key_count=512,query_key=QUERY,raw_keys=False):
    assert type(repetitions) is int and 0<=repetitions<=1000000 and type(cache) is bool,'R/cache domain'
    assert type(code) is bytes and len(code)<=24576 and type(envelope) is int and envelope>=21000,'code/envelope domain'
    keys=_keys(data,key_count=key_count,query_key=query_key,raw_keys=raw_keys);instructions=list(decode(code));shape=structure(instructions,repetitions,key_count=key_count,query_key=query_key)
    by_pc={at:(op,value) for at,op,value in instructions};jumps={at for at,op,_ in instructions if op==0x5b}
    proof_memory=bytearray(128);proof_memory[:32]=key_count.to_bytes(32,'big');proof_memory[96:128]=SENTINEL.to_bytes(32,'big')
    body=[i for i in instructions if shape['body']<=i[0]<shape['body_end']]
    proof=prove_body(body,[query_key],dict.fromkeys(keys,1),proof_memory,query_key,key_count=key_count)
    stack=[];memory=bytearray();transient={};counts=Counter();pc_counts=Counter();seed_keys=[];query_values=[]
    zero=data.count(0);nonzero=len(data)-zero;intrinsic=21000+zero*4+nonzero*16;floor=21000+10*(zero+4*nonzero);gross=intrinsic;pc=0;expansion=0;reads=0;writes=0;iterations=0;loop_visits=0
    def word(offset):return int.from_bytes(memory[offset:offset+32],'big')
    for _ in range(2000000):
        assert pc in by_pc,'invalid PC'
        if pc==shape['seed']:assert stack==[] and word(0)==len(seed_keys),'seed counter/stack'
        if pc==shape['loop']:
            assert word(64)==loop_visits and word(0)==key_count and word(96)==SENTINEL,'loop counter/sentinel'
            _entry(stack,transient,memory,query_key,proof['transient'],key_count=key_count);loop_visits+=1
        if pc==shape['body']:
            assert word(64)==iterations<repetitions,'body counter';iterations+=1
            if cache:
                apply_body(proof,stack,transient,memory);gross+=proof['gas'];counts.update(proof['counts']);reads+=64
                pc_counts.update({at:1 for at,_,_ in body});pc=shape['body_end'];assert gross<=envelope,'out of gas';continue
        at=pc;op,immediate=by_pc[pc];pc+=1+(op-0x5f if 0x60<=op<=0x7f else 0)
        counts[f'opcode:0x{op:02x}']+=1;pc_counts[at]+=1
        if 0x60<=op<=0x7f:stack.append(immediate);gross+=3
        elif op==0x80:stack.append(stack[-1]);gross+=3
        elif op==0x50:stack.pop();gross+=2
        elif op in (1,0x14,0x1b):
            x,y=stack.pop(),stack.pop();stack.append((x+y)&MASK if op==1 else int(x==y) if op==0x14 else (y<<x)&MASK);gross+=3
        elif op==0x15:stack.append(int(stack.pop()==0));gross+=3
        elif op==0x36:stack.append(len(data));gross+=2
        elif op==0x35:
            offset=stack.pop();expected=32*len(seed_keys) if at==shape['seed_load'] else 32*(key_count-1)
            assert offset==expected and offset<=32*(key_count-1),'calldata offset/order'
            stack.append(int.from_bytes(data[offset:offset+32],'big'));gross+=3
        elif op in (0x51,0x52):
            offset=stack.pop();assert offset in (0,64,96),'memory domain'
            old=len(memory)//32;new=max(old,(offset+63)//32);delta=3*new+new*new//512-3*old-old*old//512
            expansion+=delta;gross+=3+delta;memory.extend(bytes(32*(new-old)))
            if op==0x51:stack.append(word(offset))
            else:memory[offset:offset+32]=stack.pop().to_bytes(32,'big')
        elif op==0x5c:
            key=stack.pop();value=transient.get(key,0);assert key==query_key,'read query'
            if at==shape['initial_read']:assert transient=={} and reads==0 and value==0,'initial empty transient query';query_values.append(value)
            else:
                assert len(transient)==key_count and value==1,'seeded query value'
                if at==shape['seeded_read']:assert reads==1,'seeded query order';query_values.append(value)
            stack.append(value);reads+=1;gross+=100
        elif op==0x5d:
            key,value=stack.pop(),stack.pop()
            assert at==shape['seed_write'] and writes<key_count and key==keys[writes] and value==1 and key not in transient,'direct TSTORE1 order/empty previous value'
            transient[key]=value;seed_keys.append(key);writes+=1;gross+=100
        elif op in (0x56,0x57):
            target=stack.pop();taken=op==0x56 or stack.pop()!=0;gross+=8 if op==0x56 else 10
            if taken:assert target in jumps,'jump target';pc=target
        elif op==0x5b:gross+=1
        elif op in (0,0xfe):
            assert op==0 and stack==[],'guard failure/terminal stack'
            assert (word(0),word(64),word(96),len(memory))==(key_count,repetitions,SENTINEL,128),'final counter/memory'
            assert iterations==repetitions and loop_visits==repetitions+1 and reads==2+64*repetitions and writes==key_count and seed_keys==keys,'complete read/write footprint'
            assert tuple(sorted(transient.items()))==proof['transient'] and query_values==[0,1],'complete terminal transient state'
            charged=max(gross,floor);assert charged<=envelope,'charged floor exceeds envelope'
            grouped={str(k):v for k,v in sorted(pc_counts.items())};hex_keys=[f'0x{k:064x}' for k in seed_keys]
            commitment=hashlib.sha256(json.dumps(dict(keys=hex_keys,query=f'0x{query_key:064x}',pc_counts=grouped),sort_keys=True,separators=(',',':')).encode()).hexdigest()
            return dict(terminal=op,stack=stack,counter=repetitions,gross_gas=gross,charged_gas=charged,counts=counts,
                tload_count=reads,tstore_count=writes,seed_keys=hex_keys,query_values=query_values,
                transient_state_at_stop=dict.fromkeys(hex_keys,1),peak_memory_bytes=len(memory),memory_expansion_gas=expansion,
                grouped_pc_counts=grouped,read_intent_sha256=commitment)
        else:raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack)<=1024 and gross<=envelope,'stack bound/out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p=row['parameters'];request=row['request'];program=request['program'];tx=program['transactions'];k=p['key_count'];r=p['repetitions']
    raw_keys=p.get('raw_keys',False);assert type(raw_keys) is bool and ('raw_keys' not in p or raw_keys is True),'explicit raw-key metadata policy'
    text=p['query_key'];assert type(text) is str and re.fullmatch(r'0x[0-9a-f]{64}',text),'canonical query metadata'
    query_key=int(text,16)
    assert type(k) is int and 1<=k<=1024 and type(r) is int and 0<=r<=1000000,'typed K/R metadata'
    assert set(request)=={'schema_version','expected_user_success','program'} and type(request['schema_version']) is int and request['schema_version']==1 and request['expected_user_success'] is True,'request domain'
    assert set(program)=={'code_hex','input_hex','block_count','expected_route_counts','transactions'},'empty persistent/no extra code domain'
    assert set(tx)=={'kind','count_per_block','gas_limit'} and tx['kind']=='call' and type(tx['count_per_block']) is int and tx['count_per_block']==1,'single user CALL'
    assert type(program['block_count']) is int and program['block_count']==1,'one block'
    assert type(tx['gas_limit']) is int and tx['gas_limit']==p['transaction_gas_limit'] and 21000<=tx['gas_limit']<=15000000 and tx['gas_limit']<=16777216,'typed envelope/caps'
    assert p['primary_code_owner']==PRIMARY and type(p['transaction_nonce']) is int and p['transaction_nonce']==0 and p['recipient_schedule']==[PRIMARY],'owner/nonce'
    code=bytes.fromhex(program['code_hex'][2:]);data=bytes.fromhex(program['input_hex'][2:]);keys=_keys(data,key_count=k,query_key=query_key,raw_keys=raw_keys)
    assert [f'0x{key:064x}' for key in keys]==p['ordered_keys'],'literal ordered keys'
    assert hashlib.sha256(data).hexdigest()==p['calldata_sha256']==p['raw_key_sha256'] and hashlib.sha256(code).hexdigest()==p['runtime_sha256'],'raw data/code hashes'
    result=execute(code,data,tx['gas_limit'],repetitions=r,key_count=k,query_key=query_key,raw_keys=raw_keys)
    zero=data.count(0);nonzero=len(data)-zero
    gross=result['gross_gas'];charged=result['charged_gas'];intrinsic=21000+4*zero+16*nonzero;floor=21000+10*(zero+4*nonzero)
    # Byte-authenticated loop head26 + tail29, plus64*(DUP1 3+TLOAD100+POP2).
    # K affects only the directly executed seed, so its actual base is retained.
    slope=64*105+26+29;base=gross-slope*r;maximum=(15000000-1000-base)//slope
    assert type(p['full_repetitions']) is int and p['full_repetitions']==maximum>0,'maximal full-R metadata'
    assert type(p['transaction_gas_limit']) is int and tx['gas_limit']==max(base+slope*maximum,floor)+1000,'shared maximal full envelope'
    assert p['full_plus_one_constraint_failures']==['transaction_fixture_cap'] and base+slope*(maximum+1)+1000>15000000,'full+1 selection'
    level=p['workload_level'];assert type(level) is str and level in ('zero','probe','full'),'workload level'
    assert (r==0 if level=='zero' else r==maximum if level=='full' else 0<r<maximum),'level/R intent'
    expected=dict(seed_writes=k,expected_transient_reads=2+64*r,expected_transient_writes=k,initial_query_reads=1,seeded_query_reads=1,
        body_target_operations_per_user=64*r,target_query_reads=2+64*r,expected_persistent_storage_reads=0,expected_persistent_storage_writes=0,
        initial_storage_entries=0,initial_state_size=0,aggregate_state_size=0,state_size_per_account=0,
        expected_initial_storage_entry_count=0,expected_final_storage_entry_count=0,peak_transient_state_entries_per_transaction=k,
        expected_final_transient_entry_count=0,primary_code_owner_count=1,account_count=1,seed_counter_memory_offset=0,
        counter_memory_offset=64,protected_memory_offset=96,expected_counter=r,sentinel=SENTINEL,peak_memory_bytes=128,memory_words=4,
        outer_calldata_bytes=32*k,calldata_zero_bytes=zero,calldata_nonzero_bytes=nonzero,
        all_user_transactions=1,user_transactions_per_block=1,all_transactions_including_anchor=2,block_count=1,visits_per_account=1,
        transaction_envelope_reserve=1000,header_gas_limit=31000000,charged_gas_budget=30000000,expected_anchor_gas=167355,
        unroll_factor=64,bulk_operations=64*r,expected_user_gross_gas=gross,expected_user_charged_gas=charged,
        expected_user_refund_counter=0,expected_user_refund_applied=0,expected_block_charged_gas=167355+charged,
        runtime_bytes=len(code),expected_exceptional_instructions=0)
    for name,value in expected.items():assert type(p[name]) is int and p[name]==value,'typed count/gas metadata: '+name
    assert p['header_prefix_remaining_gas']==[30832645] and 30832645>=tx['gas_limit'] and 167355+charged<=30000000,'prefix/block caps'
    assert p['expected_terminal']=='STOP' and p['expected_user_success'] is True and type(p['expected_user_receipt_statuses']) is list and len(p['expected_user_receipt_statuses'])==1 and p['expected_user_receipt_statuses'][0] is True and p['expected_anchor_success'] is True,'terminal/status metadata'
    assert p['near_full'] is (167355+charged>=29700000) and p['execution_profile']=='evm-gas-only' and p['preload_created_accounts_or_code'] is False,'scope metadata'
    gas=dict(intrinsic_gas=intrinsic,eip7623_floor_gas=floor,eip7623_floor_active=gross<floor,
        ordinary_opcode_gas=gross-intrinsic-12-100*(2+64*r)-100*k,memory_expansion_gas=12,
        transient_load_gas=100*(2+64*r),transient_store_gas=100*k,transient_load_count=2+64*r,transient_store_count=k,
        body_gas_per_repetition=64*105,gross_gas=gross,charged_gas=charged,refund_counter=0,refund_applied=0,runtime_bytes=len(code))
    actual=p['gas_accounting_per_user'];assert set(actual)==set(gas),'gas metadata domain'
    for name,value in gas.items():assert type(actual[name]) is type(value) and actual[name]==value,'literal gas metadata: '+name
    routes=program['expected_route_counts'];assert all(type(v) is int and v>=0 for v in routes.values()),'route count types'
    assert result['counts']==+Counter(routes),'literal executed routes'
    result['transient_state_after_tx']={}  # The single root's state is transaction-scoped, not persistent.
    return [result]
