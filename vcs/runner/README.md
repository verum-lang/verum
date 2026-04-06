# VCS Runner Tools

The Verum Compliance Suite (VCS) includes three command-line tools for testing, fuzzing, and benchmarking Verum implementations.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Tools Overview](#tools-overview)
  - [vtest - Test Runner](#vtest---test-runner)
  - [vfuzz - Fuzzer](#vfuzz---fuzzer)
  - [vbench - Benchmark Runner](#vbench---benchmark-runner)
- [Configuration](#configuration)
- [CI/CD Integration](#cicd-integration)

---

## Installation

### Prerequisites

- Rust 1.75+ (stable)
- Cargo
- LLVM 14+ (for fuzzing with cargo-fuzz)
- Z3 4.12+ (for SMT verification tests)

### Building from Source

```bash
# Build all VCS tools
cd vcs/runner
cargo build --release

# Install to PATH (optional)
cargo install --path vtest
cargo install --path vfuzz
cargo install --path vbench
```

### Verifying Installation

```bash
vtest --version
# vtest 0.1.0 - VCS Test Runner

vfuzz --version
# vfuzz 0.1.0 - VCS Fuzzer

vbench --version
# vbench 0.1.0 - VCS Benchmark Runner
```

---

## Quick Start

```bash
# Run all tests
cd vcs
vtest run

# Run only critical tests (L0)
vtest run --level L0

# Run fuzzing for 30 minutes
vfuzz --duration 30m

# Run benchmarks and compare to main branch
vbench run --baseline main
```

---

## Tools Overview

### vtest - Test Runner

The primary tool for executing VCS specification tests.

#### Basic Usage

```bash
# Run all tests in default paths
vtest run

# Run tests from specific directory
vtest run specs/L0-critical/

# Run single test file
vtest run specs/L0-critical/lexer/keywords/reserved_keywords.vr
```

#### Filtering Tests

```bash
# By level (L0-L4)
vtest run --level L0           # Critical only
vtest run --level L0,L1        # Critical and Core
vtest run --level all          # All levels

# By execution tier
vtest run --tier 0             # Interpreter only
vtest run --tier 3             # AOT compiled
vtest run --tier 0,3           # Compare interpreter vs AOT
vtest run --tier all           # All tiers

# By tags
vtest run --tags cbgr,memory-safety
vtest run --exclude-tags slow,gpu
```

#### Parallel Execution

```bash
# Specify thread count
vtest run --parallel 8

# Auto-detect CPU count
vtest run --parallel auto

# Single-threaded (for debugging)
vtest run --parallel 1
```

#### Output Formats

```bash
# Console output (default)
vtest run --format console

# JSON output for programmatic processing
vtest run --format json --output results.json

# JUnit XML for CI integration
vtest run --format junit --output results.xml

# HTML report
vtest run --format html --output report.html
```

#### Watch Mode

```bash
# Watch for file changes and re-run
vtest watch --path specs/

# Watch specific level
vtest watch --level L0 --path specs/L0-critical/
```

#### Listing Tests

```bash
# List all discovered tests
vtest list

# List with filters
vtest list --level L0 --tags cbgr

# JSON output
vtest list --format json
```

#### CLI Reference

```
vtest - VCS Test Runner

USAGE:
    vtest <COMMAND> [OPTIONS]

COMMANDS:
    run      Execute tests
    list     List discovered tests
    watch    Watch for changes and re-run
    report   Generate report from results
    help     Show help information

OPTIONS:
    -c, --config <FILE>     Configuration file [default: vcs.toml]
    -v, --verbose           Increase verbosity
    -q, --quiet             Suppress non-error output
        --no-color          Disable colored output
    -h, --help              Show help
    -V, --version           Show version

RUN OPTIONS:
    -l, --level <LEVELS>       Test levels: L0,L1,L2,L3,L4,all [default: all]
    -t, --tier <TIERS>         Execution tiers: 0,1,2,3,all [default: all]
        --tags <TAGS>          Include tests with these tags
        --exclude-tags <TAGS>  Exclude tests with these tags
    -p, --parallel <N>         Parallel test threads [default: auto]
        --timeout <MS>         Default timeout in milliseconds [default: 30000]
    -f, --format <FORMAT>      Output format: console,json,junit,html
    -o, --output <FILE>        Output file path
        --fail-fast            Stop on first failure
        --shuffle              Randomize test order
        --seed <SEED>          Random seed for shuffle
```

---

### vfuzz - Fuzzer

Property-based fuzzer for discovering edge cases and crashes.

#### Basic Usage

```bash
# Run fuzzing with defaults (1 hour)
vfuzz

# Specify duration
vfuzz --duration 30m
vfuzz --duration 2h
vfuzz --duration 24h
```

#### Fuzzing Targets

```bash
# All targets
vfuzz --targets all

# Specific targets
vfuzz --targets lexer
vfuzz --targets parser
vfuzz --targets cbgr
vfuzz --targets types

# Multiple targets
vfuzz --targets lexer,parser,cbgr
```

#### Corpus Management

```bash
# Use seed corpus
vfuzz --corpus seeds/

# Save crashes to directory
vfuzz --crashes crashes/

# Minimize corpus
vfuzz minimize --corpus seeds/ --output seeds-min/
```

#### Generation Strategies

```bash
# Type-aware generation (default)
vfuzz --generator type_aware

# Random bytes
vfuzz --generator random

# Grammar-based
vfuzz --generator grammar

# Mutation from corpus
vfuzz --generator mutation
```

#### CLI Reference

```
vfuzz - VCS Fuzzer

USAGE:
    vfuzz [OPTIONS]
    vfuzz <COMMAND> [OPTIONS]

COMMANDS:
    run         Run fuzzer (default)
    minimize    Minimize corpus
    reproduce   Reproduce crash from artifact
    help        Show help

OPTIONS:
    -c, --config <FILE>      Configuration file [default: vcs.toml]
    -d, --duration <DUR>     Fuzzing duration [default: 1h]
    -p, --parallel <N>       Parallel fuzzer threads [default: 4]
    -t, --targets <TARGETS>  Fuzz targets [default: all]
    -g, --generator <GEN>    Generator strategy [default: type_aware]
        --corpus <DIR>       Seed corpus directory [default: fuzz/seeds/]
        --crashes <DIR>      Crash output directory [default: fuzz/crashes/]
        --max-depth <N>      Maximum AST depth [default: 10]
        --max-len <N>        Maximum input length [default: 4096]
    -v, --verbose            Verbose output
    -h, --help               Show help
    -V, --version            Show version
```

---

### vbench - Benchmark Runner

Performance benchmarking and regression detection.

#### Basic Usage

```bash
# Run all benchmarks
vbench run

# Run specific benchmark suite
vbench run --suite compilation
vbench run --suite runtime
vbench run --suite memory
```

#### Comparison and Baselines

```bash
# Compare to baseline file
vbench compare --current results.json --baseline baseline.json

# Compare to git ref
vbench run --baseline main
vbench run --baseline v0.31.0
vbench run --baseline HEAD~5

# Set threshold for regression detection
vbench compare --threshold 10.0  # 10% threshold
```

#### Benchmark Suites

| Suite | Description |
|-------|-------------|
| `compilation` | Parse speed, typecheck speed, codegen speed |
| `runtime` | Function call overhead, allocation, async overhead |
| `memory` | Stack usage, heap fragmentation, reference size |
| `cbgr` | CBGR tier latency (Tier 0-3) |
| `comparison` | vs C, Rust, Go performance |

```bash
# Run specific suites
vbench run --suite compilation,runtime

# List available suites
vbench list-suites
```

#### Output Formats

```bash
# Console table (default)
vbench run --format console

# JSON for CI
vbench run --format json --output results.json

# Markdown report
vbench compare --format markdown --output comparison.md

# CSV for spreadsheets
vbench run --format csv --output results.csv
```

#### CLI Reference

```
vbench - VCS Benchmark Runner

USAGE:
    vbench <COMMAND> [OPTIONS]

COMMANDS:
    run          Run benchmarks
    compare      Compare results to baseline
    list-suites  List available benchmark suites
    help         Show help

OPTIONS:
    -c, --config <FILE>     Configuration file [default: vcs.toml]
    -v, --verbose           Verbose output
    -h, --help              Show help
    -V, --version           Show version

RUN OPTIONS:
    -s, --suite <SUITES>     Benchmark suites [default: all]
    -b, --baseline <REF>     Git ref for baseline comparison
    -i, --iterations <N>     Measurement iterations [default: 1000]
    -w, --warmup <N>         Warmup iterations [default: 100]
    -f, --format <FORMAT>    Output format: console,json,csv,markdown
    -o, --output <FILE>      Output file path

COMPARE OPTIONS:
        --current <FILE>     Current results file
        --baseline <FILE>    Baseline results file
        --threshold <PCT>    Regression threshold percentage [default: 10.0]
    -f, --format <FORMAT>    Output format: console,markdown,json
    -o, --output <FILE>      Output file path
```

---

## Configuration

VCS tools are configured via `vcs.toml` (or specified via `--config`).

### Configuration File Location

Tools search for configuration in this order:
1. `--config` argument
2. `./vcs.toml`
3. `./config/vcs.toml`
4. `$VCS_CONFIG` environment variable

### Example Configuration

See `config/vcs.toml` for the full configuration reference.

```toml
[discovery]
paths = ["specs/", "differential/"]
pattern = "*.vr"
exclude = ["**/skip/**", "**/wip/**"]

[execution]
parallel = 8
timeout_default = 30000
tier_default = "all"

[reporting]
format = "console"
colors = true
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VCS_CONFIG` | Configuration file path | `vcs.toml` |
| `VCS_PARALLEL` | Parallel threads | CPU count |
| `VCS_TIMEOUT` | Default timeout (ms) | `30000` |
| `VCS_VERBOSE` | Verbose output | `false` |
| `VCS_NO_COLOR` | Disable colors | `false` |

---

## CI/CD Integration

### GitHub Actions

VCS includes a pre-configured GitHub Actions workflow at `.github/workflows/vcs-ci.yml`.

```yaml
# Run VCS tests in your workflow
- name: Run VCS Tests
  run: |
    vtest run --level L0,L1 --format junit --output results.xml

- name: Publish Results
  uses: mikepenz/action-junit-report@v4
  with:
    report_paths: results.xml
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All tests passed |
| 1 | Test failures |
| 2 | Configuration error |
| 3 | Runtime error |
| 4 | Timeout |

### JUnit XML Output

For CI systems that support JUnit:

```bash
vtest run --format junit --output results.xml
```

### JSON Output Schema

```json
{
  "version": "1.0",
  "timestamp": "2025-12-31T14:30:00Z",
  "summary": {
    "total": 1247,
    "passed": 1238,
    "failed": 5,
    "skipped": 4,
    "pass_percentage": 99.3
  },
  "by_level": {
    "L0": { "passed": 312, "total": 312 },
    "L1": { "passed": 456, "total": 458 }
  },
  "tests": [
    {
      "name": "lexer/keywords/reserved_keywords.vr",
      "level": "L0",
      "status": "passed",
      "duration_ms": 2
    }
  ]
}
```

---

## Troubleshooting

### Common Issues

**Tests not discovered:**
```bash
# Verify paths in configuration
vtest list --verbose

# Check file pattern
vtest list --pattern "*.vr"
```

**Timeout errors:**
```bash
# Increase timeout
vtest run --timeout 60000

# Run single test for debugging
vtest run path/to/test.vr --parallel 1 --verbose
```

**Fuzzer crashes immediately:**
```bash
# Check available memory
free -h

# Reduce parallelism
vfuzz --parallel 1 --verbose
```

---

## Further Reading

- [VCS Specification](../docs/vcs-spec.md) - Full VCS specification document
- [Test File Format](../specs/README.md) - How to write VCS test files
- [CBGR Testing](../specs/L0-critical/ownership/cbgr/README.md) - Memory safety tests
