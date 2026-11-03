#!/usr/bin/env python3
"""Generate16 shared-code DELEGATECALL chains with distinct top storage owners."""
import argparse
import inspect
import json
from pathlib import Path
import re

import storage_caught_nesting as common
from spawn_memory import digest, encoded, write
from storage_fanout import MODEL

CORPUS = 'unzen-storage-delegated-nesting-20261103'
SOURCES = ('storage_caught_nesting.py',) + common.SOURCES
# Immutable identities from the independently reviewed prior16 CALL catalog.
# These references are not measurements in this corpus and are never regenerated.
CALL_REFERENCES = ({'sample_id': 'storage-caught-nesting-stop-k16-d1-r0',
  'mode': 'stop',
  'keys_per_account': 16,
  'depth': 1,
  'workload_level': 'zero',
  'request_sha256': 'a80c5497b028c275b20592a281115a49cdd36dfd1b2aaf41d628c7337c95c2fe',
  'runtime_sha256': 'bcfbddd822dacc811a5eaecdfafb44876477e517ad741197eb091f1d5b1d1350',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k16-d1-r2107',
  'mode': 'stop',
  'keys_per_account': 16,
  'depth': 1,
  'workload_level': 'full',
  'request_sha256': 'ca6c94d02bde9949afeb047c3af46cbffa3a4b01d1225087fbbdbb23e0c2f9d0',
  'runtime_sha256': '08ac1b01595762bef630f6d129dd15898c457cebdcc9c0d4d521e35e0302bf46',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k16-d4-r0',
  'mode': 'stop',
  'keys_per_account': 16,
  'depth': 4,
  'workload_level': 'zero',
  'request_sha256': 'd9ac58ef69ad0b15e79e0420a09080d7ff537f92f92de1fa84c853a7472d0220',
  'runtime_sha256': 'bcfbddd822dacc811a5eaecdfafb44876477e517ad741197eb091f1d5b1d1350',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k16-d4-r2105',
  'mode': 'stop',
  'keys_per_account': 16,
  'depth': 4,
  'workload_level': 'full',
  'request_sha256': 'b1de739a3a510b38bc730347982069d60b7c79857afffc3b5fe8d5f238239239',
  'runtime_sha256': '77d6157ebd2284c3a84fb2de5b646eb7942db14f4f9927b008e48e5038f15ef3',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k1024-d1-r0',
  'mode': 'stop',
  'keys_per_account': 1024,
  'depth': 1,
  'workload_level': 'zero',
  'request_sha256': '920efa68d256bf7024186061005ef6fbeab7bc63eaf4e40f37af7ccf940f8fd2',
  'runtime_sha256': 'eb2241f143b4b0c4964f4816183367cdd7478f3f5bc6e0ee432c0fa782b45938',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k1024-d1-r12',
  'mode': 'stop',
  'keys_per_account': 1024,
  'depth': 1,
  'workload_level': 'full',
  'request_sha256': 'a8e71aceda09e7eaaa0b2ffb840abe3dc225986edb9af3202f8b001da97916fb',
  'runtime_sha256': 'f2fbbb69e06e1aedcac9b85f61cf2113793072182c41d9c6230dcb74afab54a9',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k1024-d4-r0',
  'mode': 'stop',
  'keys_per_account': 1024,
  'depth': 4,
  'workload_level': 'zero',
  'request_sha256': '92b8505f9ed229a967c1bc95eac395e9045cbee54f665b718fb574ceba7e5169',
  'runtime_sha256': 'eb2241f143b4b0c4964f4816183367cdd7478f3f5bc6e0ee432c0fa782b45938',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-stop-k1024-d4-r11',
  'mode': 'stop',
  'keys_per_account': 1024,
  'depth': 4,
  'workload_level': 'full',
  'request_sha256': '1478dbf4fc189dde44a26cf3fde39747067caa491dc058c21b7320e5fe8f2560',
  'runtime_sha256': '71f06c15f1f64256529f0e8253a573563f8f2644a5d7376fbe0c317c8cea34c8',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k16-d1-r0',
  'mode': 'revert',
  'keys_per_account': 16,
  'depth': 1,
  'workload_level': 'zero',
  'request_sha256': 'efbf8df5636984147bbf547a33839d11ce3444293d7f312ecd8883f0753d073b',
  'runtime_sha256': 'ba92381032d6b7db0f02321ce05c94b1aea913c286b78dfccdd41db68e275dc8',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k16-d1-r2094',
  'mode': 'revert',
  'keys_per_account': 16,
  'depth': 1,
  'workload_level': 'full',
  'request_sha256': 'fa1e0d22318c7f43b6778f5cc81dab417b5dcbbbbee480effcf229b52124052d',
  'runtime_sha256': 'cc4afc3359e306d87286a8fd233dc99a8986a264443a4daabaefe66b12e15f45',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k16-d4-r0',
  'mode': 'revert',
  'keys_per_account': 16,
  'depth': 4,
  'workload_level': 'zero',
  'request_sha256': '0bda603124391b5c54476e32cc40ba8711010b6fb966cca27307b11b8fd0c459',
  'runtime_sha256': 'ba92381032d6b7db0f02321ce05c94b1aea913c286b78dfccdd41db68e275dc8',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k16-d4-r2092',
  'mode': 'revert',
  'keys_per_account': 16,
  'depth': 4,
  'workload_level': 'full',
  'request_sha256': 'bc6a69233705ae7888a8efb77d583d918d5cc47f422f0d59d18404a8e26dba49',
  'runtime_sha256': 'a2c54877d17cee1ce53a403dbdcb95ac1dd9af59caed817963129b4d21f351d4',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k1024-d1-r0',
  'mode': 'revert',
  'keys_per_account': 1024,
  'depth': 1,
  'workload_level': 'zero',
  'request_sha256': '4d71066e23335d21437de1876c8ab8977df411c78a976813fbb75eb8c7ea68a5',
  'runtime_sha256': '7a0cb0b6c4c199e4fda10fe2a854fa3dc43110f7c7d7b60374f1e787a16eb01c',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k1024-d1-r5',
  'mode': 'revert',
  'keys_per_account': 1024,
  'depth': 1,
  'workload_level': 'full',
  'request_sha256': 'c0c24b58f19b19440063cd12760dbc76704ed10e419c094c1f6307ee3ad4cde0',
  'runtime_sha256': 'e19fb83a444c9ea6233ee4b0460ffec39a12d1ef271456a8456479395971c78b',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k1024-d4-r0',
  'mode': 'revert',
  'keys_per_account': 1024,
  'depth': 4,
  'workload_level': 'zero',
  'request_sha256': 'c459893b7d013070f3d32a6760c4f1593c9b06401f03039dd71be9d203f63fac',
  'runtime_sha256': '7a0cb0b6c4c199e4fda10fe2a854fa3dc43110f7c7d7b60374f1e787a16eb01c',
  'new_measurement': False},
 {'sample_id': 'storage-caught-nesting-revert-k1024-d4-r5',
  'mode': 'revert',
  'keys_per_account': 1024,
  'depth': 4,
  'workload_level': 'full',
  'request_sha256': 'aeb647c3d5b21d2abad4ad7f3956e5fe466ca27ef46d0a91e9a19aa8062136e3',
  'runtime_sha256': 'e19fb83a444c9ea6233ee4b0460ffec39a12d1ef271456a8456479395971c78b',
  'new_measurement': False})


