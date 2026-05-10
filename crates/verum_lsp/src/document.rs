//! Document state management for the LSP server
//!

//! Tracks the content, AST, type information, and diagnostics for each open document.
//!

//! This module provides comprehensive document tracking with:
//! - Real-time parsing and type checking
//! - Symbol table management for completion and navigation
//! - CBGR cost analysis for hover information

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use tower_lsp::lsp_types::*;
use verum_ast::{Attribute, FileId, ItemKind, LiteralKind, Module};
use verum_common::List;
use verum_diagnostics::Diagnostic;
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_types::{Type, TypeChecker};

/// Information about a symbol in the document
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// The name of the symbol
    pub name: String,
    /// The type of the symbol (if resolved)
    pub ty: Option<Type>,
    /// The span where the symbol is defined
    pub def_span: verum_ast::Span,
    /// The kind of symbol (function, type, variable, etc.)
    pub kind: SymbolKind,
    /// Documentation string (if any)
    pub docs: Option<String>,
    /// CBGR cost estimate for this symbol (if applicable)
    pub cbgr_cost: Option<CbgrCostInfo>,
}

/// Kind of symbol in the document
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Type,
    Variable,
    Parameter,
    Field,
    Variant,
    Protocol,
    Module,
    Constant,
}

/// Per-variant projection for [`SymbolKind`] — classifier flags
/// partition the nine declaration categories along orthogonal
/// axes (callable / type-defining / value-bearing / member /
/// namespace).  Used by the LSP outline / completion / rename
/// surfaces to filter symbols by structural role rather than
/// per-variant matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolKindMeta {
    /// Lower-snake-case wire form for telemetry surfaces.
    pub name: &'static str,
    /// The symbol can be *invoked* — Function singleton.  IDE
    /// completion uses this to gate `()` post-fix insertion.
    pub is_callable: bool,
    /// The symbol *introduces a type* — Type / Protocol /
    /// Variant.  Type-introducing symbols light up the
    /// type-search / "go to type definition" surface.
    pub is_type_introducing: bool,
    /// The symbol *binds a value* — Variable / Parameter /
    /// Constant.  Value bindings are the targets of
    /// assignment / dereference / reference operations.
    pub is_value_binding: bool,
    /// The symbol is a *member* of a containing declaration —
    /// Field (record member) + Variant (sum-type alternative).
    /// Members surface in the outline as nested children
    /// rather than top-level entries.
    pub is_member: bool,
    /// The symbol is a *namespace* — Module singleton.
    /// Modules are the only kind that contain other symbols
    /// recursively.
    pub is_namespace: bool,
    /// The symbol is *immutable* by language definition —
    /// Constant + Parameter (parameter immutability is
    /// enforced unless the function takes self/`&mut`).
    pub is_immutable: bool,
}

