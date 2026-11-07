"""Test-only Ethereum Keccak-256 oracle, using only Python's standard library.

Keccak-f[1600] uses24 rounds, rate136 bytes, little-endian64-bit lanes and the
original Keccak suffix0x01 (Ethereum), rather than SHA3's suffix0x06.
Constants and round steps follow the Keccak specifications summary:
https://keccak.team/keccak_specs_summary.html
"""
from functools import lru_cache

_MASK = (1 << 64)-1
_RATE = 136
_ROUND_CONSTANTS = (
    0x0000000000000001, 0x0000000000008082, 0x800000000000808A,
    0x8000000080008000, 0x000000000000808B, 0x0000000080000001,
    0x8000000080008081, 0x8000000000008009, 0x000000000000008A,
    0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
    0x000000008000808B, 0x800000000000008B, 0x8000000000008089,
    0x8000000000008003, 0x8000000000008002, 0x8000000000000080,
    0x000000000000800A, 0x800000008000000A, 0x8000000080008081,
    0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
)
_ROTATIONS = (1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14,
              27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44)
_LANES = (10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4,
          15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1)


def _rotate(value, shift):
    return ((value << shift) | (value >> (64-shift))) & _MASK


def _permute(state):
    for constant in _ROUND_CONSTANTS:
        # Theta: parity of each column, then adjacent-column diffusion.
        columns = [state[x] ^ state[x+5] ^ state[x+10] ^ state[x+15] ^ state[x+20] for x in range(5)]
        for x in range(5):
            delta = columns[(x-1) % 5] ^ _rotate(columns[(x+1) % 5], 1)
            for y in range(0, 25, 5): state[x+y] ^= delta
        # Rho and pi: lane0 stays fixed; the other24 lanes form one cycle.
        previous = state[1]
        for lane, rotation in zip(_LANES, _ROTATIONS):
            previous, state[lane] = state[lane], _rotate(previous, rotation)
        # Chi reads an unmodified row; iota injects the round constant.
        for y in range(0, 25, 5):
            row = state[y:y+5]
            for x in range(5): state[y+x] = row[x] ^ ((~row[(x+1) % 5]) & row[(x+2) % 5])
        state[0] ^= constant


@lru_cache(maxsize=32768, typed=True)
def keccak256(data):
    """Return32 Ethereum Keccak bytes; bounded caching amortizes repeated test inputs."""
    if type(data) is not bytes: raise TypeError('Keccak input must be bytes')
    padded = bytearray(data); padded.append(0x01)
    padded.extend(bytes((-len(padded)) % _RATE)); padded[-1] |= 0x80
    state = [0]*25
    for offset in range(0, len(padded), _RATE):
        for lane in range(_RATE//8):
            start = offset+lane*8; state[lane] ^= int.from_bytes(padded[start:start+8], 'little')
        _permute(state)
    return b''.join(lane.to_bytes(8, 'little') for lane in state[:4])
