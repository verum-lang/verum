#!/usr/bin/env python3
"""Fail-closed dynamic dependency inspection for generated AOT executables (T1589).

Legacy invocation: check_no_libc_link.sh [verum_binary] [self_contained_source]
Existing artifact: check_no_libc_link.sh --artifact program

Never executes an inspected binary. A passing dynamic boundary does not prove
absence of statically linked libc or certify all runtime paths. The host CLI
may use baseline OS libraries; do not apply this AOT allowlist to its packaging.
"""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
MACH_MAGICS = {b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xce"}


class InspectionError(Exception):
    """No dependency verdict is possible."""


def tool(*names: str) -> str:
    for name in names:
        bundled = REPO / "llvm/install/bin" / name
        if bundled.is_file() and os.access(bundled, os.X_OK):
            return str(bundled)
        found = shutil.which(name)
        if found:
            return found
    raise InspectionError("required inspection tool missing: " + " or ".join(names))


def run_inspector(args: list[str]) -> str:
    try:
        result = subprocess.run(args, capture_output=True, text=True,
                                env={**os.environ, "LC_ALL": "C"}, timeout=60)
    except (OSError, subprocess.SubprocessError) as error:
        raise InspectionError(f"cannot inspect artifact: {error}") from error
    if result.returncode or result.stderr.strip() or not result.stdout.strip():
        raise InspectionError(f"inspector failed or reported diagnostics ({result.returncode}): "
                              f"{result.stderr.strip() or result.stdout.strip()}")
    return result.stdout


def parse_elf(text: str) -> dict:
    if "ELF Header:" not in text or not re.search(r"Type:\s+(EXEC|DYN)\b", text):
        raise InspectionError("expected a readable ELF executable header")
    if "Program Headers:" not in text or not re.search(r"^\s*LOAD\s", text, re.M):
        raise InspectionError("expected readable ELF load segments")
    needed = [line for line in text.splitlines() if "(NEEDED)" in line]
    dependencies = []
    for line in needed:
        match = re.search(r"\(NEEDED\).*Shared library: \[([^\]]+)\]", line)
        if not match:
            raise InspectionError("unrecognized ELF dependency record: " + line)
        dependencies.append(match[1])
    interpreters = re.findall(r"Requesting program interpreter:\s*([^\]\n]+)", text)
    if re.search(r"^\s*INTERP\s", text, re.M) and not interpreters:
        raise InspectionError("unreadable ELF program interpreter")
    return {"format": "ELF", "dependencies": dependencies, "interpreters": interpreters,
            "versions": sorted(set(re.findall(r"\b(?:GLIBC|GLIBCXX|CXXABI)_[0-9.]+", text)))}


def parse_macho(text: str) -> dict:
    lines = text.splitlines()
    if not lines or not lines[0].endswith(":"):
        raise InspectionError("expected otool artifact header")
    dependencies = []
    for line in lines[1:]:
        if not line.strip():
            continue
        match = re.fullmatch(r"\s+(.+?) \(compatibility version [^,]+, current version [^)]+\)", line)
        if not match:
            raise InspectionError("unrecognized Mach-O dependency record: " + line)
        dependencies.append(match[1])
    return {"format": "Mach-O", "dependencies": dependencies, "interpreters": [], "versions": []}


def parse_pe(text: str) -> dict:
    if not re.search(r"^Format: COFF-(?:x86-64|ARM64)\s*$", text, re.M):
        raise InspectionError("expected a supported PE/COFF image header")
    # Delay imports are just as much dependencies as ordinary import records.
    records = re.findall(r"^(?:Import|DelayImport) \{\n(.*?)^\}", text, re.M | re.S)
    if len(records) != len(re.findall(r"^(?:Import|DelayImport) \{", text, re.M)):
        raise InspectionError("incomplete PE import record")
    dependencies = []
    for record in records:
        names = re.findall(r"^  Name: (.+)$", record, re.M)
        if len(names) != 1:
            raise InspectionError("PE import record has no unique DLL name")
        dependencies.append(names[0])
    # Reading the DOS/PE image with --file-headers must expose the executable flag.
    if "IMAGE_FILE_EXECUTABLE_IMAGE" not in text:
        raise InspectionError("COFF object is not a PE executable image")
    return {"format": "PE", "dependencies": dependencies, "interpreters": [], "versions": []}


def inspect(path: Path) -> dict:
    try:
        with path.open("rb") as stream:
            magic = stream.read(4)
    except OSError as error:
        raise InspectionError(str(error)) from error
    if magic == b"\x7fELF":
        text = run_inspector([tool("llvm-readelf", "readelf"), "--file-header",
                              "--program-headers", "--dynamic", "--version-info", str(path)])
        return parse_elf(text)
    if magic in MACH_MAGICS:
        return parse_macho(run_inspector([tool("otool"), "-L", str(path)]))
    if magic[:2] == b"MZ":
        return parse_pe(run_inspector([tool("llvm-readobj"), "--file-headers",
                                     "--coff-imports", str(path)]))
    raise InspectionError("unsupported executable format; no dependency verdict")


def forbidden(evidence: dict) -> list[str]:
    allowed = {"ELF": set(), "Mach-O": {"/usr/lib/libSystem.B.dylib"},
               "PE": {"kernel32.dll", "ntdll.dll"}}[evidence["format"]]
    denied = [name for name in evidence["dependencies"]
              if (name.lower() if evidence["format"] == "PE" else name) not in allowed]
    # A GNU/musl runtime loader is not a kernel service. A library-free Linux
    # control must not silently retain that loader as an architectural exception.
    denied += ["program interpreter: " + name for name in evidence["interpreters"]]
    denied += ["runtime symbol version: " + name for name in evidence["versions"]]
    return denied


def check(path: Path) -> int:
    evidence = inspect(path)
    print(f"[info] {evidence['format']} artifact: {path}")
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    print(f"[info] SHA-256: {digest.hexdigest()}")
    for name in evidence["dependencies"]:
        print(f"[info] import: {name}")
    denied = forbidden(evidence)
    for name in denied:
        print(f"[fail] outside the default AOT OS boundary: {name}")
    if denied:
        return 1
    print(f"[ok] AOT dynamic boundary: {len(evidence['dependencies'])} imports, "
          f"{len(evidence['interpreters'])} interpreters, {len(denied)} forbidden entries")
    print("[scope] Static library provenance and other runtime paths remain separate checks.")
    return 0


def build_and_check(verum: Path, source: Path | None) -> int:
    if platform.system() not in {"Linux", "Darwin", "Windows", "FreeBSD"}:
        raise InspectionError("unsupported build host; use --artifact with a supported executable")
    if not verum.is_file() or not os.access(verum, os.X_OK):
        raise InspectionError(f"verum binary is missing or not executable: {verum}")
    # Isolate source, intermediate files and output together. Never accept a
    # stale /tmp/target binary left by a previous smoke or another session.
    with tempfile.TemporaryDirectory(prefix="verum_no_libc_") as directory:
        work = Path(directory)
        smoke = work / "smoke.vr"
        if source:
            shutil.copyfile(source, smoke)
        else:
            smoke.write_text('fn main() { print("smoke"); }\n')
        print(f"[info] building isolated AOT smoke: {smoke}", flush=True)
        result = subprocess.run([str(verum), "build", str(smoke)], cwd=work)
        if result.returncode:
            raise InspectionError(f"verum build failed ({result.returncode})")
        binary = work / "target/release" / ("smoke.exe" if os.name == "nt" else "smoke")
        if not binary.is_file():
            raise InspectionError(f"build did not produce the fresh expected executable: {binary}")
        return check(binary)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("verum_binary", nargs="?")
    parser.add_argument("smoke_source", nargs="?", help="self-contained source copied into an isolated directory")
    parser.add_argument("--artifact", type=Path, help="inspect an existing AOT executable without building or running it")
    args = parser.parse_args(argv)
    if args.artifact and (args.verum_binary or args.smoke_source):
        parser.error("--artifact cannot be combined with build arguments")
    try:
        if args.artifact:
            return check(args.artifact.resolve())
        default = Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "target"))) / "debug/verum"
        verum = Path(args.verum_binary) if args.verum_binary else default
        # Retain the legacy repo-relative binary/source argument convention.
        verum = verum if verum.is_absolute() else REPO / verum
        source = Path(args.smoke_source) if args.smoke_source else None
        if source and not source.is_absolute():
            source = REPO / source
        return build_and_check(verum, source)
    except (InspectionError, OSError) as error:
        print(f"[error] {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
