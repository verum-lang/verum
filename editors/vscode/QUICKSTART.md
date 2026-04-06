# Verum VS Code Extension - Quick Start Guide

This guide helps you get the Verum VS Code extension up and running in 5 minutes.

## Prerequisites

1. **VS Code**: Version 1.75.0 or higher
2. **Node.js**: Version 18.0.0 or higher (with npm 9+)
3. **Verum LSP Server**: Optional - only needed for refinement validation

## Installation

### Option 1: Development Mode (Recommended for Testing)

```bash
# Navigate to the extension directory
cd editors/vscode

# Install dependencies
npm install

# Compile TypeScript
npm run compile

# Verify build succeeded (should show no errors)
npm run lint

# Open this folder in VS Code
code .
```

Then in VS Code:
1. Press **F5** to launch "Extension Development Host"
2. A new VS Code window opens with the extension active
3. Create a test file: `test.vr`
4. You should see syntax highlighting immediately

### Option 2: Package and Install Globally

```bash
cd editors/vscode
npm install
npm run compile
npm run package  # Creates verum-language-1.0.0.vsix

# Install globally (available in all VS Code windows)
code --install-extension verum-language-1.0.0.vsix

# To uninstall later:
code --uninstall-extension verum-lang.vr-language
```

### Option 3: Symlink for Continuous Development

```bash
cd editors/vscode
npm install
npm run compile

# Create symlink in VS Code extensions folder
# macOS:
ln -s "$(pwd)" ~/.vscode/extensions/verum-language

# Linux:
ln -s "$(pwd)" ~/.vscode/extensions/verum-language

# Windows (PowerShell as Admin):
# New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\verum-language" -Target (Get-Location)

# Restart VS Code
```

## Verify Installation

1. **Check LSP Server**:
   ```bash
   which verum-lsp-server
   # Should output: /path/to/verum-lsp-server
   ```

2. **Create Test File**:
   ```bash
   echo 'fn main() { println("Hello, Verum!") }' > test.vr
   code test.vr
   ```

3. **Verify Features**:
   - Syntax highlighting should be active
   - Status bar should show "Verum" (green checkmark)
   - Output channel "Verum Language Server" should exist

## Testing Basic Features

### 1. Syntax Highlighting

Create a file `test.vr`:

```verum
// This should have syntax highlighting
fn divide(x: Int, y: Int{i != 0}) -> Int {
    x / y
}

fn main() using [Console] {
    let result = divide(10, 2);
    println(result)
}
```

**Expected**: Keywords, types, and refinements are highlighted.

### 2. Refinement Validation

Add this to `test.vr`:

```verum
fn main() {
    let result = divide(10, 0);  // Should show error
}
```

**Expected**: Red squiggly under `0` with error message:
```
Refinement violation: value `0` fails constraint `i != 0`
Counterexample: y = 0
```

### 3. Quick Fixes

1. Click on the error
2. Press `Cmd/Ctrl+.` or click the lightbulb
3. See available fixes:
   - "Wrap with runtime check"
   - "Add inline refinement"
   - "Add assertion"

**Try**: Select "Wrap with runtime check"

**Result**:
```verum
fn main() {
    let result = divide(10, NonZero.try_from(0)?);
}
```

### 4. Commands

Test the commands:

1. **Infer Refinement** (Ctrl+Alt+R):
   - Place cursor on a variable
   - Press Ctrl+Alt+R
   - See inferred refinement type

2. **Validate at Cursor** (Ctrl+Alt+V):
   - Place cursor on an expression
   - Press Ctrl+Alt+V
   - See validation result in popup

3. **Verification Profile**:
   - Open command palette (Cmd/Ctrl+Shift+P)
   - Type "Verum: Show Verification Profile"
   - Terminal opens with profile output

## Configuration

### Basic Setup

Add to `.vscode/settings.json`:

```json
{
  "verum.lsp.enable": true,
  "verum.lsp.enableRefinementValidation": true,
  "verum.lsp.validationMode": "quick",
  "verum.lsp.showCounterexamples": true,
  "verum.lsp.diagnosticDelay": 200
}
```

