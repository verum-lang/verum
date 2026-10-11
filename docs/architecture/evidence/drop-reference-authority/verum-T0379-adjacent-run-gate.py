from pathlib import Path
import argparse, hashlib, json, os, re, shutil, signal, subprocess, time

p = argparse.ArgumentParser()
p.add_argument('--commit', required=True)
p.add_argument('--count', type=int, required=True)
p.add_argument('--output', required=True)
a = p.parse_args()
root = Path('/Users/taaliman/.codex/worktrees/configured-registry-downloads/verum')
target = Path('/Users/taaliman/.tmp/verum-codex-01a10248-types/checker-target')
targets = {'shared_method_receiver': 6, 'reference_method_receiver': 7}
out = Path(a.output)
out.mkdir(exist_ok=False)

def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()

def git(*values):
    return subprocess.run(['git', '-C', str(root), *values], check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()

expected = git('rev-parse', a.commit)
assert git('rev-parse', 'HEAD') == expected and not git('status', '--porcelain')
paths = sorted(name for name in git('ls-files', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
                                   '.cargo', 'core', 'grammar', 'crates', '.github/workflows/ci.yml').splitlines()
               if name in ['Cargo.lock', '.github/workflows/ci.yml']
               or name.endswith(('.rs', '.toml', '.ebnf'))
               or (name.startswith('core/') and name.endswith('.vr')))
def sources():
    return {name: sha(root/name) for name in paths}

artifact_root = Path('/private/tmp/verum-registry-stage23-canonical-owners/artifacts')
artifact_names = ['runtime.vbca', 'runtime.core_metadata', 'runtime.symbol_graph',
                  'runtime.vbca.checksum', 'runtime.vbca.schema']
def artifacts():
    return {name: {'bytes': (artifact_root/name).stat().st_size, 'sha256': sha(artifact_root/name)}
            for name in artifact_names}

origin_path = Path('/private/tmp/verum-registry-stage23-canonical-owners/build-manifest.json')
origin = json.loads(origin_path.read_text())
assert origin['status'] == 'passed' and origin['engine_commit'] == '7a8c86f90a493a3b7352359314fc5f0d628aaa1b'
assert origin['schema'] == 'v54-2026-10-10-declared-field-visibility'
assert sha(origin_path) == '3df266cba9190c9d201e03cb51404b28a1d1864f784d32e18421d6fb2083faef'
assert artifacts() == origin['artifacts']
assert shutil.disk_usage(target).free >= 6 * 1024**3, 'insufficient initial disk headroom'
settings = {'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '1', 'CARGO_INCREMENTAL': '0',
            'VERUM_NO_AUTO_PRECOMPILE': '1', 'RUST_MIN_STACK': '16777216'}
environment = os.environ.copy()
for name in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'CVC5_ROOT']:
    assert not environment.get(name), name
for name in ['DOCS_RS', 'VERUM_ALLOW_STALE_STDLIB', 'RUST_TEST_THREADS', 'VERUM_T1742_ARCHIVE_FIXTURE_DIR']:
    environment.pop(name, None)
environment.update(settings)
command = ['cargo', '+nightly-2026-08-20', 'test', '-vv', '--offline', '--locked',
           '--message-format=json-render-diagnostics', '--manifest-path', str(root/'Cargo.toml'),
           '-p', 'verum_vbc', *[argument for name in targets for argument in ('--test', name)], '--no-default-features', '--features', 'codegen,table_dispatch', '--', '--test-threads=1', '--nocapture']
record = {'task': 'T0379', 'source_commit': expected, 'source_tree': git('rev-parse', 'HEAD^{tree}'),
          'command': command, 'environment': settings, 'deadline_seconds': 600,
          'runner_sha256': sha(__file__), 'source_paths': sources(), 'artifacts_before': artifacts(),
          'selected_test_count': a.count, 'selected_targets': targets, 'artifact_origin_receipt': str(origin_path),
          'artifact_origin_receipt_sha256': sha(origin_path), 'artifact_origin_source': origin['engine_commit'], 'target_precompiled_directory_present': (target/'precompiled-stdlib').exists(),
          'artifact_schema': origin['schema'], 'disk_free_before': shutil.disk_usage(target).free,
          'scope': 'Existing Shared receiver codegen selection (six controls, not execution of actual stdlib Shared) plus parsed reference receiver interpreter controls (seven). No AOT acceptance. No embedded archive is consumed by this target. No ordinary CLI, automatic bake, AOT or native dependency bootstrap. Immutable Stage23 artifacts are independently pinned and must stay unchanged.', 'maximum_log_bytes': 16 * 1024**2}
(out/'started.json').write_text(json.dumps(record, indent=2)+'\n')
process = None
start = time.monotonic()
interrupted = None
harness_errors = []

