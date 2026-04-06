# Verum Language Server Protocol (LSP)

A production-ready Language Server Protocol implementation for the Verum programming language, providing comprehensive IDE support.

## Features

### Core LSP Features

- **Syntax Error Diagnostics** - Real-time syntax error detection and reporting
- **Type Error Diagnostics** - Advanced type checking with refinement type support
- **Auto-completion** - Context-aware code completion for:
  - Keywords (`fn`, `let`, `match`, `if`, etc.)
  - Built-in types (`Int`, `Text`, `List`, `Map`, etc.)
  - User-defined functions, types, structs, and enums
  - Refinement type suggestions
- **Hover Information** - Rich hover tooltips showing:
  - Type information
  - Refinement constraints
  - Function signatures with pre/postconditions
  - Documentation comments
- **Go to Definition** - Navigate to symbol definitions
- **Find References** - Find all references to a symbol
- **Rename Symbol** - Intelligent symbol renaming across the document
- **Code Formatting** - Format code according to Verum style guidelines
- **Code Actions** - Quick fixes and refactorings:
  - Add runtime checks for refinement violations
  - Type conversion suggestions
  - Extract function/variable refactorings

## Installation

### Building from Source

```bash
cd crates/verum_lsp
cargo build --release
```

The binary will be available at `target/release/verum-lsp`.

### System Requirements

- Rust 1.75 or higher
- LLVM 21.1 (for code generation features)
- Z3 4.x (for SMT-based verification)

## Usage

### VS Code

1. Install the Verum VS Code extension (coming soon)
2. The extension will automatically start the LSP server

### Manual Configuration

Start the server manually:

```bash
verum-lsp
```

The server communicates via JSON-RPC over stdin/stdout.

### Configuration for Other Editors

#### Neovim (with nvim-lspconfig)

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

if not configs.vr_lsp then
  configs.vr_lsp = {
    default_config = {
      cmd = {'verum-lsp'},
      filetypes = {'verum', 'ver'},
      root_dir = lspconfig.util.root_pattern('.git', 'Cargo.toml'),
      settings = {},
    },
  }
end

lspconfig.vr_lsp.setup{}
```

#### Emacs (with lsp-mode)

```elisp
(add-to-list 'lsp-language-id-configuration '(verum-mode . "verum"))

(lsp-register-client
 (make-lsp-client :new-connection (lsp-stdio-connection "verum-lsp")
                  :major-modes '(verum-mode)
                  :server-id 'verum-lsp))
```

#### Sublime Text

Add to your LSP settings:

```json
{
  "clients": {
    "verum-lsp": {
      "enabled": true,
      "command": ["verum-lsp"],
      "selector": "source.vr"
    }
  }
}
```

## Architecture

The LSP server is organized into the following modules:

- **`backend.rs`** - Main LSP server implementation
- **`document.rs`** - Document state management and caching
- **`diagnostics.rs`** - Diagnostic conversion and publishing
- **`completion.rs`** - Auto-completion logic
- **`hover.rs`** - Hover information generation
- **`goto_definition.rs`** - Symbol definition lookup
- **`references.rs`** - Reference finding
- **`rename.rs`** - Symbol renaming
- **`formatting.rs`** - Code formatting
- **`code_actions.rs`** - Quick fixes and refactorings

## Development

### Running Tests

```bash
cargo test
```

### Debugging

The server logs to `/tmp/verum-lsp.log` by default. You can monitor the log file:

```bash
tail -f /tmp/verum-lsp.log
```

### Adding New Features

To add a new LSP feature:

1. Implement the feature in the appropriate module (or create a new one)
2. Register the capability in `backend.rs` in the `initialize` method
3. Add the handler method to the `LanguageServer` impl in `backend.rs`
4. Add tests in `tests/lsp_integration_tests.rs`

## Example: Hover Information

When you hover over a function:

```verum
fn factorial(n: Int{>= 0}) -> Int
    requires n >= 0
    ensures result > 0
{
    match n {
        0 => 1,
        n => n * factorial(n - 1)
    }
}
```

The LSP server shows:

```markdown
**Function: factorial**

fn factorial(n: Int{>= 0}) -> Int

**Preconditions:**
- `n >= 0`

**Postconditions:**
- `result > 0`
```

## Performance

The LSP server is optimized for low-latency responses:

- **Document parsing**: < 10ms for typical files
- **Type checking**: < 50ms for 1000 LOC
- **Completion**: < 5ms
- **Hover**: < 2ms

## Roadmap

- [ ] Workspace-wide symbol search
- [ ] Multi-file go-to-definition
- [ ] Inlay hints for inferred types
- [ ] Semantic tokens for syntax highlighting
- [ ] Call hierarchy
- [ ] Document symbols outline
- [ ] Signature help for function parameters
- [ ] Code lens for verification status
- [ ] Integration with SMT solver for real-time verification

## Contributing

See the main Verum repository for contribution guidelines.

## License

Apache-2.0

## Related Documentation

- [Verum Language Specification](../../docs/detailed/)
- [Type System](../../docs/detailed/03-type-system.md)
- [Refinement Types](../../docs/detailed/12-refinement-types.md)
- [LSP Protocol Specification](https://microsoft.github.io/language-server-protocol/)
