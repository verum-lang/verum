//! Structural resource operations use the declared layout of each owned component.
use super::TypeChecker;
use crate::{Result, TypeError, context::{OverwriteProjection, ReferentOverwrite}, ty::{Type, TypeVar}};
use verum_ast::Span;
use verum_common::{List, ResourceDiscipline, Set, Text, well_known_types::RuntimeCopyStorage};

#[cfg(test)]
#[path = "../../tests/unit/structural_storage_clone.rs"]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StructuralUse {
    Clone,
    Overwrite,
}

impl TypeChecker {
    pub(super) fn check_structural_clone(&self, ty: &Type, span: Span) -> Result<()> {
        self.check_resource_layout(ty, StructuralUse::Clone, None)
            .map_err(|reason| TypeError::OtherWithCodeSpanned {
                code: "E400".into(),
                msg: format!("No implicit clone for `{ty}`: {reason}").into(),
                span,
            })
    }

    fn overwritten_value_type(&self, write: &ReferentOverwrite, span: Span) -> Option<Type> {
        let mut field = self.unifier.apply(&write.owner);
        let fields = match &write.projection {
            OverwriteProjection::SliceElement => {
                let Type::Slice { element } = field else { return None; };
                return Some(*element);
            }
            OverwriteProjection::Fields(fields) => fields,
        };
        let head = self.place_reference_head(&field, span).ok()?;
        let (referent, true, _) = head.reference_parts()? else { return None };
        field = referent.clone();
        for (index, name) in fields.iter().enumerate() {
            if index > 0 {
                let head = self.place_reference_head(&field, span).ok()?;
                field = match head.reference_parts() {
                    Some((referent, true, _)) => referent.clone(),
                    Some(_) => return None,
                    None => head,
                };
            }
            field = self.lookup_field_type(&field, name)?;
        }
        Some(self.unifier.apply(&field))
    }

    pub(super) fn check_instantiated_referent_overwrite(
        &self,
        write: &ReferentOverwrite,
        span: Span,
    ) -> Result<()> {
        let field = self.overwritten_value_type(write, span).ok_or_else(|| {
            TypeError::OtherWithCodeSpanned {
                code: "E310".into(),
                msg: "cannot establish the type of the selected value to overwrite".into(),
                span,
            }
        })?;
        self.check_overwrite_layout(&field, None, span)
    }

    pub(super) fn check_referent_overwrite(&self, ty: &Type, span: Span) -> Result<()> {
        // A generic body can defer only the exact declared input field whose
        // old value the selected callable checks after substitution. Concrete
        // linear components are rejected even inside an open declaration.
        let ty = self.unifier.apply(ty);
        let permitted = self
            .source_type_registration
            .referent_overwrites
            .iter()
            .filter(|_| self.borrow_tracker.current_closure_id().is_none())
            .find(|write| {
                write.place == span && self.overwritten_value_type(write, span).as_ref() == Some(&ty)
            })
            .map(|write| {
                write
                    .owner
                    .free_vars()
                    .into_iter()
                    .filter(|var| self.unifier.is_rigid(*var))
                    .collect::<Set<_>>()
            });
        self.check_overwrite_layout(&ty, permitted.as_ref(), span)
    }

    fn check_overwrite_layout(
        &self,
        ty: &Type,
        permitted: Option<&Set<TypeVar>>,
        span: Span,
    ) -> Result<()> {
        self.check_resource_layout(ty, StructuralUse::Overwrite, permitted)
            .map_err(|reason| TypeError::OtherWithCodeSpanned {
                code: "E310".into(),
                msg: format!("cannot overwrite `{ty}`: {reason}").into(),
                span,
            })
    }

