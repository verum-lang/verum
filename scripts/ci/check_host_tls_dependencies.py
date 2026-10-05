#!/usr/bin/env python3
"""Inspect packaged host CLI transports, separately from AOT no-libc (T1588).

This gate rejects dynamically linked Git/OpenSSL transport libraries and macOS
non-system import paths. It does not certify GLIBC/VC runtime baselines, the
complete transitive dependency closure, or execution on a clean OS image.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
import tarfile
import tempfile
import zipfile

# Share executable-format parsing, never the generated-AOT import policy.
from check_aot_dependencies import InspectionError, inspect


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def forbidden_transports(evidence: dict) -> list[str]:
    denied = []
    for name in evidence['dependencies']:
        leaf = name.replace('\\', '/').rsplit('/', 1)[-1].lower()
        if re.match(r'^(?:lib)?(?:ssl|crypto|git2|ssh2)(?:[.\-_0-9]|$)', leaf):
            denied.append(name)
        elif evidence['format'] == 'Mach-O' and not name.startswith(('/usr/lib/', '/System/Library/')):
            # The current macOS archive ships no runtime dylibs. An @rpath or
            # builder-prefix library cannot be supplied by that release asset.
            denied.append(name)
    return denied


@contextmanager
def packaged_binary(asset: Path):
    """Read only the unique root executable; never extract archive paths."""
    with tempfile.TemporaryDirectory(prefix='verum-host-deps-') as directory:
        destination = Path(directory) / 'verum'
        if asset.name.endswith('.tar.gz'):
            with tarfile.open(asset, 'r:gz') as archive:
                members = [m for m in archive.getmembers() if m.name in {'verum', './verum'}]
                if len(members) != 1 or not members[0].isfile():
                    raise InspectionError('asset must contain one regular root verum executable')
                with archive.extractfile(members[0]) as source, destination.open('wb') as target:
                    shutil.copyfileobj(source, target)
        elif asset.suffix == '.zip':
            with zipfile.ZipFile(asset) as archive:
                members = [m for m in archive.infolist() if m.filename in {'verum.exe', './verum.exe'}]
                if len(members) != 1 or members[0].is_dir():
                    raise InspectionError('asset must contain one root verum.exe executable')
                with archive.open(members[0]) as source, destination.open('wb') as target:
                    shutil.copyfileobj(source, target)
        else:
            raise InspectionError('unsupported release archive format')
        yield destination


def check(binary: Path) -> dict:
    evidence = inspect(binary)
    denied = forbidden_transports(evidence)
    return {
        'status': 'FAIL' if denied else 'PASS',
        'scope': 'host CLI direct transport imports; not AOT or full clean-OS acceptance',
        'binary_sha256': digest(binary),
        **evidence,
        'forbidden': denied,
        'remaining_acceptance': ['transitive dependency closure', 'oldest-supported OS execution',
                                 'Linux GLIBC/GLIBCXX baseline', 'Windows VC runtime availability'],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--asset', type=Path)
    source.add_argument('--binary', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args(argv)
    try:
        if args.asset:
            with packaged_binary(args.asset) as binary:
                report = check(binary)
            report['asset_sha256'] = digest(args.asset)
            report['asset'] = args.asset.name
        else:
            report = check(args.binary)
        code = 0 if report['status'] == 'PASS' else 1
    except (InspectionError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        report = {'status': 'ERROR', 'scope': 'host CLI direct transport imports', 'error': str(error)}
        code = 2
    output = json.dumps(report, indent=2) + '\n'
    if args.report:
        args.report.write_text(output)
    print(output, end='')
    return code


if __name__ == '__main__':
    sys.exit(main())
