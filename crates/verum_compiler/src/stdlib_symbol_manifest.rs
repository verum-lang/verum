//! Build-time symbol manifest reader (#102).
//!
//! Counterpart to the `build_symbol_manifest` extractor in
//! `build.rs`. Reads the embedded archive of per-module top-level
//! declarations so the type-checker can resolve `mount core.X.{Y}`
//! WITHOUT parsing `core/X.vr` — critical for the on-demand loader's
//! late-resolution path (the user might `mount core.X.{Y}` referring
//! to a symbol whose owning module hasn't been pulled in yet).
//!
//! # Why a separate manifest from the dep graph
//!
//! The dep graph (`stdlib_dep_graph.rs`) records `mount` edges.
//! The symbol manifest records `pub fn`, `pub type`, `pub const`, …
//! definitions. They serve orthogonal queries:
//!
//!   * dep graph → "if I mount X, what other modules do I drag in?"
//!   * symbol manifest → "what's defined inside X?"
//!
//! Each is target-independent (no AST, no platform-specific
//! lowering — pure textual scan); both are pre-computed at build time
//! so the compiler avoids re-doing the work per session.
//!
//! # Performance contract
//!
//!   * Decompress + parse: ~3 ms (~170 KB compressed manifest).
//!   * Symbol lookup: O(1) HashMap.
//!   * Memory: ~1 MB after deserialisation (~36800 symbols × ~30 B).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Compressed manifest archive embedded at build time.
static MANIFEST_COMPRESSED: &[u8] = include_bytes!(env!("STDLIB_SYMBOL_MANIFEST_PATH"));

/// Lazily decompressed manifest.
static MANIFEST: OnceLock<Option<SymbolManifest>> = OnceLock::new();

/// Symbol kind matching `build.rs::SYM_*` constants. Stays binary-
/// stable so the on-disk format is portable across compiler builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Type = 0,
    Function = 1,
    Const = 2,
    Theorem = 3,
    Axiom = 4,
    Lemma = 5,
    Protocol = 6,
}

impl SymbolKind {
    fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Type),
            1 => Some(Self::Function),
            2 => Some(Self::Const),
            3 => Some(Self::Theorem),
            4 => Some(Self::Axiom),
            5 => Some(Self::Lemma),
            6 => Some(Self::Protocol),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Private = 0,
    Public = 1,
}

impl Visibility {
    fn from_u8(b: u8) -> Self {
        if b == 1 { Self::Public } else { Self::Private }
    }
}

/// One declared symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub name: String,
}

/// Per-module symbol table. The vec is preserved in source-declaration
/// order so consumers that care about positional resolution (e.g.
/// `mount X.{first_thing}` heuristics) get a stable view.
#[derive(Debug, Clone)]
pub struct ModuleSymbols {
    pub symbols: Vec<Symbol>,
}

impl ModuleSymbols {
    /// Look up a symbol by name. Linear scan — small per-module
    /// tables (typically <100 symbols), and the call site is cold
    /// (only invoked during lazy resolution).
    pub fn find(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.name == name)
    }

    /// Iterator over public symbols only — used by the late-resolver
    /// to enumerate the items a `mount X.*` would expose.
    pub fn public(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.visibility == Visibility::Public)
    }
}

/// Top-level manifest: module path → symbol table.
pub struct SymbolManifest {
    by_module: HashMap<String, ModuleSymbols>,
}

impl SymbolManifest {
    fn from_compressed(compressed: &[u8]) -> Option<Self> {
        if compressed.is_empty() {
            return None;
        }
        let raw = zstd::decode_all(compressed).ok()?;
        Self::parse_archive(&raw)
    }

