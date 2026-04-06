# Verum CLI - Usage Examples

## Table of Contents
- [Getting Started](#getting-started)
- [Project Creation](#project-creation)
- [Building](#building)
- [Testing](#testing)
- [Dependencies](#dependencies)
- [Development Workflow](#development-workflow)

---

## Getting Started

### Install Verum CLI
```bash
$ cargo install --path crates/verum_cli
   Compiling verum_cli v0.1.0
    Finished release [optimized] target(s) in 45.2s
   Installing /Users/you/.cargo/bin/verum
    Installed package `verum_cli v0.1.0`
```

### Check Version
```bash
$ verum version
verum 0.1.0

$ verum version --verbose
verum 0.1.0

Build information:
  Commit: a1b2c3d
  Build date: 2025-11-15
  Rust version: 1.75
  
Verum Language Platform
https://github.com/verum-lang/verum
```

### Help
```bash
$ verum --help
Verum language toolchain - Production-grade systems programming

Usage: verum <COMMAND>

Commands:
  new      Create a new Verum project
  init     Initialize project in current directory
  build    Build the project
  run      Build and run the project
  test     Run tests
  bench    Run benchmarks
  check    Check without building
  fmt      Format source code
  lint     Run linter
  doc      Generate documentation
  clean    Remove build artifacts
  watch    Watch for changes and rebuild
  deps     Manage dependencies
  repl     Start interactive REPL
  version  Show version information
  package  Package management
  profile  Profile performance
  
Options:
  -v, --verbose  Enable verbose output
  -q, --quiet    Suppress all output except errors
  --color <auto|always|never>
  
Run 'verum <command> --help' for more information on a specific command.
```

---

## Project Creation

### Create Binary Application
```bash
$ verum new my_app
→ Creating new binary project: my_app
→ Initializing git repository
✓ Created my_app project

Next steps:
  cd my_app
  verum build
  verum run
```

**Result:**
```
my_app/
├── Verum.toml
├── .gitignore
├── README.md
├── src/
│   └── main.vr
└── tests/
    └── main_test.vr
```

### Create Library
```bash
$ verum new my_lib --template library
→ Creating new library project: my_lib
→ Initializing git repository
✓ Created my_lib project
```

### Create Web API
```bash
$ verum new my_api --template web-api
→ Creating new web-api project: my_api
→ Initializing git repository
✓ Created my_api project
```

**Generated structure:**
```
my_api/
├── Verum.toml
├── src/
│   ├── main.vr
│   └── routes/
│       └── mod.vr
└── tests/
    └── api_test.vr
```

### Create CLI App
```bash
$ verum new my_cli --template cli-app
→ Creating new cli-app project: my_cli
→ Initializing git repository
✓ Created my_cli project
```

### Initialize Existing Directory
```bash
$ mkdir existing_project && cd existing_project
$ verum init
→ Creating new binary project: existing_project
✓ Created existing_project project
```

---

## Building

### Debug Build (Default)
```bash
$ cd my_app
$ verum build
→ Starting compilation...
ℹ Found 1 source files
ℹ 1 files changed, 0 cached
→ Compiling src/main.vr
  [████████████████████] 1/1 Compiling
→ Linking...
✓ Compiled my_app (245ms)
  1 files compiled, 0 cached
  Output: /path/to/my_app/target/debug/my_app
```

### Release Build
```bash
$ verum build --release
→ Starting compilation...
→ Building in release mode
ℹ Target: native
ℹ Optimization level: 3
ℹ Jobs: 8
  [████████████████████] 1/1 Compiling
→ Linking...
✓ Compiled my_app (1.2s)
  Output: /path/to/my_app/target/release/my_app
```

### Incremental Build (Cached)
```bash
$ verum build
→ Starting compilation...
ℹ Found 1 source files
ℹ 0 files changed, 1 cached
→ Linking...
✓ Compiled my_app (42ms)
  0 files compiled, 1 cached
```

### Cross-Compilation
```bash
$ verum build --target x86_64-linux
→ Starting compilation...
ℹ Target: x86_64-linux
✓ Compiled my_app (320ms)
```

### With Features
```bash
$ verum build --features "feature1,feature2"
$ verum build --all-features
$ verum build --no-default-features --features minimal
```

---

## Running

### Simple Run
```bash
$ verum run
→ Building in debug mode
  [████████████████████] Compiling
✓ Compiled my_app (180ms)

→ Running application

Hello, Verum!
```

### With Arguments
```bash
$ verum run -- --input file.txt --verbose
→ Building in debug mode
✓ Compiled my_app (42ms)

→ Running application

Processing: file.txt
[verbose output...]
```

### Release Mode
```bash
$ verum run --release
→ Building in release mode
✓ Compiled my_app (1.1s)

→ Running application

Hello, Verum!
```

---

## Testing

### Run All Tests
```bash
$ verum test
→ Running tests

Running 3 tests

  ✓ test_example
  ✓ test_add
  ✓ test_point_distance

✓ All 3 tests passed
```

### Filter Tests
```bash
$ verum test --filter "test_add"
→ Running tests

Running 1 test

  ✓ test_add

✓ All 1 test passed
```

### Test Options
```bash
$ verum test --nocapture      # Show output
$ verum test --release        # Test optimized build
$ verum test --test-threads 1 # Sequential tests
```

---

## Benchmarks

### Run Benchmarks
```bash
$ verum bench
→ Running benchmarks

Benchmark                      Time (ns)
───────────────────────────────────────────────
bench_add                          125.5
bench_multiply                      89.3
bench_sort                        1523.7

✓ Benchmarks completed
```

### Save Baseline
```bash
$ verum bench --save-baseline main
→ Running benchmarks
✓ Saved baseline 'main'
```

### Compare to Baseline
```bash
$ verum bench --baseline main
→ Running benchmarks

Benchmark          Current    Baseline    Change
──────────────────────────────────────────────
bench_add           120.1      125.5     -4.3%
bench_multiply       85.2       89.3     -4.6%
```

---

## Dependencies

### List Dependencies
```bash
$ verum deps list
→ Listing dependencies

Dependencies:
  verum_std 0.1

Dev Dependencies:
  verum_test 0.1
```

### Tree View
```bash
$ verum deps list --tree
→ Listing dependencies

Dependencies:
  └─ verum_std
     └─ verum_core
     └─ verum_io
```

### Add Dependency
```bash
$ verum deps add verum_http
→ Adding dependency: verum_http
✓ Added verum_http

$ verum deps add verum_test --dev
→ Adding dependency: verum_test
✓ Added verum_test (dev)
```

### Remove Dependency
```bash
$ verum deps remove verum_http
→ Removing dependency: verum_http
✓ Removed verum_http
```

### Update Dependencies
```bash
$ verum deps update
→ Updating all dependencies
  Checking verum_std...
  Checking verum_test...
✓ Dependencies updated

$ verum deps update verum_std
→ Updating verum_std
✓ Dependencies updated
```

---

## Development Workflow

### Watch Mode
```bash
$ verum watch
→ Watching for changes (running 'build')
ℹ Watching for file changes... (Ctrl+C to stop)

[File changed: src/main.vr]
→ Files changed, rebuilding...
  [████████████████████] Compiling
✓ Done

ℹ Waiting for changes...
```

### Watch and Test
```bash
$ verum watch test
→ Watching for changes (running 'test')

[File changed]
→ Files changed, rebuilding...
→ Running tests
  ✓ test_add
  ✓ test_multiply
✓ Done
```

### Check (Fast Type Check)
```bash
$ verum check
→ Checking project
✓ Project is well-typed
```

### Format Code
```bash
$ verum fmt
→ Formatting source files
  Formatted src/main.vr
  Formatted src/lib.vr
✓ All files formatted

$ verum fmt --check
→ Checking formatting
✓ All files properly formatted
```

### Lint
```bash
$ verum lint
→ Running linter
✓ No lint issues found

$ verum lint --fix
→ Running linter
  Fixed 3 issues in src/main.vr
✓ All issues fixed
```

### Generate Documentation
```bash
$ verum doc
→ Generating documentation
  Documenting my_app
  Documenting dependencies
✓ Documentation generated

$ verum doc --open
→ Generating documentation
✓ Documentation generated
ℹ Opening documentation in browser
```

### Clean
```bash
$ verum clean
→ Cleaning build artifacts
✓ Removed target directory

$ verum clean --all
→ Cleaning build artifacts
✓ Removed target directory
✓ Removed cache file
```

---

## REPL

### Start REPL
```bash
$ verum repl
Verum REPL v0.1.0
Type :quit or press Ctrl+D to exit

verum[1]> 2 + 2
ℹ Expression evaluation not yet implemented

verum[2]> :help

REPL Commands:
  :help, :h      - Show this help
  :quit, :q      - Exit REPL
  :clear, :c     - Clear screen
  :type <expr>   - Show type of expression
  :load <file>   - Load and execute a file

verum[3]> :quit

Goodbye!
```

### With Prelude
```bash
$ verum repl --prelude lib.vr
Verum REPL v0.1.0
Type :quit or press Ctrl+D to exit

Loading prelude: lib.vr

verum[1]>
```

---

## Package Management

### Search Packages
```bash
$ verum package search http
→ Searching for: http

Package              Version    Description
────────────────────────────────────────────────────────
verum_http          0.2.1      HTTP client and server
verum_http2         1.0.0      HTTP/2 support
verum_https         0.1.5      HTTPS client
```

### Publish Package
```bash
$ verum package publish --dry-run
→ Performing dry run of package publish
⚠ Package publishing not yet implemented
ℹ This will upload to the Verum package registry
```

### Install Package Binary
```bash
$ verum package install verum_fmt
→ Installing verum_fmt latest
⚠ Package installation not yet implemented
```

---

## Advanced Features

### Parallel Builds
```bash
$ verum build --jobs 16    # Use 16 cores
$ verum build --jobs 1     # Sequential build
```

### Keep Intermediate Files
```bash
$ verum build --keep-temps
→ Building with intermediate files
✓ Compiled my_app
  IR files saved in target/debug/*.ll
```

### Profiling
```bash
$ verum profile
→ Profiling runtime (output: flamegraph)
⚠ Profiling not yet implemented
ℹ This will support flamegraphs and performance analysis
```

### Verbose Output
```bash
$ verum -v build
→ Starting compilation...
🐛 Debug: Found source files: [src/main.vr]
🐛 Debug: Computing file hash...
🐛 Debug: Compiling src/main.vr
...
```

### Quiet Mode
```bash
$ verum -q build
# Only errors shown
```

---

## Error Examples

### Compilation Error
```bash
$ verum build
→ Starting compilation...
✗ Compilation failed with 1 errors

error: src/main.vr:5:10: Type mismatch
  Expected Int, found Text
  
  5 |     let x: Int = "hello";
    |                  ^^^^^^^

✗ Build failed
```

### Missing File
```bash
$ verum build
✗ Source directory not found

Run 'verum new <name>' to create a new project
```

### Test Failure
```bash
$ verum test
→ Running tests

  ✓ test_add
  ✗ test_subtract
  ✓ test_multiply

✗ 1 tests failed, 2 passed
```

---

## Tips and Tricks

### Aliases
```bash
alias vb='verum build'
alias vr='verum run'
alias vt='verum test'
alias vw='verum watch'
```

### Project Templates
```bash
# Quick prototyping
verum new prototype --template binary

# Library development
verum new mylib --template library

# Web service
verum new api --template web-api

# Command-line tool
verum new tool --template cli-app
```

### Workflow Examples
```bash
# Create, build, run
verum new hello && cd hello && verum build && verum run

# Test-driven development
verum watch test

# Continuous integration
verum build --release && verum test && verum bench
```
