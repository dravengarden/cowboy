#!/usr/bin/env python3
"""Exact production writer builds in private root mount/PID/network namespaces."""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

import session_deletion_writer_conformance as ipc


PROFILE_PARENT = Path('/nix/var/nix/profiles/columbus-components')
FLOOR_PARENT = Path('/var/lib/hawk-component-deployments/cowboy-machine')
FLOOR_NAME = 'session-deletion-reader-floor.json'
MOUNT = '/run/current-system/sw/bin/mount'
UMOUNT = '/run/current-system/sw/bin/umount'


def artifact(value, writer):
    root = Path(value)
    ipc.require(root.parent == Path('/nix/store') and root.resolve() == root,
                'exact immutable store root required')
    source = json.loads((root / 'etc/cowboy-release/source.json').read_text())
    ipc.require(source['component'] == 'cowboy' and source['lane'] == 'machine'
                and source['schema'] == 1 and source['dirty'] is False
                and len(source['revision']) == 40
                and source['sessionDeletionJournal'] == {'readerSchema': 1, 'writerSchema': int(writer)},
                'unexpected production release declaration')
    native = (root / 'libexec/cowboy-machine').resolve().parent / '.cowboy-machine-wrapped'
    with native.open('rb') as file:
        ipc.require(file.read(4) == b'\x7fELF', 'native ELF required')
        file.seek(0)
        digest = hashlib.file_digest(file, 'sha256').hexdigest()
    return {'release': str(root), 'source': source, 'native': str(native), 'sha256': digest}


class NativeProcess(ipc.Process):
    def __init__(self, release, state, fixture=False, extra_env=None):
        ipc.require(fixture is False, 'only production launchers admitted')
        self.generation = release['source']['workerGeneration']
        super().__init__(release, state, fixture=False, extra_env=extra_env)


def connect(process, role):
    # Cold restart can leave the previous socket pathname until the new broker
    # binds. Retry only connection establishment, never a sent protocol frame.
    deadline = time.monotonic() + 5
    while True:
        peer = socket.socket(socket.AF_UNIX)
        peer.settimeout(5)
        try:
            peer.connect(str(process.socket))
            break
        except (FileNotFoundError, ConnectionRefusedError):
            peer.close()
            ipc.require(process.child.poll() is None and time.monotonic() < deadline,
                        f'broker did not accept connection: {process.output()}')
            time.sleep(0.01)
        except BaseException:
            peer.close()
            raise
    ipc.send(peer, {'type': 'hello', 'role': role, 'min_protocol': 1, 'max_protocol': 2,
                   'build': 'immutable-production-writer',
                   'session_id': 'sess-1' if role == 'worker' else None,
                   'worker_epoch': 'old-epoch' if role == 'worker' else None,
                   'generation': process.generation, 'executable': '/bin/false', 'fallback_for': None})
    return peer, ipc.receive(peer)


class Authority:
    def __init__(self, root, reader):
        self.profiles = root / 'profiles'
        self.floors = root / 'floors'
        self.profiles.mkdir(mode=0o755)
        self.floors.mkdir(mode=0o755)
        self.reader = reader
        subprocess.run([MOUNT, '--bind', str(self.profiles), str(PROFILE_PARENT)], check=True)
        subprocess.run([MOUNT, '--bind', str(self.floors), str(FLOOR_PARENT)], check=True)

    @property
    def floor(self):
        return self.floors / FLOOR_NAME

    @property
    def profile(self):
        return self.profiles / 'cowboy-machine'

    def select(self, release):
        self.profile.unlink(missing_ok=True)
        self.profile.symlink_to(release['release'])

    def admit(self, state):
        value = {'schema': 1, 'machine': 'release-fixture',
                 'dataset': str(state / 'session-deletions'), 'readerSchema': 1,
                 'release': self.reader['release'], 'revision': self.reader['source']['revision']}
        self.floor.write_text(json.dumps(value))
        self.floor.chmod(0o644)
        os.chown(self.floor, 0, 0)


@contextmanager
def masked_source(release, data):
    target = Path(release['release']) / 'etc/cowboy-release/source.json'
    with tempfile.TemporaryDirectory(prefix='cw-source-', dir='/tmp') as temporary:
        source = Path(temporary) / 'source.json'
        source.write_bytes(data)
        source.chmod(0o444)
        subprocess.run([MOUNT, '--bind', str(source), str(target)], check=True)
        try:
            yield
        finally:
            subprocess.run([UMOUNT, str(target)], check=True)


