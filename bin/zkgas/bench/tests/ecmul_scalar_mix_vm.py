"""Literal ECMUL-vector byte oracle; no producer assembler, counts or gas imports."""
from collections import Counter
from functools import lru_cache
from hashlib import sha256

FIELD=21888242871839275222246405745257275088696311157297823662689037894645226208583
ORDER=21888242871839275222246405745257275088548364400416034343698204186575808495617
MASK=(1<<256)-1


def integer(value,low,high):
    assert type(value) is int and low<=value<=high
    return value


@lru_cache(maxsize=256)
def multiply(scalar):
    """Affine left-to-right multiplication, independently reducing raw U256."""
    def add(a,b):
        if a is None:return b
        if b is None:return a
        x,y=a;u,v=b
        if x==u and (y+v)%FIELD==0:return None
        numerator,denominator=(3*x*x,2*y) if a==b else (v-y,u-x)
        slope=numerator*pow(denominator%FIELD,FIELD-2,FIELD)%FIELD
        nx=(slope*slope-x-u)%FIELD
        return nx,(slope*(x-nx)-y)%FIELD
    point=None
    for bit in bin(scalar%ORDER)[2:]:
        point=add(point,point)
        if bit=='1':point=add(point,(1,2))
    return (0,0) if point is None else point


def validate_payload(data,sequence_length):
    integer(sequence_length,1,64);assert type(data) is bytes and len(data)==96*sequence_length
    entries=[]
    for index in range(sequence_length):
        chunk=data[index*96:(index+1)*96]
        scalar,x,y=(int.from_bytes(chunk[n:n+32],'big') for n in (0,32,64))
        assert x<FIELD and y<FIELD and (x,y)==multiply(scalar)
        reduced=scalar%ORDER
        entries.append(dict(scalar=scalar,reduced_scalar=reduced,point=(x,y),output=chunk[32:],work_units=reduced.bit_length()+reduced.bit_count()))
    return entries


def decode(code):
    assert type(code) is bytes and 1<=len(code)<=24576
    pc=0
    while pc<len(code):
        start=pc;op=code[pc];pc+=1;width=op-0x5f if 0x60<=op<=0x7f else 0
        assert pc+width<=len(code)
        value=int.from_bytes(code[pc:pc+width],'big') if width else None
        yield start,op,value
        pc+=width


def structure(instructions,sequence_length,repetitions):
    """Read every instruction, including the R0-unexecuted body and failure tail."""
    integer(sequence_length,1,64);integer(repetitions,0,1_000_000)
    cursor=0;failures=[]
    def take(op,value=None):
        nonlocal cursor
        assert cursor<len(instructions)
        pc,actual,immediate=instructions[cursor];cursor+=1
        assert actual==op and immediate==value
        return pc
    def push(value):return take(0x62,value)
    def target():
        nonlocal cursor
        pc,op,value=instructions[cursor];assert op==0x61;cursor+=1
        return value
    def require():take(0x15);failures.append(target());take(0x57)
    def check(value):push(value);take(0x14);require()
    def load(offset):push(offset);take(0x51)
    def store(offset,value):pc=push(value);push(offset);take(0x52);return pc
    take(0x36);check(96*sequence_length)
    for offset,value in ((0,1),(32,2),(64,0),(96,0),(128,0),(160,0),(192,0),(224,0xa55a)):store(offset,value)
    loop=take(0x5b);load(192);push(repetitions);take(0x14);done=target();take(0x57)
    calls=[]
    for index in range(sequence_length):
        push(index*96);take(0x35);push(64);take(0x52)
        clear_x=store(96,0);store(128,0)
        size=push(64);push(96);push(96);push(0);address=push(7);gas=push(6000)
        pc=take(0xfa);check(1);take(0x3d);check(64)
        for memory,offset in ((96,index*96+32),(128,index*96+64)):
            load(memory);push(offset);take(0x35);take(0x14);require()
        calls.append(dict(pc=pc,entry_index=index,target_push=address,gas_push=gas,output_size_push=size,clear_x_value=clear_x))
    load(192);push(1);take(1);push(192);take(0x52);assert target()==loop;take(0x56)
    assert take(0x5b)==done
    for offset,value in ((0,1),(32,2),(160,0),(192,repetitions),(224,0xa55a)):load(offset);check(value)
    take(0);failure=take(0x5b);take(0xfe)
    assert failures and set(failures)=={failure} and cursor==len(instructions)
    return dict(loop=loop,done=done,calls=calls)


