"""Build the independent experiment from the repository root in nix develop.

Arguments: scratch directory, pinned Rust-with-WASM prefix, wasm-bindgen, clang.
Only scratch dependency sources are patched. The repository lockfile pins all
other dependencies; --locked prevents accidental dependency updates.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import urllib.request

root = Path(__file__).resolve().parent
scratch, rust, bindgen, clang = map(lambda p: Path(p).resolve(), sys.argv[1:])
assert len(sys.argv) == 5
scratch.mkdir(parents=True, exist_ok=True)
source = scratch/'gotatun-0.9.2'
checksum = '4578da6dc5281756ef0277a8ab92fd390b472b87b939c89252130be9ca28812b'
patch = root/'gotatun-wasm.patch'
fingerprint = hashlib.sha256(patch.read_bytes()).hexdigest()
if not source.exists():
    url = 'https://static.crates.io/crates/gotatun/gotatun-0.9.2.crate'
    data = urllib.request.urlopen(url, timeout=60).read()
    assert hashlib.sha256(data).hexdigest() == checksum, 'upstream source digest mismatch'
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
        archive.extractall(scratch, filter='data')
    subprocess.run(['patch', '-p1', '--input', str(patch)], cwd=source, check=True)
    (source/'.probe-patch-sha256').write_text(fingerprint)
assert (source/'.probe-patch-sha256').read_text() == fingerprint, 'use fresh scratch after changing the patch'
assert scratch not in [root, root.parent], 'scratch must not replace source files'

env = dict(os.environ, RUSTC=str(rust/'bin/rustc'), CARGO_TARGET_DIR=str(scratch/'target'), CARGO_INCREMENTAL='0')
base = [str(rust/'bin/cargo'), 'build', '--locked', '--manifest-path', str(root/'Cargo.toml'),
        '--config', f'patch.crates-io.gotatun.path={json.dumps(str(source))}']
for name, args in [('server', ['--bin', 'wireguard-browser-server']),
                   ('browser', ['--lib', '--release', '--target', 'wasm32-unknown-unknown'])]:
    current = dict(env)
    if name == 'browser':
        current.update(CARGO_ENCODED_RUSTFLAGS='--cfg\x1fgetrandom_backend="wasm_js"', CC_wasm32_unknown_unknown=str(clang))
    log_path = scratch/f'{name}-build.log'
    with log_path.open('w') as log:
        result = subprocess.run(base+args, env=current, stdout=log, stderr=subprocess.STDOUT)
    print(f'{name}: exit {result.returncode}; log {log_path}', flush=True)
    if result.returncode:
        for line in log_path.read_text().splitlines()[-30:]:
            print(line[:600])
        raise SystemExit(result.returncode)
assets = scratch/'assets'
assets.mkdir(exist_ok=True)
subprocess.run([str(bindgen), '--target', 'web', '--out-dir', str(assets),
                str(scratch/'target/wasm32-unknown-unknown/release/cowboy_browser_wireguard_probe.wasm')], check=True)
for name in ['fixture.html', 'fixture.js', 'worker.js']:
    shutil.copy2(root/name, assets/name)
print(json.dumps({'assets': str(assets), 'server': str(scratch/'target/debug/wireguard-browser-server'),
                  'wasmBytes': (assets/'cowboy_browser_wireguard_probe_bg.wasm').stat().st_size}), flush=True)
