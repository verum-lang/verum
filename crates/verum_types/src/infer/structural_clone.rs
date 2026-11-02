//! Layout evidence for the VM's implicit value copy, after declared methods miss.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use std::collections::HashMap;
use verum_ast::Span;
use verum_common::{ResourceDiscipline, Text};

impl TypeChecker {
    pub(super) fn check_structural_clone(&self, ty: &Type, span: Span) -> Result<()> {
        self.check_clone_layout(ty, 0, &mut HashMap::new())
            .map_err(|reason| TypeError::OtherWithCodeSpanned {
                code: "E400".into(),
                msg: format!("No implicit clone for `{ty}`: {reason}").into(),
                span,
            })
    }

    fn check_clone_layout(
        &self,
        ty: &Type,
        depth: usize,
        proven: &mut HashMap<Text, Vec<Vec<Type>>>,
    ) -> std::result::Result<(), String> {
        if depth >= 64 {
            return Err(format!(
                "layout of `{ty}` exceeds the supported nesting depth"
            ));
        }
        let mut component = |ty: &Type| self.check_clone_layout(ty, depth + 1, proven);
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
            Type::Record(fields) => fields.iter().try_for_each(|(name, field)| {
                component(field).map_err(|reason| format!("field `{name}`: {reason}"))
            }),
            Type::Tuple(items) => items.iter().try_for_each(component),
            Type::Array { element, .. } => component(element),
            Type::Refined { base, .. } => component(base),
            Type::Named { .. } | Type::Generic { .. } => {
                let (name, args) = self.conversion_nominal_head(ty).expect("nominal type");
                let key = if name.contains(".") {
                    name
                } else if self.current_module_declared_types.contains(name.as_str()) {
                    self.declared_type_key(name.as_str())
                } else if let Some((_, key)) = self.lookup_type_mount_scoped_with_key(&name, "") {
                    key
                } else if let Some((declaration, owner)) =
                    self.declaring_source_type_declaration(&name)
                {
                    format!("{owner}.{}", declaration.name.name).into()
                } else {
                    name
                };
                // Repeated fields share one layout check per declaration and
                // instantiation. Cache only completed checks, within this call.
                if proven
                    .get(&key)
                    .is_some_and(|instances| instances.iter().any(|types| types == args))
                {
                    return Ok(());
                }
                let discipline = self.declared_resource_discipline(&key);
                if discipline != ResourceDiscipline::Unrestricted {
                    return Err(format!("`{key}` has {discipline:?} resource discipline"));
                }
                let fields: Text = format!("__struct_fields_{key}").into();
                let layout = self
                    .ctx
                    .lookup_type(&fields)
                    .or_else(|| self.ctx.lookup_type(&format!("__tuple_fields_{key}")))
                    .or_else(|| self.ctx.lookup_type(&format!("__newtype_inner_{key}")))
                    .or_else(|| self.ctx.type_aliases.get(&key))
                    .or_else(|| self.ctx.lookup_type(&key))
                    .ok_or_else(|| format!("layout of `{key}` is unavailable"))?;
                let instantiated = self.substitute_with_params(&key, layout, args);
                match &instantiated {
                    // The selected declaration owns this variant. A raw case
                    // map with no declaration cannot supply its resource discipline.
                    Type::Variant(cases) => cases.iter().try_for_each(|(name, payload)| {
                        self.check_clone_layout(payload, depth + 1, proven)
                            .map_err(|reason| format!("case `{name}`: {reason}"))
                    }),
                    _ if instantiated != *ty => {
                        self.check_clone_layout(&instantiated, depth + 1, proven)
                    }
                    _ => Err(format!("layout of `{key}` is opaque")),
                }?;
                proven.entry(key).or_default().push(args.to_vec());
                Ok(())
            }
            _ => Err(format!("component `{ty}` requires a declared clone")),
        }
    }
}