impl SymbolKind {
    /// All variants in declaration order — drives the LSP
    /// outline ordering and drift-pin tests.
    pub const ALL: &'static [Self] = &[
        Self::Function,
        Self::Type,
        Self::Variable,
        Self::Parameter,
        Self::Field,
        Self::Variant,
        Self::Protocol,
        Self::Module,
        Self::Constant,
    ];

    /// Static fact-pack.
    pub const fn meta(self) -> SymbolKindMeta {
        match self {
            SymbolKind::Function => SymbolKindMeta {
                name: "function",
                is_callable: true,
                is_type_introducing: false,
                is_value_binding: false,
                is_member: false,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Type => SymbolKindMeta {
                name: "type",
                is_callable: false,
                is_type_introducing: true,
                is_value_binding: false,
                is_member: false,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Variable => SymbolKindMeta {
                name: "variable",
                is_callable: false,
                is_type_introducing: false,
                is_value_binding: true,
                is_member: false,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Parameter => SymbolKindMeta {
                name: "parameter",
                is_callable: false,
                is_type_introducing: false,
                is_value_binding: true,
                is_member: false,
                is_namespace: false,
                is_immutable: true,
            },
            SymbolKind::Field => SymbolKindMeta {
                name: "field",
                is_callable: false,
                is_type_introducing: false,
                is_value_binding: false,
                is_member: true,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Variant => SymbolKindMeta {
                name: "variant",
                is_callable: false,
                is_type_introducing: true,
                is_value_binding: false,
                is_member: true,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Protocol => SymbolKindMeta {
                name: "protocol",
                is_callable: false,
                is_type_introducing: true,
                is_value_binding: false,
                is_member: false,
                is_namespace: false,
                is_immutable: false,
            },
            SymbolKind::Module => SymbolKindMeta {
                name: "module",
                is_callable: false,
                is_type_introducing: false,
                is_value_binding: false,
                is_member: false,
                is_namespace: true,
                is_immutable: false,
            },
            SymbolKind::Constant => SymbolKindMeta {
                name: "constant",
                is_callable: false,
                is_type_introducing: false,
                is_value_binding: true,
                is_member: false,
                is_namespace: false,
                is_immutable: true,
            },
        }
    }

    /// Wire-form snake_case name via meta().
    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Inverse of `as_str` — recover the kind from wire form.
    pub fn from_str(s: &str) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            let v = Self::ALL[i];
            if v.meta().name.as_bytes() == s.as_bytes() {
                return Some(v);
            }
            i += 1;
        }
        None
    }
}

/// CBGR cost information for a reference or operation
#[derive(Debug, Clone)]
pub struct CbgrCostInfo {
    /// Reference tier (0 = managed, 1 = checked, 2 = unsafe)
    pub tier: u8,
    /// Estimated cost per dereference in nanoseconds
    pub deref_cost_ns: u64,
    /// Description of the cost
    pub description: String,
}

impl CbgrCostInfo {
    /// Create cost info for Tier 0 (CBGR-managed) references
    pub fn tier0() -> Self {
        Self {
            tier: 0,
            deref_cost_ns: 15,
            description: "CBGR-managed reference (~15ns per dereference)".to_string(),
        }
    }

    /// Create cost info for Tier 1 (checked) references
    pub fn tier1() -> Self {
        Self {
            tier: 1,
            deref_cost_ns: 0,
            description: "Statically-verified reference (0ns overhead)".to_string(),
        }
    }

    /// Create cost info for Tier 2 (unsafe) references
    pub fn tier2() -> Self {
        Self {
            tier: 2,
            deref_cost_ns: 0,
            description: "Unsafe reference (0ns overhead, manual safety)".to_string(),
        }
    }
}

/// Stores the parsed and analyzed state of a document
pub struct DocumentState {
    /// The full text content of the document
    pub text: String,
    /// The parsed AST (if parsing succeeded)
    pub module: Option<Module>,
    /// Parsed diagnostics (syntax and type errors)
    pub diagnostics: List<Diagnostic>,
    /// Version number (incremented on each edit)
    pub version: i32,
    /// The file ID assigned to this document
    pub file_id: FileId,
    /// Symbol table: maps symbol names to their info
    pub symbols: HashMap<String, SymbolInfo>,
    /// Type information for expressions (maps byte offset to type)
    pub type_info: HashMap<usize, Type>,
}

// Implement Debug manually since Type doesn't implement Debug
impl std::fmt::Debug for DocumentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentState")
            .field(
                "text",
                &format!("{}...", &self.text.chars().take(50).collect::<String>()),
            )
            .field("module", &self.module.is_some())
            .field("diagnostics", &self.diagnostics.len())
            .field("version", &self.version)
            .field("file_id", &self.file_id)
            .field("symbols", &self.symbols.len())
            .field("type_info", &self.type_info.len())
            .finish()
    }
}

