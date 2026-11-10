#!/usr/bin/env bash
# Check versioned and new (nonignored) sources; run from the repository root.
set -eu
inventory=$(mktemp)
trap 'rm -f "$inventory"' EXIT
git ls-files --cached --others --exclude-standard -z > "$inventory"
bad=0
while IFS= read -r -d '' path; do
    case "$path" in
        # Host-only bindings for upstream compiler APIs. Keep this list exact:
        # adding a runtime implementation alongside a binding must still fail.
        crates/llvm/verum_llvm_sys/wrappers/target.c|crates/llvm/verum_llvm_sys/src/lld/linker.cpp|\
        crates/llvm/verum_tblgen/cc/lib/Record.cpp|crates/llvm/verum_tblgen/cc/lib/RecordKeeper.cpp|crates/llvm/verum_tblgen/cc/lib/RecordVal.cpp|crates/llvm/verum_tblgen/cc/lib/TableGen.cpp|crates/llvm/verum_tblgen/cc/lib/Utility.cpp) continue ;;
        vcs/benchmarks/baselines/*) continue ;;
        *.[cC]|*.cc|*.cpp|*.cxx|*.c++|*.m|*.mm|core/*.bc|core/*.o|core/*.a|crates/*.bc|crates/*.o|crates/*.a)
            echo "Unexpected native source or runtime object: $path" >&2; bad=1 ;;
        crates/llvm/verum_llvm_sys/build.rs|crates/llvm/verum_tblgen/build.rs) continue ;;
        crates/*/build.rs|build.rs)
            if [[ -f "$path" ]] && grep -nE 'cc[[:space:]]*::[[:space:]]*Build|[-]emit-llvm|\.(c|cc|cpp|cxx)"' "$path"; then
                echo "Inspect native-source build in $path" >&2; bad=1
            fi ;;
    esac
done < "$inventory"
exit "$bad"
