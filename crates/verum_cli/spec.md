# Verum CLI Specification & Implementation Roadmap

## Vision Statement

The Verum CLI is not just a build tool – it's the primary interface through which developers experience Verum's revolutionary features: refinement types, transparent memory model, and gradual verification. Every command must embody **semantic honesty**, making costs visible and empowering evidence-based optimization decisions.

## Core Design Principles

### 1. Semantic Honesty
Every operation must transparently report its costs:
- CBGR overhead: Show ~15ns per check in real-time
- Verification time: Display compile-time vs runtime tradeoffs
- Context overhead: Differentiate 0ns static vs 5-100ns dynamic DI
- Memory usage: Report actual allocations and escape analysis

### 2. Gradual Everything
Support the full spectrum from dynamic to static verification:
- Start safe with runtime checks (fast iteration)
- Profile performance bottlenecks
- Optimize hot paths with escape analysis
- Prove correctness where it matters

### 3. Evidence-Based Evolution
Features must be data-driven:
- Profile before optimizing
- Measure actual CBGR overhead
- Report verification costs upfront
- Show optimization opportunities, don't impose them

## Verum Terminology

Verum uses its own terminology that aligns with its philosophy of semantic honesty and transparency:

### Package Organization
- **Module**: A unit of code organization (not "crate")
- **Package**: A distributable unit of verified code
- **Workspace**: A collection of related modules
- **Registry**: The central package repository (packages.vr.lang)

### Build Artifacts
- **Tier 0-3 Artifacts**: Execution-level specific outputs
- **Verification Portfolio**: Complete package including proofs and profiles
- **CBGR Profile**: Performance characteristics of reference operations
- **Context Graph**: Dependency injection requirements

### Documentation
- **Verification Documentation**: Generated docs with proof status
- **Cost Annotations**: Inline performance characteristics
- **Semantic API**: Documentation focused on meaning, not implementation

### File Extensions
- `.vr`: Verum source files (or `.vr`, `.vr`)
- `.vxpkg`: Verum eXecutable Package
- `.vjit`: Verum JIT cache (Tier 1)
- `.aot`: Verum AOT compiled binary (Tier 2-3)

This terminology emphasizes transparency and verification-first thinking, distinguishing Verum from other languages while maintaining clarity for developers.

## Command Architecture

### Primary Commands

```bash
verum <command> [options] [args]

CORE COMMANDS:
  new       Create new Verum project with language profile
  init      Initialize Verum in existing directory

  check     Fast type checking without code generation
  build     Compile with specified tier (0-3) and verification
  run       Build and execute with runtime arguments
  test      Execute test suite with coverage reporting
  bench     Performance benchmarks with tier comparison

  verify    Formal verification with cost/benefit analysis
  profile   Analyze CBGR overhead and optimization opportunities
  analyze   Deep static analysis (escape, context, refinement)

  add       Add dependency with semantic versioning
  remove    Remove dependency and clean lockfile
  update    Update dependencies with compatibility check
  tree      Visualize dependency graph with duplicates

  publish   Publish to Verum package registry
  search    Search registry for packages
  login     Authenticate with registry

  doc       Generate documentation with cost annotations
  fmt       Format code according to style guide
  lint      Lint with Verum-specific checks

  lsp       Start Language Server Protocol daemon
  repl      Interactive Verum REPL (Tier 0 interpreter)

WORKSPACE COMMANDS:
  workspace list     List all workspace members
  workspace build    Build entire workspace
  workspace test     Test entire workspace
  workspace publish  Publish all workspace packages
```

### Command Options Structure

#### Global Options (All Commands)
```bash
--color <when>       Terminal color output (auto|always|never)
--verbose|-v         Increase verbosity (-vvv for max)
--quiet|-q           Suppress non-error output
--config <path>      Override config file location
--manifest <path>    Path to verum.toml
```

#### Build Options
```bash
--tier <0-3>         Execution tier (interpreter/JIT/AOT/optimized)
--profile <name>     Build profile (dev/release/verified)
--refs <mode>        Reference system (managed/checked/mixed)
--verify <level>     Verification level (none/runtime/proof)
--features <list>    Enable feature flags
--all-features       Enable all available features
--no-default         Disable default features
--target <triple>    Target architecture
--jobs|-j <n>        Parallel build jobs
```

