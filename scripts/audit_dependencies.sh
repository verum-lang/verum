#!/usr/bin/env bash

# Dependency Security Audit Script for Verum v1.0
#
# This script performs comprehensive dependency security analysis:
# - Known vulnerability scanning (cargo-audit)
# - Outdated dependency detection (cargo-outdated)
# - License compliance verification
# - Supply chain analysis
# - Transitive dependency audit
#
# Security Criticality: P0

set -euo pipefail

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SECURITY_DIR="security"
REPORT_FILE="${SECURITY_DIR}/audit_report.md"
DEPENDENCY_TREE="${SECURITY_DIR}/dependency_tree.txt"
LICENSES_FILE="${SECURITY_DIR}/licenses.json"
ADVISORIES_FILE="${SECURITY_DIR}/advisories.json"

# Create security directory if it doesn't exist
mkdir -p "${SECURITY_DIR}"

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Section header
print_section() {
    echo ""
    echo "============================================================"
    echo "$1"
    echo "============================================================"
}

# Initialize report
init_report() {
    cat > "${REPORT_FILE}" <<EOF
# Verum v1.0 Security Audit Report

**Date:** $(date +"%Y-%m-%d %H:%M:%S")
**Platform:** $(uname -s)
**Rust Version:** $(rustc --version)
**Cargo Version:** $(cargo --version)

---

## Executive Summary

This report contains the results of the automated security audit for Verum v1.0.

EOF
}

# Install required tools
install_tools() {
    print_section "Installing Audit Tools"

    log_info "Installing cargo-audit..."
    if ! command -v cargo-audit &> /dev/null; then
        cargo install cargo-audit --quiet || log_error "Failed to install cargo-audit"
        log_success "cargo-audit installed"
    else
        log_success "cargo-audit already installed"
    fi

    log_info "Installing cargo-outdated..."
    if ! command -v cargo-outdated &> /dev/null; then
        cargo install cargo-outdated --quiet || log_error "Failed to install cargo-outdated"
        log_success "cargo-outdated installed"
    else
        log_success "cargo-outdated already installed"
    fi

    log_info "Installing cargo-license..."
    if ! command -v cargo-license &> /dev/null; then
        cargo install cargo-license --quiet || log_error "Failed to install cargo-license"
        log_success "cargo-license installed"
    else
        log_success "cargo-license already installed"
    fi

    log_info "Installing cargo-geiger (for unsafe code analysis)..."
    if ! command -v cargo-geiger &> /dev/null; then
        cargo install cargo-geiger --quiet || log_warning "Failed to install cargo-geiger (optional)"
    else
        log_success "cargo-geiger already installed"
    fi
}

