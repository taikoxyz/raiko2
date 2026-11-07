#!/usr/bin/env python3
"""Generate finite CALL-input memory/copy cases with a leaf that reads only its header."""
import argparse
import json
from pathlib import Path

from spawn_memory import make_case, prepare


def make_input_case(runtime, kind, length, count):
    row = make_case(runtime, kind, length, count)
    for key in ('sample_id', 'pair_group'):
        row[key] = row[key].replace('spawn-memory-', 'spawn-input-memory-')
    row['family'] = 'spawn-input-memory'
    del row['parameters']['output_length']
    row['parameters']['input_extra_length'] = length
    row['parameters']['inner_calldata_length'] = 128 + length
    row['memory_semantics'] = 'fresh middle-frame memory; CALL copies header plus zero-filled range; leaf STOP'
    return row


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--solc', required=True, type=Path)
    args = parser.parse_args()
    count = prepare(args.output_dir, args.solc, source=Path(__file__).with_name('SpawnInput.sol'),
                    contract='SpawnInput', case_factory=make_input_case, generator=Path(__file__),
                    purpose='CALL-input memory and copy selector ownership')
    print(json.dumps({'prepared': count}))
