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
    ///  match item.visibility {
    ///  Private => from_module == item.module,
    ///  Public => true,
    ///  PublicCrate => same_crate(item.module, from_module),
    ///  PublicSuper => from_module.is_parent_of(item.module),
    ///  PublicIn(path) => from_module.is_descendant_of(path) || from_module == path,
    ///  }
    /// }
    /// ```
    pub fn is_visible(
        &self,
        item_visibility: Visibility,
        item_module: &ModulePath,
        from_module: &ModulePath,
    ) -> bool {
        item_visibility.visible_from(
            item_module.segments().iter().map(Text::as_str),
            from_module.segments().iter().map(Text::as_str),
        )
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
