//! Admission for the declaration-owned copy operation selected by the checker.
//! Owned components are copied; borrowed components retain their permission.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use verum_ast::Span;
use verum_common::{List, ResourceDiscipline, Text};

impl TypeChecker {
    pub(super) fn check_structural_clone(&self, ty: &Type, span: Span) -> Result<()> {
        self.check_clone_layout(ty)
            .map_err(|reason| TypeError::OtherWithCodeSpanned {
                code: "E400".into(),
                msg: format!("No implicit clone for `{ty}`: {reason}").into(),
                span,
            })
    }

    fn check_clone_layout(&self, ty: &Type) -> std::result::Result<(), String> {
        // Consume expanded components instead of recursively cloning a subtree
        // at each edge. Bound both expansion depth and total work, including
        // cycles and nonregular generic declarations.
        let mut pending = List::from_iter([(ty.clone(), 0usize)]);
        let mut remaining = 4096usize;
        while let Some((ty, depth)) = pending.pop() {
            if depth >= 64 || remaining == 0 {
                return Err("copy layout exceeds the finite declaration boundary".to_owned());
            }
            remaining -= 1;
            let mut enqueue = |component| {
                if pending.len() >= remaining {
                    return Err("copy layout exceeds the finite declaration boundary".to_owned());
                }
                pending.push((component, depth + 1));
                Ok(())
            };
            match ty {
                Type::Unit
                | Type::Never
                | Type::Bool
                | Type::Int
                | Type::Float
                | Type::Char
                | Type::Text
                | Type::Reference { mutable: false, .. }
                | Type::CheckedReference { mutable: false, .. } => {}
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
                    if discipline != ResourceDiscipline::Unrestricted {
                        return Err(format!("`{key}` has {discipline:?} resource discipline"));
                    }
                    let fields: Text = format!("__struct_fields_{key}").into();
                    let layout = self
                        .ctx
                        .lookup_type(&fields)
                        .or_else(|| self.ctx.lookup_type(&format!("__newtype_inner_{key}")))
                        .or_else(|| self.ctx.lookup_type(&format!("__tuple_fields_{key}")))
                        .or_else(|| self.ctx.type_aliases.get(&key))
                        .or_else(|| self.ctx.lookup_type(&key))
                        .ok_or_else(|| format!("layout of `{key}` is unavailable"))?;
                    let instantiated = self.substitute_with_params(&key, layout, args);
                    if instantiated == ty {
                        return Err(format!("layout of `{key}` is opaque"));
                    }
                    enqueue(instantiated)?;
                }
                _ => return Err(format!("component `{ty}` requires a declared clone")),
            }
        }
        Ok(())
    }
}