impl DocumentState {
    /// Create a new document state from text
    pub fn new(text: String, version: i32, file_id: FileId) -> Self {
        let mut state = Self {
            text,
            module: None,
            diagnostics: List::new(),
            version,
            file_id,
            symbols: HashMap::new(),
            type_info: HashMap::new(),
        };
        state.reparse();
        state
    }

    /// Update the document text and reparse
    pub fn update(&mut self, text: String, version: i32) {
        self.text = text;
        self.version = version;
        self.reparse();
    }

    /// Apply incremental changes to the document
    pub fn apply_changes(&mut self, changes: Vec<TextDocumentContentChangeEvent>, version: i32) {
        self.version = version;

        for change in changes {
            if let Some(range) = change.range {
                // Incremental change
                let start_offset = self.position_to_offset(range.start) as usize;
                let end_offset = self.position_to_offset(range.end) as usize;
                self.text
                    .replace_range(start_offset..end_offset, &change.text);
            } else {
                // Full document sync
                self.text = change.text;
            }
        }

        self.reparse();
    }

    /// Convert LSP Position to byte offset
    pub fn position_to_offset(&self, position: Position) -> u32 {
        let mut offset: u32 = 0;
        let mut current_line: u32 = 0;
        let mut current_char: u32 = 0;

        for ch in self.text.chars() {
            if current_line == position.line && current_char == position.character {
                return offset;
            }

            if ch == '\n' {
                current_line += 1;
                current_char = 0;
            } else {
                current_char += 1;
            }

            offset += ch.len_utf8() as u32;
        }

        offset
    }

    /// Reparse the document and update diagnostics
    fn reparse(&mut self) {
        self.diagnostics.clear();
        self.symbols.clear();
        self.type_info.clear();

        // Parse the document
        let lexer = Lexer::new(&self.text, self.file_id);
        let parser = VerumParser::new();

        match parser.parse_module(lexer, self.file_id) {
            Ok(module) => {
                // Parsing succeeded
                self.module = Some(module.clone());

                // Build symbol table from AST
                self.build_symbol_table(&module);

                // Perform type checking
                self.run_type_checker(&module);
            }
            Err(parse_errors) => {
                // Parsing failed
                self.module = None;

                // Convert parse errors to diagnostics
                for error in parse_errors {
                    use verum_diagnostics::{DiagnosticBuilder, Severity};

                    let diagnostic = DiagnosticBuilder::new(Severity::Error)
                        .message(error.to_string())
                        .build();

                    self.diagnostics.push(diagnostic);
                }
            }
        }
    }

