"""Bounded actual-byte wrapper oracle; no pairing or proving-performance emulator.

Independent Fq/Fq2 arithmetic validates finite canonical operands and the literal
schedule. Only then does the selected mathematical product supply the stub result.
"""
from collections import Counter
from functools import lru_cache

P = 21888242871839275222246405745257275088696311157297823662689037894645226208583
R = 21888242871839275222246405745257275088548364400416034343698204186575808495617
# Independent real/imaginary convention, cross-checked with ark-bn2540.5 G2.
Q = ((10857046999023057135944570762232829481370756359578518086990519993285655852781,
      11559732032986387107991004021392285783925812861821192530917403151452391805634),
     (8495653923123431417604973247489272438418190587263600148770280649306958101930,
      4082367875863433681332203403145435568316851327593401208105741076214120093531))
MASK = (1 << 256) - 1


def fadd(a, b): return ((a[0] + b[0]) % P, (a[1] + b[1]) % P)
def fneg(a): return ((-a[0]) % P, (-a[1]) % P)
def fmul(a, b): return ((a[0] * b[0] - a[1] * b[1]) % P, (a[0] * b[1] + a[1] * b[0]) % P)
def finv(a):
    scale = pow((a[0] * a[0] + a[1] * a[1]) % P, -1, P)
    return (a[0] * scale % P, -a[1] * scale % P)


def g2add(a, b):
    if a is None: return b
    if b is None: return a
    x, y = a; u, v = b
    if x == u and fadd(y, v) == (0, 0): return None
    slope = fmul(fmul((3, 0), fmul(x, x)), finv(fmul((2, 0), y))) if a == b else fmul(fadd(v, fneg(y)), finv(fadd(u, fneg(x))))
    nx = fadd(fadd(fmul(slope, slope), fneg(x)), fneg(u))
    return nx, fadd(fmul(slope, fadd(x, fneg(nx))), fneg(y))


@lru_cache(maxsize=1)
def check_g2():
    x, y = Q; b = fmul((3, 0), finv((9, 1)))
    assert fmul(y, y) == fadd(fmul(fmul(x, x), x), b), 'G2 twist curve'
    n = R; point = Q; result = None
    while n:
        if n & 1: result = g2add(result, point)
        point = g2add(point, point); n >>= 1
    assert result is None, 'G2 subgroup order'
    return True


def g1add(a, b):
    if a is None: return b
    if b is None: return a
    x, y = a; u, v = b
    if x == u and (y + v) % P == 0: return None
    numerator, denominator = (3 * x * x, 2 * y) if a == b else (v - y, u - x)
    slope = numerator * pow(denominator, P - 2, P) % P
    nx = (slope * slope - x - u) % P
    return nx, (slope * (x - nx) - y) % P


