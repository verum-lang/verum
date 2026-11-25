#!/usr/bin/env bash
# Check the imported mathematical results and Verum bridges from source in one isolated package.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo "Usage: $0 PATH_TO_LEAN4_MF_KERNEL" >&2
  exit 2
fi
corpus=$(cd -- "$1" && pwd -P)
replay=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
cmp "$corpus/lean-toolchain" "$replay/lean-toolchain"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/verum-math-foundations.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
cd -- "$scratch"
cp "$corpus/lean-toolchain" "$corpus/lakefile.lean" .
ln -s "$corpus/MathFoundations" MathFoundations
ln -s "$corpus/MathFoundations.lean" MathFoundations.lean
lake build MathFoundations.Metatheory.RealArithmeticPA MathFoundations.Metatheory.BoundedArithmeticS21 \
  MathFoundations.Metatheory.DAGResolutionPHP MathFoundations.Metatheory.ProofSystemsEFIPS

mkdir -p .lake/build/lib/lean/VerumExternalReplay
cd -- "$replay"
lean -o "$scratch/.lake/build/lib/lean/VerumExternalReplay/GodelCantor.olean" VerumExternalReplay/GodelCantor.lean
export LEAN_PATH="$scratch/.lake/build/lib/lean"
lean corpora/RealPACantor.lean
lean corpora/MathFoundationsM1.lean
