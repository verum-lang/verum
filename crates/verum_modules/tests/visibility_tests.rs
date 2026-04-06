#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for visibility module
// Migrated from src/visibility.rs per CLAUDE.md standards

use verum_modules::ModulePath;
use verum_modules::visibility::*;

#[test]
fn test_public_visibility() {
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.parser.ast");
    let from_module = ModulePath::from_str("cog.lexer");

    assert!(checker.is_visible(Visibility::Public, &item_module, &from_module));
}

#[test]
fn test_private_visibility() {
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.parser.ast");
    let from_same = ModulePath::from_str("cog.parser.ast");
    let from_different = ModulePath::from_str("cog.lexer");

    assert!(checker.is_visible(Visibility::Private, &item_module, &from_same));
    assert!(!checker.is_visible(Visibility::Private, &item_module, &from_different));
}

#[test]
fn test_public_super_visibility() {
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.parser.ast");
    let parent_module = ModulePath::from_str("cog.parser");
    let sibling_module = ModulePath::from_str("cog.parser.lexer");
    let unrelated_module = ModulePath::from_str("cog.codegen");
    let grandparent_module = ModulePath::from_str("cog");

    // public(super) should be visible ONLY to the immediate parent module
    // public(super): visible ONLY to the immediate parent module, not siblings or ancestors
    assert!(checker.is_visible(Visibility::PublicSuper, &item_module, &parent_module));

    // public(super) should NOT be visible to sibling modules
    assert!(!checker.is_visible(Visibility::PublicSuper, &item_module, &sibling_module));

    // public(super) should NOT be visible to unrelated modules
    assert!(!checker.is_visible(Visibility::PublicSuper, &item_module, &unrelated_module));

    // public(super) should NOT be visible to grandparent modules
    assert!(!checker.is_visible(Visibility::PublicSuper, &item_module, &grandparent_module));

    // public(super) should NOT be visible from the same module
    assert!(!checker.is_visible(Visibility::PublicSuper, &item_module, &item_module));
}

#[test]
fn test_public_crate_visibility() {
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("my_cog.internal.utils");
    let same_crate_module = ModulePath::from_str("my_cog.external.api");
    let different_crate_module = ModulePath::from_str("other_cog.module");

    // internal should be visible within the same cog
    // internal: visible within the same cog only (same first path segment)
    assert!(checker.is_visible(Visibility::PublicCrate, &item_module, &same_crate_module));

    // internal should NOT be visible from a different cog
    assert!(!checker.is_visible(
        Visibility::PublicCrate,
        &item_module,
        &different_crate_module
    ));
}

#[test]
fn test_public_in_path_visibility() {
    let checker = VisibilityChecker::new();

    // Item in api.v1 with visibility public(in cog.api)
    let item_module = ModulePath::from_str("api.v1");

    // Create a path for "cog.api"
    use verum_ast::FileId;
    use verum_ast::{Ident, Path, PathSegment, span::Span};
    use verum_common::Text;

    let file_id = FileId::new(0);
    let span = Span::new(0, 0, file_id);

    let mut segments = verum_common::List::new();
    segments.push(PathSegment::Cog);
    segments.push(PathSegment::Name(Ident::new(Text::from("api"), span)));

    let path = Path::new(segments, span);
    let visibility = Visibility::PublicIn(path);

    // Should be visible from api.v2 (within api tree)
    let v2_module = ModulePath::from_str("api.v2");
    assert!(checker.is_visible(visibility.clone(), &item_module, &v2_module));

    // Should be visible from api itself
    let api_module = ModulePath::from_str("api");
    assert!(checker.is_visible(visibility.clone(), &item_module, &api_module));

    // Should NOT be visible from internal (outside api tree)
    let internal_module = ModulePath::from_str("internal");
    assert!(!checker.is_visible(visibility.clone(), &item_module, &internal_module));

    // Should NOT be visible from crate root alone
    let root_module = ModulePath::from_str("cog");
    assert!(!checker.is_visible(visibility, &item_module, &root_module));
}

#[test]
fn test_same_crate() {
    let checker = VisibilityChecker::new();
    let mod1 = ModulePath::from_str("cog.parser.ast");
    let mod2 = ModulePath::from_str("cog.lexer");
    let mod3 = ModulePath::from_str("other_cog.utils");

    assert!(checker.same_crate(&mod1, &mod2));
    assert!(!checker.same_crate(&mod1, &mod3));
}

#[test]
fn test_is_parent_of() {
    let checker = VisibilityChecker::new();

    let parent = ModulePath::from_str("cog.parser");
    let child = ModulePath::from_str("cog.parser.ast");
    let grandchild = ModulePath::from_str("cog.parser.ast.nodes");
    let sibling = ModulePath::from_str("cog.parser.lexer");

    // Direct parent should return true
    assert!(checker.is_parent_of(&parent, &child));

    // Grandparent should return false (not immediate parent)
    assert!(!checker.is_parent_of(&parent, &grandchild));

    // Sibling should return false
    assert!(!checker.is_parent_of(&child, &sibling));

    // Self should return false
    assert!(!checker.is_parent_of(&parent, &parent));
}

#[test]
fn test_is_descendant_of() {
    let checker = VisibilityChecker::new();

    let ancestor = ModulePath::from_str("cog.parser");
    let child = ModulePath::from_str("cog.parser.ast");
    let grandchild = ModulePath::from_str("cog.parser.ast.nodes");
    let unrelated = ModulePath::from_str("cog.lexer");

    // Child is descendant of ancestor
    assert!(checker.is_descendant_of(&child, &ancestor));

    // Grandchild is descendant of ancestor
    assert!(checker.is_descendant_of(&grandchild, &ancestor));

    // Unrelated module is not descendant
    assert!(!checker.is_descendant_of(&unrelated, &ancestor));

    // Self is not descendant of self
    assert!(!checker.is_descendant_of(&ancestor, &ancestor));
}

#[test]
fn test_check_visibility_error() {
    let checker = VisibilityChecker::new();
    let item_module = ModulePath::from_str("cog.internal");
    let from_module = ModulePath::from_str("other_cog.external");

    // Private access from different module should return error
    let result =
        checker.check_visibility("secret_fn", Visibility::Private, &item_module, &from_module);
    assert!(result.is_err());

    // Public access should succeed
    let result =
        checker.check_visibility("public_fn", Visibility::Public, &item_module, &from_module);
    assert!(result.is_ok());
}
