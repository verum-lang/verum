"""Build and identify one frozen ordinary CLI with automatic stdlib baking."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--revision', required=True)
parser.add_argument('--evidence-dir', type=Path, required=True)
args = parser.parse_args()
root = Path('/Users/taaliman/.codex/worktrees/returned-reference-summary/verum')
target = Path('/Users/taaliman/.tmp/verum-codex-01a10248/native-target')
assert len(args.revision) == 40 and all(c in '0123456789abcdef' for c in args.revision)
assert not {'VERUM_NO_AUTO_PRECOMPILE', 'DOCS_RS'} & os.environ.keys(), 'ordinary build must enable automatic bake'
def git(*parts):
    return subprocess.check_output(['git', '-C', str(root), *parts], text=True).strip()
def info(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1048576), b''):
            h.update(block)
    return {'bytes': path.stat().st_size, 'sha256': h.hexdigest()}
assert git('rev-parse', 'HEAD') == args.revision
assert not git('status', '--porcelain'), 'source must be committed and clean'
assert shutil.disk_usage(target).free > 12 * 1024**3, 'insufficient build space'
args.evidence_dir.mkdir(exist_ok=False)
command = ['cargo', 'build', '--locked', '--offline', '-p', 'verum_cli', '--bin', 'verum']
overrides = {'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '2', 'VERUM_LLVM_DIR': '/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install'}
record = {'engine_commit': args.revision, 'engine_tree': git('rev-parse', 'HEAD^{tree}'), 'utc_started': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'command': command, 'environment_overrides': overrides, 'automatic_precompile': True, 'deadline_seconds': 3600}
log = args.evidence_dir / 'cli-build.log'
started = time.monotonic()
with log.open('xb') as output:
    process = subprocess.Popen(command, cwd=root, env={**os.environ, **overrides}, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    (args.evidence_dir / 'started.json').write_text(json.dumps({**record, 'pid': process.pid}, indent=2) + '\n')
    timeout = False
    try:
        code = process.wait(timeout=record['deadline_seconds'])
    except subprocess.TimeoutExpired:
        timeout = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            code = process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
record.update(seconds=round(time.monotonic() - started, 3), exit_code=code, timed_out=timeout, log_sha256=info(log)['sha256'], source_unchanged=git('rev-parse', 'HEAD') == args.revision and not git('status', '--porcelain'))
if code or timeout or not record['source_unchanged']:
    (args.evidence_dir / 'build-failure.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record), flush=True)
    raise SystemExit(code if code else 1)
cli = args.evidence_dir / ('verum-' + args.revision[:9])
shutil.copy2(target / 'debug/verum', cli)
cli.chmod(0o555)
record['cli'] = {'path': str(cli), **info(cli)}
archive = target / 'precompiled-stdlib'
record['schema'] = (archive / 'runtime.vbca.schema').read_text().strip()
record['source_fingerprint_blake3'] = (archive / 'runtime.vbca.checksum').read_text().strip()
snapshot = args.evidence_dir / 'artifacts'
snapshot.mkdir()
record['artifacts'] = {}
for name in ['runtime.vbca', 'runtime.core_metadata', 'runtime.symbol_graph']:
    source = archive / name
    expected = info(source)
    shutil.copy2(source, snapshot / name)
    assert info(snapshot / name) == expected
    record['artifacts'][name] = expected
record['artifact_snapshot'] = str(snapshot)
(args.evidence_dir / 'build-manifest.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record), flush=True)
