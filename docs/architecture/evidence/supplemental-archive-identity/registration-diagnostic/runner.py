#!/usr/bin/env python3
"""One bounded diagnostic execution; inserted markers are not an acceptance oracle."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path('/Users/taaliman/.codex/worktrees/configured-registry-downloads/verum')
COMMIT = '7a8c86f90a493a3b7352359314fc5f0d628aaa1b'
FIXTURE = 'core-tests/base/semver_constraint/contract_options_test.vr'
ORIGINAL = Path('/private/tmp/verum-stage23-semver-options')
DEST = Path('/private/tmp/verum-T1744-concat-registration')
HELPER = Path('/private/tmp/verum-registry-stage23-type-boundaries.py')
HELPER_SHA = 'a373e37f6f388b1ea753af2e78fde8545564cdac40c7f2c58b8fa83718a4feff'
MANIFEST = Path('/private/tmp/verum-registry-stage23-canonical-owners/build-manifest.json')
MANIFEST_SHA = '3df266cba9190c9d201e03cb51404b28a1d1864f784d32e18421d6fb2083faef'
CLI = Path('/private/tmp/verum-registry-stage23-canonical-owners/verum-7a8c86f90')
CLI_SHA = 'c13b23c1aa15c9d85a29675e74c3623ff5315eb5c1d139a61e6686671364313d'
SCHEMA = 'v54-2026-10-10-declared-field-visibility'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    assert sha(HELPER.read_bytes()) == HELPER_SHA
    spec = importlib.util.spec_from_file_location('stage23_identity', HELPER)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    original_result = json.loads((ORIGINAL / 'result.json').read_text())
    source_hashes = original_result['source_inputs']
    source = helper.git(ROOT, 'show', f'{COMMIT}:{FIXTURE}')
    assert source == (ORIGINAL / 'inputs/options.vr').read_bytes()
    assert sha(source) == source_hashes[FIXTURE]
    assert helper.git(ROOT, 'rev-parse', 'HEAD').decode().strip() == COMMIT
    assert not helper.git(ROOT, 'status', '--porcelain')
    for name, digest in source_hashes.items():
        assert sha(helper.git(ROOT, 'show', f'{COMMIT}:{name}')) == digest
        assert sha((ROOT / name).read_bytes()) == digest
    build, cli = helper.validate_build(MANIFEST, MANIFEST_SHA, CLI_SHA, COMMIT, SCHEMA, CLI)
    declarations = source.decode().split('fn main() {', 1)[0]
    transformed = (declarations + 'fn main() {\n    print("text-boundary: direct concat begin");\n    print(">=".to_text().concat(&"1.2"));\n    print("text-boundary: direct concat end");\n    let direct = parse_cargo(&">=1.2, <2").unwrap();\n    print("text-boundary: direct formatter begin");\n    print(format_constraint(&direct));\n    print("text-boundary: direct formatter end");\n    let wrapped = cargo(&">=1.2, <2");\n    print("text-boundary: wrapped formatter begin");\n    print(format_constraint(&wrapped));\n    print("text-boundary: wrapped formatter end");\n    formatted_partial_comparators_preserve_matching();\n    print("text-boundary: diagnostic completed");\n}\n').encode()
    assert transformed.split(b'fn main() {', 1)[0] == source.split(b'fn main() {', 1)[0]
    statements = [{'kind': 'only main body replaced; retain original declarations and compare direct Text, direct parsed and helper-returned values'}]
    DEST.mkdir(exist_ok=False)
    (DEST / 'runtime').mkdir()
    entry = DEST / 'runtime/options_trace.vr'
    entry.write_bytes(transformed)
    (DEST / 'original.vr').write_bytes(source)
    (DEST / 'statement-map.json').write_text(json.dumps(statements, indent=2) + '\n')
    for path, name in ((Path(__file__), 'runner.py'), (HELPER, 'build-validator.py'),
                       (MANIFEST, 'build-manifest.json'), (ORIGINAL / 'result.json', 'original-result.json')):
        shutil.copyfile(path, DEST / name)
    for name in source_hashes:
        target = DEST / 'source' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    command = [str(cli), 'run', '--tier', 'interpret', str(entry)]
    environment = {k: v for k, v in os.environ.items() if not k.startswith('VERUM_')}
    overrides = {'TMPDIR': '/private/tmp', 'NO_COLOR': '1', 'VERUM_NO_OBJECT_CACHE': '1', 'VERUM_TRACE_CALLS': '1', 'VERUM_TRACE_CALLM_FLOW': '1', 'VERUM_TRACE_PC': 'Text.', 'VERUM_TRACE_PC_DECODE': '1', 'VERUM_TRACE_STRDICE': 'Text.concat', 'VERUM_TRACE_FNREG': 'Text.concat', 'VERUM_TRACE_TIEBREAK': '1'}
    environment.update(overrides)
    state = {'task': 'T1744', 'scope': 'Diagnostic only: direct Text.concat and direct/helper parsed values in unchanged declaration context; Text-only opcode plus existing registration/stub/parent diagnostics, no options acceptance.',
             'source_commit': COMMIT, 'source_inputs': source_hashes,
             'original_result_sha256': sha((ORIGINAL / 'result.json').read_bytes()),
             'original_sha256': sha(source), 'transformed_sha256': sha(transformed),
             'statement_map_sha256': sha((DEST / 'statement-map.json').read_bytes()),
             'runner_sha256': sha(Path(__file__).read_bytes()), 'validator_sha256': HELPER_SHA,
             'build_manifest_sha256': MANIFEST_SHA, 'cli': build['cli'],
             'artifacts': build['artifacts'], 'artifact_snapshot': build['artifact_snapshot'],
             'schema': SCHEMA, 'command': command, 'cwd': str(entry.parent),
             'environment_overrides': overrides, 'deadline_seconds': 600}
    helper.write_json(DEST / 'started.json', state)
    process = None
    code = None
    timed_out = interrupted = False
    error = None
    start = time.monotonic()
    with (DEST / 'stdout.log').open('xb') as stdout, (DEST / 'stderr.log').open('xb') as stderr:
        try:
            process = subprocess.Popen(command, cwd=entry.parent, env=environment,
                                       stdout=stdout, stderr=stderr, start_new_session=True)
            helper.write_json(DEST / 'process.json', {'pid': process.pid, 'process_group': process.pid})
            try:
                code = process.wait(timeout=600)
            except subprocess.TimeoutExpired:
                timed_out = True
                code = helper.stop(process)
            except KeyboardInterrupt:
                interrupted = True
                code = helper.stop(process)
        except (OSError, subprocess.TimeoutExpired) as problem:
            error = f'{type(problem).__name__}: {problem}'
    seconds = round(time.monotonic() - start, 3)
    checks = {
        'source_head_unchanged': helper.git(ROOT, 'rev-parse', 'HEAD').decode().strip() == COMMIT,
        'source_clean': not helper.git(ROOT, 'status', '--porcelain'),
        'source_inputs_unchanged': all(sha((ROOT / name).read_bytes()) == digest for name, digest in source_hashes.items()),
        'retained_sources_unchanged': all(sha((DEST / 'source' / name).read_bytes()) == digest for name, digest in source_hashes.items()),
        'original_fixture_unchanged': (ORIGINAL / 'inputs/options.vr').read_bytes() == source == (DEST / 'original.vr').read_bytes(),
        'original_result_unchanged': sha((ORIGINAL / 'result.json').read_bytes()) == state['original_result_sha256'],
        'transformed_input_unchanged': entry.read_bytes() == transformed,
        'runner_unchanged': sha(Path(__file__).read_bytes()) == state['runner_sha256'] == sha((DEST / 'runner.py').read_bytes()),
        'validator_unchanged': sha(HELPER.read_bytes()) == HELPER_SHA == sha((DEST / 'build-validator.py').read_bytes()),
    }
    try:
        checks['build_and_five_artifacts_unchanged'] = helper.validate_build(MANIFEST, MANIFEST_SHA, CLI_SHA, COMMIT, SCHEMA, CLI) == (build, cli)
    except Exception as problem:
        checks['build_and_five_artifacts_unchanged'] = False
        error = f'{type(problem).__name__}: {problem}'
    stdout_text = (DEST / 'stdout.log').read_text()
    state.update(returncode=code, timed_out=timed_out, interrupted=interrupted, runner_error=error,
                 seconds=seconds, checks=checks, last_markers=stdout_text.splitlines()[-8:],
                 logs={name: sha((DEST / name).read_bytes()) for name in ('stdout.log', 'stderr.log')},
                 classification='DIAGNOSTIC_OBSERVATION' if all(checks.values()) and error is None else 'INVALID_DIAGNOSTIC_IDENTITY')
    helper.write_json(DEST / 'result.json', state)
    print(json.dumps({k: state[k] for k in ('returncode', 'seconds', 'timed_out', 'classification', 'last_markers', 'checks')}))
    return 0 if all(checks.values()) and error is None else 1


if __name__ == '__main__':
    raise SystemExit(main())
