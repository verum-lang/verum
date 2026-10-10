import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

root = Path('/Users/taaliman/.codex/worktrees/registry-metadata-lookup/verum')
label, revision = sys.argv[1:]
assert label in ['baseline', 'fixed']
revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', revision], text=True).strip()
output = Path('/private/tmp/verum-T1714-producer-cache-' + label)
output.mkdir(exist_ok=False)
cache = Path('/Users/taaliman/.tmp/verum-codex-01a10248/native-target/debug/build')
blake3 = cache / 'blake3/4da0583a0558dc5e/out/libblake3-4da0583a0558dc5e.rlib'
metadata = blake3.with_suffix('.rmeta')
dependency_dirs = sorted({path.parent for path in cache.glob('*/*/out/*.rlib')})
sha256 = lambda data: hashlib.sha256(data).hexdigest()
selectors = [
    'crates/verum_ast/src/visibility.rs',
    'crates/verum_vbc/src/codegen/field_visibility.rs',
    'crates/verum_types/src/core_metadata.rs',
]
known_input = 'crates/verum_vbc/src/codegen/expressions.rs'
unrelated = 'docs/unrelated-fingerprint-control.md'
source = subprocess.check_output(['git', '-C', str(root), 'show', revision + ':crates/verum_compiler/build.rs'])
function = source[source.index(b'fn compute_core_blake3('):]
assert function.rstrip().endswith(b'}')
schema_tail = source[source.index(b'const PRECOMPILE_SCHEMA_VERSION: &str ='):]
schema = re.search(rb'^\s*"([^"\n]+)";', schema_tail, re.M).group(1).decode()
harness = output / 'fingerprint.rs'
harness.write_bytes(b'use std::path::Path;\nconst PRECOMPILE_SCHEMA_VERSION: &str = ' + json.dumps(schema).encode() + b';\n' + function + b'\nfn main() { println!("digest={}", compute_core_blake3(Path::new("unused"), &[])); }\n')
executable = output / 'fingerprint'
command = ['rustup', 'run', 'nightly-2026-08-20', 'rustc', '--edition=2024', str(harness), '--extern', 'blake3=' + str(metadata), '--extern', 'blake3=' + str(blake3), '-o', str(executable)]
for directory in dependency_dirs:
    command.extend(['-L', 'dependency=' + str(directory)])
libraries = {str(path): sha256(path.read_bytes()) for path in [blake3, metadata]}
start = time.monotonic()
compiled = subprocess.run(command, text=True, capture_output=True, timeout=60)
(output / 'build.log').write_text(compiled.stdout + compiled.stderr)
assert compiled.returncode == 0, compiled.stderr
fixture = output / 'fixture'
original = b'original producer input\n'
changed = b'changed producer input\n'
for name in [*selectors, known_input, unrelated]:
    path = fixture / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(original)
runs = []
def evaluate(name):
    result = subprocess.run([str(executable)], env={**os.environ, 'CARGO_MANIFEST_DIR': str(fixture / 'crates/verum_compiler')}, text=True, capture_output=True, timeout=10, check=True)
    (output / (name + '.log')).write_text(result.stdout + result.stderr)
    digests = [line.removeprefix('digest=') for line in result.stdout.splitlines() if line.startswith('digest=')]
    assert len(digests) == 1 and re.fullmatch('[0-9a-f]{64}', digests[0])
    runs.append({'name': name, 'digest': digests[0], 'stdout_sha256': sha256(result.stdout.encode())})
    return digests[0], result.stdout

initial, declarations = evaluate('initial')
repeated, _ = evaluate('repeated')
deltas = {}
for index, name in enumerate([*selectors, known_input, unrelated]):
    path = fixture / name
    path.write_bytes(changed)
    changed_digest, _ = evaluate('case-' + str(index) + '-changed')
    path.unlink()
    missing_digest, _ = evaluate('case-' + str(index) + '-missing')
    path.write_bytes(original)
    restored, _ = evaluate('case-' + str(index) + '-restored')
    deltas[name] = {
        'changed_digest': changed_digest != initial,
        'missing_digest': missing_digest != initial,
        'restored_digest': restored == initial,
        'rerun_declared': 'cargo:rerun-if-changed=' + str(path) in declarations.splitlines(),
    }
receipt = {
    'task': 'T1714',
    'label': label,
    'source_commit': revision,
    'build_script_sha256': sha256(source),
    'production_function_sha256': sha256(function),
    'schema': schema,
    'runner_sha256': sha256(Path(__file__).read_bytes()),
    'command': command,
    'rustc': subprocess.check_output(['rustup', 'run', 'nightly-2026-08-20', 'rustc', '-vV'], text=True),
    'linked_libraries_before': libraries,
    'linked_libraries_after': {path: sha256(Path(path).read_bytes()) for path in libraries},
    'executable_sha256': sha256(executable.read_bytes()),
    'harness_sha256': sha256(harness.read_bytes()),
    'deterministic_repeat': initial == repeated,
    'deltas': deltas,
    'runs': runs,
    'wall_seconds': round(time.monotonic() - start, 3),
    'scope': 'Exact committed production fingerprint function compiled with the existing cached blake3 library. Isolated fixture only; no Cargo, build target writes, automatic stdlib bake, native VBC lowering or ordinary CLI/AOT acceptance.',
}
(output / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
assert receipt['deterministic_repeat']
assert receipt['linked_libraries_before'] == receipt['linked_libraries_after']
for name, delta in deltas.items():
    expected = name == known_input or (label == 'fixed' and name in selectors)
    assert delta['changed_digest'] == expected, (name, delta)
    assert delta['missing_digest'] == expected, (name, delta)
    assert delta['rerun_declared'] == expected, (name, delta)
    assert delta['restored_digest'], (name, delta)
print(json.dumps({key: receipt[key] for key in ['label', 'source_commit', 'schema', 'deterministic_repeat', 'deltas', 'wall_seconds']}))
