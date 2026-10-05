#!/usr/bin/env python3
"""Host transport policy/asset tests; no compiler or native program execution."""
import contextlib
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_host_tls_dependencies as guard


def evidence(kind, *dependencies):
    return {'format': kind, 'dependencies': list(dependencies), 'interpreters': [], 'versions': []}


class HostTransportTests(unittest.TestCase):
    def test_host_os_libraries_are_not_subject_to_aot_allowlist(self):
        for row in [evidence('Mach-O', '/usr/lib/libSystem.B.dylib', '/usr/lib/libc++.1.dylib', '/System/Library/Frameworks/Security.framework/Versions/A/Security'),
                    evidence('ELF', 'libc.so.6', 'libstdc++.so.6'), evidence('PE', 'ucrtbase.dll', 'VCRUNTIME140.dll', 'KERNEL32.dll')]:
            self.assertEqual(guard.forbidden_transports(row), [])

    def test_git_and_openssl_imports_are_rejected_on_each_platform(self):
        for kind, names in [('Mach-O', ['/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib', '/usr/local/opt/openssl@3/lib/libcrypto.3.dylib', '@rpath/libgit2.1.9.dylib']),
                            ('ELF', ['libssl.so.3', 'libcrypto.so.3', 'libgit2.so.1.9', 'libssh2.so.1']),
                            ('PE', ['libssl-3-x64.dll', 'LIBCRYPTO-3-ARM64.DLL', 'libgit2.dll', 'libssh2.dll'])]:
            for name in names:
                with self.subTest(kind=kind, name=name):
                    self.assertEqual(guard.forbidden_transports(evidence(kind, name)), [name])

    def test_non_system_macos_paths_are_not_hidden_by_a_library_name(self):
        for name in ['@rpath/custom.dylib', '/opt/local/lib/libz.dylib', '/tmp/libSystem.B.dylib']:
            self.assertEqual(guard.forbidden_transports(evidence('Mach-O', name)), [name])

    def test_name_boundary_does_not_reject_unrelated_host_library(self):
        self.assertEqual(guard.forbidden_transports(evidence('ELF', 'libssltunnel-other.so')), [])

    def test_tar_inspects_the_exact_root_payload_without_extracting_other_paths(self):
        with tempfile.TemporaryDirectory() as d:
            asset = Path(d) / 'asset.tar.gz'
            with tarfile.open(asset, 'w:gz') as archive:
                for name, data in [('verum', b'payload'), ('../../escape', b'unrelated')]:
                    member = tarfile.TarInfo(name); member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))
            with guard.packaged_binary(asset) as binary:
                self.assertEqual(binary.read_bytes(), b'payload')
                temporary = binary
            self.assertFalse(temporary.exists())

    def test_zip_selects_root_executable(self):
        with tempfile.TemporaryDirectory() as d:
            asset = Path(d) / 'asset.zip'
            with zipfile.ZipFile(asset, 'w') as archive:
                archive.writestr('verum.exe', b'pe-payload')
                archive.writestr('README.md', b'docs')
            with guard.packaged_binary(asset) as binary:
                self.assertEqual(binary.read_bytes(), b'pe-payload')

    def test_missing_duplicate_or_symlink_tar_payload_is_not_success(self):
        for names, link in [([], False), (['verum', './verum'], False), (['verum'], True)]:
            with self.subTest(names=names, link=link), tempfile.TemporaryDirectory() as d:
                asset = Path(d) / 'asset.tar.gz'
                with tarfile.open(asset, 'w:gz') as archive:
                    for name in names:
                        member = tarfile.TarInfo(name)
                        if link: member.type = tarfile.SYMTYPE; member.linkname = 'somewhere'
                        archive.addfile(member)
                with self.assertRaises(guard.InspectionError), guard.packaged_binary(asset):
                    pass

    def test_package_and_payload_digests_record_the_inspected_asset(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            asset = Path(d) / 'asset.zip'; report = Path(d) / 'report.json'
            with zipfile.ZipFile(asset, 'w') as archive: archive.writestr('verum.exe', b'payload')
            with patch.object(guard, 'inspect', return_value=evidence('PE', 'KERNEL32.dll')):
                self.assertEqual(guard.main(['--asset', str(asset), '--report', str(report)]), 0)
            import json, hashlib
            result = json.loads(report.read_text())
            self.assertEqual(result['asset_sha256'], guard.digest(asset))
            self.assertEqual(result['binary_sha256'], hashlib.sha256(b'payload').hexdigest())
            self.assertTrue(result['remaining_acceptance'])

    def test_forbidden_is_exit_one_and_unreadable_is_exit_two(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            binary = Path(d) / 'verum'; binary.write_bytes(b'not-executable')
            with patch.object(guard, 'inspect', return_value=evidence('ELF', 'libssl.so.3')):
                self.assertEqual(guard.main(['--binary', str(binary)]), 1)
            self.assertEqual(guard.main(['--binary', str(binary)]), 2)
            with patch.object(guard, 'inspect', side_effect=guard.InspectionError('missing tool')):
                self.assertEqual(guard.main(['--binary', str(binary)]), 2)


if __name__ == '__main__':
    unittest.main()