## Project Structure

### Standard Project Layout
```
my-project/
├── verum.toml           # Project manifest (REQUIRED)
├── verum.lock           # Dependency lockfile
├── .gitignore           # VCS ignore patterns
├── README.md            # Project documentation
├── LICENSE              # License file
├── src/                 # Source code
│   ├── main.vr         # Entry point for applications
│   ├── lib.vr          # Entry point for libraries
│   └── modules/         # Additional modules
├── tests/               # Integration tests
│   └── integration.vr
├── benches/             # Performance benchmarks
│   └── performance.vr
├── examples/            # Example programs
│   └── usage.vr
├── docs/                # Additional documentation
└── target/              # Build artifacts (gitignored)
    ├── tier0/           # AST cache for interpreter
    ├── tier1/           # JIT compiled code
    ├── tier2/           # AOT debug binaries
    ├── tier3/           # AOT optimized binaries
    ├── cbgr-profile/    # CBGR profiling data
    └── verify-cache/    # Verification cache
```

### Configuration File: `verum.toml`

```toml
[package]
name = "my-app"
version = "0.1.0"
authors = ["Alice <alice@example.com>"]
edition = "2025"
description = "My Verum application"
license = "MIT"
repository = "https://github.com/example/my-app"

[language]
profile = "application"  # REQUIRED: application|systems|research
# application: No unsafe, refinements + runtime checks (80% users)
# systems: Full language including unsafe (15% users)
# research: Dependent types, formal proofs (5% users)

[dependencies]
http = "1.2.0"
json = { version = "2.0", features = ["derive"] }
async-runtime = { version = "3.0", optional = true }

[dev-dependencies]
test-utils = "0.1.0"
mock-framework = "1.0.0"

[build-dependencies]
cbgr-codegen = "0.1.0"

[features]
default = ["async"]
async = ["async-runtime"]
experimental = []

[profile.dev]
tier = 0                    # Interpreter for fast iteration
verification = "runtime"    # Runtime refinement checks
opt-level = 0               # No optimization
debug = true                # Full debug info
cbgr-checks = "all"         # All CBGR checks enabled

[profile.release]
tier = 2                    # AOT compilation
verification = "runtime"    # Keep safety by default
opt-level = 3               # Maximum optimization
debug = false               # No debug info
cbgr-checks = "optimized"   # Escape analysis optimization

[profile.verified]
tier = 2                    # AOT compilation
verification = "proof"      # Formal verification
opt-level = 2               # Balanced optimization
debug = "limited"           # Basic debug info
cbgr-checks = "proven"      # Only unproven checks

[workspace]
members = [
    "modules/core",
    "modules/client",
    "modules/server"
]

[lsp]
enable-cost-hints = true         # Show CBGR/verification costs
validation-mode = "incremental"  # Fast IDE feedback
auto-import = true               # Auto-import suggestions
format-on-save = true            # Format on file save

[registry]
index = "https://packages.vr.lang"
```

## Command Specifications

### 1. Project Initialization

#### `verum new`
Creates a new Verum project with proper structure and configuration.

```bash
verum new <name> [options]

Options:
  --type <profile>    Language profile (application|systems|research)
  --lib               Create library instead of binary
  --vcs <type>        Initialize VCS (git|none) [default: git]
  --template <name>   Use project template
  --path <dir>        Create in directory [default: ./<name>]

Examples:
  verum new hello-world
  verum new web-server --type application
  verum new os-kernel --type systems --vcs none
  verum new math-lib --lib --type research
```

**Implementation Requirements:**
- MUST enforce language profile selection (no default)
- Generate appropriate .gitignore for Verum projects
- Create minimal but complete verum.toml
- Include starter code demonstrating key features
- Set up test structure

#### `verum init`
Initializes Verum in an existing directory.

```bash
verum init [options]

Options:
  --type <profile>    Language profile (REQUIRED)
  --lib               Initialize as library
  --force             Overwrite existing verum.toml

Examples:
  verum init --type application
  verum init --lib --type systems
```

