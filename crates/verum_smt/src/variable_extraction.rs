//! Variable Extraction Utilities for Z3 AST
//!
//! This module provides centralized utilities for extracting variable names from Z3 AST nodes.
//! Previously, this functionality was duplicated across z3_backend.rs and interpolation.rs.
//!
//! # Features
//!
//! - **AST Traversal**: Walks Z3 AST trees to find uninterpreted constants
//! - **Memoization**: Uses visited set to avoid traversing shared subexpressions
//! - **Variable Filtering**: Filters out Z3 internal names (k!, !, numeric)
//!
//! # Performance
//!
//! - Linear time in AST size with memoization
//! - Typically < 1ms for typical verification formulas

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use z3::ast::{Ast, Dynamic};
use z3::{DeclKind, Sort};

use verum_common::{Map, Set, Text};

/// Collect all free variable names from a Z3 Boolean formula
///
/// Traverses the Z3 AST to find all uninterpreted constants (variables).
/// Handles:
/// - Simple variables (x, y, z)
/// - Variables in compound expressions (x + y, f(x))
/// - Properly skips bound variables in quantifiers
///
/// # Arguments
///
/// * `formula` - The Z3 Boolean formula to extract variables from
///
/// # Returns
///
/// A Set of variable names found in the formula
pub fn collect_variables_from_bool(formula: &z3::ast::Bool) -> Set<Text> {
    let mut variables: Set<Text> = Set::new();
    let mut visited = HashSet::new();

    let dynamic = Dynamic::from_ast(formula);
    collect_variables_from_dynamic(&dynamic, &mut variables, &mut visited);

    variables
}

/// Collect all free variable names from multiple Z3 Boolean formulas
///
/// Efficiently processes multiple formulas with shared visited set.
///
/// # Arguments
///
/// * `formulas` - Slice of Z3 Boolean formulas
///
/// # Returns
///
/// A Set of all variable names found across all formulas
pub fn collect_variables_from_formulas(formulas: &[z3::ast::Bool]) -> Set<Text> {
    let mut variables: Set<Text> = Set::new();
    let mut visited = HashSet::new();

    for formula in formulas {
        let dynamic = Dynamic::from_ast(formula);
        collect_variables_from_dynamic(&dynamic, &mut variables, &mut visited);
    }

    variables
}

/// Collect variables from a Dynamic AST node (recursive implementation)
///
/// This is the core recursive function that traverses the AST.
/// Made public for use cases requiring custom traversal.
///
/// # Arguments
///
/// * `node` - The Z3 Dynamic AST node to traverse
/// * `variables` - Output set to collect variable names into
/// * `visited` - Memoization set to avoid revisiting nodes
pub fn collect_variables_from_dynamic(
    node: &Dynamic,
    variables: &mut Set<Text>,
    visited: &mut HashSet<u64>,
) {
    // Compute unique ID for memoization using hash
    let id = compute_ast_hash(node);

    if visited.contains(&id) {
        return;
    }
    visited.insert(id);

    // Check if this is a variable (uninterpreted constant with arity 0)
    if node.is_app() {
        if let Ok(decl) = node.safe_decl() {
            if decl.arity() == 0 {
                if is_variable_decl_kind(decl.kind()) {
                    let name = decl.name();
                    // Filter out Z3 internal names
                    if is_user_variable_name(&name) {
                        variables.insert(Text::from(name));
                    }
                }
            }
        }
    }

    // Recursively process all children
    let num_children = node.num_children();
    for i in 0..num_children {
        if let Some(child) = node.nth_child(i) {
            collect_variables_from_dynamic(&child, variables, visited);
        }
    }
}

/// Collect all free variables paired with their Z3 sorts from a
/// Boolean formula.
///
/// Same traversal as [`collect_variables_from_bool`], but for each
/// uninterpreted constant we read its declaration's range (the result
/// sort of an arity-0 FuncDecl is the variable's own sort) and store
/// the `(name, sort)` pair.
///
/// Critical for callers that need to construct quantifier bindings
/// for the same variables — Z3 distinguishes constants by both name
/// and sort, so a bound `Bool x` does not bind a free `Int x`. Without
/// the sort context, quantifier elimination operates on a vacuous
/// quantifier and produces an unsound or trivially-true result.
///
/// If the same name appears with multiple sorts (which would be a
/// well-formedness error in the source formula), the first sort
/// encountered during traversal wins.
pub fn collect_typed_variables_from_bool(formula: &z3::ast::Bool) -> Map<Text, Sort> {
    let mut typed: Map<Text, Sort> = Map::new();
    let mut visited = HashSet::new();

    let dynamic = Dynamic::from_ast(formula);
    collect_typed_variables_from_dynamic(&dynamic, &mut typed, &mut visited);

    typed
}

/// Recursive walker that extracts `(name, sort)` pairs.
fn collect_typed_variables_from_dynamic(
    node: &Dynamic,
    typed: &mut Map<Text, Sort>,
    visited: &mut HashSet<u64>,
) {
    let id = compute_ast_hash(node);
    if visited.contains(&id) {
        return;
    }
    visited.insert(id);

    if node.is_app() {
        if let Ok(decl) = node.safe_decl() {
            if decl.arity() == 0 && is_variable_decl_kind(decl.kind()) {
                let name = decl.name();
                if is_user_variable_name(&name) {
                    let key = Text::from(name);
                    if !typed.contains_key(&key) {
                        // `node` IS the constant (arity 0), so its
                        // `get_sort()` is exactly the variable's sort.
                        // The FuncDecl-side `range()` returns a
                        // SortKind only, not a full Sort, so use the
                        // AST node's sort accessor.
                        typed.insert(key, node.get_sort());
                    }
                }
            }
        }
    }

    let num_children = node.num_children();
    for i in 0..num_children {
        if let Some(child) = node.nth_child(i) {
            collect_typed_variables_from_dynamic(&child, typed, visited);
        }
    }
}

/// Compute a hash for AST node memoization
#[inline]
fn compute_ast_hash(node: &Dynamic) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.hash(&mut hasher);
    hasher.finish()
}

/// Check if a DeclKind represents a user variable (uninterpreted constant)
#[inline]
fn is_variable_decl_kind(kind: DeclKind) -> bool {
    matches!(kind, DeclKind::Uninterpreted)
}

/// Check if a name is a user variable (not Z3 internal)
///
/// Filters out:
/// - Names starting with 'k!' (Z3 generated skolem constants)
/// - Names starting with '!' (Z3 internal)
/// - Names starting with ':' (SMT-LIB keywords)
/// - Pure numeric names
#[inline]
fn is_user_variable_name(name: &str) -> bool {
    !name.starts_with("k!")
        && !name.starts_with('!')
        && !name.starts_with(':')
        && !name.chars().all(|c| c.is_numeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use z3::ast::Bool;

    #[test]
    fn test_simple_variable_extraction() {
        let _ctx = z3::Context::thread_local();
        let x = Bool::new_const("x");
        let y = Bool::new_const("y");

        let formula = Bool::and(&[&x, &y]);
        let vars = collect_variables_from_bool(&formula);

        assert!(vars.contains(&Text::from("x")));
        assert!(vars.contains(&Text::from("y")));
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn test_filters_constants() {
        let _ctx = z3::Context::thread_local();
        let x = Bool::new_const("x");
        let t = Bool::from_bool(true);

        let formula = Bool::and(&[&x, &t]);
        let vars = collect_variables_from_bool(&formula);

        assert!(vars.contains(&Text::from("x")));
        // true should not be in variables
        assert!(!vars.contains(&Text::from("true")));
        assert_eq!(vars.len(), 1);
    }
}
