//! The VM's implicit clone copies one outer allocation. Anything retained
//! inside it must be safe to share for the lifetime of the returned value.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use verum_ast::Span;
use verum_common::{ResourceDiscipline, Text};

impl TypeChecker {
    pub(super) fn check_structural_clone(&self, ty: &Type, span: Span) -> Result<()> {
        self.check_clone_layout(ty, true, 0)
            .map_err(|reason| TypeError::OtherWithCodeSpanned {
                code: "E400".into(),
                msg: format!("No implicit clone for `{ty}`: {reason}").into(),
                span,
            })
    }

    fn check_clone_layout(
        &self,
        ty: &Type,
        copies_storage: bool,
        depth: usize,
    ) -> std::result::Result<(), String> {
        if depth >= 64 {
            return Err(format!(
                "layout of `{ty}` exceeds the supported nesting depth"
            ));
        }
        let component = |ty: &Type| self.check_clone_layout(ty, false, depth + 1);
        let aliases_storage = || {
            Err("a shallow clone would share nested writable storage; provide a clone method that copies it".to_owned())
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
            | Type::CheckedReference { mutable: false, .. } => Ok(()),
            Type::Record(fields) if copies_storage || fields.is_empty() => {
                fields.iter().try_for_each(|(name, field)| {
                    component(field).map_err(|reason| format!("field `{name}`: {reason}"))
                })
            }
            Type::Tuple(items) if copies_storage || items.is_empty() => {
                items.iter().try_for_each(component)
            }
            Type::Array { element, .. } if copies_storage => component(element),
            Type::Record(_) | Type::Tuple(_) | Type::Array { .. } => aliases_storage(),
            Type::Refined { base, .. } => self.check_clone_layout(base, copies_storage, depth + 1),
            Type::Named { .. } | Type::Generic { .. } => {
                let (name, args) = self.conversion_nominal_head(ty).expect("nominal type");
                let key = if name.contains(".") {
                    name
                } else if let Some((declaration, owner)) =
                    self.declaring_source_type_declaration(&name)
                {
                    format!("{owner}.{}", declaration.name.name).into()
                } else if let Some((_, key)) = self.lookup_type_mount_scoped_with_key(&name, "") {
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
                // Tuple newtypes own their single addressable slot too.
                if let Some(inner) = self.ctx.lookup_type(&format!("__newtype_inner_{key}")) {
                    return if copies_storage {
                        component(&self.substitute_with_params(&key, inner, args))
                    } else {
                        aliases_storage()
                    };
                }
                let fields: Text = format!("__struct_fields_{key}").into();
                let layout = self
                    .ctx
                    .lookup_type(&fields)
                    .or_else(|| self.ctx.lookup_type(&format!("__tuple_fields_{key}")))
                    .or_else(|| self.ctx.type_aliases.get(&key))
                    .or_else(|| self.ctx.lookup_type(&key))
                    .ok_or_else(|| format!("layout of `{key}` is unavailable"))?;
                let instantiated = self.substitute_with_params(&key, layout, args);
                match &instantiated {
                    Type::Variant(cases)
                        if copies_storage || cases.values().all(|p| matches!(p, Type::Unit)) =>
                    {
                        cases.iter().try_for_each(|(name, payload)| {
                            component(payload).map_err(|reason| format!("case `{name}`: {reason}"))
                        })
                    }
                    Type::Variant(_) => aliases_storage(),
                    _ if instantiated != *ty => {
                        self.check_clone_layout(&instantiated, copies_storage, depth + 1)
                    }
                    _ => Err(format!("layout of `{key}` is opaque")),
                }
            }
            _ => Err(format!("component `{ty}` requires a declared clone")),
        }
    }
}
