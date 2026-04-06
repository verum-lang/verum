//! Visibility checking for module items.
//!
//! Implements the five-level visibility system:
//! - Private (default): accessible only within the current module
//! - Public: accessible from any module in any crate
//! - public(crate): accessible only within the same crate
//! - public(super): accessible only to the immediate parent module
//! - public(in path): accessible within the specified module subtree
//!
//! Visibility is checked after name resolution. The algorithm traverses
//! the module hierarchy to determine access based on the modifier.

use crate::error::{ModuleError, ModuleResult};
use crate::path::ModulePath;
use verum_common::Text;

pub use verum_ast::Visibility;

/// Visibility checker - validates access permissions.
#[derive(Debug)]
pub struct VisibilityChecker {
    /// Cog ID for cog-local visibility checks
    crate_id: Option<u32>,
}

impl VisibilityChecker {
    pub fn new() -> Self {
        Self { crate_id: None }
    }

    /// Set the current cog ID
    pub fn set_crate_id(&mut self, crate_id: u32) {
        self.crate_id = Some(crate_id);
    }

    /// Check if an item is visible from another module.
    ///
    /// Implements the visibility algorithm from Section 5.2.8 of the spec:
    ///
    /// ```text
    /// fn is_visible(item: Item, from_module: Module) -> bool {
    ///     match item.visibility {
    ///         Private => from_module == item.module,
    ///         Public => true,
    ///         PublicCrate => same_crate(item.module, from_module),
    ///         PublicSuper => from_module.is_parent_of(item.module),
    ///         PublicIn(path) => from_module.is_descendant_of(path) || from_module == path,
    ///     }
    /// }
    /// ```
    pub fn is_visible(
        &self,
        item_visibility: Visibility,
        item_module: &ModulePath,
        from_module: &ModulePath,
    ) -> bool {
        match item_visibility {
            Visibility::Private => item_module == from_module,
            Visibility::Internal => item_module == from_module,
            Visibility::Protected => item_module == from_module,
            Visibility::Public => true,
            Visibility::PublicCrate => {
                // public(cog): visible only within the same cog.
                // Two modules are in the same cog if they share the same
                // first path segment (the cog name).
                self.same_crate(item_module, from_module)
            }
            Visibility::PublicSuper => {
                // public(super): visible ONLY to the immediate parent module.
                // "Parent-public visibility means visible to parent module"
                //
                // This check ensures from_module is the IMMEDIATE parent of item_module,
                // NOT any ancestor. For example:
                // - item in `database.connection` with public(super) is visible ONLY from `database`
                // - NOT visible from any other module, not even ancestors of `database`
                item_module.parent().as_ref() == Some(from_module)
            }
            Visibility::PublicIn(ref path) => {
                // public(in path): visible within the specified module subtree.
                // The accessing module must be a descendant of the target path
                // or the target path itself.
                // Convert AST path to ModulePath for comparison
                // Cog means "root of current cog" — skip it, use remaining segments
                let path_str = path
                    .segments
                    .iter()
                    .filter_map(|seg| match seg {
                        verum_ast::PathSegment::Name(ident) => {
                            Some(ident.name.as_str().to_string())
                        }
                        verum_ast::PathSegment::Cog => None, // Root marker, not a segment
                        verum_ast::PathSegment::Super => Some("super".to_string()),
                        verum_ast::PathSegment::SelfValue => Some("self".to_string()),
                        verum_ast::PathSegment::Relative => None, // Skip relative markers
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                let target_path = ModulePath::from_str(&path_str);
                target_path.is_prefix_of(from_module) || from_module == &target_path
            }
        }
    }

    /// Check visibility and return an error if not visible.
    pub fn check_visibility(
        &self,
        item_name: impl Into<Text>,
        item_visibility: Visibility,
        item_module: &ModulePath,
        from_module: &ModulePath,
    ) -> ModuleResult<()> {
        if !self.is_visible(item_visibility, item_module, from_module) {
            return Err(ModuleError::PrivateAccess {
                item_name: item_name.into(),
                item_module: item_module.clone(),
                accessing_module: from_module.clone(),
                span: None,
            });
        }
        Ok(())
    }

    /// Check if two modules are in the same cog.
    pub fn same_crate(&self, module1: &ModulePath, module2: &ModulePath) -> bool {
        // In Verum, the cog is the first segment of the path
        module1.segments().first() == module2.segments().first()
    }

    /// Check if from_module is a parent of child_module.
    pub fn is_parent_of(&self, parent: &ModulePath, child: &ModulePath) -> bool {
        if parent.depth() + 1 != child.depth() {
            return false;
        }
        parent.is_prefix_of(child)
    }

    /// Check if module is a descendant of ancestor.
    pub fn is_descendant_of(&self, module: &ModulePath, ancestor: &ModulePath) -> bool {
        module.is_descendant_of(ancestor)
    }
}

impl Default for VisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}
