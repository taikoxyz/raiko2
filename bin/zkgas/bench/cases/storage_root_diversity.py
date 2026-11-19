#!/usr/bin/env python3
"""Generate 32 storage-root diversity inputs with homogeneous references and own N0."""
import argparse
import inspect
import json
from pathlib import Path
import re

from spawn_memory import digest, encoded, write
import storage_fanout as fanout

MODEL = fanout.MODEL
CORPUS = 'unzen-storage-root-diversity-20261103'
PROFILES = ('background-values', 'disjoint-keys')
ACCOUNT_COUNTS = (8, 64)
STATE_SIZES = (8192, 16384)
MODES = ('update', 'delete')
LEVELS = ('zero', 'full')


def make_case(profile='background-values', accounts=8, state_size=8192, mode='update', level='full'):
    if type(profile) is not str or profile not in PROFILES:
        raise ValueError('profile must be background-values or disjoint-keys')
    if type(accounts) is not int or accounts not in ACCOUNT_COUNTS:
        raise ValueError('accounts must be 8 or 64')
    # Fanout remains authoritative for validated size/mode/level, runtime,
    # recipient declarations, full-N selection, gas and admission prefixes.
    row = fanout.make_case(accounts, state_size, mode, level)
    p = row['parameters']; n = p['mutations_per_user']; full = p['full_mutations_per_user']
    slots = p['state_size_per_account']; visits = p['visits_per_account']
    contracts = row['request']['program']['prestate_contracts']
    bases = {}; initial = {}; final = {}
    for i, contract in enumerate(contracts):
        owner = contract['address']; base = 0 if profile == 'background-values' else (i + 1) * 65536
        bases[owner] = base; initial[owner] = base + 1; final[owner] = base + 1 + visits * n
        if profile == 'background-values':
            # Full-N, including for own N0: only unreachable tail values differ.
            storage = {key: 1 if key <= visits * full else i + 2 for key in range(slots)}
        else:
            storage = {0: base + 1} | dict.fromkeys(range(base + 1, base + slots), 1)
        contract['initial_storage'] = {hex(key): hex(value) for key, value in storage.items()}
    p.pop('initial_phase'); p.pop('expected_final_phase_per_account')
    recipients = p['recipient_schedule']
    p.update(profile=profile, phase_base_by_account=bases, initial_phase_by_account=initial,
        expected_final_phase_by_account=final,
        start_phase_schedule=[initial[owner] + (j // accounts) * n for j, owner in enumerate(recipients)],
        final_phase_schedule=[initial[owner] + (j // accounts + 1) * n for j, owner in enumerate(recipients)],
        initial_storage_rule=(
            'background-values: exactly raw slots0..K-1; phase slot0=1, slots1..V*fullN=1, '
            'slots V*fullN+1..K-1=i+2. The fullN boundary also applies to own N0; all other slots absent.'
            if profile == 'background-values' else
            'disjoint-keys: B_i=(i+1)*65536, raw phase slot0=B_i+1; raw slots B_i+1..B_i+K-1=1; '
            'all other slots absent. Body values are unchanged; accessed phase control values differ.'),
        range_rule=(f'{profile}: user j calls account j%A; start=phase_base_by_account[recipient]+1+floor(j/A)*N. '
            'Visit start..start+N-1 ascending, with body values1->2 for update or1->0 for delete; '
            'commit start+N to raw slot0. Disjoint-key raw ranges and phase values shift by B_i.'))
    reference = row['sample_id']; prefix = f'storage-root-diversity-{profile}-a{accounts}-s{state_size}-{mode}'
    row.update(sample_id=f'{prefix}-n{n}', family='storage-root-diversity', corpus_id=CORPUS,
        baseline_sample_id=f'{prefix}-n0', homogeneous_reference_sample_id=reference,
        data_role='unfitted_storage_root_diversity_validation',
        completion_gate='Independent source review and all32 native, signed recipient/receipt, authenticated initial/final whole-account and per-account roots, canonical mutation-witness and original real-quote gates precede actuals. Require A distinct initial and final storage roots. Native success alone is not the semantic oracle.',
        scope='Finite background-values/disjoint-keys storage-root diversity under frozen c893 pricing; preserve own-N0 controls, immutable homogeneous references and failures. Distinct roots do not prove zero internal-node sharing. Compare whole-proposal actual SP1 PGU, literal per-block node estimates/sum and complete proposal quote separately. No refit, universal MPT bound, per-operation actual attribution, generated proof or throughput claim.')
    return row


def prepare(output: Path, *, model_id=MODEL) -> int:
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'requests').mkdir()
    rows = []
    for profile in PROFILES:
        for accounts in ACCOUNT_COUNTS:
            for size in STATE_SIZES:
                for mode in MODES:
                    for level in LEVELS:
                        row = make_case(profile, accounts, size, mode, level)
                        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                        write(output / path, raw)
                        rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 32
    sources = ('storage_fanout.py', 'storage_mutation.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')
    helpers = {'storage_fanout.make_case': fanout.make_case, 'storage_fanout.full_n': fanout.full_n,
        'storage_mutation._runtime': fanout.mutation._runtime, 'storage_mutation.gas_accounting': fanout.mutation.gas_accounting}
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='32 storage-root diversity inputs: background-values/disjoint-keys, accounts8/64, aggregate state8192/16384, update/delete and own-N0/full pairs. Sixteen immutable homogeneous reference identities.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in sources},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers.items()},
        profiles=list(PROFILES), account_counts=list(ACCOUNT_COUNTS), aggregate_state_sizes=list(STATE_SIZES),
        modes=list(MODES), levels=list(LEVELS), users=fanout.USERS, holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Frozen pricing validation only. Independent source review and all32 native, signed recipient/receipt, authenticated initial/final roots with A distinct per-account storage roots, canonical mutation-witness and original real-quote gates precede actuals. Root diversity does not imply zero internal-node sharing. Preserve failures and both deficit flags; 30M charged budget and31M header remain separate.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