### 2. Build & Compilation

#### `verum build`
Multi-tier compilation with transparent cost reporting.

```bash
verum build [options]

Options:
  --tier <0-3>        Execution tier:
                      0: Interpreter (instant start, slower execution)
                      1: JIT compilation (balanced)
                      2: AOT compilation (production)
                      3: AOT optimized (maximum performance)

  --profile <name>    Build profile (dev|release|verified|custom)
  --refs <mode>       Reference system:
                      managed: CBGR checks (~15ns overhead)
                      checked: Static verification (0ns)
                      mixed: Smart selection (recommended)

  --verify <level>    Verification level:
                      none: No verification (unsafe!)
                      runtime: Runtime checks (default)
                      proof: Formal verification

  --features <list>   Comma-separated feature list
  --all-features      Enable all features
  --no-default        Disable default features
  --target <triple>   Cross-compilation target
  --release           Alias for --profile release
  --verbose           Show compilation details
  --timings           Display compilation timings
  --jobs <n>          Parallel compilation jobs

Examples:
  verum build
  verum build --tier 2 --release
  verum build --verify proof --profile verified
  verum build --refs checked --tier 3
```

**Output Format:**
```
[Tier 1 - JIT] Building my-app v0.1.0
[Analyzing] Type inference: 87ms for 5,234 LOC (60K LOC/s)
[CBGR] Escape analysis: 23 refs proven non-escaping
[Verifying] 15 refinements: 14 runtime, 1 proven
[Compiling] 8 modules in parallel
[Optimizing] Inlining: 234 functions, Vectorization: 12 loops
[Complete] Built in 1.23s

Performance Profile:
  CBGR overhead: ~15ns per check (1.2% total runtime)
  Verification: 156ms compile-time, 0ns runtime for proven
  Memory usage: 45MB peak, 12MB binary size

Optimization Opportunities:
  • convert_matrix(): Use &checked for 0ns overhead (-28ms)
  • validate_input(): Prove refinement at compile-time (-5μs/call)
  Run 'verum profile --memory' for detailed analysis
```

#### `verum check`
Fast type checking without code generation.

```bash
verum check [options]

Options:
  --strict            Enforce all refinements
  --workspace         Check entire workspace
  --verbose           Show inference details

Examples:
  verum check
  verum check --strict
  verum check --workspace
```

**Output Format:**
```
[Checking] my-app v0.1.0
[Types] Inferred 234 types in 87ms
[Refinements] 3 unverified: use @verify or runtime checks
[Contexts] Missing: Database in process_order() at main.vr:42
[Complete] Type checking passed (no codegen)

Suggestions:
  • Add 'using [Database]' to process_order() signature
  • Verify age > 0 refinement with @verify annotation
```

#### `verum run`
Build and execute with runtime arguments.

```bash
verum run [options] [-- args...]

Options:
  --tier <0-3>        Execution tier
  --profile <name>    Build profile
  --release           Use release profile
  --example <name>    Run example instead of main
  --bin <name>        Run specific binary

Examples:
  verum run
  verum run --release
  verum run -- --port 8080 --verbose
  verum run --example client-demo
```

### 3. Verification & Analysis

#### `verum verify`
Formal verification with cost transparency.

```bash
verum verify [options]

Options:
  --profile           Profile verification performance
  --show-cost         Display verification costs
  --compare-modes     Compare runtime vs proof costs
  --solver <name>     SMT solver (z3|cvc5|vampire)
  --timeout <secs>    Solver timeout per function
  --cache             Enable verification cache
  --interactive       Interactive proof mode

Examples:
  verum verify
  verum verify --profile --show-cost
  verum verify --compare-modes
  verum verify --solver cvc5 --timeout 30
```

