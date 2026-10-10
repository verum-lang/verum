from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time

root = Path('/Users/taaliman/.codex/worktrees/registry-metadata-lookup/verum')
out = Path('/private/tmp/verum-T1706-full-vbc')
out.mkdir(exist_ok=False)
expected = '3c99c5e9b02b10254eb7df20f37e4cd0455013a5'
sha256 = lambda data: hashlib.sha256(data).hexdigest()
git = lambda *args: subprocess.check_output(['git', *args], cwd=root, text=True).strip()
assert git('rev-parse', 'HEAD') == expected
assert not git('status', '--porcelain')
settings = {
    'CARGO_TARGET_DIR': '/Users/taaliman/.tmp/verum-codex-01a10248-registry-client/target',
    'CARGO_INCREMENTAL': '0', 'CARGO_BUILD_JOBS': '2',
    'VERUM_NO_AUTO_PRECOMPILE': '1',
    'VERUM_LLVM_DIR': '/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install',
    'RUST_MIN_STACK': '16777216',
}
environment = os.environ.copy()
environment.update(settings)
command = ['cargo', 'test', '--locked', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '-p', 'verum_vbc', '--lib', '--no-default-features', '--features', 'compression,table_dispatch,codegen,ffi']
paths = git('ls-files', 'Cargo.toml', 'Cargo.lock', 'crates/verum_vbc', 'crates/verum_codegen', 'crates/verum_compiler/build.rs').splitlines()
artifacts = Path(settings['CARGO_TARGET_DIR']) / 'precompiled-stdlib'
artifact_names = ['runtime.vbca', 'runtime.core_metadata', 'runtime.symbol_graph', 'runtime.vbca.checksum', 'runtime.vbca.schema']
artifact_hashes = lambda: {name: sha256((artifacts / name).read_bytes()) for name in artifact_names}
source_hashes = lambda: {name: sha256((root / name).read_bytes()) for name in paths}
record = {
    'task': 'T1706', 'source_commit': expected, 'command': command,
    'environment': settings, 'timeout_seconds': 1200,
    'runner_sha256': sha256(Path(__file__).read_bytes()),
    'source_paths': source_hashes(), 'artifacts_before': artifact_hashes(),
    'idle_selection_sha256': sha256(Path('/private/tmp/verum-T1706-full-vbc-idle/selection.json').read_bytes()),
    'scope': 'One unfiltered VBC library gate with compression,table_dispatch,codegen,ffi; no retries. This does not run the separate T1710 integration control or any native lowering, ordinary CLI, automatic stdlib bake, AOT or registry acceptance. Inherited artifacts are held unchanged.',
}
(out / 'started.json').write_text(json.dumps(record, indent=2) + '\n')
failure = None
start = time.monotonic()
with (out / 'gate.log').open('w') as log:
    process = subprocess.Popen(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    (out / 'process.json').write_text(json.dumps({'pid': process.pid, 'runner_pid': os.getpid()}) + '\n')
    while process.poll() is None:
        time.sleep(1)
        content = (out / 'gate.log').read_text(errors='replace')
        if 'Compiling z3-sys ' in content:
            failure = 'refused fresh solver build'
        elif any(text in content for text in ['Compiling verum_codegen ', 'Compiling verum_stdlib_precompiler ', 'Compiling verum_llvm ']):
            failure = 'refused native or stdlib producer build'
        elif time.monotonic() - start > 1200:
            failure = 'controlled timeout'
        if failure:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            break
content = (out / 'gate.log').read_text(errors='replace')
executables = {}
for match in re.finditer(r'Running [^\n]* \(([^)]+)\)', content):
    path = Path(match.group(1))
    assert path.is_file(), path
    digest = sha256(path.read_bytes())
    retained = out / path.name
    shutil.copy2(path, retained)
    assert sha256(retained.read_bytes()) == digest
    executables[str(path)] = {'sha256': digest, 'retained': str(retained), 'retained_sha256': sha256(retained.read_bytes())}
record.update({
    'exit_code': process.returncode,
    'wall_seconds': round(time.monotonic() - start, 3),
    'failure': failure,
    'source_unchanged': git('rev-parse', 'HEAD') == expected and not git('status', '--porcelain'),
    'source_paths_unchanged': record['source_paths'] == source_hashes(),
    'artifacts_after': artifact_hashes(),
    'executables': executables,
    'log_sha256': sha256((out / 'gate.log').read_bytes()),
    'test_results': re.findall(r'test result: [^\n]+', content),
    'individual_failures': re.findall(r'^test (.*?) \.\.\. FAILED$', content, re.M),
})
record['completed_unfiltered'] = any('0 filtered out;' in result for result in record['test_results'])
record['positive_count_matches'] = any(re.search(r'test result: ok\. [1-9][0-9]{3,} passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;', result) for result in record['test_results'])
(out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps({key: record[key] for key in ['source_commit', 'exit_code', 'wall_seconds', 'failure', 'source_unchanged', 'source_paths_unchanged', 'test_results', 'completed_unfiltered', 'individual_failures']}))
print(content[-6500:])
if failure or process.returncode or not record['positive_count_matches'] or not record['source_unchanged'] or not record['source_paths_unchanged'] or record['artifacts_before'] != record['artifacts_after']:
    sys.exit(process.returncode if process.returncode and process.returncode > 0 else 1)
