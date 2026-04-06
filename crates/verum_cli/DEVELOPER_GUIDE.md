# Verum CLI Developer Guide

## Quick Reference

### Adding a New Command

1. Create command module:
```bash
touch src/commands/mycommand.rs
```

2. Implement the command:
```rust
// src/commands/mycommand.rs
use crate::error::Result;
use crate::ui;

pub fn execute(arg1: String, flag: bool) -> Result<()> {
    ui::step("Running my command");
    
    // Implementation here
    
    ui::success("Command completed");
    Ok(())
}
```

3. Register in `src/commands/mod.rs`:
```rust
pub mod mycommand;
```

4. Add to CLI enum in `src/main.rs`:
```rust
#[derive(Subcommand)]
enum Commands {
    // ...
    MyCommand {
        arg1: String,
        #[clap(long)]
        flag: bool,
    },
}
```

5. Wire up in match statement:
```rust
Commands::MyCommand { arg1, flag } => {
    commands::mycommand::execute(arg1, flag)
}
```

### Adding a New Template

1. Add template function in `src/templates.rs`:
```rust
pub mod my_template {
    use super::*;

    pub fn create(dir: &Path, name: &str) -> Result<()> {
        let content = r#"
// Template content here
"#;
        fs::write(dir.join("src/main.vr"), content)?;
        Ok(())
    }
}
```

2. Register in `src/commands/new.rs`:
```rust
match template {
    "my-template" => {
        templates::my_template::create(dir, name)?;
    }
    // ...
}
```

### Error Handling Best Practices

```rust
// Use specific error types
return Err(CliError::CompilationFailed(error_count));

// Provide context
.map_err(|e| CliError::Custom(format!("Failed to read manifest: {}", e)))?

// User-friendly messages
ui::error("Build failed - check your Verum.toml for syntax errors");
```

### UI Components Usage

```rust
// Progress bar
let progress = ui::ProgressReporter::new(total, "Compiling files");
for item in items {
    // Process item
    progress.inc(1);
}
progress.finish();

// Spinner for indeterminate tasks
let spinner = ui::Spinner::new("Loading dependencies");
// Do work
spinner.finish_with_message("Done");

// Status messages
ui::step("Starting build");      // → Cyan arrow
ui::success("Build succeeded");  // ✓ Green
ui::warn("Deprecated API used"); // ⚠ Yellow
ui::error("Compilation failed"); // ✗ Red
ui::info("Using cache");         // ℹ Blue (verbose only)
```

### Configuration Schema

```rust
// Extend Manifest struct in src/config.rs
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub package: Package,
    pub dependencies: HashMap<String, Dependency>,
    // Add your field here
    #[serde(default)]
    pub my_section: MySection,
}

#[derive(Serialize, Deserialize, Default)]
pub struct MySection {
    pub option1: bool,
    pub option2: String,
}
```

### Build Cache Integration

```rust
// Check if file changed
if cache.is_file_changed(&source_path)? {
    compile_file(&source_path)?;
    cache.update_file(&source_path)?;
}

// Track artifacts
cache.add_artifact(
    vec![source_path],  // Sources
    vec![],             // Dependencies
    output_path,        // Output
);

// Validate artifact
if cache.is_artifact_valid(&output_path)? {
    // Use cached version
} else {
    // Rebuild
}
```

## Code Organization

### Module Responsibilities

- **main.rs**: CLI parsing and command routing only
- **error.rs**: Error types and Result alias
- **ui.rs**: All terminal output and formatting
- **config.rs**: Verum.toml parsing and validation
- **cache.rs**: Build cache implementation
- **compiler/**: Compilation orchestration
- **commands/**: Individual command implementations
- **templates.rs**: Project scaffolding

### Naming Conventions

- **Commands**: Verb-based (build, run, test)
- **Modules**: Noun-based (config, cache, compiler)
- **Functions**: execute() for command entry points
- **Errors**: Descriptive variant names

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_detects_changes() {
        let cache = BuildCache::new();
        // Test implementation
    }
}
```

### Integration Tests

```rust
// tests/integration_test.rs
use assert_cmd::Command;

#[test]
fn test_new_command() {
    let mut cmd = Command::cargo_bin("verum").unwrap();
    cmd.arg("new")
       .arg("test_project")
       .assert()
       .success();
}
```

## Performance Guidelines

### DO:
- ✅ Use progress bars for operations > 1 second
- ✅ Cache file hashes to avoid re-reading
- ✅ Parallelize independent operations
- ✅ Minimize allocations in hot paths

### DON'T:
- ❌ Print in tight loops
- ❌ Re-parse configs multiple times
- ❌ Block on I/O without feedback
- ❌ Use recursive directory walks without limits

## Debugging Tips

### Enable Verbose Logging

```bash
verum -v build       # Verbose mode
verum -vv build      # Very verbose (if implemented)
```

### Test Individual Commands

```bash
# In development
cargo run -- new test_app
cargo run -- build --release
```

### Check Cache State

```bash
# Inspect cache
cd project && cat .vr_cache | xxd | head -20
```

## Common Patterns

### Manifest Loading

```rust
let manifest_dir = Manifest::find_manifest_dir()?;
let manifest = Manifest::from_file(&manifest_dir.join("Verum.toml"))?;
manifest.validate()?;
```

### File Operations

```rust
use walkdir::WalkDir;

for entry in WalkDir::new("src") {
    let entry = entry?;
    if entry.path().extension() == Some("ver") {
        process_file(entry.path())?;
    }
}
```

### User Confirmation

```rust
if ui::confirm("Delete all build artifacts?") {
    clean_target_dir()?;
}
```

## Release Checklist

- [ ] All commands compile
- [ ] Integration tests pass
- [ ] Examples work end-to-end
- [ ] README is up to date
- [ ] Version bumped in Cargo.toml
- [ ] CHANGELOG.md updated
- [ ] Performance benchmarks run
- [ ] Error messages reviewed

## Resources

- [clap Documentation](https://docs.rs/clap)
- [colored](https://docs.rs/colored)
- [indicatif](https://docs.rs/indicatif)
- [notify](https://docs.rs/notify)
- [Verum Language Spec](../../docs/)

## Getting Help

- Internal: Check existing command implementations
- External: Rust CLI Book (https://rust-cli.github.io/book/)
- Patterns: Study cargo, npm, gradle designs
