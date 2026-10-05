#!/usr/bin/env bash
# Generated AOT dependency guard; the host CLI has a separate OS baseline policy.
# Exit 0 = dynamic boundary checked; 1 = forbidden imports; 2 = inspection/build error.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$SCRIPT_DIR/check_aot_dependencies.py" "$@"
