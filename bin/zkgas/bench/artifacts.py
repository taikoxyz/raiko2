"""Bounded artifact IO; compression never discards the only verified copy."""
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat

CHUNK = 1024 * 1024
MIB = CHUNK
MAX_INPUT = 64 * MIB


def require(condition, message):
    if not condition:
        raise ValueError(message)


def regular(path):
    require(stat.S_ISREG(Path(path).lstat().st_mode), f'not a regular file: {path}')


def identity(path, compressed=False, limit=None):
    regular(path)
    digest, length = hashlib.sha256(), 0
    with (gzip.open(path, 'rb') if compressed else Path(path).open('rb')) as source:
        while block := source.read(CHUNK):
            length += len(block)
            require(limit is None or length <= limit, 'artifact exceeds byte limit')
            digest.update(block)
    return digest.hexdigest(), length


def exclusive_json(path, value):
    with Path(path).open('x') as target:
        json.dump(value, target, indent=2, sort_keys=True, allow_nan=False)
        target.write('\n')
        target.flush()
        os.fsync(target.fileno())


def read_json(path, limit=MAX_INPUT):
    regular(path)
    opener = gzip.open if Path(path).suffix == '.gz' else open
    with opener(path, 'rb') as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, 'JSON exceeds byte limit')
    return json.loads(raw)


def safe_child(root, name):
    require(isinstance(name, str) and name not in ('', '.', '..') and Path(name).name == name,
            'artifact must have a plain filename')
    path = root / name
    require(not path.is_symlink(), 'refusing symlink artifact')
    return path


def receipt(path):
    digest, length = identity(path)
    return {'path': path.name, 'sha256': digest, 'length': length}


def compress(path):
    digest, length = identity(path, limit=MAX_INPUT)
    target = path.with_name(path.name + '.gz')
    with target.open('xb') as output:
        with gzip.GzipFile(fileobj=output, mode='wb', filename='', mtime=0) as archive:
            with path.open('rb') as source:
                shutil.copyfileobj(source, archive, CHUNK)
        output.flush()
        os.fsync(output.fileno())
    require(identity(target, compressed=True, limit=MAX_INPUT) == (digest, length),
            'gzip verification failed; original retained')
    require(identity(path) == (digest, length), 'source changed during compression')
    result = receipt(target)
    result.update(uncompressed_sha256=digest, uncompressed_length=length)
    path.unlink()
    return result


def restore(archive, output, record):
    require(identity(archive) == (record['sha256'], record['length']), 'archive identity mismatch')
    with gzip.open(archive, 'rb') as source, output.open('xb') as target:
        total = 0
        while block := source.read(CHUNK):
            total += len(block)
            require(total <= min(MAX_INPUT, record['uncompressed_length']), 'decompression limit')
            target.write(block)
        target.flush()
        os.fsync(target.fileno())
    require(identity(output) == (record['uncompressed_sha256'], record['uncompressed_length']),
            'restored identity mismatch')


def validate_artifacts(root, records):
    names = set()
    for record in records:
        require(record['path'] not in names, 'duplicate artifact receipt')
        names.add(record['path'])
        path = safe_child(root, record['path'])
        require(identity(path) == (record['sha256'], record['length']), 'artifact identity mismatch')
        if 'uncompressed_sha256' in record:
            require(identity(path, compressed=True, limit=MAX_INPUT) ==
                    (record['uncompressed_sha256'], record['uncompressed_length']),
                    'compressed artifact identity mismatch')
    actual = {p.name for p in root.iterdir() if p.name not in ('started.json', 'status.json')}
    require(names == actual, 'attempt artifact closure changed')


def copy_exclusive(source, target, expected=None, limit=MAX_INPUT):
    before = identity(source, limit=limit)
    require(expected is None or before[0] == expected, 'source identity mismatch')
    with Path(source).open('rb') as original, target.open('xb') as output:
        count = 0
        while block := original.read(CHUNK):
            count += len(block)
            require(count <= limit, 'source grew beyond limit')
            output.write(block)
    require(identity(target) == before and identity(source) == before, 'copy identity mismatch')
