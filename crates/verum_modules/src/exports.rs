//! Export management for modules.
//!
//! Tracks what items are exported by each module and handles re-exports.
//! Re-exports make internal items available through a different path using
//! `public import internal.Item` or `public import internal.Item as PublicName`.
//! Re-exported types preserve their refinements across module boundaries.

use crate::error::{ModuleError, ModuleResult};
use crate::path::ModuleId;
use crate::refinement_info::{RefinementContract, RefinementInfo};
use serde::{Deserialize, Serialize};
use verum_ast::{Span, Visibility};
use verum_common::{Map, Maybe, Text};

/// An item exported by a module.
/// Includes refinement and contract information for cross-module verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedItem {
    /// Name of the exported item
    pub name: Text,
    /// Kind of item (function, type, module, etc.)
    pub kind: ExportKind,
    /// Visibility level
    pub visibility: Visibility,
    /// Source module (might differ from exporting module for re-exports)
    pub source_module: ModuleId,
    /// Original name (before renaming, if any)
    pub original_name: Option<Text>,
    /// Span in the exporting module
    pub span: Span,
    /// Refinement type information (for refined types like Int{> 0}).
    /// When a type with refinements is exported, the refinement becomes
    /// part of the public API contract and is preserved across module
    /// boundaries including through re-exports.
    pub refinement: Maybe<RefinementInfo>,
    /// Visibility of the refinement predicate (can differ from type visibility).
    /// Public predicates are reusable, internal are crate-only, private are
    /// implementation details. Validation responsibility: exporting module
    /// validates refinements; importing module trusts them.
    pub predicate_visibility: Visibility,
    /// Design-by-Contract predicates for cross-module verification.
    /// Contains @requires, @ensures, @invariant clauses extracted from
    /// function/type declarations. Used for SMT-based verification at
    /// module boundaries.
    pub contract: Maybe<RefinementContract>,
}

impl ExportedItem {
    pub fn new(
        name: impl Into<Text>,
        kind: ExportKind,
        visibility: Visibility,
        source_module: ModuleId,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            predicate_visibility: visibility.clone(),
            visibility,
            source_module,
            original_name: None,
            span,
            refinement: Maybe::None,
            contract: Maybe::None,
        }
    }

    /// Create a re-exported item with renaming
    pub fn with_rename(
        name: impl Into<Text>,
        original_name: impl Into<Text>,
        kind: ExportKind,
        visibility: Visibility,
        source_module: ModuleId,
        span: Span,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            predicate_visibility: visibility.clone(),
            visibility,
            source_module,
            original_name: Some(original_name.into()),
            span,
            refinement: Maybe::None,
            contract: Maybe::None,
        }
    }

    /// Check if this is a re-export
    pub fn is_reexport(&self) -> bool {
        self.original_name.is_some()
    }

    /// Get the effective name (used for lookups)
    pub fn effective_name(&self) -> &Text {
        &self.name
    }

    /// Create an exported item with refinement information.
    /// Refinements are preserved across module boundaries and re-exports.
    pub fn with_refinement(
        mut self,
        refinement: RefinementInfo,
        predicate_visibility: Visibility,
    ) -> Self {
        self.refinement = Maybe::Some(refinement);
        self.predicate_visibility = predicate_visibility;
        self
    }

    /// Check if this item has a refinement
    pub fn has_refinement(&self) -> bool {
        matches!(self.refinement, Maybe::Some(_))
    }

    /// Get the refinement info if present
    pub fn get_refinement(&self) -> Maybe<&RefinementInfo> {
        self.refinement.as_ref()
    }

    /// Check if the refinement predicate is accessible from a given module.
    /// Uses the standard five-level visibility system for predicate access.
    pub fn is_predicate_accessible(
        &self,
        from_module: &crate::path::ModulePath,
        module_path: &crate::path::ModulePath,
    ) -> bool {
        // Use the same visibility checking logic as for items
        match &self.predicate_visibility {
            Visibility::Public => true,
            Visibility::PublicCrate => {
                module_path.segments().first() == from_module.segments().first()
            }
            Visibility::PublicSuper => match module_path.parent() {
                Some(parent) => &parent == from_module,
                None => false,
            },
            Visibility::PublicIn(path) => {
                let path_str = path
                    .segments
                    .iter()
                    .filter_map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                let target_path = crate::path::ModulePath::from_str(&path_str);
                target_path.is_prefix_of(from_module) || from_module == &target_path
            }
            Visibility::Private | Visibility::Internal | Visibility::Protected => {
                module_path == from_module
            }
        }
    }

    /// Add Design-by-Contract predicates (@requires/@ensures/@invariant)
    /// for cross-module SMT verification.
    pub fn with_contract(mut self, contract: RefinementContract) -> Self {
        self.contract = Maybe::Some(contract);
        self
    }

    /// Check if this item has a contract
    pub fn has_contract(&self) -> bool {
        matches!(self.contract, Maybe::Some(_))
    }

    /// Get the contract if present
    pub fn get_contract(&self) -> Maybe<&RefinementContract> {
        self.contract.as_ref()
    }

    /// Check if contract predicates need runtime verification
    pub fn needs_runtime_contract_check(&self) -> bool {
        match &self.contract {
            Maybe::Some(c) => c.status.needs_runtime_check(),
            Maybe::None => false,
        }
    }
}

/// The kind of exported item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExportKind {
    /// Function or method
    Function,
    /// Type definition
    Type,
    /// Protocol (trait)
    Protocol,
    /// Module
    Module,
    /// Constant
    Const,
    /// Static
    Static,
    /// Meta (macro)
    Meta,
    /// Predicate
    Predicate,
    /// Context
    Context,
    /// Context group
    ContextGroup,
}

