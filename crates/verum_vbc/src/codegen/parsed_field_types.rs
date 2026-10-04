//! Declaration-owned substitution for field carrier names (T1545).
//!
//! Carried names retain nominal paths even when an imported TypeRef is opaque.
//! Parse that authority instead of splitting commas or rewriting identifier
//! fragments. Unsupported forms remain unknown; no partial binding is emitted.

use verum_ast::ty::{GenericArg, GenericParamKind, PathSegment, Type, TypeKind};
use verum_ast::visitor::{Visitor, walk_type};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;

const MAX_TYPE_BYTES: usize = 65_536;
const MAX_TYPE_DEPTH: usize = 64;

struct Argument<'a> {
    name: &'a str,
    text: &'a str,
    free_roots: Set<Text>,
}

#[derive(Default)]
struct TypeEdits<'a> {
    source: &'a str,
    arguments: &'a [Argument<'a>],
    bound: List<Text>,
    free_roots: Set<Text>,
    edits: List<(usize, usize, &'a str)>,
    unsupported: bool,
    depth: usize,
}

impl TypeEdits<'_> {
    fn root(&mut self, ident: &verum_ast::ty::Ident, start: usize) {
        let name = ident.name.as_str();
        let end = start + name.len();
        if self.source.get(start..end) != Some(name) {
            self.unsupported = true;
            return;
        }
        if self.bound.iter().any(|bound| bound == name) {
            return;
        }
        self.free_roots.insert(Text::from(name));
        if let Some(arg) = self.arguments.iter().find(|arg| arg.name == name) {
            // Without declaration IDs for the nested binder, alpha-renaming
            // would guess. Refuse a substitution that would capture an actual.
            if self
                .bound
                .iter()
                .any(|bound| arg.free_roots.contains(bound))
            {
                self.unsupported = true;
                return;
            }
            // Path spans start at the root but may cover `T.Item` in full.
            // Validate the root's exact bytes; some parser Ident spans point
            // at the following token. Never replace the projection suffix.
            self.edits.push((start, end, arg.text));
        }
    }
}

impl Visitor for TypeEdits<'_> {
    fn visit_type(&mut self, ty: &Type) {
        if self.unsupported || self.depth >= MAX_TYPE_DEPTH {
            self.unsupported = true;
            return;
        }
        self.depth += 1;
        match &ty.kind {
            TypeKind::Path(path) => {
                // A qualified nominal like alpha.Item is indivisible. The
                // parser represents generic `T.Item` as Qualified{self_ty:T}.
                if let [PathSegment::Name(ident)] = path.segments.as_slice() {
                    self.root(ident, path.span.start as usize);
                }
            }
            TypeKind::Function { contexts, .. } if !contexts.requirements.is_empty() => {
                self.unsupported = true;
            }
            TypeKind::Rank2Function {
                contexts,
                where_clause,
                ..
            } if !contexts.requirements.is_empty() || where_clause.is_some() => {
                self.unsupported = true;
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                ..
            } => {
                let old_len = self.bound.len();
                for param in type_params {
                    match &param.kind {
                        GenericParamKind::Type {
                            name,
                            bounds,
                            default,
                        } if bounds.is_empty() && default.is_none() => {
                            self.bound.push(name.name.clone());
                        }
                        // Field carrier rendering currently erases constraints;
                        // do not invent a scoped result for a richer carrier.
                        _ => self.unsupported = true,
                    }
                }
                for param in params {
                    self.visit_type(param);
                }
                self.visit_type(return_type);
                self.bound.truncate(old_len);
            }
            TypeKind::Qualified { self_ty, .. } => self.visit_type(self_ty),
            TypeKind::AssociatedType { base, .. } => self.visit_type(base),
            TypeKind::Generic { .. }
            | TypeKind::Tuple(_)
            | TypeKind::Array { .. }
            | TypeKind::Slice(_)
            | TypeKind::Reference { .. }
            | TypeKind::CheckedReference { .. }
            | TypeKind::UnsafeReference { .. }
            | TypeKind::Pointer { .. }
            | TypeKind::Function { .. } => walk_type(self, ty),
            kind if kind.primitive_name().is_some() => {}
            // Includes type lambdas and dependent binders. They must not be
            // traversed without their own scope/identity contract.
            _ => self.unsupported = true,
        }
        self.depth -= 1;
    }

    fn visit_expr(&mut self, expr: &verum_ast::expr::Expr) {
        use verum_ast::expr::ExprKind;
        match &expr.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Path(path) => {
                if let [PathSegment::Name(ident)] = path.segments.as_slice() {
                    self.root(ident, path.span.start as usize);
                }
            }
            // A carrier's const expression may introduce its own bindings;
            // only literal/name witnesses have an unambiguous contract here.
            _ => self.unsupported = true,
        }
    }
}

pub(super) fn instantiate(owner: &str, parameters: &[String], field: &str) -> Option<String> {
    if owner.len() > MAX_TYPE_BYTES || field.len() > MAX_TYPE_BYTES {
        return None;
    }
    let owner_ast = Parser::new(owner).parse_type().ok()?;
    let TypeKind::Generic { args, .. } = &owner_ast.kind else {
        return None;
    };
    if args.len() != parameters.len() {
        return None;
    }
    let mut arguments = List::with_capacity(args.len());
    for (name, arg) in parameters.iter().zip(args.iter()) {
        let mut free = TypeEdits {
            source: owner,
            ..Default::default()
        };
        let span = match arg {
            GenericArg::Type(ty) => {
                free.visit_type(ty);
                ty.span
            }
            GenericArg::Const(expr) => {
                free.visit_expr(expr);
                expr.span
            }
            _ => return None,
        };
        if free.unsupported {
            return None;
        }
        arguments.push(Argument {
            name,
            text: owner.get(span.start as usize..span.end as usize)?,
            free_roots: free.free_roots,
        });
    }
    let field_ast = Parser::new(field).parse_type().ok()?;
    let mut visitor = TypeEdits {
        source: field,
        arguments: &arguments,
        ..Default::default()
    };
    visitor.visit_type(&field_ast);
    if visitor.unsupported {
        return None;
    }
    visitor.edits.sort_unstable_by_key(|edit| edit.0);
    let mut output = Text::with_capacity(field.len());
    let mut position = 0;
    for (start, end, text) in visitor.edits {
        if start < position || end < start {
            return None;
        }
        output.push_str(field.get(position..start)?);
        output.push_str(text);
        if output.len() > MAX_TYPE_BYTES {
            return None;
        }
        position = end;
    }
    output.push_str(field.get(position..)?);
    (output.len() <= MAX_TYPE_BYTES).then(|| String::from(output))
}

#[cfg(test)]
#[path = "../../tests/codegen/parsed_field_types.rs"]
mod tests;
