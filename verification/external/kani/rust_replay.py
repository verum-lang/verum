#!/usr/bin/env python3
"""Source-bound Rust model checking, separate from native/Lean proof admission.

The manifest selects exact source bytes and independently names the required
harnesses and reachability covers. Rust source/harnesses are trusted executable
inputs: run this tool inside the same sandbox used to build that project.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import signal
import subprocess
import time

FORMAT = 'verum-rust-symbolic/1'
KANI_VERSION = '0.68.0'
MAX_INPUT = 16 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    with Path(path).open('rb') as stream:
        data = stream.read(MAX_INPUT + 1)
    require(len(data) <= MAX_INPUT, 'Input exceeds 16 MiB')
    return data


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'Duplicate JSON key: ' + key)
        result[key] = value
    return result


def load(data):
    def constant(value):
        raise ValueError('Non-JSON constant: ' + value)
    return json.loads(data, object_pairs_hook=unique, parse_constant=constant)


def fields(value, names):
    require(type(value) is dict and set(value) == set(names.split()), 'Unexpected fields: ' + names)


def relative(name):
    require(type(name) is str and name and '\\' not in name, 'Invalid relative path')
    path = PurePosixPath(name)
    require(not path.is_absolute() and '..' not in path.parts and str(path) == name,
            'Noncanonical relative path')
    return path


def roster(rows, key):
    require(type(rows) is list, 'Expected result array')
    result = {}
    for row in rows:
        name = row[key]
        require(type(name) is (int if key == 'id' else str) and name not in result,
                'Duplicate or invalid result identity')
        result[name] = row
    return result


def validate_result(document, expected, returncode):
    """Fail closed on missing obligations, vacuous covers and incomplete checks.

    This validates a fresh tool execution's consistency, not its mathematical
    soundness. An imported JSON status is never a kernel certificate.
    """
    require(returncode == 0, 'Checker process did not succeed')
    require(document['metadata']['version'] == '1.0' and
            document['metadata']['kani_version'] == KANI_VERSION and
            document['tools']['kani'] == KANI_VERSION, 'Unsupported Kani result version')
    require(type(expected) is dict and expected, 'No requested harnesses')
    summary = document['verification_results']['summary']
    count = len(expected)
    require(summary['status'] == 'completed' and
            all(type(summary[k]) is int and summary[k] == count
                for k in ('total_harnesses', 'executed', 'successful')) and
            type(summary['failed']) is int and summary['failed'] == 0, 'Incomplete harness execution')
    results = roster(document['verification_results']['results'], 'harness_id')
    metadata = roster(document['harness_metadata'], 'pretty_name')
    errors = roster(document['error_details'], 'harness_id')
    properties = roster(document['property_details'], 'harness_id')
    backends = roster(document['cbmc'], 'harness_id')
    require(all(set(rows) == set(expected) for rows in (results, metadata, errors, properties, backends)),
            'Harness roster does not match requested obligations')
    totals = Counter()
    for name, wanted in expected.items():
        fields(wanted, 'source covers')
        relative(wanted['source'])
        covers = wanted['covers']
        require(type(covers) is list and covers and all(type(c) is str and c for c in covers)
                and len(set(covers)) == len(covers), 'Each harness needs distinct reachability obligations')
        info, result = metadata[name], results[name]
        require(info['source']['file'] == wanted['source'], 'Unexpected harness source')
        require(info['attributes'] == {'kind': 'Proof', 'should_panic': False},
                'Only ordinary proof harnesses are admitted')
        require(info['contract'] == {'contracted_function_name': None, 'recursion_tracker': None}
                and all(info[key] is False for key in ('has_loop_contracts', 'is_automatically_generated',
                                                       'is_bounded', 'is_ctor_based')),
                'Unsupported harness abstraction')
        require(result['status'] == 'Success' and errors[name]['has_errors'] is False,
                'Harness failed or was incomplete')
        checks = result['checks']
        require(type(checks) is list and checks and len(roster(checks, 'id')) == len(checks),
                'Missing or duplicate properties')
        statuses, observed_covers, assertions = Counter(), [], 0
        for check in checks:
            status, category = check['status'], check['category']
            statuses[status] += 1
            if category == 'cover':
                require(status == 'Satisfied', 'Reachability obligation was not satisfied')
                observed_covers.append(check['description'])
            else:
                require(status in ('Success', 'Unreachable'), 'Unproved safety or assertion property')
                if category == 'assertion' and check.get('location', {}).get('file') == wanted['source']:
                    require(status == 'Success', 'Contract assertion is unreachable')
                    assertions += 1
        require(assertions > 0, 'No reachable contract assertion')
        require(Counter(observed_covers) == Counter(covers), 'Cover roster mismatch')
        actual = properties[name]['property_details']
        wanted_counts = dict(total_properties=len(checks), passed=statuses['Success'],
                             failed=0, unreachable=statuses['Unreachable'], undetermined=0,
                             solver_error=0, satisfied=statuses['Satisfied'], unsatisfiable=0,
                             covered=0, uncovered=0)
        require(all(type(v) is int and v >= 0 for v in actual.values()) and actual == wanted_counts,
                'Property summary disagrees with actual checks')
        totals.update(statuses)
    return {'harnesses': count, 'properties': sum(totals.values()), 'statuses': dict(totals)}


def prepare(manifest, base, output):
    """Materialize a new workspace from full sources or explicit byte ranges.

    Prefixes/suffixes (e.g. a child harness declaration) remain manifest data;
    no regex rewriting, type substitution or replacement bodies are implicit.
    Full original files and the assembly recipe are retained independently.
    """
    fields(manifest, 'format sources files entry harnesses premises')
    require(manifest['format'] == FORMAT, 'Unsupported source manifest')
    require(type(manifest['premises']) is list and manifest['premises'] and
            all(type(p) is str and p for p in manifest['premises']), 'Explicit premises required')
    output.mkdir(parents=True, exist_ok=False)
    original, workspace = output / 'original', output / 'workspace'
    original.mkdir(); workspace.mkdir()
    sources, source_hashes, file_hashes, selections = {}, {}, {}, []
    require(type(manifest['sources']) is dict and manifest['sources'], 'Missing source manifest')
    for name, item in manifest['sources'].items():
        relative(name)
        fields(item, 'path sha256')
        path = base / item['path']
        data = read(path)
        require(sha(data) == item['sha256'], 'Source hash mismatch: ' + name)
        target = original / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        sources[name] = data
        source_hashes[name] = sha(data)
    require(type(manifest['files']) is dict and manifest['files'], 'No compilation inputs')
    for name, parts in manifest['files'].items():
        relative(name)
        require(type(parts) is list and parts, 'Empty compilation input')
        chunks = []
        for part in parts:
            if set(part) == {'text'}:
                require(type(part['text']) is str, 'Invalid literal source')
                chunks.append(part['text'].encode('utf-8'))
            else:
                fields(part, 'source start end')
                data = sources[part['source']]
                start, end = part['start'], part['end']
                require(type(start) is int and type(end) is int and 0 <= start < end <= len(data),
                        'Invalid source byte range')
                fragment = data[start:end]
                fragment.decode('utf-8')
                chunks.append(fragment)
                selections.append({'file': name, **part, 'sha256': sha(fragment)})
        data = b''.join(chunks)
        require(len(data) <= MAX_INPUT, 'Assembled source exceeds limit')
        target = workspace / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        file_hashes[name] = sha(data)
    require(manifest['entry'] in file_hashes, 'Entry is not in the snapshot')
    for item in manifest['harnesses'].values():
        require(item['source'] in file_hashes, 'Harness source not in the snapshot')
    return workspace, {'sources': source_hashes, 'files': file_hashes, 'selections': selections}


def tool_identity(kani):
    """Hash the supplied distribution and its pinned Rust runtime, not PATH aliases."""
    root = kani.resolve().parent.parent
    require((root / 'bin/kani-compiler').is_file() and (root / 'toolchain/lib').is_dir(),
            'Expected installed Kani distribution with pinned toolchain')
    paths = set()
    for directory in ('bin', 'lib', 'library', 'toolchain/lib', 'toolchain/bin'):
        paths.update(p for p in (root / directory).rglob('*') if p.is_file())
    paths.update(root / name for name in ('rust-toolchain-version', 'rustc-version'))
    result = {}
    for path in sorted(paths):
        with path.open('rb') as stream:
            result[str(path.relative_to(root))] = hashlib.file_digest(stream, 'sha256').hexdigest()
    return root, result


def execute(command, *, cwd, env, timeout, log):
    with log.open('wb') as stream:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT,
                                   start_new_session=os.name == 'posix')
        try:
            return process.wait(timeout=timeout)
        except BaseException:
            try:
                if os.name == 'posix': os.killpg(process.pid, signal.SIGKILL)
                else: process.kill()
            except ProcessLookupError:
                pass
            process.wait()
            raise


def write(path, data):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(data, indent=2) + '\n')
    temporary.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New, retained evidence directory')
    parser.add_argument('--kani', type=Path, required=True, help='Installed bin/kani-driver')
    parser.add_argument('--timeout', type=int, default=120)
    parser.add_argument('--unwind', type=int, default=4)
    args = parser.parse_args()
    require(0 < args.timeout <= 3600 and 0 < args.unwind <= 1024, 'Invalid execution bound')
    raw = read(args.manifest)
    manifest = load(raw)
    workspace, snapshot = prepare(manifest, args.manifest.resolve().parent, args.output.resolve())
    out = args.output.resolve()
    (out / 'manifest.json').write_bytes(raw)
    recorder = read(__file__)
    (out / 'rust_replay.py').write_bytes(recorder)
    report = dict(format=FORMAT, status='running', authority='external-symbolic-check',
                  manifest_sha256=sha(raw), recorder_sha256=sha(recorder), snapshot=snapshot,
                  premises=manifest['premises'])
    write(out / 'receipt.json', report)
    started = time.monotonic()
    try:
        root, identity = tool_identity(args.kani)
        write(out / 'toolchain.json', identity)
        report['toolchain_sha256'] = sha(read(out / 'toolchain.json'))
        command = [str(args.kani.resolve()), manifest['entry'], '-Z', 'unstable-options',
                   '--default-unwind', str(args.unwind), '--harness-timeout', str(args.timeout),
                   '--export-json', str(out / 'result.json'), '--output-format', 'terse', '--keep-temps']
        # No caller-supplied compiler flags, stub features, unsound unchecked
        # unwinding, panic expectations or omitted reachability checks.
        env = {k: os.environ[k] for k in ('HOME', 'TMPDIR', 'SYSTEMROOT') if k in os.environ}
        env['PATH'] = str(root / 'bin') + os.pathsep + '/usr/bin:/bin'
        report.update(command=command, cwd=str(workspace), target=(root/'rust-toolchain-version').read_text())
        write(out / 'receipt.json', report)
        code = execute(command, cwd=workspace, env=env, timeout=args.timeout, log=out/'execution.log')
        report['returncode'] = code
        report['execution_log_sha256'] = sha(read(out/'execution.log'))
        result = read(out / 'result.json')
        report['result_sha256'] = sha(result)
        report['counts'] = validate_result(load(result), manifest['harnesses'], code)
        for name, expected in snapshot['files'].items():
            require(sha(read(workspace / name)) == expected, 'Compilation input changed during checking')
        require(tool_identity(args.kani)[1] == identity, 'Checker distribution changed during checking')
        report['status'] = 'verified'
    except BaseException as error:
        report.update(status='inconclusive', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        report['seconds'] = time.monotonic() - started
        write(out / 'receipt.json', report)
    print(json.dumps(report['counts']))


if __name__ == '__main__':
    main()