**Output Format:**
```
[Verification Profile]
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Function Analysis:

  sort_algorithm() @ algorithms.vr:42
    Status: ✓ Verified
    Time: 2.3s (Z3 solver)
    Cost: 2.3s compile, 0ns runtime
    Alternative: @verify(runtime) = 0s compile, ~5μs/call

  complex_invariant() @ core.vr:156
    Status: ⚠ Timeout
    Time: 30s (timeout)
    Issue: Nonlinear arithmetic
    Suggestions:
      • Simplify to linear constraints
      • Add intermediate assertions
      • Use @verify(runtime) for now

Summary:
  Total: 47 functions
  Verified: 43 (91.5%)
  Runtime: 4 (8.5%)
  Failed: 0

  Total time: 34.2s
  Cache hits: 38/47 (80.9%)

Recommendations:
  High-value targets for proof:
    • transfer_funds(): Called 1M times/day
    • validate_signature(): Security critical
```

#### `verum profile`
Performance profiling with CBGR analysis.

```bash
verum profile [options] [target]

Options:
  --memory            CBGR memory profiling
  --cpu               CPU profiling
  --cache             Cache analysis
  --output <format>   Output format (text|json|flamegraph)

Examples:
  verum profile --memory
  verum profile --cpu src/main.vr
  verum profile --cache --output json
```

**Output Format:**
```
[CBGR Memory Profile]
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Hot Spots (>1% runtime):

1. matrix_multiply() @ compute.vr:234
   Total time: 456ms
   CBGR checks: 234ms (51.3%)
   Average check: 15.2ns
   Check count: 15.4M

   Recommendation: Convert to &checked
     Impact: -234ms (-51.3% runtime)
     Safety: Proven by escape analysis
     Change: &Matrix → &checked Matrix

2. parse_json() @ parser.vr:89
   Total time: 123ms
   CBGR checks: 8ms (6.5%)

   Recommendation: Keep as-is
     Reason: Overhead negligible (<10%)

Tier Performance:
  Current (Tier 1): 15.2ns/check average
  Tier 2 potential: 8.3ns/check (-45%)
  Tier 3 potential: 0ns for non-escaping (-100%)

Global Statistics:
  Total CBGR overhead: 312ms (8.7% of runtime)
  Optimizable: 298ms (95.5%)
  Recommended changes: 3 functions
```

#### `verum analyze`
Deep static analysis for optimization opportunities.

```bash
verum analyze [options]

Options:
  --escape            Escape analysis for references
  --context           Context system usage
  --refinement        Refinement type coverage
  --all               Run all analyses

Examples:
  verum analyze --escape
  verum analyze --context
  verum analyze --all
```

### 4. Testing & Benchmarking

#### `verum test`
Test execution with coverage reporting.

```bash
verum test [options] [filter]

Options:
  --workspace         Test entire workspace
  --doc               Run documentation tests
  --ignored           Run ignored tests
  --verify <level>    Verification level for tests
  --threads <n>       Test threads
  --nocapture         Don't capture stdout
  --test-threads <n>  Number of test threads
  --coverage          Generate coverage report

Examples:
  verum test
  verum test test_parse
  verum test --workspace --coverage
  verum test --verify proof
```

**Output Format:**
```
[Testing] my-app v0.1.0
[Tier 0] Using interpreter for fast test execution

running 156 tests
test core::test_parser ... ok (2ms)
test core::test_types ... ok (1ms)
test refinement::positive_int ... ok (3ms)
test cbgr::escape_analysis ... ok (5ms)

Test Summary:
  Passed: 154
  Failed: 0
  Ignored: 2

Coverage Report:
  Lines: 94.3% (4,234/4,489)
  Functions: 97.2% (234/241)
  Branches: 89.1% (567/636)

Uncovered:
  error_handling.vr:45-67 (error paths)
  deprecated.vr:* (legacy code)
```

#### `verum bench`
Performance benchmarking with tier comparison.

```bash
verum bench [options] [filter]

Options:
  --compare-tiers     Compare performance across tiers
  --baseline <name>   Compare against baseline
  --save <name>       Save results as baseline
  --profile           Profile during benchmark

Examples:
  verum bench
  verum bench bench_sort
  verum bench --compare-tiers
  verum bench --baseline main --save feature-x
```

### 5. Dependency Management

#### `verum add`
Add dependencies with semantic versioning.

