#!/bin/bash
# Fix semantic types violations in verum_runtime
# HashMap → Map, HashSet → Set

set -e

cd "$(dirname "$0")/.."

echo "🔧 Fixing semantic types in verum_runtime..."
echo ""

# List of files with HashMap/HashSet violations
FILES=(
  "crates/verum_runtime/src/gc/tracing.rs"
  "crates/verum_runtime/src/gc/refcount.rs"
  "crates/verum_runtime/src/environment/errors.rs"
  "crates/verum_runtime/src/environment/contexts.rs"
  "crates/verum_runtime/src/jit/intrinsics.rs"
  "crates/verum_runtime/src/jit/tier2.rs"
  "crates/verum_runtime/src/jit/profiler.rs"
  "crates/verum_runtime/src/jit/inline.rs"
  "crates/verum_runtime/src/jit/mod.rs"
  "crates/verum_runtime/src/jit/cbgr_checks.rs"
  "crates/verum_runtime/src/jit/lazy_compiler.rs"
  "crates/verum_runtime/src/memory/numa.rs"
  "crates/verum_runtime/src/memory/monitor.rs"
  "crates/verum_runtime/src/memory/allocator_map.rs"
  "crates/verum_runtime/src/memory/pool.rs"
  "crates/verum_runtime/src/memory/arena.rs"
  "crates/verum_runtime/src/panic/mod.rs"
  "crates/verum_runtime/src/references/checked_ref.rs"
  "crates/verum_runtime/src/references/managed_ref.rs"
  "crates/verum_runtime/src/references/unsafe_ref.rs"
  "crates/verum_runtime/src/thread_pool.rs"
  "crates/verum_runtime/src/execution/tier0_interpreter.rs"
  "crates/verum_runtime/src/execution/config.rs"
  "crates/verum_runtime/src/async_executor.rs"
  "crates/verum_runtime/src/channels/mod.rs"
  "crates/verum_runtime/src/embedding/host.rs"
)

FIXED_COUNT=0

for file in "${FILES[@]}"; do
  if [ ! -f "$file" ]; then
    echo "⚠️  Skip: $file (not found)"
    continue
  fi

  # Backup
  cp "$file" "${file}.bak"

  # Replace HashMap imports
  if grep -q "use std::collections::HashMap" "$file"; then
    sed -i '' 's/use std::collections::HashMap/use verum_std::core::Map/g' "$file"
    echo "  ✅ Fixed HashMap import: $file"
    ((FIXED_COUNT++))
  fi

  # Replace HashSet imports
  if grep -q "use std::collections::HashSet" "$file"; then
    sed -i '' 's/use std::collections::HashSet/use verum_std::core::Set/g' "$file"
    echo "  ✅ Fixed HashSet import: $file"
    ((FIXED_COUNT++))
  fi

  # Replace HashMap types (preserve spacing)
  sed -i '' 's/HashMap</Map</g' "$file"
  sed -i '' 's/HashMap>/Map>/g' "$file"

  # Replace HashSet types
  sed -i '' 's/HashSet</Set</g' "$file"
  sed -i '' 's/HashSet>/Set>/g' "$file"

  # Remove backup if no changes
  if diff -q "$file" "${file}.bak" > /dev/null 2>&1; then
    rm "${file}.bak"
  fi
done

echo ""
echo "✅ Fixed $FIXED_COUNT files"
echo ""
echo "🔨 Verifying compilation..."
cd crates/verum_runtime
cargo check 2>&1 | grep -E "(Compiling|Finished|error)" | head -20

echo ""
echo "✅ Semantic types fixed in verum_runtime"
