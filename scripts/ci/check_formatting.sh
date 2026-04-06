#!/usr/bin/env bash
# Check code formatting for CI
set -euo pipefail

echo "Checking code formatting..."

# Check Rust formatting
if ! cargo fmt --all -- --check; then
  echo "❌ Code formatting check failed"
  echo ""
  echo "Run 'cargo fmt' to fix formatting issues"
  exit 1
fi

echo "✅ All code is properly formatted"

# Check for trailing whitespace
echo ""
echo "Checking for trailing whitespace..."

if git grep -I --line-number '\s$' -- '*.rs' '*.toml' '*.md' | grep -v 'Binary file'; then
  echo "❌ Trailing whitespace found"
  exit 1
fi

echo "✅ No trailing whitespace found"

echo ""
echo "✅ All formatting checks passed!"