def execute(code,data,envelope,*,sequence_length,repetitions,override=None):
    integer(envelope,21000,100_000_000)
    instructions=list(decode(code));shape=structure(instructions,sequence_length,repetitions)
    entries=validate_payload(data,sequence_length);ops={pc:(op,value) for pc,op,value in instructions}
    call_entries={c['pc']:c['entry_index'] for c in shape['calls']}
    zero=data.count(0);nz=len(data)-zero;intrinsic=21000+4*zero+16*nz;floor=21000+10*(zero+4*nz)
    gross=intrinsic;memory=bytearray();stack=[];counts=Counter();calls=[];pc=0;return_size=0;memory_gas=0
    costs={0:0,1:3,0x14:3,0x15:3,0x35:3,0x36:2,0x3d:2,0x51:3,0x52:3,0x56:8,0x57:10,0x5b:1,0xfa:0,0xfe:0}
    def pop():assert stack;return stack.pop()
    def expand(end):
        nonlocal gross,memory_gas
        assert 0<=end<=256
        old=len(memory)//32;new=(end+31)//32
        if new>old:
            delta=3*new+new*new//512-3*old-old*old//512
            gross+=delta;memory_gas+=delta;memory.extend(bytes(32*(new-old)))
    def read(offset):
        assert offset in (0,32,64,96,128,160,192,224);expand(offset+32)
        return int.from_bytes(memory[offset:offset+32],'big')
    while True:
        assert pc in ops
        op,value=ops[pc];counts[f'opcode:0x{op:02x}']+=1
        gross+=3 if 0x60<=op<=0x7f else costs[op]
        assert gross<=envelope
        next_pc=pc+1+(op-0x5f if 0x60<=op<=0x7f else 0)
        if 0x60<=op<=0x7f:stack.append(value)
        elif op==0:
            assert not stack and read(0)==1 and read(32)==2 and read(160)==0 and read(192)==repetitions and read(224)==0xa55a
            assert len(calls)==sequence_length*repetitions
            assert read(64)==(entries[-1]['scalar'] if repetitions else 0)
            assert memory[96:160]==(entries[-1]['output'] if repetitions else bytes(64))
            return dict(terminal=0,stack=list(stack),counter=read(192),completed_calls=len(calls),calls=calls,
                counts=counts,gross_gas=gross,charged_gas=max(gross,floor),intrinsic_gas=intrinsic,floor_gas=floor,
                refund_counter=0,memory_hex='0x'+memory.hex(),peak_memory_bytes=len(memory),memory_expansion_gas=memory_gas)
        elif op==1:stack.append((pop()+pop())&MASK)
        elif op==0x14:stack.append(int(pop()==pop()))
        elif op==0x15:stack.append(int(pop()==0))
        elif op==0x35:offset=pop();stack.append(int.from_bytes(data[offset:offset+32].ljust(32,b'\0'),'big'))
        elif op==0x36:stack.append(len(data))
        elif op==0x3d:stack.append(return_size)
        elif op==0x51:stack.append(read(pop()))
        elif op==0x52:
            offset=pop();value=pop();assert offset in (0,32,64,96,128,160,192,224);expand(offset+32)
            memory[offset:offset+32]=value.to_bytes(32,'big')
        elif op==0x56:
            next_pc=pop();assert ops[next_pc][0]==0x5b
        elif op==0x57:
            target=pop();condition=pop();assert ops[target][0]==0x5b
            if condition:next_pc=target
        elif op==0x5b:pass
        elif op==0xfa:
            requested,target,input_offset,input_size,output_offset,output_size=(pop() for _ in range(6))
            assert (requested,target,input_offset,input_size,output_offset,output_size)==(6000,7,0,96,96,64) and not stack
            index=call_entries[pc];entry=entries[index]
            assert index==len(calls)%sequence_length and read(192)==len(calls)//sequence_length
            assert (read(0),read(32),read(64))==(1,2,entry['scalar'])
            assert read(96)==read(128)==read(160)==0 and read(224)==0xa55a
            expand(max(input_offset+input_size,output_offset+output_size));gross+=100
            available=envelope-gross;forwarded=min(requested,available-available//64);assert forwarded==6000
            gross+=6000;assert gross<=envelope
            fault={} if override is None else override
            assert type(fault) is dict and set(fault)<=set(('status','return_size','output','write_size'))
            status=fault.get('status',1);return_size=fault.get('return_size',64);output=fault.get('output',entry['output']);written=fault.get('write_size',64)
            integer(status,0,1);integer(return_size,0,1<<20)
            assert type(output) is bytes and len(output)==64 and written==64
            memory[96:160]=output
            calls.append(dict(entry_index=index,scalar_hex=f"0x{entry['scalar']:064x}",reduced_scalar_hex=f"0x{entry['reduced_scalar']:064x}",
                input_hex='0x'+memory[:96].hex(),output_hex='0x'+output.hex(),native_gas=6000,requested_gas=requested,
                available_gas=available,forwarded_gas=forwarded,returned_gas=0,return_size=return_size,write_size=written))
            stack.append(status)
        else:raise AssertionError('unexpected or exceptional instruction')
        assert len(stack)<=1024
        pc=next_pc


def run_users(row):
    p=row['parameters'];request=row['request'];program=request['program']
    assert type(p) is dict and type(request) is dict
    assert set(request)=={'schema_version','expected_user_success','program'}
    assert type(request['schema_version']) is int and request['schema_version']==1 and request['expected_user_success'] is True
    assert type(program) is dict and set(program)=={'code_hex','input_hex','block_count','transactions','expected_route_counts'}
    assert type(program['block_count']) is int and program['block_count']==1
    descriptor=program['transactions'];assert type(descriptor) is dict and set(descriptor)=={'kind','count_per_block','gas_limit'}
    assert descriptor['kind']=='call' and type(descriptor['count_per_block']) is int and descriptor['count_per_block']==1
    assert 'prestate_contracts' not in program and 'storage_state' not in program
    k=integer(p['sequence_length'],1,64);r=integer(p['repetitions'],0,1_000_000)
    def raw_hex(value):
        assert type(value) is str and value.startswith('0x') and len(value)%2==0
        assert all(c in '0123456789abcdef' for c in value[2:])
        return bytes.fromhex(value[2:])
    code=raw_hex(program['code_hex']);data=raw_hex(program['input_hex']);entries=validate_payload(data,k)
    envelope=integer(p['transaction_gas_limit'],21000,15_000_000);assert type(program['transactions']['gas_limit']) is int and program['transactions']['gas_limit']==envelope
    result=execute(code,data,envelope,sequence_length=k,repetitions=r)
    intrinsic=result['intrinsic_gas'];floor=result['floor_gas'];calls=k*r
    ordinary=result['gross_gas']-intrinsic-24-6100*calls
    # Literal body:156 ordinary +100 access +6000 native per call;
    # loop head26 and increment/backedge29 per repetition.
    slope=6256*k+55;base=result['gross_gas']-slope*r
    maximum=(15_000_000-1000-base)//slope
    assert 0<maximum<1_000_000 and floor<=base+slope*maximum
    assert envelope==base+slope*maximum+1000
    level=p['workload_level'];assert level in ('zero','probe','full')
    assert (level=='zero' and r==0) or (level=='probe' and 0<r<maximum) or (level=='full' and r==maximum)
    expected=dict(sequence_length=k,repetitions=r,full_repetitions=maximum,
        scalars_hex=[f"0x{e['scalar']:064x}" for e in entries],reduced_scalars_hex=[f"0x{e['reduced_scalar']:064x}" for e in entries],
        source_derived_work_units=[e['work_units'] for e in entries],expected_outputs_hex=['0x'+e['output'].hex() for e in entries],
        precompile_address='0x'+'00'*19+'07',input_point=[1,2],expected_precompile_calls=calls,
        source_derived_native_gas_per_call=6000,requested_gas_per_call=6000,warm_access_gas_per_call=100,
        primary_code_owner='0x'+'66'*20,primary_code_owner_count=1,transaction_kind='call',transaction_nonce=0,user_transactions_per_block=1,all_user_transactions=1,
        block_count=1,all_transactions_including_anchor=2,transaction_gas_limit=envelope,transaction_envelope_reserve=1000,expected_anchor_gas=167355,
        header_gas_limit=31_000_000,charged_gas_budget=30_000_000,header_prefix_remaining_gas=[30_832_645],
        expected_persistent_storage_reads=0,expected_persistent_storage_writes=0,input_memory_offset=0,scalar_memory_offset=64,output_memory_offset=96,
        output_bytes=64,protected_zero_memory_offset=160,counter_memory_offset=192,protected_memory_offset=224,sentinel=0xa55a,
        peak_memory_bytes=256,memory_words=8,runtime_bytes=len(code),runtime_sha256=sha256(code).hexdigest(),calldata_bytes=len(data),
        calldata_zero_bytes=data.count(0),calldata_nonzero_bytes=len(data)-data.count(0),calldata_sha256=sha256(data).hexdigest(),
        expected_user_gross_gas=result['gross_gas'],expected_user_charged_gas=result['charged_gas'],expected_user_refund_counter=0,
        expected_user_refund_applied=0,expected_block_charged_gas=167355+result['charged_gas'],
        full_plus_one_constraint_failures=['transaction_fixture_cap'],expected_terminal='STOP',expected_user_success=True,
        expected_user_receipt_statuses=[True],expected_anchor_success=True,expected_exceptional_instructions=0,execution_profile='evm-gas-only',preload_created_accounts_or_code=False)
    expected['gas_accounting_per_user']=dict(intrinsic_gas=intrinsic,eip7623_floor_gas=floor,eip7623_floor_active=result['gross_gas']<floor,
        ordinary_opcode_gas=ordinary,memory_expansion_gas=24,native_gas_per_call=6000,warm_access_gas_per_call=100,precompile_calls=calls,
        precompile_native_gas=6000*calls,call_access_gas=100*calls,gross_gas=result['gross_gas'],charged_gas=result['charged_gas'],refund_counter=0,refund_applied=0,runtime_bytes=len(code))
    def exact(actual,wanted):
        assert type(actual) is type(wanted)
        if type(wanted) is dict:
            assert set(actual)==set(wanted)
            for key in wanted:exact(actual[key],wanted[key])
        elif type(wanted) is list:
            assert len(actual)==len(wanted)
            for a,b in zip(actual,wanted):exact(a,b)
        else:assert actual==wanted
    for key,value in expected.items():exact(p[key],value)
    assert type(p['recipe']) is str and p['recipe'] in ('low','high','mixed','custom')
    if p['recipe']!='custom':
        high=(3<<252)-1
        wanted=[0]*31 if p['recipe']=='low' else [high]*31 if p['recipe']=='high' else [0]*12+[high]*19
        assert [e['scalar'] for e in entries]==wanted
    routes=program['expected_route_counts'];assert type(routes) is dict
    assert all(type(key) is str and type(value) is int and value>=0 for key,value in routes.items())
    assert +Counter(routes)==result['counts'] and envelope<=16_777_216 and envelope<=30_832_645 and 167355+result['charged_gas']<=30_000_000
    return [result]
