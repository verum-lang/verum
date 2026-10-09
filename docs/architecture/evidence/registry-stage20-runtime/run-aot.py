"""Run one immutable fixture through the ordinary AOT CLI and retain its binary."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--build-dir', type=Path, required=True)
p.add_argument('--evidence-dir', type=Path, required=True)
p.add_argument('--fixture', type=Path, required=True)
p.add_argument('--expected', required=True)
p.add_argument('--timeout', type=int, default=600)
a = p.parse_args()
assert 0 < a.timeout <= 600

def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1048576), b''):
            h.update(block)
    return h.hexdigest()

build_path = a.build_dir / 'build-manifest.json'
build = json.loads(build_path.read_text())
cli = Path(build['cli']['path'])
assert sha(cli) == build['cli']['sha256']
a.evidence_dir.mkdir(exist_ok=False)
fixture = a.evidence_dir / a.fixture.name
fixture.write_bytes(a.fixture.read_bytes())
cwd = a.evidence_dir / 'working-directory'
cwd.mkdir()
overrides = {'NO_COLOR': '1', 'VERUM_NO_OBJECT_CACHE': '1', 'TMPDIR': '/private/tmp'}
command = [str(cli), 'run', '--tier', 'aot', str(fixture)]
r = {'language_commit': build['engine_commit'], 'build_manifest_sha256': sha(build_path),
     'cli_sha256': sha(cli), 'fixture_sha256': sha(a.fixture),
     'original_fixture': str(a.fixture), 'executed_fixture': str(fixture),
     'command': command, 'working_directory': str(cwd),
     'expected_stdout': a.expected, 'deadline_seconds': a.timeout,
     'environment_overrides': overrides,
     'scope': 'One ordinary standalone AOT fixture. Import inspection does not prove absence of statically linked libc or platform-wide conformance.'}
start = time.monotonic()
timeout = False
with (a.evidence_dir / 'stdout.log').open('xb') as stdout, (a.evidence_dir / 'stderr.log').open('xb') as stderr:
    process = subprocess.Popen(command, cwd=cwd, env={**os.environ, **overrides},
                               stdout=stdout, stderr=stderr, start_new_session=True)
    (a.evidence_dir / 'started.json').write_text(json.dumps({**r, 'pid': process.pid}, indent=2) + '\n')
    try:
        code = process.wait(timeout=a.timeout)
    except subprocess.TimeoutExpired:
        timeout = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            code = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
r.update(elapsed_seconds=round(time.monotonic() - start, 3), returncode=code, timed_out=timeout)
binary = a.evidence_dir / 'target/debug' / fixture.stem
r['native_artifact'] = None
if binary.is_file():
    details = {'path': str(binary), 'bytes': binary.stat().st_size, 'sha256': sha(binary)}
    for label, command in [('file', ['file', str(binary)]),
                           ('dynamic-libraries', ['otool', '-L', str(binary)]),
                           ('undefined-symbols', ['nm', '-u', str(binary)])]:
        with (a.evidence_dir / (label + '.log')).open('xb') as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=30)
        details[label + '_exit_code'] = result.returncode
    r['native_artifact'] = details
r['checks'] = {
    'exit_zero': code == 0,
    'within_deadline': not timeout,
    'exact_stdout': (a.evidence_dir / 'stdout.log').read_text() == a.expected,
    'source_unchanged': sha(a.fixture) == r['fixture_sha256'] and sha(fixture) == r['fixture_sha256'],
    'executable_unchanged': sha(cli) == r['cli_sha256'],
    'native_artifact_retained': r['native_artifact'] is not None,
}
r['status'] = 'passed' if all(r['checks'].values()) else 'timeout_without_verdict' if timeout else 'failed'
r['logs'] = {x.name: sha(x) for x in sorted(a.evidence_dir.glob('*.log'))}
(a.evidence_dir / 'result.json').write_text(json.dumps(r, indent=2) + '\n')
print(json.dumps({k: r[k] for k in ['status', 'returncode', 'elapsed_seconds', 'checks', 'native_artifact']}), flush=True)
raise SystemExit(0 if r['status'] == 'passed' else 1)
