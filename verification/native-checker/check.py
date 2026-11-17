"""Extract the production binding operation and recheck its Lean theorems."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
PROOF_CRATE = HERE.parents[1] / "crates" / "verum_proof"


def version(command):
    return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).strip()


def check(args):
    pin = json.loads((HERE / "toolchain.json").read_text())
    release = args.release.resolve()
    charon, aeneas, lean = release / "charon", release / "aeneas", args.lean.resolve()
    expected = [
        ([charon, "version"], pin["charon"]),
        ([charon, "toolchain-version"], pin["rust"]),
        ([aeneas, "-version"], f"aeneas {pin['aeneas']}"),
        ([lean, "--githash"], pin["lean_commit"]),
    ]
    for command, wanted in expected:
        actual = version(command)
        if actual != wanted:
            raise SystemExit(f"{command[0].name} returned {actual!r}, expected {wanted!r}")
    rustc = Path(version([charon, "toolchain-path"])) / "bin" / "rustc"
    if f"commit-hash: {pin['rust_commit']}" not in version([rustc, "-vV"]).splitlines():
        raise SystemExit("The selected Rust compiler has a different commit")
    lean_version = version([lean, "--version"])
    if not re.match(rf"^Lean \(version {re.escape(pin['lean'])},", lean_version):
        raise SystemExit(f"Unexpected Lean version: {lean_version}")

    backend = release / "backends" / "lean"
    manifest = json.loads((backend / "lake-manifest.json").read_text())
    packages = manifest["packages"]
    if not any(pkg["name"] == "mathlib" and pkg["rev"] == pin["mathlib"] for pkg in packages):
        raise SystemExit("The Aeneas backend manifest has a different mathlib revision")
    dependency_dirs = [backend / manifest["packagesDir"] / pkg["name"] for pkg in packages]
    build_lib = Path(".lake/build/lib/lean")
    if not (backend / build_lib / "Aeneas.olean").is_file():
        raise SystemExit("Build the pinned Aeneas Lean library before replay")

    with tempfile.TemporaryDirectory(prefix="verum-binding-", dir=args.scratch) as temp:
        work = Path(temp).resolve()
        env = {
            **os.environ,
            "RUSTUP_TOOLCHAIN": pin["rust"],
            "CARGO_TARGET_DIR": str(work / "target"),
        }
        llbc = work / "verum_proof.llbc"
        subprocess.run(
            [
                charon, "cargo", "--sysroot", "default", "--preset=aeneas", "--abort-on-error",
                "--start-from", "crate::binders::lookup_binding", "--dest-file", llbc,
                "--", "--locked", "--lib",
            ],
            cwd=PROOF_CRATE, env=env, check=True,
        )
        subprocess.run(
            [aeneas, "-backend", "lean", "-abort-on-error", "-dest", work, llbc],
            cwd=work, check=True,
        )
        generated = work / "VerumProof.lean"
        if generated.read_bytes() != (HERE / generated.name).read_bytes():
            raise SystemExit("The current Rust extraction differs from VerumProof.lean")

        env["LEAN_PATH"] = os.pathsep.join(
            str(path) for path in [work, backend / build_lib, *(pkg / build_lib for pkg in dependency_dirs)]
        )
        subprocess.run(
            [lean, "-DwarningAsError=true", "-o", "VerumProof.olean", generated.name],
            cwd=work, env=env, check=True,
        )
        proof = shutil.copy2(HERE / "BindingSlot.lean", work)
        subprocess.run([lean, "-DwarningAsError=true", Path(proof).name], cwd=work, env=env, check=True)
    print("Verified the production binding lookup and its Lean theorems.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, required=True, help="unpacked pinned Aeneas release")
    parser.add_argument("--lean", type=Path, required=True, help="pinned Lean compiler binary")
    parser.add_argument("--scratch", type=Path, required=True, help="existing directory for a private build")
    check(parser.parse_args())
