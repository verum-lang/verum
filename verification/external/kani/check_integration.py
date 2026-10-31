#!/usr/bin/env python3
"""Real Kani execution: exact machine arithmetic, wrong code and vacuous inputs."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from rust_replay import FORMAT, load, read, sha, write


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kani', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = read(root/'fixtures/checked_word.rs').decode()
    cases = {
        'exact': (source, True),
        'wrapping': (source.replace('value.checked_add(1)', 'Some(value.wrapping_add(1))'), False),
        'vacuous_limit': (source.replace('let value: u64 = kani::any();',
                                       'let value: u64 = kani::any(); kani::assume(value < u64::MAX);'), False),
    }
    results = []
    for name, (text, accepted) in cases.items():
        case = out/name
        case.mkdir()
        data = text.encode()
        (case/'input.rs').write_bytes(data)
        manifest = {'format': FORMAT,
                    'sources': {'word.rs': {'path': 'input.rs', 'sha256': sha(data)}},
                    'files': {'word.rs': [{'source': 'word.rs', 'start': 0, 'end': len(data)}]},
                    'entry': 'word.rs',
                    'harnesses': {'exact_word_successor': {'source': 'word.rs',
                                  'covers': ['zero', 'limit', 'last_successor']}},
                    'premises': ['Rust u64 and Option semantics in the pinned Kani toolchain']}
        write(case/'manifest.json', manifest)
        process = subprocess.run([sys.executable, str(root/'rust_replay.py'),
                                  '--manifest', str(case/'manifest.json'), '--output', str(case/'run'),
                                  '--kani', str(args.kani.resolve())], capture_output=True, text=True, timeout=180)
        (case/'driver.log').write_text(process.stdout+process.stderr)
        report = load(read(case/'run/receipt.json'))
        if accepted:
            assert process.returncode == 0 and report['status'] == 'verified', report
        else:
            assert process.returncode != 0 and report['status'] == 'inconclusive', report
            # A setup error, timeout or parser failure is not the expected control.
            actual = load(read(case/'run/result.json'))['verification_results']
            assert actual['summary']['status'] == 'completed' and actual['summary']['executed'] == 1
            checks = actual['results'][0]['checks']
            if name == 'wrapping':
                failures = [c for c in checks if c['status'] == 'Failure']
                assert failures and all(c['category'] == 'assertion' and
                                        c['location']['file'] == 'word.rs' for c in failures), failures
            else:
                assert actual['summary']['successful'] == 1
                assert any(c['category'] == 'cover' and c['description'] == 'limit' and
                           c['status'] == 'Unsatisfiable' for c in checks), checks
        results.append({'case': name, 'accepted': accepted, 'receipt_sha256': sha(read(case/'run/receipt.json'))})
        print(name, 'passed', flush=True)
    write(out/'controls.json', {'status': 'passed', 'controls': results})


if __name__ == '__main__':
    main()