impl ExportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportKind::Function => "function",
            ExportKind::Type => "type",
            ExportKind::Protocol => "protocol",
            ExportKind::Module => "module",
            ExportKind::Const => "const",
            ExportKind::Static => "static",
            ExportKind::Meta => "meta",
            ExportKind::Predicate => "predicate",
            ExportKind::Context => "context",
            ExportKind::ContextGroup => "context group",
        }
    }
}

impl std::fmt::Display for ExportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Table of all exports from a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTable {
    /// All exported items by name
    exports: Map<Text, ExportedItem>,
    /// The module ID this table belongs to
    module_id: Option<ModuleId>,
    /// The module path this table belongs to (for visibility checks)
    module_path: Option<crate::path::ModulePath>,
}

impl ExportTable {
    pub fn new() -> Self {
        Self {
            exports: Map::new(),
            module_id: None,
            module_path: None,
        }
    }

    /// Set the module ID for this export table
    pub fn set_module_id(&mut self, id: ModuleId) {
        self.module_id = Some(id);
    }

    /// Set the module path for this export table (needed for visibility checks)
    pub fn set_module_path(&mut self, path: crate::path::ModulePath) {
        self.module_path = Some(path);
    }

    /// Get the module path for this export table
    pub fn module_path(&self) -> Option<&crate::path::ModulePath> {
        self.module_path.as_ref()
    }

    /// Add an exported item
    ///
    /// Re-exports with the same name as an existing export are allowed when:
    /// - They have the same kind and source module (deduplication)
    /// - The new export is a non-module kind and existing is a Module kind
    ///   (allows `panic` module and `panic` function to coexist)
    ///
    /// In the case of same-name different-kind exports, the non-module export
    /// takes precedence to maintain Rust-like namespace semantics where
    /// re-exported items shadow module names.
    pub fn add_export(&mut self, item: ExportedItem) -> ModuleResult<()> {
        let name = item.name.clone();

        // Check for conflicts
        if let Some(existing) = self.exports.get(&name) {
            // Same kind and source module = dedupe, just skip
            if existing.kind == item.kind && existing.source_module == item.source_module {
                return Ok(());
            }

            // Allow non-module export to shadow module export
            // This enables `public module panic;` + `public import .panic.panic;`
            // where the function `panic` takes precedence over module `panic`
            if existing.kind == ExportKind::Module && item.kind != ExportKind::Module {
                // Replace module with the specific item
                self.exports.insert(name, item);
                return Ok(());
            }

            // Allow module to be shadowed by non-module (already handled above)
            // But don't allow module to shadow non-module
            if item.kind == ExportKind::Module && existing.kind != ExportKind::Module {
                // Keep the existing non-module export
                return Ok(());
            }

            // Diagnose the common domain-specific case first: a
            // variant constructor (registered as Function) clashes
            // with a Type declared in the same module. Variants
            // flatten into the parent module's namespace, so this
            // is a name-space collision, not a ModuleId bug.
            let same_module = existing.source_module == item.source_module;
            if same_module
                && existing.kind == ExportKind::Type
                && item.kind == ExportKind::Function
            {
                return Err(ModuleError::Other {
                    message: Text::from(format!(
                        "variant constructor `{name}` clashes with the `type {name}` \
                         declared in the same module — variants flatten into the \
                         parent module's namespace; rename the variant or the type"
                    )),
                    span: Some(item.span),
                });
            }
            if same_module
                && existing.kind == ExportKind::Function
                && item.kind == ExportKind::Type
            {
                return Err(ModuleError::Other {
                    message: Text::from(format!(
                        "type `{name}` clashes with the variant constructor `{name}` \
                         declared in the same module — variants flatten into the \
                         parent module's namespace; rename the variant or the type"
                    )),
                    span: Some(item.span),
                });
            }

            // Real conflict: same kind, different source.
            // At this point ModuleId dedupe (see ModuleRegistry::
            // register) guarantees `existing.source_module` is a
            // stable identifier for the original definition site.
            return Err(ModuleError::Other {
                message: Text::from(format!(
                    "conflicting export: `{name}` already exported as {kind} from {source:?}; \
                     both sides resolve to the same name in the importing scope — \
                     rename one or scope one behind a non-public re-export",
                    name = name,
                    kind = existing.kind,
                    source = existing.source_module,
                )),
                span: Some(item.span),
            });
        }

        self.exports.insert(name, item);
        Ok(())
    }

