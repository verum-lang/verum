#!/bin/bash
# Test script for terminology migration
#
# This creates a temporary test environment to validate the migration script
# without affecting the actual codebase.

set -euo pipefail

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Test cases
TEST_DIR="/tmp/verum_migration_test_$$"

setup_test_env() {
    log_info "Setting up test environment: $TEST_DIR"
    mkdir -p "$TEST_DIR/crates/test_crate/src"

    # Create test Rust files
    cat > "$TEST_DIR/crates/test_crate/src/effects.rs" << 'EOF'
//! Effect System for Verum

use std::collections::HashSet;

/// Effect in the Verum effect system
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Effect {
    Pure,
    IO,
    Async,
}

/// Effect set - collection of effects
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSet {
    effects: HashSet<Effect>,
}

impl EffectSet {
    pub fn pure() -> Self {
        let mut effects = HashSet::new();
        effects.insert(Effect::Pure);
        EffectSet { effects }
    }

    pub fn single(effect: Effect) -> Self {
        let mut effects = HashSet::new();
        effects.insert(effect);
        EffectSet { effects }
    }

    pub fn has_effect(&self, effect: &Effect) -> bool {
        self.effects.contains(effect)
    }

    pub fn add_effect(&mut self, effect: Effect) {
        self.effects.insert(effect);
    }

    pub fn get_effects(&self) -> &HashSet<Effect> {
        &self.effects
    }
}

/// Effect inference context
pub struct EffectInferenceContext {
    current_effects: EffectSet,
}

impl EffectInferenceContext {
    pub fn new() -> Self {
        EffectInferenceContext {
            current_effects: EffectSet::pure(),
        }
    }

    pub fn add_effect(&mut self, effect: Effect) {
        self.current_effects.add_effect(effect);
    }

    pub fn take_effects(&mut self) -> EffectSet {
        std::mem::replace(&mut self.current_effects, EffectSet::pure())
    }
}

// This should NOT be replaced - "side effect" is legitimate CS terminology
/// Pure functions have no side effects
pub fn is_pure_function(f: &str) -> bool {
    // Check if function has side effects
    !f.contains("println") && !f.contains("write")
}
EOF

    cat > "$TEST_DIR/crates/test_crate/src/lib.rs" << 'EOF'
pub mod effects;

pub use effects::{Effect, EffectSet, EffectInferenceContext};

// Test that we don't break this
fn foo() {
    // Pure computation with no side effects
    let x = 42;
    let _y = x + 1;
}
EOF

    log_success "Test environment created"
}

run_migration_test() {
    log_info "Running migration on test files..."

    # Copy the script to test dir
    cp "$(dirname "${BASH_SOURCE[0]}")/migrate_effect_to_context.sh" "$TEST_DIR/"

    cd "$TEST_DIR"

    # Run dry-run first
    log_info "Testing dry-run mode..."
    if bash migrate_effect_to_context.sh --dry-run --no-backup > /tmp/migration_dry_run.log 2>&1; then
        log_success "Dry-run completed successfully"
    else
        log_error "Dry-run failed"
        cat /tmp/migration_dry_run.log
        return 1
    fi

    # Run actual migration
    log_info "Running actual migration..."
    if bash migrate_effect_to_context.sh --no-backup > /tmp/migration_run.log 2>&1; then
        log_success "Migration completed"
    else
        log_error "Migration failed"
        cat /tmp/migration_run.log
        return 1
    fi
}

verify_results() {
    log_info "Verifying migration results..."

    local errors=0

    # Check that effects.rs was renamed to contexts.rs
    if [[ ! -f "$TEST_DIR/crates/test_crate/src/contexts.rs" ]]; then
        log_error "contexts.rs not created"
        ((errors++))
    else
        log_success "File renamed: effects.rs → contexts.rs"
    fi

    # Check that imports were updated in lib.rs
    if grep -q "pub mod contexts" "$TEST_DIR/crates/test_crate/src/lib.rs"; then
        log_success "Import updated in lib.rs"
    else
        log_error "Import not updated in lib.rs"
        ((errors++))
    fi

    # Check that types were renamed
    if grep -q "pub enum Context" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "Effect → Context renamed"
    else
        log_error "Effect not renamed to Context"
        ((errors++))
    fi

    if grep -q "pub struct ContextSet" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "EffectSet → ContextSet renamed"
    else
        log_error "EffectSet not renamed to ContextSet"
        ((errors++))
    fi

    if grep -q "pub struct ContextInferenceEngine" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "EffectInferenceContext → ContextInferenceEngine renamed"
    else
        log_error "EffectInferenceContext not renamed"
        ((errors++))
    fi

    # Check that functions were renamed
    if grep -q "pub fn has_context" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "has_effect → has_context renamed"
    else
        log_error "has_effect not renamed"
        ((errors++))
    fi

    if grep -q "pub fn add_context" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "add_effect → add_context renamed"
    else
        log_error "add_effect not renamed"
        ((errors++))
    fi

    # Check that "side effect" was NOT replaced
    if grep -q "no side effects" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "'side effect' terminology preserved"
    else
        log_error "'side effect' was incorrectly replaced"
        ((errors++))
    fi

    # Check that comment "side effects" was preserved
    if grep -q "side effects" "$TEST_DIR/crates/test_crate/src/contexts.rs"; then
        log_success "Comment 'side effects' preserved"
    else
        log_error "Comment 'side effects' incorrectly changed"
        ((errors++))
    fi

    return $errors
}

cleanup() {
    if [[ -d "$TEST_DIR" ]]; then
        log_info "Cleaning up test environment..."
        rm -rf "$TEST_DIR"
        log_success "Cleanup complete"
    fi
}

main() {
    echo -e "${GREEN}════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Migration Script Test Suite          ${NC}"
    echo -e "${GREEN}════════════════════════════════════════${NC}"
    echo ""

    # Trap cleanup on exit
    trap cleanup EXIT

    # Run tests
    setup_test_env
    run_migration_test

    echo ""
    log_info "Verification phase..."
    echo ""

    if verify_results; then
        echo ""
        echo -e "${GREEN}════════════════════════════════════════${NC}"
        echo -e "${GREEN}  ✓ All tests passed!                   ${NC}"
        echo -e "${GREEN}════════════════════════════════════════${NC}"
        echo ""
        log_info "Test files location: $TEST_DIR"
        log_info "Review the migrated files to see the changes:"
        echo ""
        echo "  cat $TEST_DIR/crates/test_crate/src/contexts.rs"
        echo "  cat $TEST_DIR/crates/test_crate/src/lib.rs"
        echo ""
        exit 0
    else
        echo ""
        echo -e "${RED}════════════════════════════════════════${NC}"
        echo -e "${RED}  ✗ Some tests failed                   ${NC}"
        echo -e "${RED}════════════════════════════════════════${NC}"
        echo ""
        log_error "Review the test output above for details"
        exit 1
    fi
}

main "$@"