def make_case(mode='stop', keys=16, depth=1, level='full'):
    row = common.make_case(mode, keys, depth, level, call_kind='delegatecall')
    p = row['parameters']; prefix = f'storage-delegated-nesting-{mode}-k{keys}-d{depth}'
    reference = next(ref for ref in CALL_REFERENCES if
        (ref['mode'], ref['keys_per_account'], ref['depth'], ref['workload_level']) == (mode, keys, depth, level))
    row.update(sample_id=f"{prefix}-r{p['rounds_per_key']}", family='storage-delegated-nesting',
        corpus_id=CORPUS, baseline_sample_id=f'{prefix}-r0',
        data_role='unfitted_storage_delegated_nesting_validation',
        scope='Finite shared-code delegated storage composition under frozen c893 pricing. Actual storage owners differ from prior CALL; separate account/code/global support prevents isolated opcode attribution. No per-operation actual allocation, refit, global or production bound, generated proof or throughput claim.')
    p['call_reference_sample_id'] = reference['sample_id']
    return row


def helpers():
    return common.helpers() | {'storage_delegated_nesting.make_case': make_case}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows=[]
    for mode in common.MODES:
        for keys in common.KEY_COUNTS:
            for depth in common.DEPTHS:
                for level in common.LEVELS:
                    row=make_case(mode, keys, depth, level); raw=encoded(row.pop('request'))
                    path=f"requests/{row['sample_id']}.json"; write(output/path,raw)
                    rows.append(row | dict(request_path=path,request_sha256=digest(raw)))
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 16
    write(output/'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name:digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name:digest(inspect.getsource(helper).encode()) for name,helper in helpers().items()},
        purpose='16 new shared-leaf-code delegated storage inputs;16 immutable matched prior CALL references are not rerun or recounted.',
        modes=list(common.MODES), keys_per_account=list(common.KEY_COUNTS), depths=list(common.DEPTHS),
        levels=list(common.LEVELS), users=common.USERS, holdout_only=True, actuals_used_for_fitting=False,
        call_references=CALL_REFERENCES,
        required_comparison_quantities=['actual same-proposal SP1 PGU','node per-block estimates and literal sum','complete proposal estimate'],
        interpretation='Whole-input finite evidence only. Independent all16 native/signatures/receipts/world/authenticated account-specific roots/canonical mutation witnesses/quotes/actual storage-context counters precede freeze and actuals; preserve failures. Shared code does not merge storage owners; equal root multisets do not establish per-owner storage-support identity. No prices/refit/global/production/proof/throughput claim.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,required=True); parser.add_argument('--model-id',default=MODEL)
    args=parser.parse_args(); print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id)}))
