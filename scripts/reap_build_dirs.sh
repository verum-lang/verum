#!/usr/bin/env bash
# Reap stale build-script output directories (T0968).
#
# Cargo never deletes a build script's OUT_DIR.  Every recompile of a
# build-script crate mints a fresh `<crate>-<hash>/` beside the old ones,
# so a long-lived target directory accumulates dozens of copies of the
# same crate's output.  Measured on this repo: 22 copies of `zstd-sys`,
# 15 of `z3-sys`, 87 GB of the 119 GB in one target tree.
#
# The failure this prevents is not "disk is full" — it is what a full
# disk LOOKS like here.  A truncated write into the embedded stdlib
# metadata blob surfaces as
#   "embeds stdlib typecheck metadata that failed to decode"
# which reads as a serialisation defect, not as an out-of-space error.
#
# Keeps the KEEP newest directories per crate, deletes the rest.
#
#   scripts/reap_build_dirs.sh [--keep N] [--dry-run] [TARGET_DIR ...]
#
# With no TARGET_DIR, reaps every `build/` directory under ./target.
# REFUSES to touch a tree that another process wrote to in the last
# 20 minutes — a concurrent cargo owns those directories.

set -uo pipefail   # not -e: an empty glob or a vanished directory is normal here

KEEP=4
DRY_RUN=0
TARGETS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --keep)    KEEP="$2"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help)
            sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)         TARGETS+=("$1"); shift ;;
    esac
done

if [[ ${#TARGETS[@]} -eq 0 ]]; then
    TARGETS=(target)
fi

total_freed=0

for root in "${TARGETS[@]}"; do
    if [[ ! -d "$root" ]]; then
        echo "skip (no such directory): $root" >&2
        continue
    fi

    # A tree written to very recently belongs to a running build.
    #
    # `-mmin` and not `-newermt`: on this machine `find` resolves to
    # `bfs`, which rejects `-newermt '-20 minutes'` as an invalid
    # timestamp and prints the error on stdout — so the guard fired on
    # its own instrument's error message rather than on a real writer.
    # Positive control: the same predicate with a large window must
    # return files, otherwise the guard cannot see a writer at all.
    if [[ -z "$(find "$root" -type f -mmin -20000 2>/dev/null | head -1)" ]]; then
        echo "SKIP $root — the freshness probe finds no files at all; refusing to reap blind" >&2
        continue
    fi
    recent=$(find "$root" -type f -mmin -20 2>/dev/null | head -1)
    if [[ -n "$recent" ]]; then
        echo "SKIP $root — written to in the last 20 minutes (a build owns it): $recent" >&2
        continue
    fi

    while IFS= read -r build_dir; do
        # Layout is `<build>/<crate>/<hash>/`: cargo keeps one directory
        # per crate, holding one subdirectory per build-script run.  Keep
        # the KEEP newest hashes per crate, drop the rest.
        # (`mapfile` is bash 4+; macOS ships bash 3.2.)
        while IFS= read -r crate_dir; do
            [ -z "$crate_dir" ] && continue
            stale=$(ls -1dt "$crate_dir"/*/ 2>/dev/null | tail -n "+$((KEEP + 1))")
            while IFS= read -r dir; do
                [ -z "$dir" ] && continue
                size=$(du -sk "$dir" 2>/dev/null | cut -f1 || echo 0)
                total_freed=$((total_freed + size))
                if [ "$DRY_RUN" -eq 1 ]; then
                    echo "would remove $dir ($((size / 1024)) MB)"
                else
                    rm -rf "$dir"
                fi
            done <<< "$stale"
        done <<< "$(find "$build_dir" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)"
    done < <(find "$root" -type d -name build -maxdepth 4 2>/dev/null)
done

if [[ $DRY_RUN -eq 1 ]]; then
    echo "would free $((total_freed / 1024)) MB (keeping $KEEP newest per crate)"
else
    echo "freed $((total_freed / 1024)) MB (kept $KEEP newest per crate)"
fi