    /// Build symbol table from parsed AST
    fn build_symbol_table(&mut self, module: &Module) {
        use verum_ast::decl::TypeDeclBody;

        for item in module.items.iter() {
            match &item.kind {
                ItemKind::Function(func) => {
                    // Determine CBGR cost based on reference types in signature
                    let cbgr_cost = self.analyze_function_cbgr_cost(func);

                    // Extract documentation from attributes
                    let docs = self.extract_doc_from_attributes(&item.attributes);

                    let symbol = SymbolInfo {
                        name: func.name.as_str().to_string(),
                        ty: None, // Will be filled by type checker
                        def_span: func.span,
                        kind: SymbolKind::Function,
                        docs,
                        cbgr_cost,
                    };
                    self.symbols.insert(func.name.as_str().to_string(), symbol);

                    // Add parameters to symbol table
                    for param in func.params.iter() {
                        if let verum_ast::decl::FunctionParamKind::Regular { pattern, .. } =
                            &param.kind
                            && let verum_ast::PatternKind::Ident { name, .. } = &pattern.kind
                        {
                            let param_symbol = SymbolInfo {
                                name: name.as_str().to_string(),
                                ty: None,
                                def_span: pattern.span,
                                kind: SymbolKind::Parameter,
                                docs: None,
                                cbgr_cost: None,
                            };
                            self.symbols.insert(name.as_str().to_string(), param_symbol);
                        }
                    }
                }
                ItemKind::Type(type_decl) => {
                    // Extract documentation from attributes
                    let docs = self.extract_doc_from_attributes(&item.attributes);

                    let symbol = SymbolInfo {
                        name: type_decl.name.as_str().to_string(),
                        ty: None,
                        def_span: type_decl.span,
                        kind: SymbolKind::Type,
                        docs,
                        cbgr_cost: None,
                    };
                    self.symbols
                        .insert(type_decl.name.as_str().to_string(), symbol);

                    // Add variant constructors and record fields
                    match &type_decl.body {
                        TypeDeclBody::Variant(variants) => {
                            for variant in variants {
                                let variant_symbol = SymbolInfo {
                                    name: variant.name.as_str().to_string(),
                                    ty: None,
                                    def_span: variant.span,
                                    kind: SymbolKind::Variant,
                                    docs: None,
                                    cbgr_cost: None,
                                };
                                let full_name = format!(
                                    "{}::{}",
                                    type_decl.name.as_str(),
                                    variant.name.as_str()
                                );
                                self.symbols.insert(full_name, variant_symbol);
                            }
                        }
                        TypeDeclBody::Record(fields) => {
                            for field in fields {
                                let field_symbol = SymbolInfo {
                                    name: field.name.as_str().to_string(),
                                    ty: None,
                                    def_span: field.span,
                                    kind: SymbolKind::Field,
                                    docs: None,
                                    cbgr_cost: None,
                                };
                                let full_name =
                                    format!("{}.{}", type_decl.name.as_str(), field.name.as_str());
                                self.symbols.insert(full_name, field_symbol);
                            }
                        }
                        _ => {}
                    }
                }
                ItemKind::Protocol(protocol) => {
                    // Extract documentation from attributes
                    let docs = self.extract_doc_from_attributes(&item.attributes);

                    let symbol = SymbolInfo {
                        name: protocol.name.as_str().to_string(),
                        ty: None,
                        def_span: protocol.span,
                        kind: SymbolKind::Protocol,
                        docs,
                        cbgr_cost: None,
                    };
                    self.symbols
                        .insert(protocol.name.as_str().to_string(), symbol);
                }
                ItemKind::Const(const_decl) => {
                    // Extract documentation from attributes
                    let docs = self.extract_doc_from_attributes(&item.attributes);

                    let symbol = SymbolInfo {
                        name: const_decl.name.as_str().to_string(),
                        ty: None,
                        def_span: const_decl.span,
                        kind: SymbolKind::Constant,
                        docs,
                        cbgr_cost: None,
                    };
                    self.symbols
                        .insert(const_decl.name.as_str().to_string(), symbol);
                }
                ItemKind::Module(mod_decl) => {
                    // Extract documentation from attributes
                    let docs = self.extract_doc_from_attributes(&item.attributes);

                    let symbol = SymbolInfo {
                        name: mod_decl.name.as_str().to_string(),
                        ty: None,
                        def_span: mod_decl.span,
                        kind: SymbolKind::Module,
                        docs,
                        cbgr_cost: None,
                    };
                    self.symbols
                        .insert(mod_decl.name.as_str().to_string(), symbol);
                }
                _ => {}
            }
        }
    }