def stop():
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)

try:
    with (out/'gate.log').open('w') as log:
        process = subprocess.Popen(command, cwd=root, env=environment, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        (out/'process.json').write_text(json.dumps({'runner_pid': os.getpid(), 'pid': process.pid})+'\n')
        while process.poll() is None:
            time.sleep(1)
            content = (out/'gate.log').read_text(errors='replace')
            if re.search(r'Compiling (?:z3-sys|z3-src|verum_stdlib_precompiler|verum_compiler|verum_codegen|verum_llvm_sys|verum_mlir) ', content):
                interrupted = 'refused solver or nested stdlib producer build'
            elif any('crates/cvc5-sys/build.rs' in line and any(
                    ('feature=\"'+feature+'\"') in line or ('CARGO_FEATURE_'+feature.upper()+'=') in line
                    for feature in ['vendored', 'static', 'system']) for line in content.splitlines()):
                interrupted = 'refused non-stub CVC5 build-script features'
            elif any(token in content for token in ['cvc5-sys: building CVC5 from source',
                    'cvc5-sys: using CVC5 source at', 'Refreshing stdlib precompile artefacts',
                    'Building LLVM from source', 'Building MLIR from source', 'cmake --build', 'ninja -C']):
                interrupted = 'refused automatic bake or native dependency bootstrap'
            elif shutil.disk_usage(target).free < 2 * 1024**3:
                interrupted = 'disk headroom fell below 2 GiB'
            elif (out/'gate.log').stat().st_size > record['maximum_log_bytes']:
                interrupted = '16 MiB log bound'
            elif time.monotonic() - start > record['deadline_seconds']:
                interrupted = 'bounded deadline'
            if interrupted:
                stop()
                break
except BaseException as error:
    harness_errors.append(repr(error))
    stop()

content = (out/'gate.log').read_text(errors='replace') if (out/'gate.log').exists() else ''
executables = {}
for line in content.splitlines():
    if not line.startswith('{'):
        continue
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if (event.get('reason') != 'compiler-artifact' or event.get('target', {}).get('name') not in targets
            or not event.get('profile', {}).get('test') or not event.get('executable')):
        continue
    path = Path(event['executable'])
    try:
        digest = sha(path)
        retained = out/path.name
        if not retained.exists():
            subprocess.run(['cp', '-c', str(path), str(retained)], check=True,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert sha(retained) == digest
        executables[event['target']['name']] = {'path': str(path), 'sha256': digest, 'retained': str(retained),
                                    'retained_sha256': sha(retained), 'fresh': event.get('fresh')}
    except BaseException as error:
        harness_errors.append('retaining '+str(path)+': '+repr(error))

record.update(exit_code=process.returncode if process is not None else None,
              wall_seconds=round(time.monotonic()-start, 3), interrupted=interrupted,
              harness_errors=harness_errors, executables=executables,
              source_unchanged=git('rev-parse', 'HEAD') == expected and not git('status', '--porcelain'),
              source_paths_unchanged=record['source_paths'] == sources(), artifacts_after=artifacts(),
              log_sha256=sha(out/'gate.log'), test_results=re.findall(r'^test result: [^\n]+', content, re.M),
              announced_test_counts=[int(n) for n in re.findall(r'^running ([0-9]+) tests?$', content, re.M)],
              individual_failures=re.findall(r'^test (.*?) \.\.\. FAILED$', content, re.M),
              cargo_compilations=re.findall(r'^\s*Compiling ([^\n]+)', content, re.M),
              disk_free_after=shutil.disk_usage(target).free)
record['artifacts_unchanged'] = record['artifacts_before'] == record['artifacts_after']
record['selected_tests_complete'] = (set(executables) == set(targets)
                                    and len(record['test_results']) == len(targets)
                                    and sorted(record['announced_test_counts']) == sorted(targets.values())
                                    and sum(record['announced_test_counts']) == a.count)
(out/'result.json').write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps({key: record[key] for key in ['source_commit', 'exit_code', 'wall_seconds',
    'interrupted', 'harness_errors', 'test_results', 'source_unchanged', 'source_paths_unchanged',
    'artifacts_unchanged', 'selected_tests_complete']}))
print(content[-5000:])

success = (record['exit_code'] == 0 and not interrupted and not harness_errors
           and all(record[key] for key in ('source_unchanged','source_paths_unchanged','artifacts_unchanged','selected_tests_complete'))
           and all(entry['fresh'] is False for entry in executables.values())
           and all(any(f'{count} passed; 0 failed; 0 ignored;' in result for result in record['test_results']) for count in targets.values()))
raise SystemExit(0 if success else 1)