def cold(release, state, authority, deleted, writer):
    authority.select(release)
    authority.admit(state)
    with NativeProcess(release, state, fixture=False) as process:
        process.ready(fixture=False)
        ipc.require(f'writer_enabled={str(writer).lower()}' in process.output(), 'wrong runtime writer admission')
        peer, reply = connect(process, 'worker')
        with peer:
            ipc.require(reply['type'] == ('reject' if deleted else 'welcome'), f'wrong cold outcome: {reply}')
            if deleted:
                ipc.require('deleted' in reply['reason'], 'wrong terminal fence')


def refuse(release, state, reason=None):
    with NativeProcess(release, state, fixture=False) as process:
        process.refused(reason or 'admitting component Session deletion writer')
    ipc.require(not (state / 'session-deletions').exists(), 'refused writer opened journal namespace')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-writer', required=True)
    parser.add_argument('--new-writer', required=True)
    parser.add_argument('--reader-release', required=True)
    parser.add_argument('--default-reader-release', required=True)
    parser.add_argument('--host-mount-ns', required=True)
    parser.add_argument('--host-net-ns', required=True)
    parser.add_argument('--receipt', required=True)
    args = parser.parse_args()
    ipc.require(os.geteuid() == 0, 'private root namespace required')
    ipc.require(os.readlink('/proc/self/ns/mnt') != args.host_mount_ns
                and os.readlink('/proc/self/ns/net') != args.host_net_ns,
                'refuse authority mounts in host namespaces')
    subprocess.run([MOUNT, '--make-rprivate', '/'], check=True)
    old = artifact(args.old_writer, True)
    new = artifact(args.new_writer, True)
    reader = artifact(args.reader_release, False)
    default_reader = artifact(args.default_reader_release, False)
    ipc.require(old['source']['revision'] != new['source']['revision'] and old['sha256'] != new['sha256'],
                'independently built production writer revisions and ELFs required')
    ipc.require(old['source']['workerGeneration'] == new['source']['workerGeneration']
                == reader['source']['workerGeneration'], 'retained generation must match')
    observations = []
    with tempfile.TemporaryDirectory(prefix='cw-authority-', dir='/tmp') as authority_root:
        authority = Authority(Path(authority_root), reader)
        with tempfile.TemporaryDirectory(prefix='cw-default-', dir='/tmp') as temporary:
            state = Path(temporary)
            authority.select(new)
            with NativeProcess(default_reader, state, extra_env={
                    'COWBOY_SESSION_DELETION_WRITER_BUILD': 'schema1',
                    'COWBOY_SESSION_DELETION_WRITER_REVISION': new['source']['revision']}) as process:
                process.ready(fixture=False)
                ipc.require('writer_enabled=false' in process.output(), 'runtime environment enabled default writer')
                peer, welcome = connect(process, 'core')
                with peer:
                    ipc.require(welcome['type'] == 'welcome', 'default core welcome missing')
                    ipc.stop(peer)
                    ipc.ack(peer, True)
                ipc.require(not (state / 'session-deletions/deletions.json').exists(), 'default reader wrote a terminal record')
                ipc.require(not (state / 'session-cleanups').exists(), 'default reader opened the cleanup continuation namespace')
            observations.append({'case': 'default-build-runtime-env-cannot-enable-writer', 'writerEnabled': False})
        for writer in (old, new):
            with tempfile.TemporaryDirectory(prefix='cw-ack-', dir='/tmp') as temporary:
                state = Path(temporary)
                authority.select(writer)
                authority.admit(state)
                record = state / 'session-deletions/deletions.json'
                with NativeProcess(writer, state, fixture=False) as process:
                    process.ready(fixture=False)
                    ipc.require('writer_enabled=true' in process.output(), 'actual writer did not admit')
                    peer, welcome = connect(process, 'core')
                    with peer:
                        ipc.require(welcome['type'] == 'welcome', 'core welcome missing')
                        ipc.stop(peer)
                        ipc.ack(peer, True)
                        captured = record.read_bytes()
                        inode = record.stat().st_ino
                        ipc.stop(peer, 'duplicate-delete')
                        ipc.ack(peer, True, 'duplicate-delete')
                        ipc.require(record.read_bytes() == captured and record.stat().st_ino == inode,
                                    'dedup rewrote committed evidence')
                        if writer is new:
                            namespace = state / 'session-cleanups'
                            ipc.require((namespace / '.lock').is_file() and not (namespace / 'cleanups.json').exists(),
                                        'admitted writer did not own an empty cleanup continuation namespace')
                    contender = new if writer is old else old
                    authority.select(contender)
                    with NativeProcess(contender, state, fixture=False) as rejected:
                        rejected.refused('already owned')
                    authority.select(writer)
                ipc.require(json.loads(captured)['deleted'] == ['sess-1'], 'wrong terminal record')
                for reopener in (old, new, old):
                    cold(reopener, state, authority, True, True)
                    ipc.require(record.read_bytes() == captured and record.stat().st_ino == inode,
                                'writer reopen rewrote evidence')
                cold(reader, state, authority, True, False)
                ipc.require(record.read_bytes() == captured, 'reader-only fallback changed evidence')
                observations.append({'case': 'ack-dedup-sigkill-old-new-old-reader-fallback',
                                     'revision': writer['source']['revision'],
                                     'recordSha256': hashlib.sha256(captured).hexdigest()})
            with tempfile.TemporaryDirectory(prefix='cw-lock-', dir='/tmp') as temporary:
                state = Path(temporary)
                authority.select(writer)
                authority.admit(state)
                with NativeProcess(writer, state, fixture=False) as process:
                    process.ready(fixture=False)
                    namespace = state / 'session-deletions'
                    (namespace / '.lock').rename(namespace / 'retained-lock')
                    (namespace / '.lock').write_bytes(b'replacement evidence')
                    peer, welcome = connect(process, 'core')
                    with peer:
                        ipc.require(welcome['type'] == 'welcome', 'core welcome missing')
                        ipc.stop(peer)
                        outcome = ipc.ack(peer, False)
                        ipc.require('lock was replaced' in outcome['reason'], 'wrong lock failure')
                    ipc.require(sorted(path.name for path in namespace.iterdir()) == ['.lock', 'retained-lock'],
                                'lock replacement refusal changed evidence')
                observations.append({'case': 'lock-replacement', 'revision': writer['source']['revision'], 'negativeAck': True})
            with tempfile.TemporaryDirectory(prefix='cw-storage-', dir='/tmp') as temporary:
                state = Path(temporary)
                authority.select(writer)
                authority.admit(state)
                with NativeProcess(writer, state, fixture=False) as process:
                    process.ready(fixture=False)
                    worker, welcome = connect(process, 'worker')
                    with worker:
                        ipc.require(welcome['type'] == 'welcome', 'worker welcome missing')
                        peer, welcome = connect(process, 'core')
                        with peer:
                            ipc.require(welcome['type'] == 'welcome', 'core welcome missing')
                            ipc.require(ipc.receive(worker)['type'] == 'replay', 'initial replay missing')
                            (state / 'session-deletions/deletions.json').mkdir()
                            ipc.stop(peer)
                            ipc.require('durable Session deletion was not confirmed' in ipc.ack(peer, False)['reason'],
                                        'wrong storage refusal')
                        worker.settimeout(0.2)
                        try:
                            worker.recv(1)
                            raise RuntimeError('failed commit changed surviving worker')
                        except socket.timeout:
                            pass
                        peer, outcome = connect(process, 'worker')
                        with peer:
                            ipc.require(outcome['type'] == 'reject' and 'fenced after a storage failure' in outcome['reason'],
                                        'storage failure did not poison admission')
                for reopener in (old, new, reader):
                    authority.select(reopener)
                    with NativeProcess(reopener, state, fixture=False) as rejected:
                        rejected.refused('not a regular file')
                observations.append({'case': 'storage-failure', 'revision': writer['source']['revision'],
                                     'negativeAck': True, 'survivingWorkerUntouched': True, 'furtherAdmissionFenced': True})
        conditions = ('absent', 'corrupt', 'duplicate', 'unknown', 'trailing', 'foreign-machine',
                      'foreign-dataset', 'reader', 'release', 'revision', 'directory', 'symlink',
                      'fifo', 'oversized', 'mutable', 'wrong-uid', 'profile-owner', 'profile-parent',
                      'untrusted-link', 'other-writer', 'reader-selected')
        for condition in conditions:
            with tempfile.TemporaryDirectory(prefix='cw-refusal-', dir='/tmp') as temporary:
                state = Path(temporary)
                authority.select(new)
                authority.admit(state)
                floor = json.loads(authority.floor.read_text())
                if condition == 'absent':
                    authority.floor.unlink()
                elif condition in ('corrupt', 'duplicate', 'unknown', 'trailing', 'oversized'):
                    text = authority.floor.read_text()
                    if condition == 'corrupt': text = '{}'
                    if condition == 'duplicate': text = text[:-1] + ',"schema":1}'
                    if condition == 'unknown': text = text[:-1] + ',"writer":true}'
                    if condition == 'trailing': text += ' {}'
                    if condition == 'oversized': text = ' ' * 8193
                    authority.floor.write_text(text)
                elif condition in ('foreign-machine', 'foreign-dataset', 'reader', 'release', 'revision'):
                    field, value = {'foreign-machine': ('machine', 'foreign'),
                                    'foreign-dataset': ('dataset', str(state / 'other')),
                                    'reader': ('readerSchema', 0), 'release': ('release', '/nix/store/root/subdir'),
                                    'revision': ('revision', 'invalid')}[condition]
                    floor[field] = value
                    authority.floor.write_text(json.dumps(floor))
                elif condition in ('directory', 'symlink', 'fifo'):
                    authority.floor.unlink()
                    if condition == 'directory': authority.floor.mkdir()
                    if condition == 'symlink': authority.floor.symlink_to(state / 'absent')
                    if condition == 'fifo': os.mkfifo(authority.floor)
                elif condition == 'mutable': authority.floor.chmod(0o666)
                elif condition == 'wrong-uid': os.chown(authority.floor, 1000, 1000)
                elif condition == 'profile-owner': os.lchown(authority.profile, 1000, 1000)
                elif condition == 'profile-parent': authority.profiles.chmod(0o777)
                elif condition == 'untrusted-link':
                    outside = state / 'selection'
                    outside.symlink_to(new['release'])
                    authority.profile.unlink()
                    authority.profile.symlink_to(outside)
                elif condition == 'other-writer': authority.select(old)
                elif condition == 'reader-selected': authority.select(reader)
                try:
                    refuse(new, state)
                finally:
                    authority.profiles.chmod(0o755)
                    if authority.floor.is_dir(): authority.floor.rmdir()
                    else: authority.floor.unlink(missing_ok=True)
                observations.append({'case': 'authority-' + condition, 'beforeJournalOpen': True})
        # A forged source declaration cannot make the selected reader's different
        # native executable authorize this writer, even with the same revision.
        with tempfile.TemporaryDirectory(prefix='cw-native-', dir='/tmp') as temporary:
            state = Path(temporary)
            authority.select(reader)
            authority.admit(state)
            with masked_source(reader, json.dumps(new['source']).encode()):
                refuse(new, state, 'native executable is not selected')
            observations.append({'case': 'selected-native-mismatch', 'beforeJournalOpen': True})
        for condition in ('unknown', 'duplicate', 'oversized'):
            with tempfile.TemporaryDirectory(prefix='cw-source-', dir='/tmp') as temporary:
                state = Path(temporary)
                authority.select(new)
                authority.admit(state)
                data = json.dumps(new['source'])
                if condition == 'unknown': data = data[:-1] + ',"enableWriter":true}'
                if condition == 'duplicate': data = data[:-1] + ',"schema":1}'
                if condition == 'oversized': data = ' ' * 8193
                with masked_source(new, data.encode()):
                    refuse(new, state)
                observations.append({'case': 'source-' + condition, 'beforeJournalOpen': True})
    receipt = {'schema': 1, 'accepted': True, 'oldWriter': old, 'newWriter': new,
               'readerOnlyFallback': reader, 'defaultReader': default_reader, 'observations': observations,
               'privateRootMountPidNetworkNamespaces': True, 'hostProductionActivation': False,
               'productionCheckpointHooks': False, 'powerLossAcceptance': False,
               'scope': 'exact production writer release binaries with synthetic root selection/floor and private broker IPC; startup authority, ACK/SIGKILL/reopen, failure and reader-only fallback'}
    Path(args.receipt).write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'accepted': True, 'cases': len(observations), 'receipt': args.receipt}))


if __name__ == '__main__':
    main()