```bash
verum add <package> [options]

Options:
  --version <req>     Version requirement
  --features <list>   Enable features
  --optional          Mark as optional
  --dev               Add to dev-dependencies
  --build             Add to build-dependencies
  --git <url>         Git repository source
  --path <path>       Local path source

Examples:
  verum add http
  verum add serde --features derive,json
  verum add tokio@3.0 --optional
  verum add test-utils --dev
```

#### `verum remove`
Remove dependencies and clean lockfile.

```bash
verum remove <package> [options]

Options:
  --dev               Remove from dev-dependencies
  --build             Remove from build-dependencies

Examples:
  verum remove old-lib
  verum remove test-framework --dev
```

#### `verum update`
Update dependencies with compatibility checking.

```bash
verum update [package] [options]

Options:
  --workspace         Update workspace dependencies
  --aggressive        Allow breaking changes
  --dry-run           Show changes without applying

Examples:
  verum update
  verum update http
  verum update --workspace --dry-run
```

#### `verum tree`
Visualize dependency graph.

```bash
verum tree [options]

Options:
  --duplicates        Highlight duplicate dependencies
  --depth <n>         Maximum depth to display
  --all-features      Include all features

Examples:
  verum tree
  verum tree --duplicates
  verum tree --depth 2
```

### 5.1 Advanced Distribution Features

> **Note**: These commands implement the comprehensive module distribution infrastructure described in the [Module Distribution Specification](../../docs/detailed/29-module-distribution.md), including support for decentralized sources (IPFS), tier-specific artifacts, and verification proof distribution.

#### `verum publish`
Publish package to registry with tier-specific artifacts.

```bash
verum publish [options]

Options:
  --dry-run           Validate without publishing
  --sign              Sign package with Ed25519
  --verify-proofs     Verify formal proofs before publish
  --pin-ipfs          Pin to IPFS network
  --tier <0-3>        Include specific tier artifacts
  --all-tiers         Include all tier artifacts

Examples:
  verum publish
  verum publish --dry-run
  verum publish --sign --pin-ipfs
  verum publish --verify-proofs --tier 2
```

#### `verum add` (Extended)
Add dependencies from multiple sources.

```bash
# From IPFS
verum add crypto --ipfs QmXoypizjW3WknFiJnKLwHCnL72vedxjQkDDP1mXWo6uco

# From Git with verification
verum add parser --git https://github.com/verum/parser --verify

# With CBGR profile selection
verum add http --cbgr-profile optimized

# Prefer decentralized sources
verum add math --prefer-ipfs
```

#### `verum audit`
Security and verification auditing.

```bash
verum audit [options]

Options:
  --verify-checksums  Verify against checksum database
  --verify-signatures Check package signatures
  --verify-proofs     Validate formal proofs
  --cbgr-profiles     Show CBGR overhead analysis
  --fix               Auto-update vulnerable dependencies

Examples:
  verum audit
  verum audit --verify-checksums
  verum audit --cbgr-profiles
  verum audit --fix
```

#### `verum mirror`
Local registry mirroring for offline/airgapped environments.

```bash
verum mirror [options]

Options:
  --sync              Sync with upstream registry
  --serve <port>      Serve local mirror
  --include-proofs    Include verification proofs
  --include-sources   Include source code

Examples:
  verum mirror --sync
  verum mirror --serve 8080
```

#### `verum bundle`
Create offline distribution bundles.

```bash
verum bundle [options]

Options:
  --output <file>     Output bundle file
  --include-toolchain Include Verum toolchain
  --tier <0-3>        Include specific tier
  --compress          Compress bundle

Examples:
  verum bundle --output app.vxb
  verum bundle --include-toolchain --tier 2
```

### 6. IDE Integration

#### `verum lsp`
Start Language Server Protocol daemon.

```bash
verum lsp [options]

Options:
  --stdio             Use stdio for communication (default)
  --socket <port>     Use TCP socket
  --pipe <name>       Use named pipe

Examples:
  verum lsp
  verum lsp --socket 7777
```

**LSP Features:**
- Real-time type checking
- Refinement validation with counterexamples
- CBGR cost hints inline
- Context system validation
- Auto-completion with type info
- Hover documentation with costs
- Code actions (add context, verify, optimize)
- Go to definition/references
- Rename refactoring
- Format on save

