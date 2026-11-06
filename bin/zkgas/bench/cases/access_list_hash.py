"""Frozen b76 generic U256 Hash and reproducible access-list test catalogs.

This is not the journal fixed32 hash. Own runtime seeds and hashes still need
independent observation; generation does not certify placement or workload.
"""
from hashlib import sha256

ELF_SHA256 = 'b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034'
MASK64 = (1 << 64) - 1


def _uint(value, bits):
    if type(value) is not int or not 0 <= value < (1 << bits):
        raise ValueError(f'expected a strict U{bits} integer')
    return value


def _count(value):
    if type(value) is not int or not 1 <= value <= 8192:
        raise ValueError('access-list catalog count must be in 1..8192')
    return value


def _fold(left, right):
    product = left * right
    return (product & MASK64) ^ (product >> 64)


def hash_key(key, seed, shared0, shared1):
    """Exact source-level generic hash, including array length and finalization."""
    _uint(key, 256)
    for value in (seed, shared0, shared1):
        _uint(value, 64)
    l0, l1, l2, l3 = ((key >> (64 * i)) & MASK64 for i in range(4))
    rotated = ((seed >> 32) | (seed << 32)) & MASK64
    first = _fold(l0 ^ rotated, l2 ^ shared0)
    second = _fold(l1 ^ ((rotated + shared1) & MASK64), l3 ^ shared0)
    body = _fold(l2 ^ first, l0 ^ shared0) ^ _fold(l3 ^ second, l1 ^ shared0)
    return _fold(body ^ 4, shared0)


def concentrated_keys(shared0, count):
    """Distinct keys sharing the first28 compared bytes; own shared0 is required.

    If the actual set uses this shared0, every hash is fold(4, shared0), generally
    nonzero. A guessed or earlier input's seed does not establish realization.
    """
    _uint(shared0, 64)
    _count(count)
    return [shared0 | (shared0 << 64) | (index << 224)
            for index in range(1, count + 1)]


def concentrated_short_prefix_keys(shared0, count):
    """Conditional equal-hash keys with distinct first four compared LE bytes.

    L0=L1=i and L2=L3=own shared0 give body0 and hash fold(4,shared0),
    independently of S/G1. Generated shared0 must match this input's own G0;
    computation does not qualify dynamic hashes or placements. Raw low32 values
    are i=1..count, so every prefix-pair group is a singleton.
    """
    _uint(shared0, 64)
    _count(count)
    return [index | (index << 64) | (shared0 << 128) | (shared0 << 192)
            for index in range(1, count + 1)]


def dispersed_keys(count):
    """Deterministic control hypothesis; verify its distribution under own seeds."""
    _count(count)
    return [int.from_bytes(sha256(b'warm-access-list-control\x00'
                                 + index.to_bytes(4, 'big')).digest(), 'big')
            for index in range(1, count + 1)]