### Performance Tuning

For large projects:

```json
{
  "verum.lsp.validationMode": "quick",
  "verum.lsp.diagnosticDelay": 500,
  "verum.lsp.cacheValidationResults": true,
  "verum.lsp.smtTimeout": 50
}
```

### Development Mode

For debugging:

```json
{
  "verum.lsp.trace.server": "verbose",
  "verum.cbgr.enableProfiling": true,
  "verum.verification.showCostWarnings": true
}
```

## Troubleshooting

### Issue: "Language Server Not Starting"

**Check**:
1. LSP server is in PATH:
   ```bash
   which verum-lsp-server
   ```
2. LSP server is executable:
   ```bash
   verum-lsp-server --help
   ```
3. Check output channel:
   - View → Output → "Verum Language Server"

**Fix**:
- Set explicit path in settings:
  ```json
  {
    "verum.lsp.serverPath": "/full/path/to/verum-lsp-server"
  }
  ```
- Restart extension:
  - Command Palette → "Verum: Restart Language Server"

### Issue: "No Syntax Highlighting"

**Check**:
1. File extension is `.vr`
2. Language mode is set to "Verum" (bottom-right corner)

**Fix**:
- Click language mode in status bar
- Select "Verum" from list

### Issue: "Validation Too Slow"

**Fix**:
- Reduce validation mode:
  ```json
  {
    "verum.lsp.validationMode": "quick",
    "verum.lsp.diagnosticDelay": 500
  }
  ```
- Disable validation during typing:
  ```json
  {
    "verum.lsp.enableRefinementValidation": false
  }
  ```

### Issue: "Quick Fixes Not Appearing"

**Check**:
1. Diagnostic is from "verum" source
2. Cursor is on the error
3. LSP server is running

**Fix**:
- Restart language server
- Check LSP communication in output channel

## Development Workflow

### Watch Mode

```bash
cd editors/vscode
npm run watch
```

Then press F5 in VS Code to launch Extension Development Host.

### Making Changes

1. Edit TypeScript files in `src/`
2. Save (auto-compiles in watch mode)
3. Reload Extension Development Host (Cmd/Ctrl+R)

### Adding Features

1. **New Command**:
   - Add to `package.json` → `contributes.commands`
   - Register in `src/extension.ts` → `registerCommands()`

2. **New Setting**:
   - Add to `package.json` → `contributes.configuration.properties`
   - Access in code: `vscode.workspace.getConfiguration('verum.lsp').get('settingName')`

3. **New Syntax**:
   - Edit `syntaxes/verum.tmLanguage.json`
   - Reload Extension Development Host

### Testing

```bash
# Run tests
npm test

# Lint code
npm run lint

# Type check
npm run compile
```

## Next Steps

1. **Read Documentation**:
   - [README.md](./README.md) - User guide
   - [IMPLEMENTATION.md](./IMPLEMENTATION.md) - Technical details
   - [CHANGELOG.md](./CHANGELOG.md) - Version history

2. **Explore Features**:
   - Try all commands
   - Test quick fixes
   - Enable inlay hints
   - Run verification profile

3. **Configure for Your Workflow**:
   - Adjust validation mode
   - Set up keybindings
   - Configure SMT solver

4. **Contribute**:
   - Report issues
   - Suggest features
   - Submit pull requests

## Example Project

Create a sample project to test all features:

```bash
mkdir verum-demo
cd verum-demo
```

Create `demo.vr`:

```verum
// Refinement types demonstration
fn safe_divide(x: Int, y: Int{i != 0}) -> Int {
    x / y
}

fn safe_index(arr: List<Int>, idx: Int{i >= 0 && i < len(arr)}) -> Int {
    arr[idx]
}

fn non_empty_list(items: (v: List<Int>, proof: len(v) > 0)) -> Int {
    items.0[0]  // Safe: proof guarantees non-empty
}

// CBGR references
fn process_managed(data: &List<Int>) {
    // 15ns overhead per check
    for i in 0..data.len() {
        println(data[i]);
    }
}

fn process_checked(data: &checked List<Int>) {
    // 0ns overhead - verified at compile time
    for i in 0..data.len() {
        println(data[i]);
    }
}

// Context system
fn main() using [Console, FileSystem] {
    // Test refinements
    let result = safe_divide(10, 2);
    println(result);

    // This should error:
    // let bad = safe_divide(10, 0);

    // Test with list
    let numbers = [1, 2, 3, 4, 5];
    let value = safe_index(numbers, 2);
    println(value);
}
```

