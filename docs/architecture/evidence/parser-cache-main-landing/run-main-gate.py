from pathlib import Path
import argparse, hashlib, json, os, re, signal, subprocess, time

p = argparse.ArgumentParser()
p.add_argument('--commit', required=True)
p.add_argument('--kind', choices=['alias', 'compiler'], required=True)
p.add_argument('--output', required=True)
a = p.parse_args()
root = Path('/Users/taaliman/.codex/worktrees/configured-registry-downloads/verum')
target = Path('/Users/taaliman/.tmp/verum-codex-01a10248-types/checker-target' if a.kind == 'alias' else '/Users/taaliman/.tmp/verum-codex-01a10248/native-target')
out = Path(a.output)
out.mkdir(exist_ok=False)
sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
def git(*values):
    return subprocess.run(['git', '-C', str(root), *values], check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()
expected = git('rev-parse', a.commit)
assert git('rev-parse', 'HEAD') == expected and not git('status', '--porcelain')
selected = {'generic_alias_parameter_classification': 12} if a.kind == 'alias' else {'stdlib_cache_identity': 7, 'stdlib_cache_alias_archive': 1}
paths = sorted(name for name in git('ls-files', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'core', 'grammar', 'crates', '.github/workflows/ci.yml').splitlines() if name in ['Cargo.lock', '.github/workflows/ci.yml'] or name.endswith(('.rs', '.toml', '.ebnf')) or (name.startswith('core/') and name.endswith('.vr')))
def sources(): return {name: sha(root/name) for name in paths}
artifact_root = target/'precompiled-stdlib'
artifact_names = ['runtime.vbca', 'runtime.core_metadata', 'runtime.symbol_graph', 'runtime.vbca.checksum', 'runtime.vbca.schema']
def artifacts(): return {name: {'bytes': (artifact_root/name).stat().st_size, 'sha256': sha(artifact_root/name)} for name in artifact_names if (artifact_root/name).is_file()}
origin_path = Path('/private/tmp/verum-registry-stage22-field-authority/build-manifest.json')
origin = json.loads(origin_path.read_text())
if a.kind == 'compiler':
    assert origin['status'] == 'passed' and origin['engine_commit'] == 'e86f2abd77e041e2f29cdcbf30d9629435e64f7e'
    assert origin['schema'] == 'v54-2026-10-10-declared-field-visibility'
    assert artifacts() == origin['artifacts']
settings = {'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '1', 'CARGO_INCREMENTAL': '0', 'VERUM_NO_AUTO_PRECOMPILE': '1', 'RUST_MIN_STACK': '16777216'}
if a.kind == 'compiler': settings['VERUM_LLVM_DIR'] = '/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install'
environment = os.environ.copy()
for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN']: assert not environment.get(name), name
for name in ['DOCS_RS', 'VERUM_ALLOW_STALE_STDLIB', 'RUST_TEST_THREADS', 'VERUM_LLVM_DIR']: environment.pop(name, None)
environment.update(settings)
command = ['cargo', '+nightly-2026-08-20', 'test', '-vv', '--offline', '--locked', '--no-fail-fast', '--message-format=json-render-diagnostics', '--manifest-path', str(root/'Cargo.toml'), '-p', 'verum_types' if a.kind == 'alias' else 'verum_compiler']
for name in selected: command.extend(['--test', name])
command.extend(['--', '--test-threads=1', '--nocapture'])
record = {'tasks': ['T1720', 'T1728'], 'kind': a.kind, 'source_commit': expected, 'source_tree': git('rev-parse', 'HEAD^{tree}'), 'base_main': 'd4f68f3d8745e895d1663231b5fac5a720e05581', 'command': command, 'environment': settings, 'deadline_seconds': 600 if a.kind == 'alias' else 900, 'runner_sha256': sha(__file__), 'source_paths': sources(), 'artifacts_before': artifacts(), 'selected_targets': selected, 'scope': 'Exact main-based parser/cache source; no ordinary CLI, automatic stdlib bake, AOT, or registry acceptance. The archive control emits and reads a new minimal fixture archive via compile_core; inherited complete stdlib files are preserved and not newly produced.'}
if a.kind == 'compiler': record.update(artifact_origin_receipt=str(origin_path), artifact_origin_receipt_sha256=sha(origin_path), artifact_origin_source=origin['engine_commit'], artifact_schema=origin['schema'])
(out/'started.json').write_text(json.dumps(record, indent=2)+'\n')
process = None
start = time.monotonic()
interrupted = None
harness_errors = []
def stop():
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try: process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)
try:
    with (out/'gate.log').open('w') as log:
        process = subprocess.Popen(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        (out/'process.json').write_text(json.dumps({'runner_pid': os.getpid(), 'pid': process.pid})+'\n')
        while process.poll() is None:
            time.sleep(1)
            content = (out/'gate.log').read_text(errors='replace')
            if re.search(r'Compiling (?:z3-sys|cvc5-sys|verum_stdlib_precompiler) ', content): interrupted = 'refused solver or nested stdlib producer build'
            elif any(token in content for token in ['Refreshing stdlib precompile artefacts', 'Building LLVM from source', 'Building MLIR from source', 'cmake --build', 'ninja -C']): interrupted = 'refused automatic bake or native dependency bootstrap'
            elif time.monotonic()-start > record['deadline_seconds']: interrupted = 'bounded deadline'
            if interrupted:
                stop()
                break
except BaseException as error:
    harness_errors.append(repr(error))
    stop()
content = (out/'gate.log').read_text(errors='replace') if (out/'gate.log').exists() else ''
executables = {}
for line in content.splitlines():
    if not line.startswith('{'): continue
    try: event = json.loads(line)
    except json.JSONDecodeError: continue
    name = event.get('target', {}).get('name')
    if event.get('reason') != 'compiler-artifact' or name not in selected or not event.get('executable'): continue
    path = Path(event['executable'])
    try:
        digest = sha(path)
        retained = out/path.name
        if not retained.exists(): subprocess.run(['cp', '-c', str(path), str(retained)], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert sha(retained) == digest
        executables[name] = {'path': str(path), 'sha256': digest, 'retained': str(retained), 'retained_sha256': sha(retained), 'fresh': event.get('fresh')}
    except BaseException as error: harness_errors.append('retaining '+str(path)+': '+repr(error))
record.update(exit_code=process.returncode if process else None, wall_seconds=round(time.monotonic()-start, 3), interrupted=interrupted, harness_errors=harness_errors, executables=executables, source_unchanged=git('rev-parse','HEAD')==expected and not git('status','--porcelain'), source_paths_unchanged=record['source_paths']==sources(), artifacts_after=artifacts(), log_sha256=sha(out/'gate.log'), test_results=re.findall(r'^test result: [^\n]+',content,re.M), announced_test_counts=[int(n) for n in re.findall(r'^running ([0-9]+) tests?$',content,re.M)], individual_failures=re.findall(r'^test (.*?) \.\.\. FAILED$',content,re.M), cargo_compilations=re.findall(r'^\s*Compiling ([^\n]+)',content,re.M))
record['artifacts_unchanged'] = record['artifacts_before'] == record['artifacts_after']
record['selected_targets_complete'] = set(executables)==set(selected) and len(record['test_results'])==len(selected) and sorted(record['announced_test_counts'])==sorted(selected.values())
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({key:record[key] for key in ['source_commit','kind','exit_code','wall_seconds','interrupted','harness_errors','test_results','source_unchanged','source_paths_unchanged','artifacts_unchanged','selected_targets_complete']}))
print(content[-2500:])
