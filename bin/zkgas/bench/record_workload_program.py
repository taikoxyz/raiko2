#!/usr/bin/env python3
"""Record an explicit experimental program's ELF and reviewed-source identities.

This is a provenance declaration, not an attestation that arbitrary ELF bytes implement
the declared policy. Build the dedicated source, review the snapshot, and qualify its
behavior with controls and an input exceeding the production legacy budget.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess


def digest(data):
    return hashlib.sha256(data).hexdigest()


def source_revision(root):
    return subprocess.run(['git', '-C', str(root), 'rev-parse', 'HEAD'], check=True,
                          capture_output=True, text=True, timeout=5).stdout.strip()


def source_paths(root):
    paths = {root / 'Cargo.toml', root / 'config/chain_spec_list_default.json',
             root / 'guests/workload-sp1/Cargo.toml',
             root / 'guests/workload-sp1/Cargo.lock',
             root / 'guests/sp1/src/crypto.rs', root / 'guests/sp1/src/sys.rs'}
    for relative in ('guests/workload-sp1/src', 'crates/guest-common', 'crates/primitives',
                     'crates/primitives-shasta', 'crates/protocol-shasta', 'crates/protocol',
                     'crates/stateless'):
        base = root / relative
        paths.update(p for p in base.rglob('*') if p.is_file() and
                     (p.suffix == '.rs' or p.name == 'Cargo.toml') and 'target' not in p.parts)
    return sorted(paths)


def record(root, elf, output):
    if elf.is_symlink() or not elf.is_file() or not 64 <= elf.stat().st_size <= 64 * 1024 * 1024:
        raise ValueError('expected a regular RISC-V ELF of 64 bytes to 64 MiB')
    data = elf.read_bytes()
    if data[:7] != b'\x7fELF\x02\x01\x01' or struct.unpack_from('<HHI', data, 16) != (2, 243, 1):
        raise ValueError('expected a 64-bit little-endian RISC-V executable ELF')
    manifest = {'schema_version': 1, 'execution_profile': 'evm-gas-only',
                'sp1_sdk_version': '6.3.0', 'elf_sha256': digest(data),
                'source_revision': source_revision(root),
                'source_sha256': {p.relative_to(root).as_posix(): digest(p.read_bytes())
                                  for p in source_paths(root)},
                'provenance_scope': 'Declared reviewed source snapshot and selected ELF identity; '
                                    'not a build attestation or proof of arbitrary ELF policy.'}
    with output.open('x') as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write('\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, default=Path(__file__).resolve().parents[3])
    args = parser.parse_args()
    value = record(args.source_root.resolve(), args.elf, args.output)
    print(json.dumps({'elf_sha256': value['elf_sha256'],
                      'execution_profile': value['execution_profile']}))