    /// Analyze CBGR cost for a function based on its reference types
    fn analyze_function_cbgr_cost(&self, func: &verum_ast::FunctionDecl) -> Option<CbgrCostInfo> {
        use verum_ast::ty::TypeKind;

        let mut has_managed_ref = false;
        let has_checked_ref = false;
        let has_unsafe_ref = false;

        // Check parameter types for references
        for param in func.params.iter() {
            if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind {
                match &ty.kind {
                    TypeKind::Reference { .. } => {
                        // Default references are CBGR-managed (Tier 0)
                        has_managed_ref = true;
                    }
                    TypeKind::Path(path) => {
                        // Check for Heap<T> or Shared<T> types
                        if let Some(seg) = path.segments.first()
                            && let verum_ast::ty::PathSegment::Name(ident) = seg
                        {
                            match ident.as_str() {
                                "Heap" | "Shared" => has_managed_ref = true,
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check return type
        if let Some(ret_ty) = &func.return_type
            && let TypeKind::Reference { .. } = &ret_ty.kind
        {
            has_managed_ref = true;
        }

        // Determine overall CBGR tier
        if has_unsafe_ref {
            Some(CbgrCostInfo::tier2())
        } else if has_checked_ref {
            Some(CbgrCostInfo::tier1())
        } else if has_managed_ref {
            Some(CbgrCostInfo::tier0())
        } else {
            None
        }
    }

    /// Extract documentation from item attributes
    ///

    /// Searches for doc comment attributes (/// or //!) and combines them into
    /// a single documentation string.
    fn extract_doc_from_attributes(&self, attributes: &[Attribute]) -> Option<String> {
        let mut doc_lines = Vec::new();

        for attr in attributes.iter() {
            // Check for doc attribute
            if attr.is_named("doc") {
                // Extract doc comment text from attribute args
                if let Some(ref args) = attr.args {
                    for arg in args.iter() {
                        // Doc comments are typically stored as string literal expressions
                        if let verum_ast::expr::ExprKind::Literal(lit) = &arg.kind
                            && let LiteralKind::Text(text) = &lit.kind
                        {
                            doc_lines.push(text.to_string());
                        }
                    }
                }
            }
        }

        if doc_lines.is_empty() {
            None
        } else {
            Some(doc_lines.join("\n"))
        }
    }

    /// Run type checker on the module
    fn run_type_checker(&mut self, module: &Module) {
        let mut type_checker = TypeChecker::new();

        // Register builtins (primitives like Int, Float, Bool, Text, etc.)
        // and built-in functions like print, assert, panic
        type_checker.register_builtins();

        // Type check each item in the module
        for item in module.items.iter() {
            match type_checker.check_item(item) {
                Ok(()) => {
                    // Type checking succeeded for this item
                }
                Err(type_error) => {
                    // Convert to diagnostic and store
                    let diagnostic = type_error.to_diagnostic();
                    self.diagnostics.push(diagnostic);
                }
            }
        }
    }

    /// Get symbol info by name
    pub fn get_symbol(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(name)
    }

    /// Get type at a specific byte offset
    pub fn get_type_at_offset(&self, offset: usize) -> Option<&Type> {
        self.type_info.get(&offset)
    }

    /// Find all symbols matching a prefix
    pub fn find_symbols_with_prefix(&self, prefix: &str) -> Vec<&SymbolInfo> {
        self.symbols
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, info)| info)
            .collect()
    }

    /// Get all symbols of a specific kind
    pub fn get_symbols_by_kind(&self, kind: SymbolKind) -> Vec<&SymbolInfo> {
        self.symbols
            .values()
            .filter(|info| info.kind == kind)
            .collect()
    }

    /// Get the word at a given position.
    ///

    /// Uses `verum_common::text_utf8::find_word_bounds` for the
    /// UTF-8-safe walk. `position_to_offset` returns a byte offset;
    /// the helper handles multi-byte clamping and identifier
    /// boundary detection.
    pub fn word_at_position(&self, position: Position) -> Option<String> {
        let offset = self.position_to_offset(position) as usize;
        let (start, end) = verum_common::text_utf8::find_word_bounds(
            self.text.as_str(),
            offset,
            is_identifier_char,
        )?;
        Some(self.text[start..end].to_string())
    }

    /// Get the line at a given line number
    pub fn get_line(&self, line_number: u32) -> Option<&str> {
        self.text.lines().nth(line_number as usize)
    }
}

/// Check if a character can be part of an identifier
fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Global document store
pub struct DocumentStore {
    documents: DashMap<Url, RwLock<DocumentState>>,
    next_file_id: RwLock<u32>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            next_file_id: RwLock::new(1),
        }
    }

    /// Get or create a file ID for a URL
    fn get_file_id(&self, _uri: &Url) -> FileId {
        let mut next_id = self.next_file_id.write();
        let id = *next_id;
        *next_id += 1;
        FileId::new(id)
    }

    /// Open a new document
    pub fn open(&self, uri: Url, text: String, version: i32) -> FileId {
        let file_id = self.get_file_id(&uri);
        let state = DocumentState::new(text, version, file_id);
        self.documents.insert(uri, RwLock::new(state));
        file_id
    }

    /// Close a document
    pub fn close(&self, uri: &Url) {
        self.documents.remove(uri);
    }

    /// Update a document with new text
    pub fn update(&self, uri: &Url, text: String, version: i32) {
        if let Some(entry) = self.documents.get(uri) {
            entry.write().update(text, version);
        }
    }

    /// Apply incremental changes to a document
    pub fn apply_changes(
        &self,
        uri: &Url,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) {
        if let Some(entry) = self.documents.get(uri) {
            entry.write().apply_changes(changes, version);
        }
    }

    /// Get diagnostics for a document
    pub fn get_diagnostics(&self, uri: &Url) -> List<Diagnostic> {
        self.documents
            .get(uri)
            .map(|entry| entry.read().diagnostics.clone())
            .unwrap_or_default()
    }

    /// Get the module for a document
    pub fn get_module(&self, uri: &Url) -> Option<Module> {
        self.documents
            .get(uri)
            .and_then(|entry| entry.read().module.clone())
    }

    /// Get the text content of a document
    pub fn get_text(&self, uri: &Url) -> Option<String> {
        self.documents
            .get(uri)
            .map(|entry| entry.read().text.clone())
    }

    /// Execute a function with read access to a document
    pub fn with_document<F, R>(&self, uri: &Url, f: F) -> Option<R>
    where
        F: FnOnce(&DocumentState) -> R,
    {
        self.documents.get(uri).map(|entry| f(&entry.read()))
    }

    /// Execute a function with write access to a document
    pub fn with_document_mut<F, R>(&self, uri: &Url, f: F) -> Option<R>
    where
        F: FnOnce(&mut DocumentState) -> R,
    {
        self.documents.get(uri).map(|entry| f(&mut entry.write()))
    }

    /// Iterate over all documents in the store.
    ///

    /// This method enables workspace-wide operations like symbol search.
    /// The callback receives each document's URI and state for processing.
    pub fn for_each_document<F>(&self, mut f: F)
    where
        F: FnMut(&Url, &DocumentState),
    {
        for entry in self.documents.iter() {
            f(entry.key(), &entry.value().read());
        }
    }

    /// Collect results from all documents.
    ///

    /// Maps a function over all documents and collects non-None results.
    pub fn collect_from_documents<F, R>(&self, mut f: F) -> Vec<R>
    where
        F: FnMut(&Url, &DocumentState) -> Option<R>,
    {
        let mut results = Vec::new();
        for entry in self.documents.iter() {
            if let Some(result) = f(entry.key(), &entry.value().read()) {
                results.push(result);
            }
        }
        results
    }

    /// Flat-map results from all documents.
    ///

    /// Maps a function over all documents and flattens the results.
    pub fn flat_collect_from_documents<F, R>(&self, mut f: F) -> Vec<R>
    where
        F: FnMut(&Url, &DocumentState) -> Vec<R>,
    {
        let mut results = Vec::new();
        for entry in self.documents.iter() {
            results.extend(f(entry.key(), &entry.value().read()));
        }
        results
    }

    /// Get the count of open documents
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Get all document URIs
    pub fn document_uris(&self) -> Vec<Url> {
        self.documents
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get a reference to a document's RwLock for advanced access patterns
    ///

    /// Returns an owned reference that can be stored/passed around.
    /// For most cases, prefer `with_document` or `with_document_mut`.
    pub fn get_document(
        &self,
        uri: &Url,
    ) -> Option<dashmap::mapref::one::Ref<'_, Url, RwLock<DocumentState>>> {
        self.documents.get(uri)
    }
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod symbol_kind_meta_drift_pins {
    use super::*;

    /// Drift-pin: `SymbolKind::meta()` is the canonical
    /// per-variant data table.  Pins variant count, six
    /// classifier flags partition the nine declaration kinds,
    /// cross-cutting invariants, and the from_str/as_str
    /// round-trip.
    #[test]
    fn meta_pin_symbol_kind_round_trip_and_partitions() {
        // 1. Variant count + names + uniqueness.
        assert_eq!(SymbolKind::ALL.len(), 9);
        let mut seen = std::collections::HashSet::new();
        for k in SymbolKind::ALL {
            let m = k.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?}: name not snake_case",
                k
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", k);
            assert_eq!(SymbolKind::from_str(m.name), Some(*k));
        }
        assert_eq!(SymbolKind::from_str("nope"), None);

        // 2. is_callable: Function singleton.
        let call: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_callable)
            .copied()
            .collect();
        assert_eq!(call, vec![SymbolKind::Function]);

        // 3. is_type_introducing: Type + Variant + Protocol.
        let ty: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_type_introducing)
            .copied()
            .collect();
        assert_eq!(
            ty,
            vec![SymbolKind::Type, SymbolKind::Variant, SymbolKind::Protocol],
        );

        // 4. is_value_binding: Variable + Parameter + Constant.
        let vb: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_value_binding)
            .copied()
            .collect();
        assert_eq!(
            vb,
            vec![
                SymbolKind::Variable,
                SymbolKind::Parameter,
                SymbolKind::Constant,
            ],
        );

