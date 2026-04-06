# Changelog

All notable changes to the Verum VS Code extension will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-03-15

### Added
- Initial release of Verum Language Support for VS Code
- Syntax highlighting for `.vr` files with TextMate grammar
  - Keywords: fn, let, is, using, provide, context, protocol
  - Types: Int, Float, Bool, Text, List, Map, Set, Maybe, Result
  - Refinement types: Int{i > 0}, List{len(l) > 0}
  - Operators, comments, strings, numbers
  - Attributes: @verify(proof), @hint("split")
- LSP integration with `verum-lsp-server`
  - Real-time refinement type validation
  - Custom LSP methods: validateRefinement, promoteToChecked, inferRefinement
  - Incremental document synchronization
- Refinement validation features
  - On-type validation with 200ms debouncing
  - Concrete counterexample display
  - Multiple validation modes: quick, thorough, complete
  - SMT solver integration (Z3, CVC5)
- Quick fix actions (6 categories)
  - Runtime check wrapping (Result<T, E>)
  - Inline refinement constraints
  - Sigma type conversion
  - Runtime assertions
  - Refinement weakening
  - Promotion to &checked references
- Code action provider
  - Context-aware quick fixes
  - Refactoring suggestions
  - Performance optimization hints
- Inlay hints provider
  - Inferred refinement types
  - Type information display
  - Configurable visibility
- Commands
  - verum.promoteToChecked (Ctrl+Alt+C)
  - verum.addRuntimeCheck
  - verum.inferRefinement (Ctrl+Alt+R)
  - verum.validateRefinementAtCursor (Ctrl+Alt+V)
  - verum.showVerificationProfile
  - verum.showCbgrProfile
  - verum.restartLanguageServer
- Language configuration
  - Bracket matching
  - Auto-closing pairs
  - Comment toggling
  - Indentation rules
  - Folding markers
- Comprehensive settings
  - LSP server configuration
  - Validation mode selection
  - SMT solver settings
  - CBGR profiling options
  - Diagnostic customization
- Status bar integration
  - Server status indicator
  - Real-time connection status
  - Error notifications
- Output channel for debugging
  - LSP communication tracing
  - Server logs
  - Error messages

### Developer Features
- TypeScript implementation with strict mode
- ESLint configuration
- Comprehensive type definitions
- Modular architecture
  - extension.ts: Main extension logic
  - refinementValidator.ts: Validation engine
  - codeActionProvider.ts: Quick fix provider
  - inlayHintsProvider.ts: Type hints
- Build scripts
  - Compile: `npm run compile`
  - Watch mode: `npm run watch`
  - Package: `npm run package`
  - Publish: `npm run publish`
- Testing infrastructure
  - Unit tests
  - Integration tests
  - Extension host testing

### Documentation
- Comprehensive README.md
- Configuration guide
- Usage examples
- Troubleshooting section
- Development guide
- Changelog

## [Unreleased]

### Added
- **100+ Code Snippets** covering all Verum language constructs:
  - Function declarations (13 variants): `fn`, `async-fn`, `fn-using`, `fn-contract`, `meta-fn`
  - Type definitions (12 variants): `type`, `type-refined`, `type-sigma`, `newtype`, `type-tensor`
  - Protocols and implementations: `protocol`, `impl`, `impl-for`
  - Context system (7 snippets): `context`, `provide`, `using-group`
  - Verification constructs: `theorem`, `lemma`, `proof`, `forall`, `exists`
  - Control flow: `if`, `match`, `for`, `while`, `loop`, `try-catch`
  - Async patterns: `spawn`, `stream`, `select`
  - Testing: `test`, `test-async`, `bench`

- **Enhanced TextMate Grammar** with 57+ new keywords:
  - Async keywords: `async`, `await`, `spawn`, `stream`
  - Proof keywords: `theorem`, `axiom`, `lemma`, `corollary`, `proof`
  - Verification: `requires`, `ensures`, `invariant`, `decreases`
  - Tactics: `calc`, `have`, `show`, `induction`, `smt`, `qed`
  - Reference modifiers: `checked`, `unsafe`
  - Type modifiers: `tensor`, `affine`, `linear`, `newtype`
  - Tagged literals: `sql#"..."`, `rx#"..."`, `mat#[...]`, `vec#[...]`
  - Interpolated strings: `f"Hello, {name}!"`
  - Operators: `|>` (pipeline), `??` (null coalescing), `?.` (optional chain)

- **Semantic Token Types** for LSP-enhanced highlighting:
  - `refinementType`, `contextProvider`, `contextConsumer`
  - `protocol`, `theorem`, `lemma`, `tactic`
  - `cbgrReference`, `checkedReference`, `unsafeReference`
  - `taggedLiteral`, `contractKeyword`, `metaFunction`

- **Semantic Token Modifiers**:
  - `refinement`, `checked`, `unsafe`, `managed`
  - `pure`, `async`, `fallible`, `contextual`
  - `verified`, `proof`, `meta`, `linear`, `affine`

- **Enhanced Language Configuration**:
  - Colorized bracket pairs
  - Auto-closing for angle brackets (generics)
  - `onEnterRules` for doc comments (`///`, `//!`)
  - Folding markers for `fn`, `type`, `protocol`, `theorem`, `proof`
  - Improved indentation rules

- **Comprehensive Local Testing Documentation**:
  - Quick verification checklist
  - Snippet testing guide
  - Syntax highlighting test file
  - Troubleshooting section

### Now Implemented (LSP Production Quality Upgrade)
- Cross-file goto definition, find references, and rename via workspace index
- Completion resolve (deferred documentation lookup)
- CodeLens resolve (lazy reference counting)
- Delta semantic tokens for efficient incremental highlighting
- Deep selection ranges (word → expression → statement → block → function → file)
- Type hierarchy navigation (supertypes/subtypes via protocol impl blocks)
- Diagnostic pull model support
- Inline values for let bindings
- Progress notifications for workspace indexing
- On-type formatting for `=>` (match arms) and `|>` (pipeline alignment)
- Hover provider with type information
- Signature help for function calls
- Call hierarchy support (incoming/outgoing calls)
- Document symbols provider (outline view)
- Workspace symbols provider (global search)
- Definition provider (AST + syntax tree)
- References provider (AST-based categorized search)
- Rename provider (with validation and cross-file support)
- Format document provider (trivia-preserving)
- Code lens for reference counts, run/test/verify actions
- Performance dashboard in sidebar (CBGR profiling)

### Planned Features
- Debugger integration
- Interactive verification mode
- Proof assistant integration
- Git integration for verification caching
- Remote LSP server support
- Multi-root workspace support
- Notebook support for Verum