### 7. Documentation

#### `verum doc`
Generate documentation with cost annotations.

```bash
verum doc [options]

Options:
  --open              Open in browser
  --workspace         Document workspace
  --private           Include private items
  --no-deps           Skip dependencies
  --output <path>     Output directory

Examples:
  verum doc --open
  verum doc --workspace
  verum doc --private --output ./api-docs
```

### 8. Code Quality

#### `verum fmt`
Format code according to style guide.

```bash
verum fmt [options] [path]

Options:
  --check             Check without modifying
  --workspace         Format workspace

Examples:
  verum fmt
  verum fmt --check
  verum fmt src/
```

#### `verum lint`
Verum-specific linting.

```bash
verum lint [options]

Options:
  --fix               Auto-fix issues
  --workspace         Lint workspace
  --deny <lint>       Treat lint as error

Examples:
  verum lint
  verum lint --fix
  verum lint --deny missing-context
```

**Verum-Specific Lints:**
- `unnecessary-cbgr`: Reference could be &checked
- `missing-context`: Function uses context without declaration
- `unverified-refinement`: Refinement could be proven
- `escape-opportunity`: Non-escaping ref with CBGR
- `costly-verification`: Verification >5s
- `missing-cost-doc`: Public API lacks cost documentation

## Performance Requirements

All CLI commands must meet these targets:

```yaml
Startup Time:
  verum --version: <50ms
  verum check: <100ms to first output
  verum build: <200ms to start compilation

Type Checking:
  Speed: >100K LOC/second
  Memory: <100MB for 10K LOC

Compilation:
  Tier 0: Instant (<100ms)
  Tier 1: >10K LOC/second
  Tier 2: >5K LOC/second
  Tier 3: >2K LOC/second

LSP Response:
  Hover: <50ms
  Completion: <100ms
  Validation: <200ms

Package Operations:
  verum add: <2s including dependency resolution
  verum update: <5s for typical project
```

## Integration Requirements

### CI/CD Integration

The CLI must support standard CI/CD workflows:

```yaml
# GitHub Actions example
- name: Install Verum
  uses: verum-lang/setup-verum@v1
  with:
    version: '1.0.0'

- name: Check
  run: verum check --strict

- name: Test
  run: verum test --coverage

- name: Verify
  run: verum verify --profile

- name: Benchmark
  run: verum bench --baseline main
```

### Editor Integration

Support for major editors via LSP:

```json
// VS Code settings.json
{
  "verum.lsp.enable": true,
  "verum.lsp.showCostHints": true,
  "verum.validation.mode": "incremental",
  "verum.format.onSave": true
}
```

### Docker Support

```dockerfile
FROM verum-lang/verum:1.0 as builder
WORKDIR /app
COPY . .
RUN verum build --release --tier 2

FROM verum-lang/verum-runtime:1.0
COPY --from=builder /app/target/tier2/release/app /app
CMD ["/app"]
```

## Error Message Guidelines

All error messages must follow these principles:

### 1. Show the Problem Clearly
```
error[E0308]: Refinement constraint not satisfied
  --> main.vr:42:15
   |
42 |   let age: Positive = -5;
   |                       ^^ value `-5` fails constraint `> 0`
```

### 2. Explain the Why
```
   |
   = note: `Positive` requires all values to be greater than 0
   = note: This prevents runtime errors in age-dependent calculations
```

### 3. Provide Solutions with Costs
```
   |
   = help: Options to fix:
   = help: 1. Use runtime check: `Positive::try_from(-5)?`
   =         Cost: ~5μs per call, returns Result<Positive, Error>
   = help: 2. Prove at compile-time: `@verify(age > 0)`
   =         Cost: ~500ms compile time, 0ns runtime
   = help: 3. Use default: `Positive::default()` (returns 1)
```

### 4. Link to Documentation
```
   |
   = docs: https://docs.vr.lang/refinement-types#positive
```

## Implementation Phases

### Phase 1: Foundation (Months 1-2)
**Goal:** Basic development workflow

