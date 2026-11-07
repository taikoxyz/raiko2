"""Clean only exact SDK object identities receipted by the owned sampler before execution."""
import os
from pathlib import Path
import re
import stat

from artifacts import read_json, require

NAME = re.compile(r'(?:sp1_[A-Za-z0-9_-]{22}==_[mt]|sem\.sp1_[A-Za-z0-9_-]{22}==_t_(?:filled|empty))')


def cleanup(receipt, producer_pid, directory=Path('/dev/shm')):
    if not receipt.exists():
        return {'status': 'missing_receipt', 'objects': []}
    document = read_json(receipt, limit=1024 * 1024)
    require(document['schema_version'] == 1 and document['stage'] == 'sp1_owned_shared_memory' and
            document['producer_pid'] == producer_pid, 'shared-memory receipt producer mismatch')
    resources = document['resources']
    require(isinstance(resources, list) and len(resources) == 4, 'receipt must contain four SDK resources')
    declared = {item['name'] for item in resources}
    memory_names = [name for name in declared if isinstance(name, str) and name.startswith('sp1_') and name.endswith('_m')]
    require(len(memory_names) == 1, 'receipt must identify one SDK runner')
    sdk_id = memory_names[0][:-2]
    require(declared == {sdk_id + '_m', sdk_id + '_t', 'sem.' + sdk_id + '_t_filled',
                         'sem.' + sdk_id + '_t_empty'}, 'receipt SDK resource set is incomplete or mixed')
    names, checked, result = set(), [], []
    # Validate the complete set before deleting anything. A changed inode/name is retained.
    for item in resources:
        name = item['name']
        require(isinstance(name, str) and NAME.fullmatch(name) and name not in names,
                'invalid/duplicate SDK resource name')
        names.add(name)
        path = directory / name
        try:
            current = path.lstat()
        except FileNotFoundError:
            result.append({'name': name, 'status': 'already_removed'})
            continue
        expected = item['device'], item['inode'], item['uid']
        require(stat.S_ISREG(current.st_mode) and current.st_uid == os.getuid() and
                (current.st_dev, current.st_ino, current.st_uid) == expected,
                'SDK resource identity changed; refusing cleanup')
        checked.append((path, expected))
    for path, expected in checked:
        current = path.lstat()
        require((current.st_dev, current.st_ino, current.st_uid) == expected,
                'SDK resource changed before unlink')
        path.unlink()
        result.append({'name': path.name, 'status': 'unlinked'})
    return {'status': 'verified', 'objects': result}