    fn parse_archive(data: &[u8]) -> Option<Self> {
        if data.len() < 4 { return None; }
        let module_count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut cursor = 4usize;
        let mut by_module: HashMap<String, ModuleSymbols> = HashMap::with_capacity(module_count);

        for _ in 0..module_count {
            let module = read_str(data, &mut cursor)?;
            if cursor + 2 > data.len() { return None; }
            let sym_count =
                u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
            cursor += 2;

            let mut symbols = Vec::with_capacity(sym_count);
            for _ in 0..sym_count {
                if cursor + 2 > data.len() { return None; }
                let kind = SymbolKind::from_u8(data[cursor])?;
                let vis = Visibility::from_u8(data[cursor + 1]);
                cursor += 2;
                let name = read_str(data, &mut cursor)?;
                symbols.push(Symbol { kind, visibility: vis, name });
            }
            by_module.insert(module, ModuleSymbols { symbols });
        }

        Some(Self { by_module })
    }

    /// Look up the symbol table for a module path
    /// (e.g. `core.shell.exec`).
    pub fn module(&self, path: &str) -> Option<&ModuleSymbols> {
        self.by_module.get(path)
    }

    /// Look up a single symbol by `(module_path, name)`.
    pub fn lookup(&self, module: &str, name: &str) -> Option<&Symbol> {
        self.by_module.get(module)?.find(name)
    }

    /// Number of modules in the manifest.
    pub fn module_count(&self) -> usize {
        self.by_module.len()
    }

    /// Iterator over `(path, ModuleSymbols)` pairs for diagnostics.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ModuleSymbols)> {
        self.by_module.iter()
    }
}

fn read_str(data: &[u8], cursor: &mut usize) -> Option<String> {
    if *cursor + 2 > data.len() { return None; }
    let len = u16::from_le_bytes([data[*cursor], data[*cursor + 1]]) as usize;
    *cursor += 2;
    if *cursor + len > data.len() { return None; }
    let s = String::from_utf8(data[*cursor..*cursor + len].to_vec()).ok()?;
    *cursor += len;
    Some(s)
}

/// Get the global manifest. Builds on first call; later calls are
/// HashMap reads. Returns `None` if the embedded manifest is
/// unavailable (e.g. minimal builds without `core/`).
pub fn get_manifest() -> Option<&'static SymbolManifest> {
    MANIFEST
        .get_or_init(|| SymbolManifest::from_compressed(MANIFEST_COMPRESSED))
        .as_ref()
}

/// Whether the embedded manifest is available.
pub fn has_manifest() -> bool {
    !MANIFEST_COMPRESSED.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_loads() {
        let Some(m) = get_manifest() else { return; };
        assert!(m.module_count() > 0, "embedded manifest should have modules");
    }

    #[test]
    fn shell_module_has_symbols() {
        let Some(m) = get_manifest() else { return; };
        // `core.shell.exec` should have at least one public function
        // — it's the canonical shell-dispatch module.
        if let Some(syms) = m.module("core.shell.exec") {
            let public_fns: Vec<_> =
                syms.public().filter(|s| s.kind == SymbolKind::Function).collect();
            assert!(!public_fns.is_empty(),
                "core.shell.exec should declare at least one public fn");
        }
    }

    #[test]
    fn lookup_returns_none_for_unknown_symbol() {
        let Some(m) = get_manifest() else { return; };
        let result = m.lookup("core.shell.exec", "ZZZ_NONEXISTENT_SYMBOL_ZZZ");
        assert!(result.is_none());
    }

    #[test]
    fn protocol_symbols_classified_separately() {
        let Some(m) = get_manifest() else { return; };
        // Walk all modules; some module should have at least one
        // `Protocol` symbol — the stdlib has many `type X is protocol`
        // declarations.
        let any_protocol = m
            .iter()
            .any(|(_, ms)| ms.symbols.iter().any(|s| s.kind == SymbolKind::Protocol));
        assert!(any_protocol, "stdlib must contain protocol declarations");
    }

    #[test]
    fn private_visibility_default() {
        let Some(m) = get_manifest() else { return; };
        // Walk to find a private symbol — the stdlib uses internal
        // helpers in many modules.
        let any_private = m
            .iter()
            .any(|(_, ms)| ms.symbols.iter().any(|s| s.visibility == Visibility::Private));
        assert!(any_private, "stdlib should have at least one private symbol");
    }
}
