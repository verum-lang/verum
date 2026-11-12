//! Stored references follow declaration-owned fields and parameter bindings.
//! Inspect borrowed semantic nodes without materializing recursive type instances.
use super::TypeChecker;
use crate::Type;
use verum_common::{List, Map, Set};

const MAX_LOAN_SHAPE_STEPS: usize = 4096;

#[derive(Clone, Copy)]
struct Node<'a> {
    ty: &'a Type,
    scope: usize,
}

impl Node<'_> {
    fn key(self) -> (usize, usize) {
        (std::ptr::from_ref(self.ty).addr(), self.scope)
    }
}

type Bindings<'a> = List<(&'a Type, Node<'a>)>;

fn queue<'a>(
    pending: &mut List<Node<'a>>,
    children: impl IntoIterator<Item = &'a Type>,
    scope: usize,
    remaining: usize,
) -> bool {
    for ty in children {
        if pending.len() >= remaining {
            return false;
        }
        pending.push(Node { ty, scope });
    }
    true
}

fn immediate(ty: &Type) -> Option<bool> {
    match ty {
        Type::Reference { .. }
        | Type::CheckedReference { .. }
        | Type::UnsafeReference { .. }
        | Type::GenRef { .. } => Some(true),
        Type::Unit
        | Type::Never
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::Char
        | Type::Text
        | Type::Pointer { .. }
        | Type::VolatilePointer { .. }
        | Type::Meta { .. }
        | Type::Universe { .. }
        | Type::Prop
        | Type::Interval
        | Type::Lifetime { .. } => Some(false),
        _ => None,
    }
}

fn same_parameter(parameter: &Type, candidate: &Type) -> bool {
    match (parameter, candidate) {
        (Type::Var(left), Type::Var(right)) => left == right,
        // Legacy declaration metadata can retain a named lexical parameter.
        // Only a recorded empty argument slot may bind such a spelling.
        (
            Type::Named {
                path: left,
                args: a,
            },
            Type::Named {
                path: right,
                args: b,
            },
        ) if a.is_empty() && b.is_empty() => left == right,
        _ => false,
    }
}

impl TypeChecker {
    fn loan_node<'a>(
        &'a self,
        mut node: Node<'a>,
        scopes: &[Bindings<'a>],
        remaining: &mut usize,
    ) -> Option<Node<'a>> {
        loop {
            *remaining = remaining.checked_sub(1)?;
            let bound = scopes[node.scope]
                .iter()
                .find(|(parameter, _)| same_parameter(parameter, node.ty))
                .map(|(_, value)| *value)
                .or_else(|| match node.ty {
                    Type::Var(variable) => {
                        self.unifier
                            .get_substitution()
                            .get(variable)
                            .map(|ty| Node {
                                ty,
                                scope: node.scope,
                            })
                    }
                    _ => None,
                });
            let Some(next) = bound else {
                return Some(node);
            };
            if next.key() == node.key() {
                return Some(node);
            }
            node = next;
        }
    }

    /// Only value-bearing positions retain loans. Parameter names are bound by
    /// their exact declaration roster; phantom arguments are not stored fields.
    /// A callable signature does not describe its captures, so callable storage
    /// remains unknown, like any other missing layout evidence.
    pub(super) fn carries_stored_loan(&self, ty: &Type) -> bool {
        if let Some(present) = immediate(ty) {
            return present;
        }
        let mut pending = List::from_iter([Node { ty, scope: 0 }]);
        let mut scopes: List<Bindings<'_>> = List::from_iter([List::new()]);
        let mut scope_ids = Map::new();
        let mut visited = Set::new();
        // Non-regular recursive applications can keep constructing distinct
        // environments. Exhaustion means unknown, never reference-free storage.
        let mut remaining = MAX_LOAN_SHAPE_STEPS;
        while let Some(node) = pending.pop() {
            let Some(node) = self.loan_node(node, &scopes, &mut remaining) else {
                return true;
            };
            if !visited.insert(node.key()) {
                continue;
            }
            if let Some(present) = immediate(node.ty) {
                if present {
                    return true;
                }
                continue;
            }
            let child = |ty| Node {
                ty,
                scope: node.scope,
            };
            match node.ty {
                Type::Tuple(parts) => {
                    if !queue(&mut pending, parts, node.scope, remaining) {
                        return true;
                    }
                }
                Type::Record(fields) | Type::Variant(fields) => {
                    if !queue(&mut pending, fields.values(), node.scope, remaining) {
                        return true;
                    }
                }
                Type::ExtensibleRecord { fields, row_var } => {
                    if row_var.is_some() {
                        return true;
                    }
                    if !queue(&mut pending, fields.values(), node.scope, remaining) {
                        return true;
                    }
                }
                Type::Array { element, .. }
                | Type::Slice { element }
                | Type::Tensor { element, .. } => pending.push(child(element)),
                Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                    pending.push(child(base))
                }
                Type::Ownership { inner, .. }
                | Type::Quantified { inner, .. }
                | Type::Exists { body: inner, .. }
                | Type::Forall { body: inner, .. } => pending.push(child(inner)),
                Type::Named { .. } | Type::Generic { .. } => {
                    let Some((name, args)) = self.conversion_nominal_head(node.ty) else {
                        return true;
                    };
                    let Some(body) = self
                        .ctx
                        .lookup_type(&format!("__struct_fields_{name}"))
                        .or_else(|| self.ctx.lookup_type(&name))
                    else {
                        return true;
                    };
                    // A nominal identity alone is not a stored layout. This is
                    // distinct from a recursive edge inside a declared body.
                    if self
                        .conversion_nominal_head(body)
                        .is_some_and(|(owner, _)| owner == name)
                    {
                        return true;
                    }
                    let parameters = match self.ctx.lookup_type(&format!("__type_params_{name}")) {
                        Some(Type::Record(parameters)) if parameters.len() == args.len() => {
                            Some(parameters)
                        }
                        None if args.is_empty() => None,
                        _ => return true,
                    };
                    let mut bindings = List::new();
                    let mut key = List::new();
                    if let Some(parameters) = parameters {
                        for (parameter, argument) in parameters.values().zip(args) {
                            let Some(argument) =
                                self.loan_node(child(argument), &scopes, &mut remaining)
                            else {
                                return true;
                            };
                            key.push((std::ptr::from_ref(parameter).addr(), argument.key()));
                            bindings.push((parameter, argument));
                        }
                    }
                    let scope = if key.is_empty() {
                        0
                    } else if let Some(id) = scope_ids.get(&key) {
                        *id
                    } else {
                        let id = scopes.len();
                        scopes.push(bindings);
                        scope_ids.insert(key, id);
                        id
                    };
                    if !queue(&mut pending, [body], scope, remaining) {
                        return true;
                    }
                }
                _ => return true,
            }
        }
        false
    }
}

#[cfg(test)]
#[path = "../../tests/unit/stored_loans.rs"]
mod tests;
