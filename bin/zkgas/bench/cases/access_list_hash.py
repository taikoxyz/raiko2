"""Frozen b76 access-list hash source and reproducible test catalogs.

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


def concentrated_owner_addresses(shared0, count, *, prefix_bytes=6):
    """Conditional Fb20 owner hashes H=G, independent of the map's unknown S.

    X1=own G and X2=1 force H=fold(1,G)=G for every X0. This Address20
    path differs from generic U256 and fixed32 storage-journal hashes. The selected shared0
    must match the new input's observed own G; a prior G is only a hypothesis.
    Prefix length concerns first compared network-memory bytes, not low32.
    """
    _uint(shared0, 64)
    if type(count) is not int or not 1 <= count <= 16384:
        raise ValueError('owner catalog count must be in 1..16384')
    if type(prefix_bytes) is not int or prefix_bytes not in (0, 4, 6):
        raise ValueError('owner prefix_bytes must be a strict integer in (0, 4, 6)')
    prefix = b'\xa5' + bytes(prefix_bytes - 1) if prefix_bytes else b''
    suffix = shared0.to_bytes(8, 'little') + (1).to_bytes(4, 'little')
    return ['0x' + (prefix + index.to_bytes(8 - prefix_bytes, 'little') + suffix).hex()
            for index in range(1, count + 1)]