    /// Get an exported item by name
    pub fn get(&self, name: &Text) -> Maybe<&ExportedItem> {
        match self.exports.get(name) {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }

    /// Check if an item is exported
    pub fn contains(&self, name: &str) -> bool {
        self.exports.contains_key(&Text::from(name))
    }

    /// Get all exported items
    pub fn all_exports(&self) -> impl Iterator<Item = (&Text, &ExportedItem)> {
        self.exports.iter()
    }

    /// Get exports of a specific kind
    pub fn exports_of_kind(&self, kind: ExportKind) -> impl Iterator<Item = &ExportedItem> {
        self.exports.values().filter(move |item| item.kind == kind)
    }

    /// Get public exports only
    pub fn public_exports(&self) -> impl Iterator<Item = &ExportedItem> {
        self.exports
            .values()
            .filter(|item| item.visibility == Visibility::Public)
    }

    /// Check if a name is visible from another module (by ModuleId - requires module_path to be set)
    ///
    /// Note: For proper visibility checking, prefer `is_visible_from_path` which takes a ModulePath.
    /// This method is maintained for backward compatibility but returns false for non-Public
    /// visibility if module_path is not set.
    pub fn is_visible_from(&self, name: &Text, _from_module: ModuleId) -> bool {
        if let Maybe::Some(item) = self.get(name) {
            match &item.visibility {
                Visibility::Public => true,
                // Without module path information, we cannot properly verify these
                // Return false to be safe (conservative approach)
                Visibility::PublicCrate | Visibility::PublicSuper | Visibility::PublicIn(_) => {
                    false
                }
                Visibility::Private => false,
                Visibility::Internal => false,
                Visibility::Protected => false,
            }
        } else {
            false
        }
    }

    /// Check if a name is visible from another module path.
    ///
    /// Implements proper five-level visibility checking (Private/Public/PublicCrate/PublicSuper/PublicIn)
    ///
    /// - `Public`: visible everywhere
    /// - `PublicCrate`: visible only within the same crate (first path segment)
    /// - `PublicSuper`: visible to parent module
    /// - `PublicIn(path)`: visible within specified module tree
    /// - `Private`: visible only within the same module
    pub fn is_visible_from_path(&self, name: &Text, from_module: &crate::path::ModulePath) -> bool {
        let item_module = match &self.module_path {
            Some(path) => path,
            None => return false, // Cannot check visibility without module path
        };

        if let Maybe::Some(item) = self.get(name) {
            match &item.visibility {
                Visibility::Public => true,

                Visibility::PublicCrate => {
                    // public(crate): same crate check via first path segment
                    // Check if from_module is in the same crate as item_module
                    // Crate is determined by the first segment of the path
                    item_module.segments().first() == from_module.segments().first()
                }

                Visibility::PublicSuper => {
                    // public(super): immediate parent module check
                    // Check if from_module is the parent of item_module
                    match item_module.parent() {
                        Some(parent) => &parent == from_module,
                        None => false, // Root module has no parent
                    }
                }

                Visibility::PublicIn(path) => {
                    // public(in path): module subtree membership check
                    // Check if from_module is within the specified path scope
                    // Convert verum_ast::Path to ModulePath for comparison
                    let path_str = path
                        .segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::PathSegment::Name(ident) => Some(ident.name.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(".");
                    let target_path = crate::path::ModulePath::from_str(&path_str);

                    // from_module is visible if it equals target_path or is a descendant
                    target_path.is_prefix_of(from_module) || from_module == &target_path
                }

                Visibility::Private => {
                    // Private items are only visible within the same module
                    item_module == from_module
                }

                Visibility::Internal => {
                    // Internal items are only visible within the same module (same as Private)
                    item_module == from_module
                }

                Visibility::Protected => {
                    // Protected items are only visible within the same module (same as Private)
                    item_module == from_module
                }
            }
        } else {
            false
        }
    }

    /// Merge another export table (for re-exports)
    ///
    /// Re-exported types preserve their refinements and contracts.
    pub fn merge(&mut self, other: &ExportTable, visibility: Visibility) -> ModuleResult<()> {
        for (_name, item) in other.all_exports() {
            // Only merge items that are at least as visible as requested
            // Public items can be re-exported at any visibility
            // PublicCrate items can only be re-exported at crate or lower visibility
            // PublicSuper and PublicIn items need special handling
            let should_merge = match (&item.visibility, &visibility) {
                (Visibility::Public, _) => true,
                (
                    Visibility::PublicCrate,
                    Visibility::PublicCrate | Visibility::PublicSuper | Visibility::Private,
                ) => true,
                (Visibility::PublicCrate, Visibility::PublicIn(_)) => true,
                (Visibility::PublicSuper, Visibility::PublicSuper | Visibility::Private) => true,
                (Visibility::PublicIn(_), Visibility::Private) => true,
                _ => false,
            };

            if should_merge {
                let mut reexport = item.clone();
                reexport.visibility = visibility.clone();

                // Preserve refinement information when re-exporting
                // Re-export preserves refinement and contract information
                // Refinement and predicate visibility are preserved from the original item

                self.add_export(reexport)?;
            }
        }
        Ok(())
    }

    /// Validate that refinement predicates are accessible when re-exporting
    ///
    /// Validates that all exported refinement predicates are accessible
    /// from the requesting module using the five-level visibility system.
    pub fn validate_refinement_accessibility(
        &self,
        from_module: &crate::path::ModulePath,
    ) -> ModuleResult<()> {
        let module_path = match &self.module_path {
            Some(path) => path,
            None => return Ok(()), // Cannot validate without module path
        };

        for (_name, item) in self.all_exports() {
            if item.refinement.is_some() {
                // Check if predicate is accessible
                if !item.is_predicate_accessible(from_module, module_path) {
                    return Err(ModuleError::Other {
                        message: Text::from(format!(
                            "Refinement predicate for '{}' is not accessible from module '{}'",
                            item.name, from_module
                        )),
                        span: Some(item.span),
                    });
                }
            }
        }
        Ok(())
    }

    /// Get the number of exports
    pub fn len(&self) -> usize {
        self.exports.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }

    /// Clear all exports
    pub fn clear(&mut self) {
        self.exports.clear();
    }
}

impl Default for ExportTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract an ExportTable from a module's AST.
///
/// This function analyzes all items in the module and adds public items
/// to the export table. It handles:
/// - Functions (including meta functions)
/// - Types (structs, enums, type aliases)
/// - Protocols (traits)
/// - Constants and statics
/// - Nested modules
/// - Context declarations
///
/// Extracts exports from module AST items based on visibility modifiers.
/// Re-exports (public import) flatten module hierarchy for public API.
pub fn extract_exports_from_module(
    module: &verum_ast::Module,
    module_id: ModuleId,
    module_path: &crate::path::ModulePath,
) -> ModuleResult<ExportTable> {
    use verum_ast::ItemKind;
    use verum_ast::decl::Visibility as AstVisibility;

    let mut export_table = ExportTable::new();
    export_table.set_module_id(module_id);
    export_table.set_module_path(module_path.clone());

    for item in &module.items {
        match &item.kind {
            ItemKind::Function(func) => {
                if func.visibility == AstVisibility::Public {
                    let kind = if func.is_meta {
                        ExportKind::Meta
                    } else {
                        ExportKind::Function
                    };
                    let exported = ExportedItem::new(
                        func.name.name.as_str(),
                        kind,
                        convert_visibility(&func.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::Type(type_decl) => {
                if type_decl.visibility == AstVisibility::Public {
                    // Export the type itself
                    let exported = ExportedItem::new(
                        type_decl.name.name.as_str(),
                        ExportKind::Type,
                        convert_visibility(&type_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;

                    // For variant types, also export variant constructors as functions
                    // e.g., `type Maybe<T> is None | Some(T)` exports: Maybe (type), None (function), Some (function)
                    //
                    // BUT Verum's variant syntax is overloaded — the same `type X is A | B;`
                    // form is used both for fresh constructors (`type Maybe is None | Some(T)`)
                    // AND for type unions of pre-existing types (`type AccessMode is ReadOnly | WriteOnly`
                    // where `ReadOnly` and `WriteOnly` are already-declared unit types). Registering the
                    // union-case names as new constructors collides with the pre-existing Type
                    // exports, producing spurious "Conflicting export" warnings.
                    //
                    // Disambiguation heuristic: a variant is a fresh constructor only when it
                    // carries payload data (tuple fields `Some(T)` or record fields `Node { ... }`)
                    // OR when its name is NOT already exported as a Type in this same module.
                    // Payload-less, already-declared-as-Type names are type-union references,
                    // not new constructors, and must not be re-registered.
                    if let verum_ast::decl::TypeDeclBody::Variant(variants) = &type_decl.body {
                        for variant in variants {
                            let has_payload = match &variant.data {
                                Maybe::None => false,
                                Maybe::Some(verum_ast::decl::VariantData::Tuple(fields)) => {
                                    !fields.is_empty()
                                }
                                Maybe::Some(verum_ast::decl::VariantData::Record(fields)) => {
                                    !fields.is_empty()
                                }
                            };
                            let already_type_in_this_module = match export_table
                                .get(&Text::from(variant.name.name.as_str()))
                            {
                                Maybe::Some(existing) => {
                                    existing.kind == ExportKind::Type
                                        && existing.source_module == module_id
                                }
                                Maybe::None => false,
                            };
                            if !has_payload && already_type_in_this_module {
                                // Type-union reference, not a fresh constructor. Skip.
                                continue;
                            }
                            let variant_exported = ExportedItem::new(
                                variant.name.name.as_str(),
                                ExportKind::Function, // Constructors are functions
                                convert_visibility(&type_decl.visibility),
                                module_id,
                                variant.span,
                            );
                            export_table.add_export(variant_exported)?;
                        }
                    }
                }
            }

            ItemKind::Protocol(proto) => {
                if proto.visibility == AstVisibility::Public {
                    // Context protocols (declared with `context protocol`) are exported
                    // as ExportKind::Context so they can be registered with the context
                    // resolver when imported via `using [...]` clauses.
                    //
                    // Context system integration: async contexts (using/provide)
                    // are exported with their type information for cross-module
                    // dependency injection. Contexts are NOT types; they use
                    // `context Logger { }` syntax and are accessed via vtable
                    // lookup in task-local storage (~5-30ns overhead).
                    let kind = if proto.is_context {
                        ExportKind::Context
                    } else {
                        ExportKind::Protocol
                    };
                    let exported = ExportedItem::new(
                        proto.name.name.as_str(),
                        kind,
                        convert_visibility(&proto.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::Const(const_decl) => {
                if const_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        const_decl.name.name.as_str(),
                        ExportKind::Const,
                        convert_visibility(&const_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::Static(static_decl) => {
                if static_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        static_decl.name.name.as_str(),
                        ExportKind::Static,
                        convert_visibility(&static_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::Module(mod_decl) => {
                if mod_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        mod_decl.name.name.as_str(),
                        ExportKind::Module,
                        convert_visibility(&mod_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::Context(ctx_decl) => {
                if ctx_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        ctx_decl.name.name.as_str(),
                        ExportKind::Context,
                        convert_visibility(&ctx_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            ItemKind::ContextGroup(group_decl) => {
                if group_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        group_decl.name.name.as_str(),
                        ExportKind::ContextGroup,
                        convert_visibility(&group_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            // Meta declarations (macros) - export separately
            ItemKind::Meta(meta_decl) => {
                if meta_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        meta_decl.name.name.as_str(),
                        ExportKind::Meta,
                        convert_visibility(&meta_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            // Predicates
            ItemKind::Predicate(pred_decl) => {
                if pred_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        pred_decl.name.name.as_str(),
                        ExportKind::Predicate,
                        convert_visibility(&pred_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            // Public links (re-exports) - add linked items to export table
            // Re-export: make internal items public through different path
            ItemKind::Mount(mount_decl) => {
                if mount_decl.visibility == AstVisibility::Public {
                    // Extract item names from the mount tree and add to exports
                    add_reexports_from_link(
                        &mount_decl.tree,
                        module_id,
                        item.span,
                        &mut export_table,
                    )?;
                }
            }

            // Axioms are trusted declarations that can be referenced by
            // name in both proof positions and (when they have parameters
            // + return type, like univalence `ua`) expression positions.
            // Export as Function so callers can resolve `mount …{ua}` the
            // same way as regular functions.
            ItemKind::Axiom(axiom_decl) => {
                if axiom_decl.visibility == AstVisibility::Public {
                    let exported = ExportedItem::new(
                        axiom_decl.name.name.as_str(),
                        ExportKind::Function,
                        convert_visibility(&axiom_decl.visibility),
                        module_id,
                        item.span,
                    );
                    export_table.add_export(exported)?;
                }
            }

            // Implementation blocks and other items don't export themselves
            ItemKind::Impl(_)
            | ItemKind::FFIBoundary(_)
            | ItemKind::Theorem(_)
            | ItemKind::Lemma(_)
            | ItemKind::Corollary(_)
            | ItemKind::Tactic(_)
            | ItemKind::View(_)
            | ItemKind::Pattern(_)
            | ItemKind::Layer(_) => {}

            // Extern blocks contain FFI function declarations that
            // should be exported so sibling modules can import them
            // via `mount core.sys.darwin.libsystem.{mach_absolute_time}`.
            ItemKind::ExternBlock(extern_block) => {
                for func in &extern_block.functions {
                    if func.visibility == AstVisibility::Public {
                        let exported = ExportedItem::new(
                            func.name.name.as_str(),
                            ExportKind::Function,
                            Visibility::Public,
                            module_id,
                            func.span,
                        );
                        export_table.add_export(exported)?;
                    }
                }
            }
        }
    }

    Ok(export_table)
}

/// Add re-exported items from a public link to the export table.
///
/// This handles `pub link .path.{Item1, Item2}` statements, which re-export
/// linked items as part of the current module's public interface.
///
/// Extracts exports from module AST items based on visibility modifiers.
/// Re-exports (public import) flatten module hierarchy for public API.
fn add_reexports_from_link(
    tree: &verum_ast::decl::MountTree,
    module_id: ModuleId,
    span: verum_ast::span::Span,
    export_table: &mut ExportTable,
) -> ModuleResult<()> {
    use verum_ast::decl::MountTreeKind;

    use verum_ast::ty::PathSegment;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Single item link: `pub link .errors.RegistryError`
            // The last segment is the item name; use alias if present
            if let Some(last_segment) = path.segments.last()
                && let PathSegment::Name(ident) = last_segment
            {
                let item_name = if let Some(ref alias) = tree.alias {
                    alias.name.as_str()
                } else {
                    ident.name.as_str()
                };
                // Re-exports are treated as Type by default (we don't know the actual kind yet)
                // The actual kind will be resolved when the link is processed
                let exported = ExportedItem::new(
                    item_name,
                    ExportKind::Type, // Default to Type for re-exports
                    Visibility::Public,
                    module_id,
                    span,
                );
                export_table.add_export(exported)?;
            }
        }
        MountTreeKind::Glob(_) => {
            // Glob links: `pub link .package.*`
            // We can't know the items at this stage without resolving the link
            // These will need to be handled during link resolution
        }
        MountTreeKind::Nested { prefix: _, trees } => {
            // Nested links: `pub link .package.{Package, PackageVersion}`
            for subtree in trees {
                match &subtree.kind {
                    MountTreeKind::Path(path) => {
                        // Each path in the nested link is an item name
                        // If an alias is present (e.g., `safe_read as read`), use the alias
                        if let Some(first_segment) = path.segments.first()
                            && let PathSegment::Name(ident) = first_segment
                        {
                            let item_name = if let Some(ref alias) = subtree.alias {
                                alias.name.as_str()
                            } else {
                                ident.name.as_str()
                            };
                            let exported = ExportedItem::new(
                                item_name,
                                ExportKind::Type, // Default to Type for re-exports
                                Visibility::Public,
                                module_id,
                                subtree.span,
                            );
                            export_table.add_export(exported)?;
                        }
                    }
                    MountTreeKind::Glob(_) | MountTreeKind::Nested { .. } => {
                        // Recursively handle nested structures
                        add_reexports_from_link(subtree, module_id, span, export_table)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resolve glob re-exports after all modules are loaded.
///
/// This function processes `public import path.*` statements and copies
/// all exports from the source module to the current module's export table.
///
/// This must be called after all modules are initially loaded since it needs
/// access to the source modules' export tables.
///
/// Returns the number of glob re-exports resolved.
///
/// **MOD-MED-3 (transitive closure).** When chain A → B → C (each
/// `public mount`-re-exports the next), a single pass would leave A's
/// exports table with B's items (filled before B's pass added C's),
/// missing C's items. The fix: drive the resolver to a fixed point —
/// keep iterating until no new exports are added in a full pass. Cap
/// at `MAX_REEXPORT_DEPTH = 16` so a misbehaving cycle (which
/// `resolve_export_kind_with_reexports_inner` is supposed to catch
/// upstream) can't block the build forever.
///
/// Extracts exports from module AST items based on visibility modifiers.
/// Re-exports (public import) flatten module hierarchy for public API.
pub fn resolve_glob_reexports(
    module_registry: &mut crate::ModuleRegistry,
) -> ModuleResult<usize> {
    /// Maximum re-export chain depth. Hard cap protects against
    /// pathological cycles slipping past the upstream cycle guard.
    /// Sixteen is two orders of magnitude above any realistic
    /// stdlib chain (the deepest path in `core/` today is 3).
    const MAX_REEXPORT_DEPTH: usize = 16;
    let mut grand_total = 0usize;
    for _depth in 0..MAX_REEXPORT_DEPTH {
        let added = resolve_glob_reexports_one_pass(module_registry)?;
        grand_total += added;
        if added == 0 {
            return Ok(grand_total);
        }
    }
    // We hit the cap: surface a soft warning (but don't fail) so the
    // build still finishes. The upstream `resolve_export_kind_with_
    // reexports_inner` cycle guard catches truly cyclic chains; this
    // path triggers when the chain is long-but-finite (≥ 16) which
    // is itself worth flagging to the user.
    eprintln!(
        "warning<E_MODULE_INVALID_REEXPORT>: re-export chain reached depth {} \
         without converging — capping the closure to keep the build moving. \
         Either flatten the re-export tree or split it into independent stages.",
        MAX_REEXPORT_DEPTH,
    );
    Ok(grand_total)
}

/// Single pass over the registry: collect every glob re-export and
/// apply each one's source exports onto the target. Returns the
/// number of exports added in this pass — used by the closure
/// driver above to detect a fixed point (zero added = stable).
fn resolve_glob_reexports_one_pass(
    module_registry: &mut crate::ModuleRegistry,
) -> ModuleResult<usize> {
    use verum_ast::ItemKind;
    use verum_ast::decl::Visibility as AstVisibility;
    use verum_common::Maybe;

    let total_resolved = 0;

    // First, collect all glob re-exports we need to resolve
    // (module_id, source_path_str)
    let mut glob_reexports: verum_common::List<(crate::path::ModuleId, Text)> = verum_common::List::new();

    for (_id, module_info_shared) in module_registry.all_modules() {
        // Deref Shared to get ModuleInfo
        let module_info: &crate::ModuleInfo = module_info_shared;
        for item in &module_info.ast.items {
            if let ItemKind::Mount(mount_decl) = &item.kind {
                if mount_decl.visibility == AstVisibility::Public {
                    // Find glob links in this tree
                    let mut globs: verum_common::List<(crate::path::ModuleId, Text, Span)> = verum_common::List::new();
                    collect_glob_links(
                        &mount_decl.tree,
                        module_info.id,
                        &module_info.path,
                        &mut globs,
                    );
                    for (id, path, _span) in globs {
                        glob_reexports.push((id, path));
                    }
                }
            }
        }
    }

    // Now resolve each glob import - collect source exports first
    let mut updates: verum_common::List<(crate::path::ModuleId, ExportTable)> = verum_common::List::new();

    for (module_id, source_path_str) in &glob_reexports {
        // Get the source module's exports
        if let Maybe::Some(source_info_shared) = module_registry.get_by_path(source_path_str.as_str()) {
            let source_info: &crate::ModuleInfo = &source_info_shared;
            let source_exports = &source_info.exports;
            let export_count = source_exports.len();

            if export_count > 0 {
                updates.push((*module_id, source_exports.clone()));
            }
        }
    }
    let _ = total_resolved; // shadow the legacy "raw count" — see below.

    // Apply each update. We count ONLY newly-added exports — pre-existing
    // entries don't count because the closure driver above uses zero-added
    // as the fixed-point signal.
    let mut newly_added_total = 0usize;
    for (target_id, source_exports) in updates {
        newly_added_total += module_registry.add_exports_to_module(target_id, &source_exports);
    }

    Ok(newly_added_total)
}

/// Resolve the ExportKind for specific item re-exports after all modules are loaded.
///
/// This handles `public import path.{Item1, Item2}` statements where the default
/// ExportKind::Type was assigned during initial extraction. Now that all modules
/// are loaded, we can look up the actual kind from the source module.
///
/// This is critical for variant constructors: when `std.core` does
/// `public import .maybe.{None, Some}`, we need to look up that `Some` is
/// actually a Function (variant constructor), not a Type.
///
/// Returns the number of re-exports updated.
///
/// Extracts exports from module AST items based on visibility modifiers.
/// Re-exports (public import) flatten module hierarchy for public API.
pub fn resolve_specific_reexport_kinds(
    module_registry: &mut crate::ModuleRegistry,
) -> ModuleResult<usize> {
    use verum_ast::ItemKind;
    use verum_ast::decl::Visibility as AstVisibility;

    let mut total_updated = 0;

    // Collect updates: (target_module_id, item_name, correct_kind, source_module_id)
    let mut updates: verum_common::List<(crate::path::ModuleId, Text, ExportKind, crate::path::ModuleId)> =
        verum_common::List::new();

    // First pass: collect all specific item re-exports and their correct kinds
    for (_id, module_info_shared) in module_registry.all_modules() {
        let module_info: &crate::ModuleInfo = module_info_shared;
        let current_module_path = &module_info.path;

        for item in &module_info.ast.items {
            if let ItemKind::Mount(mount_decl) = &item.kind {
                if mount_decl.visibility != AstVisibility::Public {
                    continue;
                }

                // Collect specific item links from this declaration
                collect_specific_links_for_kind_resolution(
                    &mount_decl.tree,
                    module_info.id,
                    current_module_path,
                    module_registry,
                    &mut updates,
                );
            }
        }
    }

    // Second pass: apply the updates
    for (target_id, item_name, correct_kind, source_module) in updates {
        if let Some(existing) = module_registry.modules.get(&target_id) {
            let mut updated = (**existing).clone();

            // Update the export kind if we have the item
            if let Some(existing_export) = updated.exports.exports.get(&item_name) {
                // Only update if the current kind is the default Type and we have a better kind
                if existing_export.kind == ExportKind::Type && correct_kind != ExportKind::Type {
                    let mut updated_export = existing_export.clone();
                    updated_export.kind = correct_kind;
                    updated_export.source_module = source_module;
                    updated.exports.exports.insert(item_name.clone(), updated_export);
                    total_updated += 1;
                }
            }

            // Update the module in registry
            let path_str = updated.path.to_string();
            module_registry.modules.insert(target_id, verum_common::Shared::new(updated));
            module_registry.path_to_id.insert(Text::from(path_str.as_str()), target_id);
        }
    }

    Ok(total_updated)
}

/// Helper to collect specific item links for kind resolution.
fn collect_specific_links_for_kind_resolution(
    tree: &verum_ast::decl::MountTree,
    target_module_id: crate::path::ModuleId,
    current_module_path: &crate::path::ModulePath,
    module_registry: &crate::ModuleRegistry,
    result: &mut verum_common::List<(crate::path::ModuleId, Text, ExportKind, crate::path::ModuleId)>,
) {
    use verum_ast::decl::MountTreeKind;
    use verum_ast::ty::PathSegment;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Single item link: `pub link .errors.RegistryError`
            // The path segments before the last are the module path, the last is the item name
            if path.segments.len() >= 2 {
                if let Some(PathSegment::Name(item_ident)) = path.segments.last() {
                    let item_name = item_ident.name.as_str();

                    // Get the module path (all segments except the last)
                    let module_path = verum_ast::ty::Path {
                        segments: path.segments.iter().take(path.segments.len() - 1).cloned().collect(),
                        span: path.span,
                    };

                    if let Some(resolved_module_path) = resolve_link_path(&module_path, current_module_path) {
                        // Look up the source module's exports
                        if let verum_common::Maybe::Some(source_info) = module_registry.get_by_path(resolved_module_path.as_str()) {
                            if let Some(source_export) = source_info.exports.get(&Text::from(item_name)) {
                                result.push((
                                    target_module_id,
                                    Text::from(item_name),
                                    source_export.kind,
                                    source_info.id,
                                ));
                            }
                        }
                    }
                }
            }
        }
        MountTreeKind::Nested { prefix, trees } => {
            // Nested links: `pub link .package.{Package, PackageVersion}`
            if let Some(resolved_module_path) = resolve_link_path(prefix, current_module_path) {
                // Look up the source module
                if let verum_common::Maybe::Some(source_info) = module_registry.get_by_path(resolved_module_path.as_str()) {
                    // Process each item in the nested link
                    for subtree in trees {
                        match &subtree.kind {
                            MountTreeKind::Path(item_path) => {
                                // Each path is an item name (possibly with renaming, but we care about the first segment)
                                if let Some(PathSegment::Name(item_ident)) = item_path.segments.first() {
                                    let item_name = item_ident.name.as_str();

                                    // Look up in source module's exports
                                    if let Some(source_export) = source_info.exports.get(&Text::from(item_name)) {
                                        result.push((
                                            target_module_id,
                                            Text::from(item_name),
                                            source_export.kind,
                                            source_info.id,
                                        ));
                                    }
                                }
                            }
                            MountTreeKind::Glob(_) => {
                                // Skip globs, handled by resolve_glob_reexports
                            }
                            MountTreeKind::Nested { .. } => {
                                // Recursively handle nested
                                collect_specific_links_for_kind_resolution(
                                    subtree,
                                    target_module_id,
                                    current_module_path,
                                    module_registry,
                                    result,
                                );
                            }
                        }
                    }
                }
            }
        }
        MountTreeKind::Glob(_) => {
            // Skip globs, handled by resolve_glob_reexports
        }
    }
}

/// Collect glob link paths from a link tree.
fn collect_glob_links(
    tree: &verum_ast::decl::MountTree,
    module_id: crate::path::ModuleId,
    current_module_path: &crate::path::ModulePath,
    result: &mut verum_common::List<(crate::path::ModuleId, Text, Span)>,
) {
    use verum_ast::decl::MountTreeKind;

    match &tree.kind {
        MountTreeKind::Glob(path) => {
            // Resolve the path to an absolute module path
            if let Some(resolved_path) = resolve_link_path(path, current_module_path) {
                result.push((module_id, resolved_path, tree.span));
            }
        }
        MountTreeKind::Nested { prefix: _, trees } => {
            // Recursively check nested trees
            for subtree in trees {
                collect_glob_links(subtree, module_id, current_module_path, result);
            }
        }
        MountTreeKind::Path(_) => {
            // Not a glob link
        }
    }
}

/// Resolve a link path relative to the current module.
///
/// Handles:
/// - `super.core` -> parent module's `core` submodule
/// - `.package` -> current module's `package` submodule (relative link)
/// - `std.core` -> absolute path
fn resolve_link_path(
    path: &verum_ast::Path,
    current_module: &crate::path::ModulePath,
) -> Option<Text> {
    use verum_ast::ty::PathSegment;

    // Get name segments (filter out special markers)
    let name_segments: verum_common::List<&str> = path.segments.iter()
        .filter_map(|seg| match seg {
            PathSegment::Name(ident) => Some(ident.name.as_str()),
            PathSegment::Super => None, // Handle specially
            PathSegment::SelfValue => None,
            PathSegment::Cog => None,
            PathSegment::Relative => None, // Handle specially
        })
        .collect();

    // Check if path starts with `super`
    let has_super = path.segments.first().is_some_and(|seg| matches!(seg, PathSegment::Super));

    // Check if path starts with leading dot (relative import)
    let has_relative = path.segments.first().is_some_and(|seg| matches!(seg, PathSegment::Relative));

    if has_super {
        // Count number of `super` prefixes
        let super_count = path.segments.iter()
            .take_while(|seg| matches!(seg, PathSegment::Super))
            .count();

        // Get parent path by going up `super_count` levels
        let mut parent = current_module.clone();
        for _ in 0..super_count {
            if let Some(p) = parent.parent() {
                parent = p;
            }
        }

        // Append remaining segments
        let remaining: verum_common::List<&str> = path.segments.iter()
            .skip(super_count)
            .filter_map(|seg| match seg {
                PathSegment::Name(ident) => Some(ident.name.as_str()),
                _ => None,
            })
            .collect();

        if remaining.is_empty() {
            let parent_str = parent.to_string();
            Some(Text::from(parent_str.as_str()))
        } else {
            let joined = remaining.iter().copied().collect::<Vec<_>>().join(".");
            let child = parent.join(joined.as_str());
            let child_str = child.to_string();
            Some(Text::from(child_str.as_str()))
        }
    } else if has_relative {
        // Relative import: `.maybe` means `current_module.maybe`
        if name_segments.is_empty() {
            // Just `.` means current module
            let current_str = current_module.to_string();
            Some(Text::from(current_str.as_str()))
        } else {
            // `.submodule` means `current_module.submodule`
            let joined = name_segments.iter().copied().collect::<Vec<_>>().join(".");
            let child = current_module.join(joined.as_str());
            let child_str = child.to_string();
            Some(Text::from(child_str.as_str()))
        }
    } else if name_segments.is_empty() {
        None
    } else {
        // No explicit prefix (no super, no leading dot).
        // Treat as relative to current module first (e.g., `arithmetic` in `std.intrinsics`
        // means `std.intrinsics.arithmetic`), unless the first segment is a known root.
        let first = name_segments[0];
        let is_absolute = matches!(first, "std" | "core" | "cog" | "self");
        if is_absolute {
            let path_str = name_segments.iter().copied().collect::<Vec<_>>().join(".");
            Some(Text::from(path_str.as_str()))
        } else {
            // Relative to current module
            let joined = name_segments.iter().copied().collect::<Vec<_>>().join(".");
            let child = current_module.join(joined.as_str());
            let child_str = child.to_string();
            Some(Text::from(child_str.as_str()))
        }
    }
}

/// Convert AST visibility to module visibility
fn convert_visibility(ast_vis: &verum_ast::decl::Visibility) -> Visibility {
    match ast_vis {
        verum_ast::decl::Visibility::Public => Visibility::Public,
        verum_ast::decl::Visibility::Private => Visibility::Private,
        verum_ast::decl::Visibility::PublicCrate => Visibility::PublicCrate,
        verum_ast::decl::Visibility::PublicSuper => Visibility::PublicSuper,
        verum_ast::decl::Visibility::PublicIn(path) => Visibility::PublicIn(path.clone()),
        verum_ast::decl::Visibility::Internal => Visibility::Internal,
        verum_ast::decl::Visibility::Protected => Visibility::Protected,
    }
}

/// Information about an exported context type.
///
/// Contexts can be explicit `context` declarations or protocols that can be
/// used in `using [Context]` clauses for dependency injection.
///
/// Extracts context declarations from a module's AST. Contexts are the
/// Level 2 (Dynamic) dependency injection system: `context Logger { fn log(...) }`.
/// Functions declare required contexts with `using [Context]` after return type.
/// Contexts are provided with `provide Context = impl` for lexically-scoped DI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedContext {
    /// Name of the context
    pub name: Text,
    /// Kind of context source (explicit context or protocol)
    pub kind: ContextSourceKind,
    /// Source module
    pub source_module: ModuleId,
    /// Span in the source
    pub span: Span,
}

/// The kind of source for a context type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContextSourceKind {
    /// Explicit `context` declaration
    Context,
    /// Protocol that can be used as a context (DI pattern)
    Protocol,
    /// Context group declaration
    ContextGroup,
}

impl ContextSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextSourceKind::Context => "context",
            ContextSourceKind::Protocol => "protocol",
            ContextSourceKind::ContextGroup => "context group",
        }
    }
}

/// Extract all context-capable types from a module.
///
/// This function extracts:
/// - Explicit `context` declarations
/// - Protocols (which can serve as context types in `using` clauses)
/// - Context groups
///
/// These are needed for cross-file context resolution when a function uses
/// `using [Database, Auth]` where these types are defined in other modules.
///
/// Extracts context declarations from a module's AST. Contexts are the
/// Level 2 (Dynamic) dependency injection system: `context Logger { fn log(...) }`.
/// Functions declare required contexts with `using [Context]` after return type.
/// Contexts are provided with `provide Context = impl` for lexically-scoped DI. Section 2.5 - Type System Integration
pub fn extract_contexts_from_module(
    module: &verum_ast::Module,
    module_id: ModuleId,
) -> verum_common::List<ExportedContext> {
    use verum_ast::ItemKind;
    use verum_ast::decl::Visibility as AstVisibility;

    let mut contexts = verum_common::List::new();

    for item in &module.items {
        match &item.kind {
            // Explicit context declarations
            ItemKind::Context(ctx_decl) => {
                // Only export public contexts
                if ctx_decl.visibility == AstVisibility::Public {
                    contexts.push(ExportedContext {
                        name: Text::from(ctx_decl.name.name.as_str()),
                        kind: ContextSourceKind::Context,
                        source_module: module_id,
                        span: item.span,
                    });
                }
            }

            // Context protocols (declared with `context protocol`) can be used as contexts
            // for dependency injection via `using [...]` clauses.
            //
            // Regular protocols (without `context` modifier) are NOT included here as they
            // cannot be used in `using [...]` clauses.
            // Context protocols are interface-based DI with ~5-30ns overhead
            // per context call via vtable lookup in task-local storage.
            ItemKind::Protocol(proto) => {
                // Only export public context protocols
                if proto.visibility == AstVisibility::Public && proto.is_context {
                    contexts.push(ExportedContext {
                        name: Text::from(proto.name.name.as_str()),
                        kind: ContextSourceKind::Protocol,
                        source_module: module_id,
                        span: item.span,
                    });
                }
            }

            // Context groups
            ItemKind::ContextGroup(group_decl) => {
                if group_decl.visibility == AstVisibility::Public {
                    contexts.push(ExportedContext {
                        name: Text::from(group_decl.name.name.as_str()),
                        kind: ContextSourceKind::ContextGroup,
                        source_module: module_id,
                        span: item.span,
                    });
                }
            }

            // Types declared with `context type X is protocol { ... }` pattern
            // This is another way to define context protocols in Verum.
            //
            // Only context protocols (with `is_context = true`) are included here.
            // Context protocol types use `context type X is protocol { ... }` pattern.
            ItemKind::Type(type_decl) => {
                if type_decl.visibility == AstVisibility::Public {
                    // Check if this is a context protocol type definition
                    if is_context_protocol_type_definition(&type_decl.body) {
                        contexts.push(ExportedContext {
                            name: Text::from(type_decl.name.name.as_str()),
                            kind: ContextSourceKind::Protocol,
                            source_module: module_id,
                            span: item.span,
                        });
                    }
                }
            }

            _ => {}
        }
    }

    contexts
}

/// Check if a type definition is a context protocol type.
///
/// Matches patterns like `context type X is protocol { ... }` where the protocol
/// has `is_context = true`. Only context protocols can be used in `using [...]`
/// dependency injection clauses.
///
/// Extracts context declarations from a module's AST. Contexts are the
/// Level 2 (Dynamic) dependency injection system: `context Logger { fn log(...) }`.
/// Functions declare required contexts with `using [Context]` after return type.
/// Contexts are provided with `provide Context = impl` for lexically-scoped DI. Section 2.5 - Type System Integration
fn is_context_protocol_type_definition(type_body: &verum_ast::decl::TypeDeclBody) -> bool {
    use verum_ast::decl::TypeDeclBody;

    match type_body {
        TypeDeclBody::Protocol(protocol_body) => protocol_body.is_context,
        _ => false,
    }
}
