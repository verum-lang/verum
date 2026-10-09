import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--build-dir', required=True, type=Path)
p.add_argument('--project-dir', required=True, type=Path)
p.add_argument('--evidence-dir', required=True, type=Path)
p.add_argument('--task', default='T1635')
p.add_argument('--timeout', type=int, default=600)
args = p.parse_args()
assert 0 < args.timeout <= 600
build_path = args.build_dir / 'build-manifest.json'
build = json.loads(build_path.read_text())
cli = Path(build['cli']['path'])
root = args.project_dir
def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as handle:
        for block in iter(lambda: handle.read(1048576), b''):
            h.update(block)
    return h.hexdigest()
def git(*parts):
    return subprocess.check_output(['git', '-C', str(root), *parts], text=True).strip()
def inputs():
    names = set(git('ls-files', '*.vr', '*.toml').splitlines())
    names.update(str(path.relative_to(root)) for path in (root / 'src').rglob('*.vr'))
    return {name: sha(root / name) for name in sorted(names)}
assert sha(cli) == build['cli']['sha256']
assert cli.stat().st_size == build['cli']['bytes']
assert not git('status', '--porcelain'), 'project must be committed and clean'
record = {'task': args.task, 'registry_commit': git('rev-parse', 'HEAD'), 'registry_tree': git('rev-parse', 'HEAD^{tree}'), 'language_product_commit': build['engine_commit'], 'cli_sha256': build['cli']['sha256'], 'cli_bytes': build['cli']['bytes'], 'schema': build['schema'], 'build_manifest_sha256': sha(build_path), 'source_fingerprint_blake3': build['source_fingerprint_blake3'], 'command': ['verum', 'check'], 'working_directory': 'registry repository root', 'deadline_seconds': args.timeout, 'project_inputs': inputs(), 'environment_overrides': {'TMPDIR': '/private/tmp', 'VERUM_NO_OBJECT_CACHE': '1'}}
args.evidence_dir.mkdir(exist_ok=False)
log = args.evidence_dir / 'check.log'
start = time.monotonic()
with log.open('xb') as output:
    process = subprocess.Popen([str(cli), 'check'], cwd=root, env={**os.environ, **record['environment_overrides']}, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    (args.evidence_dir / 'started.json').write_text(json.dumps({**record, 'pid': process.pid}, indent=2) + '\n')
    timed_out = False
    try:
        code = process.wait(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            code = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
unchanged = git('rev-parse', 'HEAD') == record['registry_commit'] and not git('status', '--porcelain') and inputs() == record['project_inputs']
cli_unchanged = sha(cli) == record['cli_sha256']
status = 'timeout_without_verdict' if timed_out else ('passed' if code == 0 else 'failed')
if not unchanged or not cli_unchanged:
    status = 'invalid_changed_inputs'
record.update(elapsed_seconds=round(time.monotonic() - start, 3), process_returncode=code, status=status, source_unchanged=unchanged, cli_unchanged=cli_unchanged, log_sha256=sha(log), scope='Ordinary argument-less project check. Runtime, proofs, registry publication/install, generated AOT and deployment require their own acceptance.')
(args.evidence_dir / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record), flush=True)
raise SystemExit(124 if timed_out else (code if unchanged and cli_unchanged else 1))
