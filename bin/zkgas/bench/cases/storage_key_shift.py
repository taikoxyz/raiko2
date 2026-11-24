#!/usr/bin/env python3
"""Generate 24 finite high-limb storage-key inputs with eight existing references."""
import argparse
import inspect
import json
from pathlib import Path
import re

from spawn_memory import digest, encoded, write
import storage_dirty_keys as dirty

CORPUS = 'unzen-storage-key-shift-20261103'
MODEL = dirty.MODEL
MODES = dirty.MODES
KEY_COUNTS = (16, 1024)
SHIFTS = (64, 128, 192)
LEVELS = dirty.LEVELS
SOURCES = ('storage_dirty_keys.py',) + dirty.SOURCES


def make_case(mode='stop', keys=16, shift=64, level='full') -> dict:
    if type(keys) is not int or keys not in KEY_COUNTS:
        raise ValueError('keys must be 16 or 1024')
    if type(shift) is not int or shift not in SHIFTS:
        raise ValueError('shift must be 64, 128 or 192')
    row = dirty.make_case(mode, keys, level, slot_shift=shift)
    p = row['parameters']; rounds = p['rounds_per_key']
    prefix = f'storage-key-shift-{mode}-k{keys}-s{shift}'
    row.update(sample_id=f'{prefix}-r{rounds}', family='storage-key-shift', corpus_id=CORPUS,
        baseline_sample_id=f'{prefix}-r0', data_role='unfitted_storage_key_shift_validation',
        completion_gate='Independent source review and all24 native/signature/receipt/whole-world/authenticated per-account-root/canonical mutation-witness/original-real-quote/counter gates precede freeze and actuals. Native acceptance alone is insufficient.',
        scope='Finite high-limb raw-slot distributions under frozen c893 pricing. Preserve own-R0 controls, eight exact existing unshifted references and failures. Compare actual same-proposal SP1 PGU, literal node block estimates/sum and complete proposal estimate separately. Whole-input evidence; no hash-collision, refit, per-operation actual allocation, global or production bound, generated proof or GPU/throughput claim.')
    p['slot_shift'] = shift
    p['phase_slot'] = '0x0'
    for name in ('phase_base_by_account', 'initial_phase_by_account', 'expected_final_phase_by_account'):
        p[name] = {owner: hex(value) for owner, value in p[name].items()}
    for name in ('start_phase_schedule', 'final_phase_schedule'):
        p[name] = [hex(value) for value in p[name]]
    p.update(
        raw_body_slot_formula='start+(j<<slot_shift), j=0..K-1; start=B_i+1; B_i=(i+1)*65536',
        initial_storage_rule='B_i=(i+1)*65536; read-only raw phase0=B_i+1; raw body slot_j=start+(j<<slot_shift)=1, j=0..K-1; all others absent. Identical own-R0/full maps. Raw U256 slots use hexadecimal strings.',
        range_rule='Four distinct users visit four distinct contracts once: seed all K high-limb slots 1->5; each key R warm dirty 5->7->5 pairs in64-pair batches plus tail; cleanup all K 5->1. Shift index once in each seed/dirty/cleanup traversal; phase0 unchanged.')
    return row


def unshifted_references():
    """Join keys to preceding immutable observations; never new sampled actuals."""
    rows = []
    for mode in MODES:
        for keys in KEY_COUNTS:
            for level in LEVELS:
                row = dirty.make_case(mode, keys, level)
                rows.append(dict(sample_id=row['sample_id'], corpus_id=dirty.CORPUS,
                    request_sha256=digest(encoded(row['request'])), runtime_sha256=row['parameters']['runtime_sha256'],
                    mode=mode, keys_per_account=keys, workload_level=level,
                    rounds_per_key=row['parameters']['rounds_per_key'], slot_shift=0, newly_sampled=False))
    return rows


def helpers():
    return dirty.helpers() | {
        'storage_dirty_keys.make_case': dirty.make_case,
        'storage_key_shift.make_case': make_case,
        'storage_key_shift.unshifted_references': unshifted_references}


def prepare(output: Path, *, model_id=MODEL) -> int:
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for mode in MODES:
        for keys in KEY_COUNTS:
            for shift in SHIFTS:
                for level in LEVELS:
                    row = make_case(mode, keys, shift, level)
                    raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                    write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 24
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='24 new finite high-limb raw-slot inputs: stop/revert, K16/1024, shifts64/128/192, own-R0/maximal-uniform-full; eight existing shift0 references are joined, never newly sampled.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        modes=list(MODES), keys_per_account=list(KEY_COUNTS), slot_shifts=list(SHIFTS), levels=list(LEVELS), users=dirty.USERS,
        holdout_only=True, actuals_used_for_fitting=False, unshifted_references=unshifted_references(),
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Finite frozen-pricing whole-input evidence. Independent source review and all24 native/signature/receipt/whole-world/authenticated-root/canonical-witness/original-real-quote/counter gates precede freeze and actuals. Raw slots use hex, never custom hashed-slot injection. Maximal uniform R can be limited by refund/envelope prefixes or granularity before99% charged occupancy. Whole-world fees/nonces/Anchor change. Existing references retain their original source/config/ELF/actual/quote identities. Preserve controls and failures; no pricing fit, isolated-operation actual allocation or global bound.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
