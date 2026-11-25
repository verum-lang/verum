#!/usr/bin/env bash
# Rebuild the external calculus and Verum arithmetic from source in one isolated package.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo "Usage: $0 PATH_TO_LEAN4_MF_KERNEL" >&2
  exit 2
fi
corpus=$(cd -- "$1" && pwd -P)
replay=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
cmp "$corpus/lean-toolchain" "$replay/lean-toolchain"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/verum-real-pa.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
cd -- "$scratch"
cp "$corpus/lean-toolchain" "$corpus/lakefile.lean" .
ln -s "$corpus/MathFoundations" MathFoundations
ln -s "$corpus/MathFoundations.lean" MathFoundations.lean
lake build MathFoundations.Metatheory.RealArithmeticPA

mkdir -p .lake/build/lib/lean/VerumExternalReplay
cd -- "$replay"
lean -o "$scratch/.lake/build/lib/lean/VerumExternalReplay/GodelCantor.olean" VerumExternalReplay/GodelCantor.lean
LEAN_PATH="$scratch/.lake/build/lib/lean" lean corpora/RealPACantor.lean