Open in VS Code and explore:
- Syntax highlighting
- Refinement validation
- Quick fixes
- Inlay hints
- Commands

## Local Testing Guide

### Quick Verification Checklist

After installing, verify these features work:

| Feature | How to Test | Expected Result |
|---------|-------------|-----------------|
| Syntax highlighting | Open any `.vr` file | Keywords colored |
| Snippets | Type `fn` and press Tab | Function template inserted |
| Bracket matching | Click on `{` | Matching `}` highlighted |
| Comment toggle | Select lines, press `Cmd/Ctrl+/` | Lines commented |
| Folding | Click fold icon near `fn` | Function body collapses |

### Testing Snippets

1. Create a new file `test.vr`
2. Type these prefixes and press **Tab**:
   - `fn` - Basic function
   - `async-fn` - Async function
   - `type` - Type alias
   - `match` - Match expression
   - `theorem` - Formal theorem
3. All 100+ snippets are listed in Command Palette: "Insert Snippet"

### Testing Syntax Highlighting

Create a file with various Verum constructs:

```verum
// All of these should have proper highlighting:

// Keywords
fn test() using [Database, Logger] {
    let x = 42;
    match x {
        0 => println("zero"),
        _ => println("other")
    }
}

// Refinement types
type Positive = Int{i > 0};
type NonEmpty<T> = List<T>{len(l) > 0};

// Async
async fn fetch() -> Result<Data, Error> {
    let response = await http.get(url);
    Ok(response)
}

// Proof keywords
theorem add_comm(a: Int, b: Int)
    ensures result == b + a
{
    calc {
        a + b == b + a by { ring }
    }
}

// Tagged literals
let query = sql#"SELECT * FROM users WHERE id = ?";
let pattern = rx#"^\d{3}-\d{4}$";

// References
fn process(data: &List<Int>) { }           // Managed
fn fast(data: &checked List<Int>) { }      // Verified
fn raw(data: &unsafe Int) { }              // Unsafe
```

### Running Unit Tests

```bash
cd editors/vscode

# Run all tests (downloads VS Code if needed)
npm test

# Expected output:
# Extension Test Suite
#   ✓ Extension should be present
#   ✓ Extension should activate
#   ✓ Should register verum language
#   ✓ Should register all commands
#   ...
```

### Troubleshooting Local Testing

**Problem**: F5 doesn't launch Extension Development Host

**Solution**:
1. Ensure you opened the `editors/vscode` folder (not parent)
2. Check `.vscode/launch.json` exists
3. Run `npm run compile` first

**Problem**: Syntax highlighting not working

**Solution**:
1. Check file extension is `.vr`
2. Click language mode in status bar → select "Verum"
3. Reload window: `Cmd/Ctrl+Shift+P` → "Reload Window"

**Problem**: Snippets not appearing

**Solution**:
1. Type prefix slowly (e.g., `f`, `n`)
2. Wait for autocomplete popup
3. Check snippets file is valid JSON: `node -e "require('./snippets/verum.json')"`

**Problem**: Tests fail with "Extension not found"

**Solution**:
1. Ensure `npm run compile` succeeded
2. Check `out/` directory contains `.js` files
3. Run `npm install` to ensure dependencies

## Resources

- **Language Server**: See `crates/verum_lsp/`
- **Specification**: `docs/detailed/25-developer-tooling.md`
- **Examples**: `examples/*.vr`
- **Tests**: `src/test/`

## Support

- **Issues**: GitHub Issues
- **Discussions**: GitHub Discussions
- **Documentation**: https://verum-lang.org/docs

---

**You're ready to use Verum with VS Code!**
