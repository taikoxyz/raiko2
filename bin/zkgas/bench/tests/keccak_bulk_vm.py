"""Independent actual-byte EVM oracle with bounded, exact lazy Keccak values.

A token owns an immutable input snapshot. Any consumer forces the maintained
stdlib Ethereum reference. A provable immediate full-word overwrite may discard
the previous unread value. No fabricated digest reaches an EVM operation.
"""
from collections import Counter, deque
import hashlib
import importlib.util
from pathlib import Path
import sys

MASK = (1 << 256) - 1
BASE_ADDRESS = 0x3000000000000000000000000000000000010000
_REFERENCE_NAME = '_zkgas_keccak_bulk_vm_ethereum_reference'
if _REFERENCE_NAME not in sys.modules:
    _spec = importlib.util.spec_from_file_location(_REFERENCE_NAME, Path(__file__).with_name('ethereum_hash.py'))
    _module = importlib.util.module_from_spec(_spec); sys.modules[_REFERENCE_NAME] = _module; _spec.loader.exec_module(_module)
_REFERENCE = sys.modules[_REFERENCE_NAME].keccak256.__wrapped__


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


class _Token:
    __slots__ = ('message', 'value')
    def __init__(self, message): self.message, self.value = message, None


def execute(code, envelope, *, length, pattern, repetitions, tag, owner=None, lazy=True, strict=True):
    assert type(envelope) is int and envelope >= 21000, 'gas envelope'
    assert type(length) is int and length in (0, 135, 136, 137, 65536), 'length domain'
    assert type(pattern) is str and pattern in ('zero', 'a5') and (length or pattern == 'zero'), 'pattern domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetitions domain'
    assert type(tag) is int and tag in (0, 1), 'tag domain'
    owner = f'0x{BASE_ADDRESS + tag:040x}' if owner is None else owner
    assert owner == f'0x{BASE_ADDRESS + tag:040x}', 'owner identity'
    assert len(code) < 24576, 'code size'
    instructions = list(decode(code)); jumps = {at for at, op, _ in instructions if op == 0x5b}
    if strict: assert instructions[-1][1] == 0xfe, 'failure terminal'
    extent = max(32, 32 * ((length + 31) // 32)); size = 64 + extent; byte = 165 if pattern == 'a5' else 0
    expected = bytearray(size); expected[64:] = bytes([byte]) * extent
    if length and length % 32: expected[64 + length:] = bytes([238]) * (extent - length)
    seed = int.from_bytes(bytes([byte]) * 32, 'big')
    stores = [(size - 32, int.from_bytes(bytes([238]) * 32, 'big')), (0, 0), (32, 0), (64, seed)]
    if length and length % 32: stores.append((size - 32, int.from_bytes(expected[-32:], 'big')))
    prep = []; filled = 32
    while filled < extent:
        count = min(filled, extent - filled); prep.append((64, 64 + filled, count)); filled += count
    offsets = sorted({32, 64, size - 32}); guards_seen = []; store_index = 0; prep_index = 0
    stack = []; memory = bytearray(); pending = None; counts = Counter(); pc = 0; gas = 21000
    prepared = False; loop_pc = None; loop_phase = 0; hash_count = 0; hash_sequence = hashlib.sha256(); counter_sequence = hashlib.sha256()
    counter_entries = 0; samples_first = []; samples_last = deque(maxlen=4)
    reference_calls = 0; peak_pending_inputs = 0; peak_pending_bytes = 0; mcopy_gas = 0; keccak_gas = 0

    def force(value):
        nonlocal reference_calls
        if isinstance(value, _Token):
            if value.value is None:
                value.value = int.from_bytes(_REFERENCE(value.message), 'big'); reference_calls += 1
                value.message = b''
            return value.value
        return value

    def materialize():
        nonlocal pending
        if pending is not None:
            at, token = pending; memory[at:at + 32] = force(token).to_bytes(32, 'big'); pending = None

    def overlap(offset, count):
        return pending is not None and count > 0 and offset < pending[0] + 32 and pending[0] < offset + count

    def expand(offset, count):
        nonlocal gas
        assert type(offset) is int and type(count) is int and 0 <= offset <= (size if strict else 131200) and 0 <= count <= (size if strict else 131200), 'memory range'
        assert offset + count <= (size if strict else 131200), 'memory extent'
        old = len(memory) // 32; new = max(old, (offset + count + 31) // 32) if count else old
        gas += (3 * new + new * new // 512) - (3 * old + old * old // 512)
        memory.extend(bytes(32 * (new - old)))

    def read(offset, count):
        expand(offset, count)
        if overlap(offset, count): materialize()
        return bytes(memory[offset:offset + count])

    def word(offset): return int.from_bytes(read(offset, 32), 'big')

    def write(offset, count, value):
        nonlocal pending, peak_pending_inputs, peak_pending_bytes
        expand(offset, count)
        if overlap(offset, count):
            if offset <= pending[0] and offset + count >= pending[0] + 32 and not any(item is pending[1] for item in stack):
                pending[1].message = b''; pending = None
            else: materialize()
        if isinstance(value, _Token) and count == 32:
            if pending is not None: materialize()
            pending = (offset, value); peak_pending_inputs = max(peak_pending_inputs, 1)
            peak_pending_bytes = max(peak_pending_bytes, len(value.message))
        elif isinstance(value, bytes): memory[offset:offset + count] = value
        else: memory[offset:offset + count] = (force(value) & ((1 << (count * 8)) - 1)).to_bytes(count, 'big')

    for _ in range(max(10000, repetitions * 45 + 1000)):
        assert 0 <= pc < len(code), 'execution range'
        at = pc; op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; assert pc + width <= len(code), 'truncated PUSH'
            stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op in (1, 0x14):
            assert len(stack) >= 2, 'stack underflow'; a, b = force(stack.pop()), force(stack.pop())
            stack.append((a + b) & MASK if op == 1 else int(a == b)); gas += 3
        elif op == 0x15: stack.append(int(force(stack.pop()) == 0)); gas += 3
        elif op == 0x50: force(stack.pop()); gas += 2
        elif op == 0x80: stack.append(force(stack[-1])); gas += 3
        elif op in (0x51, 0x52, 0x53):
            offset = force(stack.pop()); gas += 3
            if op == 0x51:
                stack.append(word(offset))
                if strict and prepared and hash_count == repetitions and at > next_done and offset != 0: guards_seen.append(offset)
            else:
                value = stack.pop()
                if strict and not prepared:
                    assert op == 0x52 and store_index < len(stores) and (offset, force(value)) == stores[store_index], 'preparation store schedule'
                    store_index += 1
                elif strict and offset == 64:
                    assert loop_phase == 0 and hash_count < repetitions and force(value) == ((tag << 128) | hash_count) and word(0) == hash_count, 'nonce/tag/index schedule'
                    loop_phase = 1
                elif strict and offset == 0:
                    assert loop_phase == 3 and force(value) == hash_count and hash_count > 0, 'counter progression'
                    loop_phase = 0
                elif strict and offset == 32:
                    assert loop_phase == 2 and hash_count > 0 and (isinstance(value, _Token) or not lazy), 'digest store schedule'
                    loop_phase = 3
                elif strict: raise AssertionError('unexpected memory write')
                write(offset, 1 if op == 0x53 else 32, value)
        elif op == 0x5e:
            destination, source, count = force(stack.pop()), force(stack.pop()), force(stack.pop())
            if strict:
                assert not prepared and prep_index < len(prep) and (source, destination, count) == prep[prep_index], 'preparation MCOPY schedule'
                assert len(memory) == size, 'preallocation'; prep_index += 1
            content = read(source, count); write(destination, count, content)
            cost = 3 + 3 * ((count + 31) // 32); gas += cost; mcopy_gas += cost
        elif op == 0x20:
            offset, count = force(stack.pop()), force(stack.pop()); message = read(offset, count)
            if strict:
                assert prepared and loop_phase == 1 and (offset, count) == (64, length), 'hash operand schedule'
                loop_phase = 2
                assert hash_count < repetitions and word(0) == hash_count, 'hash counter/count schedule'
                intended = (((tag << 128) | hash_count).to_bytes(32, 'big') + bytes([byte]) * (length - 32)) if length else b''
                assert message == intended and word(64) == ((tag << 128) | hash_count), 'hash message/nonce intent'
            # Fuse only the guaranteed immediate full-word overwrite. Its PUSH3
            # and MSTORE still execute and are separately counted/priced below.
            if pending is not None:
                if code[pc:pc + 5] == bytes.fromhex('6200002052') and pending[0] == 32 and not any(item is pending[1] for item in stack):
                    pending[1].message = b''; pending = None
                else: materialize()
            stack[:] = [force(item) for item in stack]
            token = _Token(message)
            stack.append(token if lazy else force(token))
            if lazy:
                peak_pending_inputs = max(peak_pending_inputs, 1); peak_pending_bytes = max(peak_pending_bytes, len(message))
            message_sha = hashlib.sha256(message).digest()
            hash_sequence.update(bytes.fromhex(owner[2:]) + hash_count.to_bytes(8, 'big') + offset.to_bytes(8, 'big') + count.to_bytes(8, 'big') + message_sha)
            record = dict(index=hash_count, offset=offset, length=count, message_sha256=message_sha.hex())
            if hash_count < 4: samples_first.append(record)
            samples_last.append(record); hash_count += 1
            cost = 30 + 6 * ((count + 31) // 32); gas += cost; keccak_gas += cost
        elif op in (0x56, 0x57):
            target = force(stack.pop()); taken = op == 0x56 or force(stack.pop()) != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'invalid jump destination'; pc = target
        elif op == 0x5b:
            gas += 1
            if strict and not prepared:
                assert store_index == len(stores) and prep_index == len(prep), 'incomplete preparation'
                assert memory == expected and pending is None, 'prepared memory bytes'
                prepared = True; loop_pc = at
                next_done = next(pos for pos, opcode, _ in instructions if opcode == 0x5b and pos > at)
            if strict and at == loop_pc:
                value = word(0); assert value == counter_entries and value <= repetitions, 'loop counter trajectory'
                counter_sequence.update(value.to_bytes(8, 'big')); counter_entries += 1
        elif op in (0, 0xfe):
            assert op == 0, 'failed final guard/terminal'; materialize()
            if strict:
                assert prepared and loop_phase == 0 and hash_count == repetitions == word(0), 'final hash/counter count'
                assert counter_entries == repetitions + 1, 'loop entry count'
                expected[:32] = repetitions.to_bytes(32, 'big')
                if repetitions:
                    expected[64:96] = ((tag << 128) | (repetitions - 1)).to_bytes(32, 'big')
                    # The executed digest value was already checked by the actual
                    # bytecode guard; independently expected input was checked at issue.
                    expected[32:64] = memory[32:64]
                assert memory == expected, 'final memory intent'
                assert guards_seen == offsets, 'final guard reads/order'; assert stack == [], 'terminal stack'
            unique = {record['index']: record for record in samples_first + list(samples_last)}
            return dict(terminal=op, gas=gas, counts=counts, stack=[force(v) for v in stack], memory=memory,
                counter=word(0) if len(memory) >= 32 else 0, hash_count=hash_count,
                hash_sequence_sha256=hash_sequence.hexdigest(), counter_sequence_sha256=counter_sequence.hexdigest(),
                counter_entries=counter_entries, hash_samples=[unique[index] for index in sorted(unique)],
                reference_calls=reference_calls, peak_pending_inputs=peak_pending_inputs, peak_pending_bytes=peak_pending_bytes,
                mcopy_gas=mcopy_gas, keccak_gas=keccak_gas,
                last_digest='0x' + bytes(memory[32:64]).hex(), last_input_word='0x' + bytes(memory[64:96]).hex(),
                guards=['0x' + bytes(memory[o:o + 32]).hex() for o in offsets])
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit'; assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p = row['parameters']; program = row['request']['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert len(contracts) == 2 and all(c['initial_storage'] == {} for c in contracts), 'persistent domain'
    owners = [c['address'] for c in contracts]
    assert len(set(owners)) == 2 and owners == [f'0x{BASE_ADDRESS + tag:040x}' for tag in range(2)], 'distinct owner identities'
    assert tx['kind'] == 'call' and tx['count_per_block'] == len(tx['recipients']) == 2, 'two user schedule'
    assert tx['recipients'] == owners == p['recipient_schedule'], 'owner call schedule'
    assert p['owner_tags'] == [0, 1] and program['input_hex'] == '0x' and program['block_count'] == 1, 'outer domain'
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]; assert len(set(codes)) == 2, 'distinct tagged code'
    assert [op for _, op, _ in decode(codes[0])] == [op for _, op, _ in decode(codes[1])], 'code shape'
    return [execute(code, tx['gas_limit'], length=p['input_bytes'], pattern=p['pattern'], tag=tag, repetitions=p['repetitions'], owner=owners[tag]) for tag, code in enumerate(codes)]
