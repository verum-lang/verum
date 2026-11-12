//! Reflection reads complete declaration witnesses shared by both VBC tiers.
use crate::{
    module::{VbcModule, qualify_module_name},
    types::{CbgrTier, Mutability, TypeId, TypeRef},
};
use verum_common::{List, Text};

/// A diagnostic name is not an identity or permission to cast a value.
/// Normalization bounds the shape before this renderer traverses it.
pub fn name(module: &VbcModule, ty: &TypeRef) -> Result<Text, Text> {
    render(module, &crate::type_identity::canonical_type(module, ty)?)
}

fn nominal_name(module: &VbcModule, id: TypeId) -> Result<Text, Text> {
    if let Some(name) = id.well_known_name()
        && TypeId::from_well_known_scalar_name(name).is_some()
    {
        return Ok(name.into());
    }
    let declaration = module
        .get_type(id)
        .ok_or_else(|| Text::from("type name has no declaration"))?;
    let name = module
        .get_string(declaration.name)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Text::from("type declaration has no name"))?;
    let owner = match declaration.origin_module {
        Some(owner) => module
            .get_string(owner)
            .ok_or_else(|| Text::from("type declaration has no module name"))?,
        None => &module.name,
    };
    Ok(qualify_module_name(owner, name).into())
}

fn render(module: &VbcModule, ty: &TypeRef) -> Result<Text, Text> {
    let components = |items: &[TypeRef]| -> Result<Text, Text> {
        Ok(items
            .iter()
            .map(|item| render(module, item))
            .collect::<Result<List<_>, _>>()?
            .join(", ")
            .into())
    };
    Ok(match ty {
        TypeRef::Concrete(id) => nominal_name(module, *id)?,
        TypeRef::Instantiated { base, args } => {
            format!("{}<{}>", nominal_name(module, *base)?, components(args)?).into()
        }
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => {
            let mutability = if *mutability == Mutability::Mutable {
                "mut "
            } else {
                ""
            };
            let tier = match tier {
                CbgrTier::Tier0 => "",
                CbgrTier::Tier1 => "checked ",
                CbgrTier::Tier2 => "unsafe ",
            };
            format!("&{mutability}{tier}{}", render(module, inner)?).into()
        }
        TypeRef::Array { element, length } => {
            format!("[{}; {length}]", render(module, element)?).into()
        }
        TypeRef::Slice(element) => format!("[{}]", render(module, element)?).into(),
        TypeRef::Tuple(parts) => {
            let suffix = if parts.len() == 1 { "," } else { "" };
            format!("({}{suffix})", components(parts)?).into()
        }
        TypeRef::ConstValue(value) => value.to_string().into(),
        TypeRef::ConstSequence(values) => values.to_string().into(),
        _ => return Err("type name requires a complete renderable semantic witness".into()),
    })
}

/// Report work in the actual destruction plan, not a type-name heuristic.
pub fn needs_drop(module: &VbcModule, ty: &TypeRef) -> Result<bool, Text> {
    let ty = crate::type_identity::canonical_type(module, ty)?;
    let plan = crate::drop_plan::build(module, &ty)?;
    Ok(!matches!(plan, crate::drop_plan::DropPlan::Ignore))
}