        // 5. is_member: Field + Variant.
        let mem: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_member)
            .copied()
            .collect();
        assert_eq!(mem, vec![SymbolKind::Field, SymbolKind::Variant]);

        // 6. is_namespace: Module singleton.
        let ns: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_namespace)
            .copied()
            .collect();
        assert_eq!(ns, vec![SymbolKind::Module]);

        // 7. is_immutable: Parameter + Constant.
        let im: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_immutable)
            .copied()
            .collect();
        assert_eq!(im, vec![SymbolKind::Parameter, SymbolKind::Constant]);

        // 8. Cross-cutting: is_callable / is_namespace are
        //    disjoint from each other and from
        //    is_value_binding.  A function is neither a value
        //    binding nor a namespace; a module is neither
        //    callable nor a value.
        for k in SymbolKind::ALL {
            let m = k.meta();
            assert!(
                !(m.is_callable && m.is_value_binding),
                "{:?}: callable ⊕ value_binding",
                k
            );
            assert!(
                !(m.is_namespace && m.is_callable),
                "{:?}: namespace ⊕ callable",
                k
            );
            assert!(
                !(m.is_namespace && m.is_value_binding),
                "{:?}: namespace ⊕ value_binding",
                k
            );
        }

        // 9. is_immutable ⇒ is_value_binding (only value
        //    bindings have immutability semantics — types and
        //    namespaces aren't "mutable" in a meaningful
        //    sense).
        for k in SymbolKind::ALL {
            let m = k.meta();
            assert!(
                !m.is_immutable || m.is_value_binding,
                "{:?}: immutable ⇒ value_binding",
                k
            );
        }

        // 10. Variant is the unique symbol that's both a
        //     member AND a type-introducing kind — sum-type
        //     alternatives are members of their parent type
        //     declaration but each introduces a constructor
        //     name in the type space.  Pinned so a future
        //     refactor that removes one of those bits surfaces
        //     here.
        let intersection: Vec<_> = SymbolKind::ALL
            .iter()
            .filter(|k| k.meta().is_member && k.meta().is_type_introducing)
            .copied()
            .collect();
        assert_eq!(intersection, vec![SymbolKind::Variant]);
    }
}