    fn check_resource_layout(&self, ty: &Type, operation: StructuralUse, permitted: Option<&Set<TypeVar>>) -> std::result::Result<(), Text> {
        // Consume expanded components instead of recursively cloning a subtree
        // at each edge. Bound both expansion depth and total work, including
        // cycles and nonregular generic declarations.
        let mut pending = List::from_iter([(ty.clone(), 0usize)]);
        let mut enclosing_storage: List<(usize, RuntimeCopyStorage, List<Type>)> = List::new();
        let mut remaining = 4096usize;
        while let Some((ty, depth)) = pending.pop() {
            if depth >= 64 || remaining == 0 {
                return Err(Text::from("resource layout exceeds the finite declaration boundary"));
            }
            remaining -= 1;
            while enclosing_storage.last().is_some_and(|(ancestor, _, _)| *ancestor >= depth) {
                enclosing_storage.pop();
            }
            let mut enqueue = |component| {
                if pending.len() >= remaining {
                    return Err(Text::from("resource layout exceeds the finite declaration boundary"));
                }
                pending.push((component, depth + 1));
                Ok(())
            };
            match ty {
                Type::Var(var) if permitted.is_some_and(|vars| vars.contains(&var)) => {}
                Type::Unit
                | Type::Never
                | Type::Bool
                | Type::Int
                | Type::Float
                | Type::Char
                | Type::Text
                | Type::Reference { mutable: false, .. }
                | Type::CheckedReference { mutable: false, .. } => {}
                Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
                | Type::Pointer { .. } | Type::VolatilePointer { .. } | Type::Function { .. }
                | Type::Universe { .. }
                    if operation == StructuralUse::Overwrite => {}
                Type::Record(fields) | Type::Variant(fields) => {
                    for field in fields.into_values() {
                        enqueue(field)?;
                    }
                }
                Type::Tuple(items) => {
                    for item in items {
                        enqueue(item)?;
                    }
                }
                Type::Array { element, .. } | Type::Refined { base: element, .. } => {
                    enqueue(*element)?
                }
                Type::Named { .. } | Type::Generic { .. } => {
                    if self.has_registered_scalar_storage(&ty) {
                        continue;
                    }
                    let (name, args) = self.conversion_nominal_head(&ty).expect("nominal type");
                    let key = if name.contains(".") {
                        name
                    } else if let Some((declaration, owner)) =
                        self.declaring_source_type_declaration(&name)
                    {
                        format!("{owner}.{}", declaration.name.name).into()
                    } else if let Some((_, key)) = self.lookup_type_mount_scoped_with_key(&name, "")
                    {
                        key
                    } else if self.current_module_declared_types.contains(name.as_str()) {
                        self.declared_type_key(name.as_str())
                    } else {
                        name
                    };
                    let discipline = self.declared_resource_discipline(&key);
                    if operation == StructuralUse::Overwrite && discipline == ResourceDiscipline::Linear {
                        return Err(format!("linear value `{key}` must be consumed exactly once").into());
                    }
                    if discipline == ResourceDiscipline::Unknown
                        || (operation == StructuralUse::Clone && discipline != ResourceDiscipline::Unrestricted) {
                        return Err(format!("`{key}` has {discipline:?} resource discipline").into());
                    }
                    if let Some(carrier) = self.declared_runtime_storage_carrier(&key) {
                        if carrier.retained_storage_parameters().len() != args.len() {
                            return Err(format!("storage argument count does not match `{key}`").into());
                        }
                        if operation == StructuralUse::Overwrite {
                            for &position in carrier.owned_storage_parameters() {
                                enqueue(args[position].clone())?;
                            }
                        } else if let Some(storage) = carrier.runtime_copy_storage()
                            && !carrier.owned_storage_parameters().is_empty()
                        {
                            // Only a repeated, identically instantiated backing
                            // allocation guards recursion. Sibling payloads
                            // remain pending and naked record cycles still fail.
                            if !enclosing_storage.iter().any(|(_, kind, applied)|
                                *kind == storage && applied.as_slice() == args) {
                                enclosing_storage.push((depth, storage, args.iter().cloned().collect()));
                                for &position in carrier.owned_storage_parameters() {
                                    enqueue(args[position].clone())?;
                                }
                            }
                            continue;
                        }
                    }
                    let fields: Text = format!("__struct_fields_{key}").into();
                    let layout = self
                        .ctx
                        .lookup_type(&fields)
                        .or_else(|| self.ctx.lookup_type(&format!("__newtype_inner_{key}")))
                        .or_else(|| self.ctx.lookup_type(&format!("__tuple_fields_{key}")))
                        .or_else(|| self.ctx.type_aliases.get(&key))
                        .or_else(|| self.ctx.lookup_type(&key))
                        .ok_or_else(|| Text::from(format!("layout of `{key}` is unavailable")))?;
                    let instantiated = self.substitute_with_params(&key, layout, args);
                    if instantiated == ty {
                        return Err(format!("layout of `{key}` is opaque").into());
                    }
                    enqueue(instantiated)?;
                }
                _ => return Err(match operation {
                    StructuralUse::Clone => format!("component `{ty}` requires a declared clone").into(),
                    StructuralUse::Overwrite => format!("cannot establish that component `{ty}` may be discarded").into(),
                }),
            }
        }
        Ok(())
    }
}