@lru_cache(maxsize=6)
def expected_points(kind, pairs):
    assert kind in ('identity', 'nonidentity') and pairs in (2, 16, 64), 'operand domain'
    result = []
    # Independent binary multiplication from P, rather than producer's recurrence.
    for scalar in range(1, 1 + (pairs // 2 if kind == 'identity' else pairs)):
        n = scalar; addend = (1, 2); point = None
        while n:
            if n & 1: point = g1add(point, addend)
            addend = g1add(addend, addend); n >>= 1
        result.append(point)
        if kind == 'identity': result.append((point[0], -point[1] % P))
    exponent = 0 if kind == 'identity' else pairs * (pairs + 1) // 2
    assert (exponent % R == 0) == (kind == 'identity'), 'product exponent'
    return tuple(result)


@lru_cache(maxsize=32)
def validate_payload(payload, kind, pairs, *, ordered=True):
    assert len(payload) == 192 * pairs, 'pairing input length'
    check_g2(); points = []
    for at in range(0, len(payload), 192):
        x, y, xi, xr, yi, yr = (int.from_bytes(payload[i:i + 32], 'big') for i in range(at, at + 192, 32))
        assert all(0 <= n < P for n in (x, y, xi, xr, yi, yr)), 'noncanonical field'
        assert (x, y) != (0, 0) and (y * y - x * x * x - 3) % P == 0, 'finite G1 curve'
        assert ((xr, xi), (yr, yi)) == Q, 'canonical finite G2 generator'
        points.append((x, y))
    expected = expected_points(kind, pairs)
    assert (tuple(points) == expected if ordered else sorted(points) == sorted(expected)), 'literal ordered point schedule'
    return int(kind == 'identity')


def decode(code):
    """Decode the fixture instruction prefix only; appended bytes are opaque data."""
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated executed PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width
        if op == 0xfe: break


def execute(code, envelope, *, kind, pairs, override=None):
    assert type(envelope) is int and envelope >= 21000, 'invalid gas envelope'
    assert len(code) <= 24576, 'code size'
    override = {} if override is None else override
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; pc = 0
    instructions = list(decode(code)); jumps = {at for at, op, _ in instructions if op == 0x5b}
    executable_end = instructions[-1][0] + 1
    assert instructions[-1][1] == 0xfe, 'missing failure terminal'
    size = 192 * pairs; output = 32 + size; copied = None; calls = []; returndata_size = 0
    attempted = completed = 0

    def expand(offset, length):
        nonlocal gas
        assert 0 <= offset <= 32 * (2 + 6 * pairs) and 0 <= length <= size, 'memory range'
        end = offset + length; assert end <= 32 * (2 + 6 * pairs), 'memory range'
        old = len(memory) // 32; new = max(old, (end + 31) // 32) if length else old
        gas += 3 * new + new * new // 512 - 3 * old - old * old // 512
        memory.extend(bytes(32 * (new - old)))

    def finish(op):
        assert gas <= envelope, 'out of gas'
        if op == 0:
            assert copied is not None, 'missing CODECOPY'
            validate_payload(copied, kind, pairs, ordered=True)
            assert int.from_bytes(memory[:32], 'big') == completed, 'counter/call mismatch'
        return dict(terminal=op, gas=gas, counts=counts, stack=stack, memory=memory,
            copied=copied, calls=calls, attempted_calls=attempted, completed_calls=completed,
            counter=int.from_bytes(memory[:32], 'big'), output=int.from_bytes(memory[output:output + 32], 'big'))

    for _ in range(5_000_000):
        assert 0 <= pc < executable_end, 'execution entered embedded data'
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; assert pc + width <= executable_end, 'truncated executed PUSH'
            stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op == 1: stack.append((stack.pop() + stack.pop()) & MASK); gas += 3
        elif op == 0x14: stack.append(int(stack.pop() == stack.pop())); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); expand(offset, 32); gas += 3
            if op == 0x51: stack.append(int.from_bytes(memory[offset:offset + 32], 'big'))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x39:
            dest, source, length = stack.pop(), stack.pop(), stack.pop()
            assert dest == 32 and length == size and source == executable_end, 'CODECOPY input range'
            assert source + length <= len(code), 'CODECOPY source range'
            expand(dest, length); gas += 3 + 3 * ((length + 31) // 32)
            copied = bytes(code[source:source + length]); memory[dest:dest + length] = copied
        elif op == 0x5a:
            gas += 2; assert gas <= envelope, 'out of gas'; stack.append(envelope - gas)
        elif op == 0xfa:
            requested, target, start, length, out, out_size = (stack.pop() for _ in range(6))
            attempted += 1
            assert (target, start, length, out, out_size) == (8, 32, size, output, 32), 'STATICCALL operands'
            expand(start, length); expand(out, out_size); gas += 100
            assert gas <= envelope, 'out of gas'
            available = envelope - gas; forwarded = min(requested, available - available // 64)
            operand = bytes(memory[start:start + length]); value = validate_payload(operand, kind, pairs, ordered=True)
            sentinel = int.from_bytes(memory[out:out + 32], 'big'); assert sentinel == 2, 'output sentinel not reset'
            native = 45000 + 34000 * pairs; assert forwarded >= native, 'insufficient forwarded gas'
            status = override.get('status', 1); returned = forwarded - native; gas += native
            returndata_size = override.get('return_size', 32); value = override.get('result', value)
            returned_bytes = value.to_bytes(32, 'big'); write_size = min(out_size, returndata_size, override.get('write_size', 32))
            memory[out:out + write_size] = returned_bytes[:write_size]; stack.append(status)
            if status == 1: completed += 1
            calls.append(dict(input=operand, sentinel=sentinel, available_gas=available,
                requested_gas=requested, forwarded_gas=forwarded, returned_gas=returned, native_gas=native,
                status=status, return_size=returndata_size, result=value, gas_after_call=envelope - gas))
        elif op == 0x3d: stack.append(returndata_size); gas += 2
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0
            gas += 8 if op == 0x56 else 10
            if taken:
                assert target in jumps, 'invalid jump destination'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe): return finish(op)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit exceeded'
        assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound exceeded')


def run_users(row):
    program = row['request']['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert len(contracts) == 1 and contracts[0]['initial_storage'] == {}, 'persistent domain'
    assert tx['count_per_block'] == len(tx['recipients']) == 2, 'two user schedule'
    assert tx['recipients'] == [contracts[0]['address']] * 2, 'shared code account'
    code = bytes.fromhex(contracts[0]['code_hex'][2:]); p = row['parameters']
    return [execute(code, tx['gas_limit'], kind=p['kind'], pairs=p['pairs']) for _ in tx['recipients']]