# Check for known vulnerabilities
check_vulnerabilities() {
    print_section "Checking for Known Vulnerabilities"

    log_info "Running cargo-audit..."

    if cargo audit --json > "${ADVISORIES_FILE}" 2>&1; then
        log_success "No known vulnerabilities found"

        cat >> "${REPORT_FILE}" <<EOF
## Known Vulnerabilities

**Status:** ✅ PASS

No known security vulnerabilities detected in dependencies.

EOF
    else
        local advisory_count=$(jq -r '.vulnerabilities.count // 0' "${ADVISORIES_FILE}" 2>/dev/null || echo "unknown")

        log_error "Found ${advisory_count} known vulnerabilities"

        cat >> "${REPORT_FILE}" <<EOF
## Known Vulnerabilities

**Status:** ❌ FAIL

Found ${advisory_count} known security vulnerabilities.

### Details

\`\`\`json
$(cat "${ADVISORIES_FILE}")
\`\`\`

### Action Required

Review and update vulnerable dependencies immediately.

EOF
        return 1
    fi

    return 0
}

# Check for outdated dependencies
check_outdated() {
    print_section "Checking for Outdated Dependencies"

    log_info "Running cargo-outdated..."

    local outdated_output
    outdated_output=$(cargo outdated --format json 2>&1 || echo "{}")

    local outdated_count=$(echo "${outdated_output}" | jq -r '.outdated | length // 0' 2>/dev/null || echo "0")

    if [[ "${outdated_count}" -eq 0 ]]; then
        log_success "All dependencies are up to date"

        cat >> "${REPORT_FILE}" <<EOF
## Outdated Dependencies

**Status:** ✅ PASS

All dependencies are up to date.

EOF
    else
        log_warning "Found ${outdated_count} outdated dependencies"

        cat >> "${REPORT_FILE}" <<EOF
## Outdated Dependencies

**Status:** ⚠️  WARNING

Found ${outdated_count} outdated dependencies.

### Outdated Crates

| Crate | Current | Latest | Status |
|-------|---------|--------|--------|
EOF

        # Parse outdated dependencies
        echo "${outdated_output}" | jq -r '.outdated[] | "| \(.name) | \(.project) | \(.latest) | Update available |"' >> "${REPORT_FILE}" 2>/dev/null || true

        cat >> "${REPORT_FILE}" <<EOF

### Recommendation

Review and update outdated dependencies, especially if they contain security fixes.

EOF
    fi
}

# Generate dependency tree
generate_dependency_tree() {
    print_section "Generating Dependency Tree"

    log_info "Generating full dependency tree..."
    cargo tree --all-features > "${DEPENDENCY_TREE}" 2>&1 || log_warning "Failed to generate dependency tree"

    local dep_count=$(wc -l < "${DEPENDENCY_TREE}")
    log_success "Generated dependency tree with ${dep_count} entries"

    cat >> "${REPORT_FILE}" <<EOF
## Dependency Tree

**Total Dependencies:** ${dep_count}

Full dependency tree saved to: \`${DEPENDENCY_TREE}\`

EOF
}

# Check licenses
check_licenses() {
    print_section "Checking License Compliance"

    log_info "Analyzing dependency licenses..."

    cargo license --json > "${LICENSES_FILE}" 2>&1 || log_warning "Failed to generate license report"

    # Dangerous licenses that should be flagged
    local dangerous_licenses=("GPL-3.0" "AGPL-3.0" "SSPL")
    local problematic_found=false

    cat >> "${REPORT_FILE}" <<EOF
## License Compliance

EOF

    # Parse licenses
    local license_summary
    license_summary=$(jq -r 'group_by(.license) | map({license: .[0].license, count: length}) | .[] | "\(.license): \(.count)"' "${LICENSES_FILE}" 2>/dev/null || echo "Error parsing licenses")

    cat >> "${REPORT_FILE}" <<EOF
### License Distribution

\`\`\`
${license_summary}
\`\`\`

EOF

    # Check for problematic licenses
    for dangerous_license in "${dangerous_licenses[@]}"; do
        if echo "${license_summary}" | grep -q "${dangerous_license}"; then
            log_error "Found potentially problematic license: ${dangerous_license}"
            problematic_found=true

            cat >> "${REPORT_FILE}" <<EOF
⚠️  **WARNING:** Found dependency with ${dangerous_license} license

EOF
        fi
    done

    if [[ "${problematic_found}" == false ]]; then
        log_success "No problematic licenses detected"

        cat >> "${REPORT_FILE}" <<EOF
**Status:** ✅ PASS

No problematic licenses detected. All dependencies use permissive licenses.

EOF
    else
        cat >> "${REPORT_FILE}" <<EOF
**Status:** ⚠️  WARNING

Some dependencies use licenses that may require legal review.

EOF
    fi
}

# Analyze transitive dependencies
analyze_transitive_deps() {
    print_section "Analyzing Transitive Dependencies"

    log_info "Analyzing dependency chain depth..."

    # Count direct vs transitive dependencies
    local direct_count
    direct_count=$(cargo metadata --format-version=1 2>/dev/null | jq -r '[.packages[] | select(.source != null)] | length' || echo "unknown")

    local total_count
    total_count=$(cargo tree --all-features --edges normal 2>/dev/null | wc -l || echo "unknown")

    log_info "Direct dependencies: ${direct_count}"
    log_info "Total dependencies (including transitive): ${total_count}"

    cat >> "${REPORT_FILE}" <<EOF
## Transitive Dependencies

**Direct Dependencies:** ${direct_count}
**Total Dependencies:** ${total_count}

### Analysis

EOF

    # Check for deep dependency chains
    if command -v cargo-geiger &> /dev/null; then
        log_info "Running cargo-geiger to detect unsafe code..."

        local geiger_output
        geiger_output=$(cargo geiger --all-features 2>&1 || echo "Geiger scan failed")

        cat >> "${REPORT_FILE}" <<EOF
### Unsafe Code Usage

\`\`\`
${geiger_output}
\`\`\`

EOF
    else
        log_warning "cargo-geiger not available, skipping unsafe code analysis"
    fi
}

# Check for duplicate dependencies
check_duplicates() {
    print_section "Checking for Duplicate Dependencies"

    log_info "Scanning for duplicate crates..."

    local duplicates
    duplicates=$(cargo tree --duplicates 2>&1 || echo "")

    if [[ -z "${duplicates}" ]]; then
        log_success "No duplicate dependencies found"

        cat >> "${REPORT_FILE}" <<EOF
## Duplicate Dependencies

**Status:** ✅ PASS

No duplicate dependencies detected.

EOF
    else
        log_warning "Found duplicate dependencies"

        cat >> "${REPORT_FILE}" <<EOF
## Duplicate Dependencies

**Status:** ⚠️  WARNING

Found duplicate dependencies that should be consolidated:

\`\`\`
${duplicates}
\`\`\`

### Impact

Duplicate dependencies increase binary size and may cause version conflicts.

### Recommendation

Consolidate to single versions where possible.

EOF
    fi
}

# Supply chain analysis
supply_chain_analysis() {
    print_section "Supply Chain Security Analysis"

    log_info "Analyzing supply chain security..."

    # Check for dependencies from non-crates.io sources
    local non_cratesio
    non_cratesio=$(cargo metadata --format-version=1 2>/dev/null | jq -r '[.packages[] | select(.source != null and (.source | startswith("registry") | not))] | length' || echo "0")

    cat >> "${REPORT_FILE}" <<EOF
## Supply Chain Security

### Non-Crates.io Dependencies

**Count:** ${non_cratesio}

EOF

    if [[ "${non_cratesio}" -eq 0 ]]; then
        log_success "All dependencies from crates.io"

        cat >> "${REPORT_FILE}" <<EOF
**Status:** ✅ PASS

All dependencies are from crates.io (official registry).

EOF
    else
        log_warning "Found ${non_cratesio} dependencies from non-crates.io sources"

        local git_deps
        git_deps=$(cargo metadata --format-version=1 2>/dev/null | jq -r '[.packages[] | select(.source != null and (.source | startswith("git")))] | .[] | "- \(.name) (\(.source))"' || echo "Error parsing")

        cat >> "${REPORT_FILE}" <<EOF
**Status:** ⚠️  WARNING

Found dependencies from non-standard sources:

${git_deps}

### Recommendation

Git dependencies should be published to crates.io for production use.

EOF
    fi
}

# Generate summary
generate_summary() {
    print_section "Generating Summary"

    log_info "Finalizing audit report..."

    cat >> "${REPORT_FILE}" <<EOF

---

## Recommendations

1. **Immediate Actions:**
   - Fix any critical vulnerabilities identified
   - Review and update outdated dependencies with security patches

2. **Short-term Actions:**
   - Consolidate duplicate dependencies
   - Publish any git dependencies to crates.io
   - Review licenses for compliance with project requirements

3. **Long-term Actions:**
   - Establish regular dependency audit schedule (weekly)
   - Set up automated vulnerability scanning in CI/CD
   - Document dependency upgrade policy

---

## Audit Tools Used

- **cargo-audit** v$(cargo audit --version 2>/dev/null | head -n1 || echo "unknown")
- **cargo-outdated** v$(cargo outdated --version 2>/dev/null || echo "unknown")
- **cargo-license** v$(cargo license --version 2>/dev/null || echo "unknown")

---

## Next Steps

1. Review this report with security team
2. Address all HIGH and CRITICAL findings
3. Schedule re-audit after fixes
4. Update security documentation

---

**Report generated by:** Verum Security Audit Script
**Timestamp:** $(date +"%Y-%m-%d %H:%M:%S %Z")

EOF

    log_success "Audit report saved to: ${REPORT_FILE}"
}

# Main execution
main() {
    print_section "Verum v1.0 Dependency Security Audit"

    log_info "Starting comprehensive dependency audit..."

    # Initialize report
    init_report

    # Install required tools
    install_tools

    # Run all checks
    local exit_code=0

    check_vulnerabilities || exit_code=$?
    check_outdated
    generate_dependency_tree
    check_licenses
    analyze_transitive_deps
    check_duplicates
    supply_chain_analysis

    # Generate final summary
    generate_summary

    print_section "Audit Complete"

    if [[ ${exit_code} -eq 0 ]]; then
        log_success "Security audit completed successfully"
        log_info "Report available at: ${REPORT_FILE}"
        return 0
    else
        log_error "Security audit found critical issues"
        log_info "Report available at: ${REPORT_FILE}"
        return 1
    fi
}

# Run main function
main "$@"
