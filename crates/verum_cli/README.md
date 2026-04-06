# Verum CLI

Production-grade command-line interface for the Verum programming language.

## Features

- **Project Scaffolding**: Create new projects with multiple templates (binary, library, web-api, cli-app)
- **Build System**: Fast incremental compilation with SHA-256 caching
- **Dependency Management**: Add, remove, and update dependencies
- **Testing**: Integrated test runner with filtering and parallel execution
- **Benchmarking**: Performance benchmarking with baseline comparison
- **Watch Mode**: Automatic rebuild on file changes
- **REPL**: Interactive Read-Eval-Print Loop for experimentation
- **Documentation**: Automated documentation generation
- **Package Registry**: Publish and search packages

## Installation

```bash
cargo install --path crates/verum_cli
```

## Quick Start

### Create a New Project

```bash
# Binary application (default)
verum new my_app

# Library
verum new my_lib --template library

# Web API
verum new my_api --template web-api

# CLI application
verum new my_cli --template cli-app
```

### Build and Run

```bash
cd my_app
verum build              # Debug build
verum build --release    # Optimized release build
verum run               # Build and run
verum run -- arg1 arg2  # Run with arguments
```

### Testing and Benchmarking

```bash
verum test                    # Run all tests
verum test --filter my_test   # Run specific tests
verum bench                   # Run benchmarks
```

### Code Quality

```bash
verum fmt              # Format code
verum lint             # Run linter
verum check            # Type check without building
verum doc --open       # Generate and open docs
```

### Dependency Management

```bash
verum deps add verum_http            # Add dependency
verum deps add verum_test --dev      # Add dev dependency
verum deps list                      # List dependencies
verum deps list --tree               # Show dependency tree
verum deps update                    # Update all dependencies
```

### Watch Mode

```bash
verum watch           # Watch and rebuild
verum watch test      # Watch and run tests
verum watch --clear   # Clear screen before each build
```

### REPL

```bash
verum repl                    # Start interactive REPL
verum repl --prelude lib.vr  # Load file before REPL
```

## Commands

### Project Management
- `verum new <name>` - Create a new project
- `verum init` - Initialize project in current directory
- `verum clean` - Remove build artifacts

### Build Commands
- `verum build` - Compile the project
- `verum run` - Build and execute
- `verum check` - Type check without building
- `verum test` - Run tests
- `verum bench` - Run benchmarks

### Development Tools
- `verum fmt` - Format source code
- `verum lint` - Run static analysis
- `verum doc` - Generate documentation
- `verum watch` - Rebuild on file changes
- `verum repl` - Interactive shell

### Package Management
- `verum deps add <name>` - Add dependency
- `verum deps remove <name>` - Remove dependency
- `verum deps update` - Update dependencies
- `verum deps list` - Show dependencies
- `verum package publish` - Publish to registry
- `verum package search <query>` - Search packages

### Utility Commands
- `verum version` - Show version info
- `verum profile` - Performance profiling

## Configuration

Projects are configured via `Verum.toml`:

```toml
[package]
name = "my_app"
version = "0.1.0"
authors = ["Your Name <you@example.com>"]
license = "MIT OR Apache-2.0"

[dependencies]
verum_std = "0.1"

[build]
target = "native"
opt_level = 2
incremental = true

[features]
default = ["std"]
std = []

[profile.dev]
opt_level = 0
debug = true
incremental = true

[profile.release]
opt_level = 3
debug = false
lto = true
```

## Templates

### Binary Template
Basic executable application with main function.

### Library Template
Reusable library with public API and tests.

### Web API Template
HTTP server with routing and middleware support.

### CLI App Template
Command-line application with argument parsing.

## Build Features

### Incremental Compilation
- SHA-256 file hashing for change detection
- Parallel compilation using all CPU cores
- Artifact caching in `.vr_cache`

### Cross-Compilation
```bash
verum build --target x86_64-linux
verum build --target aarch64-darwin
```

### Optimization Levels
- `0` - No optimization (fastest compile)
- `1` - Basic optimization
- `2` - Recommended (default release)
- `3` - Maximum optimization

## Environment Variables

- `VERUM_API_TOKEN` - Authentication token for package registry
- `VERUM_CACHE_DIR` - Override cache directory location
- `VERUM_LOG` - Set logging level (error, warn, info, debug, trace)

## Performance

The CLI is designed for speed:
- Incremental builds only recompile changed files
- Parallel compilation uses all available cores
- SHA-256 caching avoids redundant work
- Binary artifacts cached between builds

## User Experience

- Colored output for readability
- Progress bars for long operations
- Helpful error messages with suggestions
- Tab completion support (planned)

## Contributing

See the main project CONTRIBUTING.md for guidelines.

## License

MIT OR Apache-2.0
