//! Induction for a derived comparison of finite, owned data. The induction
//! step must hold for an arbitrary smaller element with the same relation.
use super::TypeChecker;
use crate::{Result, Type, protocol::WhereClause, ty::TypeVar};
use verum_ast::{
    TypeDeclBody,
    decl::{ImplDecl, ImplKind, VariantData},
    ty::{TypeBound, TypeBoundKind, TypeKind},
};
use verum_common::{List, Map};

const PARTIAL_EQ: &str = "core.base.protocols.PartialEq";
const EQ: &str = "core.base.protocols.Eq";

impl TypeChecker {
    pub(super) fn proves_derived_equality_component(
        &mut self,
        source: &ImplDecl,
        receiver: &Type,
        field: &verum_ast::Type,
        subject: &Type,
        bounds: &[TypeBound],
    ) -> Result<bool> {
        let ImplKind::Protocol {
            protocol,
            for_type,
            protocol_args,
        } = &source.kind
        else {
            return Ok(false);
        };
        let [
            TypeBound {
                kind: TypeBoundKind::Protocol(requirement),
                span,
            },
        ] = bounds
        else {
            return Ok(false);
        };
        // Generated field obligations carry the actual field's source location.
        // Written where predicates have their own source location and must keep
        // their ordinary meaning, including on a structural equality impl.
        if field.span.is_empty()
            || *span != field.span
            || for_type.span != source.span
            || !protocol_args.is_empty()
            || !matches!(
                (
                    self.path_to_string(protocol).as_str(),
                    self.path_to_string(requirement).as_str()
                ),
                (PARTIAL_EQ, PARTIAL_EQ | EQ) | (EQ, EQ)
            )
        {
            return Ok(false);
        }
        let Some((root, parameters)) = self.conversion_nominal_head(receiver) else {
            return Ok(false);
        };
        let variable = TypeVar::fresh();
        let mut abstracted = subject.clone();
        let mut pending = List::from_iter([&mut abstracted]);
        let mut replaced = false;
        let mut visited = 0;
        while let Some(node) = pending.pop() {
            visited += 1;
            if visited > 4096 {
                return Ok(false);
            }
            if self
                .conversion_nominal_head(node)
                .is_some_and(|(name, args)| {
                    name == root
                        && args.len() == parameters.len()
                        && args.iter().zip(parameters).all(|(a, b)| {
                            self.protocol_checker.read().types_structurally_equal(a, b)
                        })
                })
            {
                *node = Type::Var(variable);
                replaced = true;
            } else {
                node.for_each_child_mut(|child| pending.push(child));
            }
        }
        drop(pending);
        if !replaced || abstracted == Type::Var(variable) {
            return Ok(false);
        }
        let owns_field = if let Some(declaration) = self.source_type_registration.declaration(&root)
        {
            declares_field(&declaration.body, field)
        } else {
            self.source_receiver_declaration(receiver)
                .map(|(_, decl)| decl)
                .or_else(|| {
                    self.declaring_source_type_declaration(&root)
                        .map(|(decl, _)| decl)
                })
                .is_some_and(|decl| declares_field(&decl.body, field))
        };
        if !owns_field || !self.owns_recursive_elements(&abstracted, variable) {
            return Ok(false);
        }
        let required = self.elaborate_family_bounds(bounds)?;
        let assumption = WhereClause {
            ty: Type::Var(variable),
            bounds: required.clone(),
        };
        Ok(self
            .prove_associated_bounds(
                &abstracted,
                &required,
                &[assumption],
                &Map::new(),
                field.span,
            )
            .is_ok())
    }

    fn owns_recursive_elements(&self, component: &Type, element: TypeVar) -> bool {
        let mut pending = List::from_iter([component.clone()]);
        let mut budget = 4096usize;
        let mut stored = false;
        while let Some(mut node) = pending.pop() {
            // Charge the applied shape before expanding another declaration.
            // A recursive constructor can duplicate or grow its type arguments.
            let mut syntax = List::from_iter([&mut node]);
            let mut contains = false;
            while let Some(part) = syntax.pop() {
                let Some(remaining) = budget.checked_sub(1) else {
                    return false;
                };
                budget = remaining;
                contains |= matches!(part, Type::Var(id) if *id == element);
                part.for_each_child_mut(|child| syntax.push(child));
            }
            drop(syntax);
            if !contains {
                continue;
            }
            match node {
                Type::Var(actual) if actual == element => stored = true,
                Type::Tuple(fields) => pending.extend(fields),
                Type::Record(fields) | Type::Variant(fields) => {
                    pending.extend(fields.into_values())
                }
                Type::Array {
                    element,
                    size: None | Some(crate::ty::ArrayLength::Known(_)),
                }
                | Type::Refined { base: element, .. } => pending.push(*element),
                Type::Named { .. } | Type::Generic { .. } => {
                    let (name, args) = self.conversion_nominal_head(&node).expect("nominal type");
                    if let Some(carrier) = self.declared_runtime_storage_carrier(&name) {
                        if args.len() != carrier.retained_storage_parameters().len() {
                            return false;
                        }
                        for (position, arg) in args.iter().enumerate() {
                            if arg.free_vars().contains(&element) {
                                if !carrier.owned_storage_parameters().contains(&position) {
                                    return false;
                                }
                                pending.push(arg.clone());
                            }
                        }
                    } else {
                        let Some(layout) = self
                            .ctx
                            .lookup_type(&format!("__struct_fields_{name}"))
                            .or_else(|| self.ctx.lookup_type(&format!("__tuple_fields_{name}")))
                            .or_else(|| self.ctx.lookup_type(&format!("__newtype_inner_{name}")))
                            .or_else(|| self.ctx.type_aliases.get(&name))
                            .or_else(|| self.ctx.lookup_type(&name))
                        else {
                            return false;
                        };
                        let layout = self.substitute_with_params(&name, layout, args);
                        if layout == node || !layout.free_vars().contains(&element) {
                            return false;
                        }
                        pending.push(layout);
                    }
                }
                // A callable, borrowed, erased or opaque occurrence supplies
                // no owned structural descent to the recursive comparison.
                _ => return false,
            }
        }
        stored
    }
}

fn declares_field(body: &TypeDeclBody, field: &verum_ast::Type) -> bool {
    let matches = |mut actual: &verum_ast::Type| {
        while let TypeKind::Refined { base, .. } = &actual.kind {
            actual = base;
        }
        actual == field
    };
    match body {
        TypeDeclBody::Newtype(ty) => matches(ty),
        TypeDeclBody::Tuple(types) => types.iter().any(matches),
        TypeDeclBody::Record(fields) => fields.iter().any(|f| matches(&f.ty)),
        TypeDeclBody::Variant(variants) => variants.iter().any(|v| match &v.data {
            Some(VariantData::Tuple(types)) => types.iter().any(matches),
            Some(VariantData::Record(fields)) => fields.iter().any(|f| matches(&f.ty)),
            None => false,
        }),
        _ => false,
    }
}
