#!/usr/bin/env python3
"""Check actual Rust clock ownership and restart code, including semantic mutants.

The complete clock module and its original parent declarations are bound to the
same native receipt as the Lean trace corpus. This sequential Rust/MIR check
covers arbitrary machine fields, not whole-application or concurrent behavior.
"""
import argparse
import copy
import io
from pathlib import Path
import subprocess
import sys
import tarfile

from rust_replay import load, read, require, sha, write


def archive(root, name):
    metadata = load(read(root / (name + '.json')))
    data = read(root / (name + '.tar.gz'))
    require(sha(data) == metadata['archive_sha256'], 'Archive hash mismatch')
    blobs = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tar:
        members = tar.getmembers()
        require(len(members) == len(metadata['members']) and
                {m.name for m in members} == set(metadata['members']), 'Archive roster mismatch')
        for member in members:
            require(member.isfile() and Path(member.name).name == member.name and
                    member.size <= 16 * 1024 * 1024, 'Invalid archive member')
            value = tar.extractfile(member).read()
            require(sha(value) == metadata['members'][member.name], 'Archive member changed')
            blobs[member.name] = value
    return metadata, blobs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kani', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    metadata, blobs = archive(root / 'fixtures', 'native-clock-v1')
    _, native = archive(root.parent / 'lean/fixtures', 'source-cycle-native-v1')
    receipt = load(native['native-receipt.json'])
    for source, path in metadata['native_sources'].items():
        require(sha(blobs[source]) == receipt['source'][path]['sha256'],
                'Symbolic source differs from the actual native build')
    write(out / 'native-source.json', {
        'receipt_sha256': sha(native['native-receipt.json']),
        'whole_run_status': receipt['status'],
        'sources': metadata['native_sources'],
    })
    base = load(blobs['manifest.json'])
    original = blobs['physical_time.rs'].decode()
    cases = (
        ('exact', None, None),
        ('restart_without_ownership_guard', '        self.check_restart()?;', ''),
        ('terminal_mints_tick_point',
         '        if self.edge.transition != Transition::Tick { return None; }', ''),
    )
    results = []
    for name, before, after in cases:
        case = out / name
        case.mkdir()
        manifest = copy.deepcopy(base)
        text = original
        if before is not None:
            require(text.count(before) == 1, 'Mutation target is not unique')
            text = text.replace(before, after)
            manifest['premises'].append('Deliberate semantic mutation: ' + name)
        data = text.encode()
        for source in base['sources']:
            (case / source).write_bytes(data if source == 'physical_time.rs' else blobs[source])
        manifest['sources']['physical_time.rs']['sha256'] = sha(data)
        manifest['files']['organism/causal_coordinates/physical_time.rs'][0]['end'] = len(data)
        write(case / 'manifest.json', manifest)
        process = subprocess.run([
            sys.executable, str(root / 'rust_replay.py'), '--manifest', str(case / 'manifest.json'),
            '--output', str(case / 'run'), '--kani', str(args.kani.resolve()), '--timeout', '75',
        ], capture_output=True, text=True, timeout=90)
        (case / 'driver.log').write_text(process.stdout + process.stderr)
        report = load(read(case / 'run/receipt.json'))
        if before is None:
            require(process.returncode == 0 and report['status'] == 'verified', str(report))
        else:
            require(process.returncode != 0 and report['status'] == 'inconclusive', str(report))
            actual = load(read(case / 'run/result.json'))['verification_results']
            require(actual['summary']['status'] == 'completed' and
                    actual['summary']['executed'] == len(manifest['harnesses']),
                    'Mutation check did not complete')
            checks = [c for result in actual['results'] for c in result['checks']]
            failures = [c for c in checks if c['status'] == 'Failure']
            require(failures and all(c['category'] == 'assertion' and
                    c['location']['file'].endswith('/verification.rs') for c in failures),
                    'Mutation was not rejected by a contract counterexample')
            require(not any(c['status'] in ('Undetermined', 'SolverError') for c in checks),
                    'Mutation check has incomplete properties')
        results.append({'case': name, 'accepted': before is None,
                        'receipt_sha256': sha(read(case / 'run/receipt.json'))})
        print(name, 'passed', flush=True)
    write(out / 'controls.json', {'status': 'passed', 'controls': results})


if __name__ == '__main__':
    main()
