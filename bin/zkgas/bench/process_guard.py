"""Linux sequential process supervisor, including orphaned/escaped descendants."""
from contextlib import contextmanager
import ctypes
import os
from pathlib import Path
import resource
import select
import shutil
import signal
import subprocess
import time

from artifacts import MIB, exclusive_json, require
from shared_memory import cleanup as cleanup_shared_memory


class StageFailure(RuntimeError):
    def __init__(self, stage, failure, exit_code=None):
        self.stage, self.failure, self.exit_code = stage, failure, exit_code
        super().__init__(f'{stage}: {failure} (exit={exit_code})')


def process_table():
    table = {}
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
            # PID start time prevents cleanup from killing a reused PID.
            table[int(entry.name)] = (int(fields[1]), int(fields[2]), int(fields[19]), fields[0])
        except (FileNotFoundError, ProcessLookupError):
            continue
    return table


def ownership(root, known):
    table = process_table()
    owned = {pid for pid, start in known.items() if pid in table and table[pid][2] == start}
    # The runner is a subreaper: detached grandchildren reparent here after leader exit.
    owned |= {pid for pid, row in table.items() if row[1] == root or row[0] == os.getpid()}
    if root in table and (root not in known or table[root][2] == known[root]):
        owned.add(root)
    while True:
        expanded = owned | {pid for pid, row in table.items() if row[0] in owned}
        if expanded == owned:
            break
        owned = expanded
    known.update({pid: table[pid][2] for pid in owned if pid in table})
    live = {pid for pid in owned if pid in table and table[pid][3] != 'Z'}
    return known, live, table


def tree(root, known):
    known, live, _ = ownership(root, known)
    pss = 0
    for pid in live | {os.getpid()}:
        try:
            rows = Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines()
        except (FileNotFoundError, ProcessLookupError):
            continue
        values = [line.split() for line in rows if line.startswith('Pss:')]
        require(len(values) == 1 and values[0][2] == 'kB', 'PSS accounting unavailable')
        pss += int(values[0][1]) * 1024
    return known, live, pss


def terminate(process, known):
    # A child can fork/detach after the last resource sample. Repeated ownership scans
    # discover those children through their parent or the runner's subreaper adoption.
    # Cleanup must remain available even when reading PSS itself failed.
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        known, _, table = ownership(process.pid, known)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        for pid, started in known.items():
            if pid not in table or table[pid][2] != started:
                continue
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.poll()
        # Reap adopted descendants; their parents may exit during any iteration.
        for pid in known:
            if pid == process.pid:
                continue
            try:
                os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                pass
        known, live, remaining = ownership(process.pid, known)
        retained = {pid for pid, started in known.items()
                    if pid in remaining and remaining[pid][2] == started}
        if process.poll() is not None and not live and not retained:
            return
        time.sleep(.02)
    raise RuntimeError('owned descendants survived SIGKILL cleanup')


@contextmanager
def interruption_handlers():
    previous = {}

    def interrupted(number, frame):
        raise InterruptedError(f'received signal {number}')

    try:
        for number in (signal.SIGTERM, signal.SIGHUP):
            previous[number] = signal.signal(number, interrupted)
        yield
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)


def run(command, folder, stage, limits, growing=(), resource_receipt=None):
    require(Path('/proc/self/smaps_rollup').is_file(), 'Linux /proc PSS is required')
    require(ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) == 0,
            'cannot enable child subreaper')
    started, peak, failure = time.monotonic(), 0, None
    record = {'stage': stage, 'command': command, 'limits': limits,
              'memory_accounting': 'sampled runner + full descendant PSS',
              'artifact_accounting': 'sampled output/disk bounds; strict retained log byte cap',
              'kernel_file_size_cap': stage != 'sp1'}
    exclusive_json(folder / f'{stage}-command.json', record)
    process, known = None, {}

    def child_limits():
        # Native/quote ordinary files can use a kernel cap. SP1 reserves large sparse
        # POSIX shared-memory buffers via ftruncate; RLIMIT_FSIZE also covers those
        # descriptors, so it cannot represent an on-disk artifact limit for SP1.
        if stage == 'sp1':
            return
        ceiling = max(limits['output_mib'], limits['log_mib']) * MIB
        resource.setrlimit(resource.RLIMIT_FSIZE, (ceiling, ceiling))

    with (folder / f'{stage}.log').open('xb') as log, interruption_handlers():
        try:
            process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       start_new_session=True, preexec_fn=child_limits)
            os.set_blocking(process.stdout.fileno(), False)

            def drain_log(final=False):
                overflow = False
                # Bound work per sample so a continuously writing process cannot starve
                # memory/time/disk monitoring. The pipe also applies backpressure.
                reads = 0
                while final or reads < 16:
                    reads += 1
                    try:
                        block = os.read(process.stdout.fileno(), 65536)
                    except BlockingIOError:
                        break
                    if not block:
                        break
                    remaining = limits['log_mib'] * MIB - log.tell()
                    log.write(block[:remaining])
                    if len(block) > remaining:
                        overflow = True
                        break
                log.flush()
                return overflow

            while True:
                log_overflow = drain_log()
                known, live, pss = tree(process.pid, known)
                peak = max(peak, pss)
                if pss > limits['pss_mib'] * MIB:
                    failure = 'process_tree_pss_limit'
                elif time.monotonic() - started > limits['timeout_seconds']:
                    failure = 'wall_time_limit'
                elif shutil.disk_usage(folder).free < limits['min_free_mib'] * MIB:
                    failure = 'minimum_free_disk_guard'
                elif sum(p.stat().st_size for p in growing if p.exists()) > limits['output_mib'] * MIB:
                    failure = 'output_size_limit'
                elif log_overflow:
                    failure = 'log_size_limit'
                code = process.poll()
                if failure:
                    terminate(process, known)
                    drain_log(final=True)
                    break
                if code is not None:
                    # A leader may fork/detach after the resource snapshot and then exit.
                    # poll() reaps it, so adopted children are discoverable now.
                    known, live, _ = ownership(process.pid, known)
                    if live - {process.pid}:
                        failure = 'descendants_survived_leader'
                        terminate(process, known)
                        drain_log(final=True)
                    elif drain_log(final=True):
                        failure = 'log_size_limit'
                    elif code != 0:
                        failure = 'file_size_limit' if code == -signal.SIGXFSZ else 'nonzero_exit'
                    break
                select.select([process.stdout], [], [], .1)
        except BaseException as error:
            failure = f'{type(error).__name__}: {error}'
            if process is not None:
                try:
                    known, _, _ = tree(process.pid, known)
                finally:
                    terminate(process, known)
            raise
        finally:
            if resource_receipt is not None and process is not None and process.poll() is not None:
                try:
                    known, live, _ = ownership(process.pid, known)
                    require(not live, 'cannot clean resources while owned processes survive')
                    cleanup = cleanup_shared_memory(resource_receipt, process.pid)
                    record['shared_memory_cleanup'] = cleanup
                    if cleanup['status'] != 'verified' and failure is None:
                        failure = 'shared_memory_receipt_missing'
                except Exception as error:
                    record['shared_memory_cleanup'] = {'status': 'failed', 'error': str(error)}
                    failure = failure or 'shared_memory_cleanup_failed'
            record.update(exit_code=process.poll() if process else None, failure=failure,
                          peak_process_tree_pss_bytes=peak,
                          wall_time_seconds=time.monotonic() - started)
            exclusive_json(folder / f'{stage}-outcome.json', record)
            if process is not None:
                process.stdout.close()
    if failure:
        raise StageFailure(stage, failure, process.returncode)
    return record
