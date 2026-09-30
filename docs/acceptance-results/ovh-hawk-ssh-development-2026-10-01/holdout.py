"""Independent behavioral acceptance, supplied only after the agent commits."""
from concurrent.futures import ThreadPoolExecutor
import csv
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

CLI = Path('/workspace/taskboard-acceptance/taskboard.py')


def invoke(db, *args):
    return subprocess.run([sys.executable, str(CLI), '--db', str(db), *args],
                          capture_output=True, text=True, timeout=40)


with tempfile.TemporaryDirectory() as directory:
    db = Path(directory) / 'tasks.json'
    rows = [dict(id=i, title='中文, "quoted"\nline-' + str(i), done=i % 2 == 0,
                 tags=['work', 'alpha']) for i in range(1, 101)]
    db.write_text(json.dumps(dict(next_id=101, tasks=list(reversed(rows))), ensure_ascii=False))
    db.chmod(0o600)
    original = db.read_bytes()
    exported = invoke(db, 'export', '--format', 'csv', '--tag', 'work')
    assert exported.returncode == 0, exported.stderr
    parsed = list(csv.DictReader(io.StringIO(exported.stdout)))
    assert len(parsed) == 100
    for actual, expected in zip(parsed, rows):
        assert actual == dict(id=str(expected['id']), title=expected['title'],
                              done=str(expected['done']).lower(), tags='alpha;work')
    assert db.read_bytes() == original
    before_files = set(Path(directory).iterdir())
    Path(directory).chmod(0o500)
    try:
        readonly = invoke(db, 'export', '--format', 'csv')
        assert readonly.returncode == 0, readonly.stderr
        assert set(Path(directory).iterdir()) == before_files
    finally:
        Path(directory).chmod(0o700)
    print('holdout: 100 CSV records with Unicode, quotes, commas and newlines passed', flush=True)
    if '--feature-only' not in sys.argv:
        stopped = threading.Event()
        read_failures = []
        reads = [0]

        def reader():
            while not stopped.wait(0.001):
                try:
                    state = json.loads(db.read_bytes())
                    assert state['next_id'] == len(state['tasks']) + 1
                    reads[0] += 1
                except Exception as error:
                    read_failures.append(type(error).__name__)

        observer = threading.Thread(target=reader)
        observer.start()
        try:
            with ThreadPoolExecutor(max_workers=12) as pool:
                results = list(pool.map(lambda i: invoke(db, 'add', 'parallel-' + str(i)), range(144)))
        finally:
            stopped.set()
            observer.join()
        assert all(result.returncode == 0 for result in results), 'a writer failed'
        assert not read_failures, read_failures[:5]
        state = json.loads(db.read_bytes())
        assert len(state['tasks']) == 244
        assert sorted(row['id'] for row in state['tasks']) == list(range(1, 245))
        assert {row['title'] for row in state['tasks'] if row['id'] > 100} == {
            'parallel-' + str(i) for i in range(144)}
        assert db.stat().st_mode & 0o777 == 0o600
        print('holdout: 144 concurrent writes and', reads[0], 'complete JSON reads passed', flush=True)
        db.chmod(0o640)
        assert invoke(db, 'add', 'preserve custom permissions').returncode == 0
        assert db.stat().st_mode & 0o777 == 0o640
        seed = dict(next_id=1001, tasks=[dict(id=i, title='x' * 3000, done=False, tags=[])
                                       for i in range(1, 1001)])
        for attempt in range(12):
            db.write_text(json.dumps(seed))
            child = subprocess.Popen([sys.executable, str(CLI), '--db', str(db), 'add', 'interrupted'],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(0.04 + (attempt % 4) * 0.015)
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=10)
            saved = json.loads(db.read_bytes())
            assert saved['tasks'][:1000] == seed['tasks']
            assert len(saved['tasks']) in (1000, 1001)
            assert saved['next_id'] == len(saved['tasks']) + 1
        print('holdout: custom permissions and 12 interrupted-writer JSON checks passed', flush=True)
        db.write_bytes(b'broken existing input\n')
        broken = invoke(db, 'add', 'must not overwrite')
        assert broken.returncode != 0 and db.read_bytes() == b'broken existing input\n'
        print('holdout: corrupt input preserved and rejected', flush=True)