/// Track writes directly rooted at the lexical receiver across statement blocks.
/// An unrecognized place or nested callable retains its ordinary body check.
pub(super) fn indexed_receiver_writes(function: &verum_ast::FunctionDecl) -> List<Span> {
    use verum_ast::{Block, Expr, ExprKind, FunctionParamKind, PathSegment, StmtKind, decl::FunctionBody};
    enum Scope<'a> { Expr(&'a Expr), Block(&'a Block) }
    if !matches!(function.params.first().map(|parameter| &parameter.kind),
        Some(FunctionParamKind::SelfRefMut | FunctionParamKind::SelfRefCheckedMut | FunctionParamKind::SelfRefUnsafeMut))
    { return List::new(); }
    let first = match &function.body {
        Some(FunctionBody::Block(block)) => Scope::Block(block),
        Some(FunctionBody::Expr(expr)) => Scope::Expr(expr),
        None => return List::new(),
    };
    let mut pending = List::from_single(first);
    let mut writes = List::new();
    let mut remaining = 4096usize;
    while let Some(scope) = pending.pop() {
        let Some(rest) = remaining.checked_sub(1) else { return List::new(); };
        remaining = rest;
        match scope {
            Scope::Block(block) => {
                if block.stmts.len() > remaining { return List::new(); }
                pending.extend(block.stmts.iter().filter_map(|statement| match &statement.kind {
                    StmtKind::Expr { expr, .. } => Some(Scope::Expr(expr)),
                    _ => None,
                }));
                pending.extend(block.expr.as_deref().map(Scope::Expr));
            }
            Scope::Expr(expr) => match &expr.kind {
                ExprKind::Binary { op, left, .. } if op.is_assignment() => {
                    if let ExprKind::Index { expr, .. } = &left.kind
                        && let ExprKind::Path(path) = &expr.kind
                        && matches!(path.segments.as_slice(), [PathSegment::SelfValue])
                    { writes.push(left.span); }
                }
                ExprKind::Block(block) | ExprKind::Loop { body: block, .. }
                | ExprKind::While { body: block, .. } | ExprKind::For { body: block, .. } =>
                    pending.push(Scope::Block(block)),
                ExprKind::If { then_branch, else_branch, .. } => {
                    pending.push(Scope::Block(then_branch));
                    pending.extend(else_branch.as_deref().map(Scope::Expr));
                }
                ExprKind::Match { arms, .. } => {
                    if arms.len() > remaining { return List::new(); }
                    pending.extend(arms.iter().map(|arm| Scope::Expr(&arm.body)));
                }
                _ => {}
            },
        }
        if pending.len() > remaining { return List::new(); }
    }
    writes
}