- [x] Project structure defined
- [ ] `verum new` command
- [ ] `verum init` command
- [ ] `verum build` (Tier 0 only)
- [ ] `verum run` (basic)
- [ ] `verum check` (type checking)
- [ ] `verum test` (basic)
- [ ] Basic error messages

**Success Metrics:**
- Create and run "Hello World" in <1 minute
- Type check 1K LOC in <100ms
- Run tests with basic coverage

### Phase 2: Core Features (Months 2-3)
**Goal:** Production-ready builds

- [ ] Multi-tier compilation (Tiers 0-3)
- [ ] `verum verify` with cost reporting
- [ ] `verum profile --memory` (CBGR analysis)
- [ ] `verum analyze --escape`
- [ ] Dependency management (`add`, `remove`, `update`)
- [ ] Enhanced error messages with costs
- [ ] `verum bench` with tier comparison

**Success Metrics:**
- Compile 10K LOC in <2s (Tier 2)
- Profile CBGR overhead accurately
- Verify 50 functions in <10s

### Phase 3: Developer Experience (Months 3-4)
**Goal:** IDE integration and tooling

- [ ] `verum lsp` implementation
- [ ] VS Code extension
- [ ] `verum fmt` formatter
- [ ] `verum lint` linter
- [ ] `verum doc` with cost annotations
- [ ] Package registry integration
- [ ] `verum publish` command

**Success Metrics:**
- LSP response <100ms
- Format 10K LOC in <1s
- Publish package in <10s

### Phase 4: Advanced Features (Months 4-6)
**Goal:** Enterprise features

- [ ] Workspace support
- [ ] Incremental compilation
- [ ] Distributed builds
- [ ] Custom profiles
- [ ] Build caching
- [ ] Cross-compilation
- [ ] REPL implementation

**Success Metrics:**
- Build workspace with 100K LOC in <30s
- Cache hit rate >80%
- Cross-compile to 3+ targets

## Testing Strategy

### Unit Tests
- Each command module tested independently
- Mock filesystem and network operations
- Test error conditions and edge cases
- Coverage target: >95%

### Integration Tests
- Full command execution tests
- Real project creation and building
- Dependency resolution testing
- Multi-tier compilation verification

### Performance Tests
- Benchmark all commands
- Regression testing (<5% tolerance)
- Memory usage profiling
- Startup time validation

### User Acceptance Tests
- "Hello World" in <1 minute
- Build real 10K LOC project
- IDE integration workflow
- CI/CD pipeline integration

## Monitoring & Telemetry

### Anonymous Usage Metrics (Opt-in)
- Command frequency
- Build times by tier
- Verification success rates
- Error message effectiveness
- Feature adoption rates

### Performance Metrics
- Type checking speed (LOC/s)
- Compilation speed by tier
- CBGR overhead distribution
- Verification time percentiles

### Quality Metrics
- Error message clarity (user surveys)
- Time to first successful build
- Documentation effectiveness
- Support ticket frequency

## Success Criteria

The Verum CLI will be considered successful when:

### Technical Metrics
- Type check: >100K LOC/second
- Compile: >50K LOC/second (Tier 2)
- CBGR overhead: <15ns average
- LSP response: <100ms p95
- Startup time: <100ms

### User Metrics
- Time to "Hello World": <1 minute
- Time to production app: <1 day
- User satisfaction: >90%
- Documentation coverage: 100%
- Error clarity rating: >4.5/5

### Adoption Metrics
- 1,000+ projects created in first month
- 100+ packages published
- 10+ IDE integrations
- 50+ CI/CD pipelines
- 5+ major projects using Verum

## Conclusion

The Verum CLI is the gateway to experiencing Verum's revolutionary approach to systems programming. By embodying semantic honesty, supporting gradual verification, and providing transparent cost analysis, the CLI transforms Verum from a language specification into a practical development platform.

Every command, every error message, and every optimization suggestion must reinforce Verum's core value proposition: **safety without hidden costs**, **performance with transparency**, and **verification when it matters**.

The CLI is not just a tool – it's the proof that Verum's ambitious vision is achievable in practice.