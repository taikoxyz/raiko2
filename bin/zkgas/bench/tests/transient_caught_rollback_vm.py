"""Literal bounded transient-frame oracle with real checkpoint reversion.

No producer imports or pricing model. This is a finite instruction interpreter,
not a general EVM; success requires actual empty REVERT, never exceptional halt.
"""
from collections import Counter
import copy
import hashlib
import re

MASK = (1 << 256) - 1


def uint(value, low=0, high=MASK):
    """Declared counts/gas use strict integers, never bool or float equality."""
    assert type(value) is int and low <= value <= high, 'strict declared integer'
    return value


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        immediate = int.from_bytes(code[pc:pc+width], 'big'); pc += width
        yield at, op, immediate


def run_users(row):
    """Execute every requested user with fresh transient state and address warmth."""
    program = row['request']['program']; tx = program['transactions']
    assert row['request']['schema_version'] == 1 and type(row['request']['schema_version']) is int
    assert row['request']['expected_user_success'] is True
    assert type(program['block_count']) is int and program['block_count'] == 2 and tx['kind'] == 'call'
    data = bytes.fromhex(program['input_hex'].removeprefix('0x'))
    parameters = row['parameters']; k = parameters['key_count']; repeats = parameters['caught_calls']
    assert type(parameters['raw_key_layout']) is dict and set(parameters['raw_key_layout']) == {'L1','L3'}
    word = parameters['raw_key_layout']['L1']
    uint(k,1,512);uint(repeats,1,128);uint(word,0,(1<<64)-1);uint(parameters['raw_key_layout']['L3'],0,0)
    keys=[(i<<24)|(word<<64) for i in range(1,k+1)]
    assert len(data) == 32*k and [int.from_bytes(data[i:i+32],'big') for i in range(0,len(data),32)] == keys, 'raw-key data or sentinel alias'
    assert parameters['ordered_keys'] == [f'0x{key:064x}' for key in keys]
    assert parameters['query_key'] == f'0x{keys[-1]:064x}'
    assert uint(parameters['ordinal_shift'],24,24) == 24
    assert parameters['transient_sentinel_key'] == '0x0' and uint(parameters['transient_sentinel_value'],1,1) == 1
    assert parameters['calldata_sha256'] == hashlib.sha256(data).hexdigest()
    assert uint(parameters['outer_calldata_bytes']) == len(data)
    assert uint(parameters['calldata_zero_bytes']) == data.count(0)
    assert uint(parameters['calldata_nonzero_bytes']) == len(data)-data.count(0)
    parent,leaf = parameters['parent_owner'],parameters['leaf_code_address']
    assert all(type(address) is str and re.fullmatch('0x[0-9a-f]{40}',address) for address in (parent,leaf)) and parent != leaf
    contracts = {c['address']: bytes.fromhex(c['code_hex'].removeprefix('0x'))
                 for c in program['prestate_contracts']}
    assert len(program['prestate_contracts']) == len(contracts) == 2 and set(contracts) == {parent,leaf}
    assert all(c['initial_storage'] == {} for c in program['prestate_contracts'])
    for role,address in [('parent',parent),('leaf',leaf)]:
        assert parameters[role+'_runtime_sha256'] == hashlib.sha256(contracts[address]).hexdigest(), 'runtime identity'
        assert uint(parameters[role+'_runtime_bytes']) == len(contracts[address])
    users=uint(parameters['all_user_transactions'],1,2)
    for counts in (tx['counts_by_block'],parameters['user_transactions_by_block']):
        assert type(counts) is list and counts == [0,users] and all(type(n) is int for n in counts), 'declared user schedule'
    assert tx['recipients'] == parameters['recipient_schedule'] == [parent]*users, 'declared owner/user routes'
    assert uint(parameters['all_transactions_including_anchor']) == users+2 and uint(parameters['transaction_nonce'],0,0) == 0
    assert uint(tx['gas_limit'],21000,15_000_000) == uint(parameters['transaction_gas_limit'])
    assert type(parameters['expected_user_receipt_statuses']) is list and parameters['expected_user_receipt_statuses'] == [True]*users and all(x is True for x in parameters['expected_user_receipt_statuses'])
    assert uint(parameters['expected_child_status'],0,0) == uint(parameters['expected_leaf_return_bytes'],0,0) == 0
    assert parameters['expected_leaf_terminal'] == 'REVERT' and parameters['expected_terminal'] == 'STOP'
    routes=program['expected_route_counts'];assert type(routes) is dict
    assert all(type(name) is str and re.fullmatch('opcode:0x[0-9a-f]{2}',name) and type(n) is int and n>=0 for name,n in routes.items()), 'strict route declarations'
    for field in ('expected_user_gross_gas','expected_user_charged_gas','expected_user_refund_counter','expected_user_refund_applied'):
        uint(parameters[field])
    gas=parameters['gas_accounting_per_user'];assert type(gas) is dict
    for field in ('intrinsic_gas','eip7623_floor_gas','gross_gas','charged_gas','refund_counter','refund_applied','leaf_execution_gas','all_leaf_execution_gas','call_target_gas'):
        uint(gas[field])
    records = []
    for recipient in tx['recipients']:
        transient = {}; warm = {recipient}; calls = []; observed = []

        def frame(address, owner, calldata, budget, depth):
            assert depth <= 1 and address in contracts
            code = contracts[address]; instructions = dict((pc, (op, imm)) for pc, op, imm in decode(code))
            destinations = {pc for pc, (op, _) in instructions.items() if op == 0x5b}
            checkpoint = copy.deepcopy(transient); stack = []; memory = bytearray()
            counts = Counter(); pc = 0; used = 0; returndata = b''

            def charge(amount):
                nonlocal used
                used += amount
                assert used <= budget, 'out of gas, not an expected REVERT'

            def mem(offset, size):
                assert 0 <= offset <= 1 << 20 and 0 <= size <= 1 << 20
                if not size:
                    return
                words = (offset + size + 31) // 32; old = len(memory) // 32
                if words > old:
                    charge(3*words + words*words//512 - 3*old - old*old//512)
                    memory.extend(bytes(32*(words-old)))

            for _ in range(2_000_000):
                assert pc in instructions, 'PC out of bounds'
                at = pc; op, immediate = instructions[pc]
                pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
                counts[f'opcode:0x{op:02x}'] += 1
                if 0x60 <= op <= 0x7f:
                    charge(3); stack.append(immediate)
                elif op == 0x80:
                    charge(3); stack.append(stack[-1])
                elif op == 1:
                    charge(3); stack.append((stack.pop()+stack.pop()) & MASK)
                elif op == 0x14:
                    charge(3); stack.append(int(stack.pop() == stack.pop()))
                elif op == 0x15:
                    charge(3); stack.append(int(stack.pop() == 0))
                elif op == 0x1b:
                    charge(3); shift = stack.pop(); value = stack.pop(); stack.append((value << shift) & MASK)
                elif op == 0x35:
                    charge(3); offset = stack.pop(); stack.append(int.from_bytes(calldata[offset:offset+32].ljust(32,b'\0'),'big'))
                elif op == 0x36:
                    charge(2); stack.append(len(calldata))
                elif op == 0x37:
                    destination, source, size = stack.pop(), stack.pop(), stack.pop()
                    charge(3 + 3*((size+31)//32)); mem(destination, size)
                    memory[destination:destination+size] = calldata[source:source+size].ljust(size,b'\0')
                elif op == 0x3d:
                    charge(2); stack.append(len(returndata))
                elif op == 0x50:
                    charge(2); stack.pop()
                elif op == 0x51:
                    charge(3); offset = stack.pop(); mem(offset,32); stack.append(int.from_bytes(memory[offset:offset+32],'big'))
                elif op == 0x52:
                    charge(3); offset, value = stack.pop(), stack.pop(); mem(offset,32); memory[offset:offset+32] = value.to_bytes(32,'big')
                elif op in (0x56,0x57):
                    charge(8 if op == 0x56 else 10); target = stack.pop()
                    if op == 0x56 or stack.pop():
                        assert target in destinations, 'invalid jump target'; pc = target
                elif op == 0x5b:
                    charge(1)
                elif op == 0x5c:
                    charge(100); key = stack.pop(); value = transient.get(owner,{}).get(key,0)
                    observed.append(dict(depth=depth,kind='read',owner=owner,key=key,value=value))
                    stack.append(value)
                elif op == 0x5d:
                    charge(100); key, value = stack.pop(), stack.pop(); state = transient.setdefault(owner,{})
                    if value: state[key] = value
                    else: state.pop(key,None)
                    observed.append(dict(depth=depth,kind='write',owner=owner,key=key,value=value))
                elif op == 0xf4:
                    assert depth == 0, 'only parent delegates'
                    requested, target, offset, size, out, outsize = [stack.pop() for _ in range(6)]
                    target = f'0x{target:040x}'
                    assert address == owner == parent and target == leaf, 'declared delegate storage/code owner'
                    assert out == outsize == 0 and offset == 128 and size == len(data), 'delegate input range'
                    mem(offset,size); fee = 100 if target in warm else 2600; charge(fee); warm.add(target)
                    available = budget-used; forwarded = min(requested,available-available//64)
                    before = copy.deepcopy(transient)
                    child = frame(target,owner,bytes(memory[offset:offset+size]),forwarded,depth+1)
                    assert child['terminal'] == 0xfd, 'expected actual REVERT, not status-only failure'
                    assert child['returndata'] == b'' and child['remaining'] > 0
                    assert transient == before, 'child checkpoint must restore caller sentinel and remove inserts'
                    charge(child['gas']); counts.update(child['counts']); returndata = child['returndata']; stack.append(0)
                    calls.append(dict(status=0,terminal=child['terminal'],returndata=returndata,
                        forwarded=forwarded,gas=child['gas'],remaining=child['remaining'],available=available,
                        requested=requested,target_fee=fee,code_address=target,storage_owner=owner,
                        parent_remaining=budget-used,transient_before=before,transient_after=copy.deepcopy(transient)))
                elif op in (0,0xfd):
                    if op == 0xfd:
                        offset,size = stack.pop(),stack.pop(); mem(offset,size); returndata=bytes(memory[offset:offset+size])
                        transient.clear(); transient.update(checkpoint)
                    else:
                        assert depth == 0, 'child STOP cannot masquerade as caught rollback'
                    assert stack == [], 'unconsumed stack at terminal'
                    return dict(terminal=op,gas=used,remaining=budget-used,counts=counts,
                                stack=list(stack),returndata=returndata,memory_bytes=len(memory),pc=at)
                elif op == 0xf1:
                    raise AssertionError('CALL changes storage owner; DELEGATECALL required')
                elif op == 0xfe:
                    raise AssertionError('guard INVALID, not an expected REVERT')
                else:
                    raise AssertionError(f'unsupported opcode {op:02x}')
                assert len(stack) <= 1024
            raise AssertionError('instruction bound exceeded')

        zero = data.count(0); intrinsic=21000+4*zero+16*(len(data)-zero)
        result = frame(recipient,recipient,data,tx['gas_limit']-intrinsic,0)
        assert result['terminal'] == 0 and len(calls) == repeats, 'successful parent must catch every iteration'
        assert all(event['key'] != 0 for event in observed if event['depth'] == 1 and event['kind'] == 'write'), 'child cannot overwrite sentinel'
        gross=intrinsic+result['gas']; floor=21000+10*(zero+4*(len(data)-zero))
        assert max(gross,floor) <= tx['gas_limit'], 'charged calldata floor exceeds transaction gas limit'
        parent_final=copy.deepcopy(transient.get(recipient,{})); transient.clear()
        result.update(gross_gas=gross,gas=max(gross,floor),refund=0,transient_before={},
                      transient_after=copy.deepcopy(transient),parent_final_transient=parent_final,
                      calls=calls,transient_events=list(observed))
        assert result['gross_gas'] == parameters['expected_user_gross_gas'] == gas['gross_gas']
        assert result['gas'] == parameters['expected_user_charged_gas'] == gas['charged_gas']
        assert result['refund'] == parameters['expected_user_refund_counter'] == gas['refund_counter'] == 0
        assert parameters['expected_user_refund_applied'] == gas['refund_applied'] == 0
        assert intrinsic == gas['intrinsic_gas'] and floor == gas['eip7623_floor_gas']
        assert all(call['gas'] == gas['leaf_execution_gas'] for call in calls)
        assert sum(call['gas'] for call in calls) == gas['all_leaf_execution_gas']
        assert sum(call['target_fee'] for call in calls) == gas['call_target_gas']
        assert uint(parameters['expected_transient_reads_per_user']) == result['counts']['opcode:0x5c']
        assert uint(parameters['expected_transient_writes_per_user']) == result['counts']['opcode:0x5d']
        records.append(result)
    aggregate=Counter()
    for record in records:aggregate.update(record['counts'])
    assert len(records) == users and aggregate == Counter(routes), 'declared whole-user opcode routes'
    return records
