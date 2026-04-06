# Verum Language Support for VS Code

Official VS Code extension for the Verum programming language.

## Features

### Language Server Protocol (LSP) Integration
- **Real-time type checking** with inference time display
- **Refinement validation** with counterexamples
- **CBGR cost hints** inline (~15ns overhead visualization)
- **Context system validation**
- **Auto-completion** with type information
- **Hover documentation** with costs and verification status
- **Go to definition/references**
- **Rename refactoring**

### Code Quality
- **Format on save** with `verum fmt`
- **Linting** with Verum-specific checks:
  - `unnecessary-cbgr`: Reference could be &checked (0ns)
  - `missing-context`: Function uses context without declaration
  - `unverified-refinement`: Refinement could be proven
  - `escape-opportunity`: Non-escaping ref with CBGR overhead
  - `costly-verification`: Verification >5s
  - `missing-cost-doc`: Public API lacks cost documentation

### Verification & Performance
- **Verification status badges**: Proven / Runtime / Unverified
- **CBGR overhead visualization** in editor
- **Performance profiling integration**
- **Code actions** for optimization suggestions

## Requirements

- Verum toolchain installed (`verum --version`)
- VS Code 1.75.0 or higher

## Installation

### From VS Code Marketplace
1. Open VS Code
2. Press `Ctrl+P` (or `Cmd+P` on macOS)
3. Type `ext install verum-lang.vr` (package name remains for backwards compatibility)

### From Source
```bash
cd crates/verum_cli/editors/vscode
npm install
npm run compile
code --install-extension .
```

## Configuration

Configure the extension in VS Code settings (`Ctrl+,` or `Cmd+,`):

```json
{
  "verum.lsp.enable": true,
  "verum.lsp.showCostHints": true,
  "verum.validation.mode": "incremental",
  "verum.format.onSave": true,
  "verum.verify.showStatus": true,
  "verum.trace.server": "off"
}
```

### Settings Reference

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `verum.lsp.enable` | boolean | `true` | Enable Verum Language Server |
| `verum.lsp.showCostHints` | boolean | `true` | Show CBGR cost hints inline |
| `verum.validation.mode` | enum | `incremental` | When to run validation (`incremental` / `on-save` / `on-demand`) |
| `verum.format.onSave` | boolean | `true` | Format files automatically on save |
| `verum.verify.showStatus` | boolean | `true` | Show verification status badges |
| `verum.trace.server` | enum | `off` | LSP trace level (`off` / `messages` / `verbose`) |

## Commands

Access commands via Command Palette (`Ctrl+Shift+P` or `Cmd+Shift+P`):

- **Restart Verum Language Server** - Restart LSP server
- **Show CBGR Costs** - Display performance cost analysis
- **Run Verification** - Trigger formal verification
- **Profile Performance** - Open performance profiler

## Keyboard Shortcuts

- `Ctrl+Shift+V` (or `Cmd+Shift+V` on macOS) - Run verification

## Usage

### CBGR Cost Hints
When `verum.lsp.showCostHints` is enabled, you'll see inline hints showing CBGR overhead:

```verum
fn process(data: &List<T>) {  // ~15ns per check
    // ...
}
```

### Verification Status
Functions show verification status:
- ✅ **Proven** - Formally verified at compile-time (0ns runtime cost)
- ⚠️ **Runtime** - Runtime checks enabled (~5μs/call)
- ❌ **Unverified** - No verification

### Code Actions
The LSP provides quick fixes and refactorings:
- Add `using [Context]` declarations
- Convert `&T` to `&checked T` for 0ns overhead
- Add `@verify` annotations
- Optimize CBGR-heavy functions

## Troubleshooting

### LSP Server Not Starting
1. Verify Verum is installed: `verum --version`
2. Check LSP logs: Set `verum.trace.server` to `verbose`
3. Restart server: Run "Restart Verum Language Server" command

### Performance Issues
- Switch validation mode to `on-save` for large files
- Disable cost hints: Set `verum.lsp.showCostHints` to `false`

### Format Not Working
- Ensure `verum fmt` works from terminal
- Check file is saved (format on save requires save action)

## Development

### Building from Source
```bash
cd crates/verum_cli/editors/vscode
npm install
npm run compile
```

### Running Extension
1. Open VS Code in the extension directory
2. Press `F5` to launch Extension Development Host
3. Open a `.vr` file to test

### Debugging LSP
Enable verbose tracing:
```json
{
  "verum.trace.server": "verbose"
}
```

View logs in Output panel: View → Output → Select "Verum Language Server"

## Contributing

Contributions welcome! Please see [CONTRIBUTING.md](../../CONTRIBUTING.md) for guidelines.

## License

MIT OR Apache-2.0

## Links

- [Verum Language](https://verum-lang.org)
- [Documentation](https://docs.vr-lang.org)
- [GitHub Repository](https://github.com/verum-lang/verum)
- [Report Issues](https://github.com/verum-lang/verum/issues)
