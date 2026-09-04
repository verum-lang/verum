#!/bin/zsh
# Sweep core/ with `verum check` and KEEP the diagnostics.
#
# scripts/ci/check_core_compiles.sh answers "which files fail" and
# throws the reasons away, which is the right shape for a gate and the
# wrong one for a diagnosis. This keeps every line, so
# classify_no_method.py can split the failures by CAUSE afterwards
# without a second 45-minute pass.
#
# USAGE
#   sweep_core.sh <verum-binary> <out-dir> [file-list]
#
# Resumable: a file whose .out already exists is skipped, so a killed
# run continues where it stopped rather than starting over.
#
# THREE THINGS IT REFUSES TO DO SILENTLY, each paid for by a real
# session:
#
#  1. A file that produces NO OUTPUT AT ALL is not a pass. A full disk
#     and an exhausted swap both kill this binary without a message,
#     and the resulting run reports a number that means nothing. Those
#     land in `mute.txt` and the summary states the count first.
#  2. Disk is sampled DURING the sweep, not after — a transient trough
#     is invisible to an endpoint reading, and a trough is what
#     corrupts a run.
#  3. The binary is stated in the summary. "Measured on tdiag" names
#     nothing: that path holds v32, v33, v34 and v35 in turn.
#
#  4. A SWEEP OVER A SHARED CORPUS IS A SMEAR, NOT A SNAPSHOT. Other
#     sessions repair core/ while this runs — measured 2026-09-04, the
#     known-failures baseline moved 345 -> 310 between file 1 and file
#     700 of one pass. A file scanned early was measured against
#     different source than one scanned late, so the result is stale in
#     a knowable direction rather than wrong.
#
#     THE FIX IS CHEAP AND IS PART OF THE PROCEDURE: when the pass
#     finishes, re-run it over `failed.txt` ALONE (a few hundred files,
#     minutes). That list is then consistent at one instant, and the
#     difference between the two passes measures what the other
#     sessions repaired while you watched.
#
#         census_core_check_diagnostics.sh <bin> <out2> <out1>/failed.txt
#
set -uo pipefail
V=${1:?usage: sweep_core.sh <verum-binary> <out-dir> [file-list]}
OUT=${2:?usage: sweep_core.sh <verum-binary> <out-dir> [file-list]}
REPO=/Users/taaliman/projects/oldman/verum-lang/verum

if [ ! -x "$V" ]; then echo "PROBE-FAILED: no binary at '$V'"; exit 2; fi
mkdir -p "$OUT"
: > "$OUT/mute.txt"
: > "$OUT/failed.txt"
: > "$OUT/all_diagnostics.txt"

echo "BINARY: $V"
echo "        $(ls -la $V | awk '{print $6, $7, $8, $5}')"
echo "DISK:   $(df -h /private/tmp | tail -1 | awk '{print $4}') free at start"
echo

if [ $# -ge 3 ] && [ -f "$3" ]; then
    files=("${(@f)$(cat $3)}")
else
    files=("${(@f)$(cd $REPO && find core -name '*.vr' | sort)}")
fi
total=${#files[@]}
echo "sweeping $total files"

i=0
min_disk=""   # empty until SAMPLED — an unmeasured value must not print as a measurement
for f in $files; do
    i=$((i + 1))
    key=${f//\//__}
    dest="$OUT/$key.out"
    [ -s "$dest" ] && continue
    out=$(cd $REPO && timeout 180 "$V" check "$f" 2>&1)
    rc=$?
    print -r -- "$out" > "$dest"
    # A run that says nothing did not answer the question.
    if ! print -r -- "$out" | grep -q 'Checking\|Finished\|error'; then
        echo "$f" >> "$OUT/mute.txt"
    elif [ $rc -ne 0 ]; then
        echo "$f" >> "$OUT/failed.txt"
        { echo "=== $f"; print -r -- "$out"; } >> "$OUT/all_diagnostics.txt"
    fi
    if (( i % 100 == 0 )); then
        free=$(df -m /private/tmp | tail -1 | awk '{print $4}')
        if [ -z "$min_disk" ] || (( free < min_disk )); then min_disk=$free; fi
        echo "  $i/$total  failed=$(wc -l < $OUT/failed.txt | tr -d ' ')  mute=$(wc -l < $OUT/mute.txt | tr -d ' ')  disk=${free}MB"
        if (( free < 2000 )); then
            echo "  ABORTING: free disk below 2GB — a sweep on a full disk reports nothing useful"
            exit 3
        fi
    fi
done

nm=$(wc -l < "$OUT/mute.txt" | tr -d ' ')
nf=$(wc -l < "$OUT/failed.txt" | tr -d ' ')
echo
echo "MUTE (no output at all — NOT passes): $nm"
[ "$nm" -gt 0 ] && head -5 "$OUT/mute.txt" | sed 's/^/    /'
echo "FAILED: $nf of $total"
if [ -n "$min_disk" ]; then
    echo "disk never went below ${min_disk}MB during the sweep"
else
    echo "disk: NOT SAMPLED (fewer than 100 files) — no claim made"
fi
echo
echo "Now: python3 scripts/ci/census_classify_no_method_diagnostics.py \\"
echo "         $OUT/all_diagnostics.txt --repo $REPO"
