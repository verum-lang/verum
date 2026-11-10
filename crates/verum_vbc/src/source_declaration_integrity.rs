//! Compatibility between a semantic declaration and its runtime descriptor.
//!
//! Universe terms, constraints, and method signatures live in the AST, not in
//! `TypeRef`. This check binds that AST to the runtime declaration's structural
//! slots; it does not establish the declaration's typing or proof obligations.
//! Incomplete import stubs must be enriched before carrying a semantic AST.

use crate::module::StringTable;
use crate::types::{FieldDescriptor, TypeDescriptor, TypeKind, TypeRef, VariantKind};
use verum_ast::{
    TypeDeclBody,
    decl::{FunctionParamKind, ProtocolItemKind, VariantData},
    ty::GenericParamKind,
};
use verum_common::{List, Set, Text};

/// A declaration compatibility failure retains the innermost source node
/// responsible for it. Archive-only diagnostics may have no source location.
#[derive(Debug, Clone)]
pub(crate) struct SourceDeclarationDiagnostic {
    pub message: Text,
    pub span: Option<verum_ast::Span>,
}

impl SourceDeclarationDiagnostic {
    fn at(mut self, span: verum_ast::Span) -> Self {
        self.span.get_or_insert(span);
        self
    }
}

impl From<Text> for SourceDeclarationDiagnostic {
    fn from(message: Text) -> Self {
        Self {
            message,
            span: None,
        }
    }
}

impl From<&str> for SourceDeclarationDiagnostic {
    fn from(message: &str) -> Self {
        Text::from(message).into()
    }
}

/// Require both string operands to denote one binder and a complete function type.
pub(crate) fn validate_impl_function_bounds(
    implementation: &crate::types::ProtocolImpl,
    strings: &StringTable,
) -> Result<(), Text> {
    for (parameter, bound) in &implementation.type_param_fn_bounds {
        let (Some(parameter), Some(bound)) = (strings.get(*parameter), strings.get(*bound)) else {
            return Err("unresolved implementation function-bound string".into());
        };
        if !verum_fast_parser::FastParser::is_descriptor_function_bound(parameter, bound) {
            return Err("invalid implementation function-bound declaration".into());
        }
    }
    Ok(())
}

/// Validate a declaration-bearing descriptor before publishing or importing it.
/// Ordinary ABI-only descriptors retain their existing validation path.
pub fn validate_type_descriptor(
    descriptor: &TypeDescriptor,
    strings: &StringTable,
) -> Result<(), Text> {
    validate_type_descriptor_diagnostic(descriptor, strings).map_err(|error| error.message)
}

pub(crate) fn validate_type_descriptor_diagnostic(
    descriptor: &TypeDescriptor,
    strings: &StringTable,
) -> Result<(), SourceDeclarationDiagnostic> {
    for implementation in &descriptor.protocols {
        validate_impl_function_bounds(implementation, strings)?;
    }
    if descriptor.kind != TypeKind::Protocol
        && descriptor
            .variants
            .iter()
            .any(|v| v.method_dispatch.is_some())
    {
        return Err("data variant cannot declare a protocol method receiver".into());
    }
    // Bound every retained AST node before any semantic traversal, including
    // the dependency-pattern walk used for record-field correspondence below.
    if let Some(declaration) = &descriptor.source_declaration {
        verum_ast::source_signature::validate_source_type_declaration(declaration)
            .map_err(|message| SourceDeclarationDiagnostic::from(message).at(declaration.span))?;
    }
    validate_record_projections(descriptor)?;
    let Some(declaration) = &descriptor.source_declaration else {
        return Ok(());
    };
    let fail = |detail: &str| -> SourceDeclarationDiagnostic {
        SourceDeclarationDiagnostic::from(Text::from(format!(
            "semantic type {}: {detail}",
            declaration.name.name
        )))
        .at(declaration.span)
    };
    let name = strings
        .get(descriptor.name)
        .ok_or_else(|| fail("descriptor has no name"))?;
    if name.rsplit('.').next() != Some(declaration.name.name.as_str()) {
        return Err(fail("declaration does not belong to its descriptor"));
    }
    if descriptor.resource_discipline != declaration.resource_discipline() {
        return Err(fail("runtime resource policy differs from declaration"));
    }

    // All runtime binders share one dense positional roster. Value arguments
    // occupy witness slots too; only lifetimes are erased. The source AST is
    // the authority for binder kinds, which TypeParamDescriptor does not encode.
    let mut actual_parameters = descriptor.type_params.iter();
    let mut dense_index = 0usize;
    for parameter in &declaration.generics {
        let (name, is_value) = match &parameter.kind {
            GenericParamKind::Type { name, .. } | GenericParamKind::HigherKinded { name, .. } => {
                (name, false)
            }
            GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } => {
                (name, true)
            }
            GenericParamKind::Lifetime { .. } => continue,
            _ => return Err(fail("unsupported runtime generic parameter kind")),
        };
        let actual = actual_parameters
            .next()
            .ok_or_else(|| fail("missing runtime type parameter"))?;
        let expected_id = u16::try_from(dense_index)
            .map_err(|_| fail("type parameter index exceeds runtime range"))?;
        if strings.get(actual.name) != Some(name.name.as_str()) || actual.id.0 != expected_id {
            return Err(fail(
                "runtime type parameter order or identity differs from declaration",
            ));
        }
        if is_value
            && (actual.default.is_some()
                || !actual.bounds.is_empty()
                || !actual.type_bounds.is_empty())
        {
            return Err(fail(
                "runtime value parameter contains type-only constraints",
            ));
        }
        dense_index += 1;
    }
    if actual_parameters.next().is_some() {
        return Err(fail("extra runtime type parameter"));
    }

    let (kind, transparent) = match &declaration.body {
        TypeDeclBody::Record(_) => (TypeKind::Record, false),
        TypeDeclBody::Variant(_) => (TypeKind::Sum, false),
        TypeDeclBody::Protocol(_) => (TypeKind::Protocol, false),
        TypeDeclBody::Alias(_) => (TypeKind::Alias, false),
        // These are the explicit representation erasures used by codegen:
        // tuples/newtypes have record slots, with a single slot transparent.
        TypeDeclBody::Newtype(_) => (TypeKind::Record, true),
        TypeDeclBody::Tuple(elements) => (TypeKind::Record, elements.len() == 1),
        TypeDeclBody::Unit => (TypeKind::Unit, false),
        TypeDeclBody::Inductive(_)
        | TypeDeclBody::Coinductive(_)
        | TypeDeclBody::SigmaTuple(_)
        | TypeDeclBody::Quotient { .. } => {
            return Err(fail(
                "declaration has no supported runtime descriptor representation",
            ));
        }
    };
    if descriptor.kind != kind || descriptor.is_transparent_wrapper != transparent {
        return Err(fail(
            "runtime kind or representation differs from declaration",
        ));
    }
    if let TypeDeclBody::Alias(target) = &declaration.body
        && let verum_ast::ty::TypeKind::Path(path) = &target.kind
        && let Some(marker) = path.as_ident()
        && verum_ast::ty::CubicalTypePrimitive::from_marker(marker.name.as_str()).is_some()
    {
        if descriptor.alias_target.is_some()
            || descriptor
                .alias_target_name
                .and_then(|name| strings.get(name))
                != Some(marker.name.as_str())
        {
            return Err(fail(
                "cubical type marker requires its exact source spelling and no runtime target",
            ));
        }
    } else if (kind == TypeKind::Alias) != descriptor.alias_target.is_some() {
        return Err(fail("runtime alias target does not match declaration kind"));
    }
    if !matches!(kind, TypeKind::Sum | TypeKind::Protocol) && !descriptor.variants.is_empty() {
        return Err(fail("non-variant declaration has runtime variants"));
    }
    match &declaration.body {
        TypeDeclBody::Protocol(body) => {
            if !descriptor.fields.is_empty() {
                return Err(fail("protocol declaration has runtime fields"));
            }
            // Protocol method signatures use the existing variant table as an
            // ordered vtable roster. They are not data-type payload variants.
            let mut actual_methods = descriptor.variants.iter();
            let mut index = 0u32;
            let mut names = Set::new();
            for item in &body.items {
                if let ProtocolItemKind::Function { decl, .. } = &item.kind {
                    let actual = actual_methods
                        .next()
                        .ok_or_else(|| fail("missing runtime protocol method"))?;
                    if actual.method_dispatch.is_some_and(|dispatch| {
                        dispatch
                            != verum_ast::method_dispatch::MethodDispatch::from_declaration(decl)
                    }) {
                        return Err(fail("protocol method dispatch differs from declaration"));
                    }
                    let regular_params = decl
                        .params
                        .iter()
                        .filter(|param| matches!(param.kind, FunctionParamKind::Regular { .. }))
                        .count();
                    if strings.get(actual.name) != Some(decl.name.name.as_str())
                        || actual.tag != index
                        || actual.kind != VariantKind::Unit
                        || actual.arity != 0
                        || !actual.fields.is_empty()
                        || !names.insert(decl.name.name.clone())
                    {
                        return Err(fail(
                            "runtime protocol method name or order differs from declaration",
                        ));
                    }
                    if !matches!(&actual.payload, Some(TypeRef::Function { is_async, params, .. }) if params.len() == regular_params && *is_async == decl.is_async)
                    {
                        return Err(fail(
                            "runtime protocol method callable shape differs from declaration",
                        ));
                    }
                    index = index
                        .checked_add(1)
                        .ok_or_else(|| fail("protocol method index exceeds runtime range"))?;
                }
            }
            if actual_methods.next().is_some() {
                return Err(fail("extra runtime protocol method"));
            }
        }
        TypeDeclBody::Record(fields) => {
            check_fields(
                &descriptor.fields,
                fields.iter().map(|field| field.name.name.to_string()),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
            validate_record_projection_correspondence(descriptor, fields).map_err(
                |mut error| {
                    error.message =
                        format!("semantic type {}: {}", declaration.name.name, error.message)
                            .into();
                    error.at(declaration.span)
                },
            )?;
        }
        TypeDeclBody::Newtype(_) => {
            check_fields(
                &descriptor.fields,
                std::iter::once("_0".to_owned()),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
        }
        TypeDeclBody::Tuple(elements) => {
            check_fields(
                &descriptor.fields,
                (0..elements.len()).map(|index| format!("_{index}")),
                strings,
            )
            .map_err(|message| fail(message.as_str()))?;
        }
        TypeDeclBody::Variant(variants) => {
            if !descriptor.fields.is_empty() || descriptor.variants.len() != variants.len() {
                return Err(fail("runtime variant roster differs from declaration"));
            }
            let mut names = Set::new();
            for (index, (actual, expected)) in descriptor.variants.iter().zip(variants).enumerate()
            {
                let tag =
                    u32::try_from(index).map_err(|_| fail("variant tag exceeds runtime range"))?;
                if strings.get(actual.name) != Some(expected.name.name.as_str())
                    || actual.tag != tag
                    || !names.insert(expected.name.name.clone())
                {
                    return Err(fail(
                        "runtime variant name, order, or tag differs from declaration",
                    ));
                }
                // Current producers describe each payload slot in `fields`;
                // the optional aggregate payload is deliberately absent.
                if actual.payload.is_some() {
                    return Err(fail("unexpected aggregate variant payload"));
                }
                match expected.data.as_ref() {
                    None => {
                        if actual.kind != VariantKind::Unit
                            || actual.arity != 0
                            || !actual.fields.is_empty()
                        {
                            return Err(fail("unit variant has runtime payload slots"));
                        }
                    }
                    Some(VariantData::Tuple(elements)) => {
                        let arity = u8::try_from(elements.len())
                            .map_err(|_| fail("variant arity exceeds runtime range"))?;
                        let kind = if elements.is_empty() {
                            VariantKind::Unit
                        } else {
                            VariantKind::Tuple
                        };
                        if actual.kind != kind || actual.arity != arity {
                            return Err(fail(
                                "runtime tuple variant kind or arity differs from declaration",
                            ));
                        }
                        check_fields(
                            &actual.fields,
                            (0..elements.len()).map(|index| format!("_{index}")),
                            strings,
                        )
                        .map_err(|message| fail(message.as_str()))?;
                    }
                    Some(VariantData::Record(fields)) => {
                        if actual.kind != VariantKind::Record || actual.arity != 0 {
                            return Err(fail(
                                "runtime record variant kind or arity differs from declaration",
                            ));
                        }
                        check_fields(
                            &actual.fields,
                            fields.iter().map(|field| field.name.name.to_string()),
                            strings,
                        )
                        .map_err(|message| fail(message.as_str()))?;
                    }
                }
            }
        }
        _ => {
            if !descriptor.fields.is_empty() {
                return Err(fail("declaration has unexpected runtime fields"));
            }
        }
    }
    Ok(())
}

/// A record projection is bound by one named-field declaration's telescope,
/// never by a function generic binder or an ABI-only descriptor. Walk all
/// descriptor type positions, including nested callable and associated types,
/// so wrappers cannot smuggle a free projection across the archive boundary.
fn validate_record_projections(descriptor: &TypeDescriptor) -> Result<(), Text> {
    let source_fields = descriptor
        .source_declaration
        .as_ref()
        .and_then(|declaration| match &declaration.body {
            TypeDeclBody::Record(fields) if descriptor.kind == TypeKind::Record => Some(fields),
            _ => None,
        });
    let mut pending = List::new();
    for (index, field) in descriptor.fields.iter().enumerate() {
        let scope = source_fields.map(|_| index);
        pending.push((&field.type_ref, scope));
        if let Some(ty) = &field.declaration_type {
            pending.push((ty, scope));
        }
    }
    for parameter in &descriptor.type_params {
        if let Some(ty) = &parameter.default {
            pending.push((ty, None));
        }
        pending.extend(parameter.type_bounds.iter().map(|ty| (ty, None)));
    }
    if let Some(ty) = &descriptor.alias_target {
        pending.push((ty, None));
    }
    for implementation in &descriptor.protocols {
        pending.extend(
            implementation
                .associated_types
                .iter()
                .map(|(_, ty)| (ty, None)),
        );
    }
    for variant in &descriptor.variants {
        if let Some(ty) = &variant.payload {
            pending.push((ty, None));
        }
        for field in &variant.fields {
            pending.push((&field.type_ref, None));
            if let Some(ty) = &field.declaration_type {
                pending.push((ty, None));
            }
        }
    }
    while let Some((ty, scope)) = pending.pop() {
        match ty {
            TypeRef::RecordFieldProjection { owner, ordinal } => {
                let (Some(fields), Some(index)) = (source_fields, scope) else {
                    return Err("record field projection has no source record field binder".into());
                };
                if *owner != descriptor.id {
                    return Err("record field projection belongs to a different owner".into());
                }
                let ordinal = usize::try_from(*ordinal)
                    .map_err(|_| Text::from("record field projection index exceeds host range"))?;
                if ordinal >= index || index >= fields.len() {
                    return Err(
                        "record field projection must reference a preceding source field".into(),
                    );
                }
                if !matches!(
                    fields[ordinal].ty.kind,
                    verum_ast::ty::TypeKind::Universe { .. }
                ) {
                    return Err(
                        "record field projection binder is not a source universe field".into(),
                    );
                }
            }
            TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
                pending.extend(args.iter().map(|ty| (ty, scope)));
            }
            TypeRef::Function {
                params,
                return_type,
                ..
            }
            | TypeRef::Rank2Function {
                params,
                return_type,
                ..
            } => {
                pending.extend(params.iter().map(|ty| (ty, scope)));
                pending.push((return_type.as_ref(), scope));
            }
            TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => {
                pending.push((inner.as_ref(), scope));
            }
            TypeRef::Array { element, .. } => pending.push((element.as_ref(), scope)),
            TypeRef::AssociatedProjection { base, qualifier, args, .. } => {
                pending.push((base.as_ref(), scope));
                pending.extend(args.iter().map(|argument| (argument, scope)));
                if let Some(application) = qualifier {
                    pending.extend(application.args.iter().map(|argument| (argument, scope)));
                }
            },
            TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) => {}
        }
    }
    Ok(())
}

/// Only the dependency structure is projected here. Nominal lookup, universe
/// formation and typing remain the compiler's responsibility. An absent pattern
/// means no record-field template may occur, not that any type is well-typed.
#[derive(Debug)]
enum ProjectionPattern {
    Field(u32),
    Node(ProjectionShape, List<Option<ProjectionPattern>>),
}

#[derive(Debug)]
enum ProjectionShape {
    Tuple,
    Generic,
    Function(Option<u16>),
    Reference(crate::types::Mutability, crate::types::CbgrTier),
    Array,
    DynamicArray,
    Slice,
    Associated(Text),
}

fn projection_node(
    shape: ProjectionShape,
    children: List<Option<ProjectionPattern>>,
) -> Option<ProjectionPattern> {
    children
        .iter()
        .any(Option::is_some)
        .then_some(ProjectionPattern::Node(shape, children))
}

fn shadow_record_binders(
    scope: &verum_common::Map<Text, u32>,
    parameters: &[verum_ast::ty::GenericParam],
) -> verum_common::Map<Text, u32> {
    let mut scope = scope.clone();
    for parameter in parameters {
        let name = match &parameter.kind {
            GenericParamKind::Type { name, .. }
            | GenericParamKind::HigherKinded { name, .. }
            | GenericParamKind::Meta { name, .. }
            | GenericParamKind::Const { name, .. }
            | GenericParamKind::Context { name }
            | GenericParamKind::Level { name }
            | GenericParamKind::KindAnnotated { name, .. } => &name.name,
            GenericParamKind::Lifetime { .. } => continue,
        };
        scope.remove(name);
    }
    scope
}

/// Unsupported representations must not silently erase a source dependency.
/// This scan observes AST binders, never identifier substrings or rendered types.
fn contains_source_projection(ty: &verum_ast::Type, scope: &verum_common::Map<Text, u32>) -> bool {
    use verum_ast::visitor::{Visitor, walk_type};
    struct Scan<'a> {
        scope: &'a verum_common::Map<Text, u32>,
        found: bool,
    }
    impl Visitor for Scan<'_> {
        fn visit_type(&mut self, ty: &verum_ast::Type) {
            use verum_ast::ty::TypeKind as S;
            if self.found {
                return;
            }
            match &ty.kind {
                S::Path(path) => {
                    self.found = path
                        .as_ident()
                        .is_some_and(|name| self.scope.contains_key(&name.name))
                }
                S::Rank2Function {
                    type_params,
                    params,
                    return_type,
                    ..
                } => {
                    let scope = shadow_record_binders(self.scope, type_params);
                    self.found = params
                        .iter()
                        .chain(std::iter::once(return_type.as_ref()))
                        .any(|ty| contains_source_projection(ty, &scope));
                }
                S::TypeLambda { params, body } => {
                    let mut scope = self.scope.clone();
                    for parameter in params {
                        scope.remove(&parameter.name);
                    }
                    self.found = contains_source_projection(body, &scope);
                }
                S::Record { fields, .. } => {
                    let mut scope = self.scope.clone();
                    for field in fields {
                        self.found |= contains_source_projection(&field.ty, &scope);
                        scope.remove(&field.name.name);
                    }
                }
                S::TypeConstructor { base, .. } => self.visit_type(base),
                _ => walk_type(self, ty),
            }
        }
    }
    let mut scan = Scan {
        scope,
        found: false,
    };
    scan.visit_type(ty);
    scan.found
}

fn source_projection_pattern(
    ty: &verum_ast::Type,
    scope: &verum_common::Map<Text, u32>,
    preserve_refs: bool,
) -> Result<Option<ProjectionPattern>, SourceDeclarationDiagnostic> {
    source_projection_pattern_inner(ty, scope, preserve_refs).map_err(|error| error.at(ty.span))
}

fn source_projection_pattern_inner(
    ty: &verum_ast::Type,
    scope: &verum_common::Map<Text, u32>,
    preserve_refs: bool,
) -> Result<Option<ProjectionPattern>, SourceDeclarationDiagnostic> {
    use crate::types::{CbgrTier, Mutability};
    use verum_ast::ty::{GenericArg, TypeKind as S};
    let child = |ty| source_projection_pattern(ty, scope, preserve_refs);
    let single = |shape, pattern| projection_node(shape, List::from_single(pattern));
    Ok(match &ty.kind {
        S::Path(path) => path
            .as_ident()
            .and_then(|name| scope.get(&name.name))
            .copied()
            .map(ProjectionPattern::Field),
        S::Refined { base, .. } => child(base)?,
        S::Reference { inner, mutable }
        | S::CheckedReference { inner, mutable }
        | S::UnsafeReference { inner, mutable } => {
            if preserve_refs {
                let tier = match ty.kind {
                    S::CheckedReference { .. } => CbgrTier::Tier1,
                    S::UnsafeReference { .. } => CbgrTier::Tier2,
                    _ => CbgrTier::Tier0,
                };
                single(
                    ProjectionShape::Reference(
                        if *mutable {
                            Mutability::Mutable
                        } else {
                            Mutability::Immutable
                        },
                        tier,
                    ),
                    child(inner)?,
                )
            } else if matches!(ty.kind, S::UnsafeReference { .. }) {
                None // layout is a machine pointer; declaration_type retains the binder
            } else {
                child(inner)?
            }
        }
        S::Tuple(elements) => projection_node(
            ProjectionShape::Tuple,
            elements.iter().map(child).collect::<Result<_, _>>()?,
        ),
        S::Slice(inner) => single(ProjectionShape::Slice, child(inner)?),
        S::Array { element, size } => {
            let inferred = size.as_deref().is_some_and(|expr| {
                let mut expr = expr;
                while let verum_ast::ExprKind::Paren(inner) = &expr.kind { expr = inner; }
                matches!(&expr.kind,
                    verum_ast::ExprKind::Path(path) if path.as_ident().is_some_and(|name| name.name.as_str() == "_"))
            });
            single(
                if inferred {
                    ProjectionShape::DynamicArray
                } else {
                    ProjectionShape::Array
                },
                child(element)?,
            )
        }
        S::Generic { base, args } => {
            if contains_source_projection(base, scope) {
                return Err("record field used as a type constructor has no runtime template representation".into());
            }
            let mut children = List::new();
            for arg in args {
                match arg {
                    GenericArg::Lifetime(_) => continue,
                    GenericArg::Type(ty) => children.push(child(ty)?),
                    GenericArg::Const(expression) => {
                        // Bracket syntax can still denote a type argument. This
                        // descriptor alone does not own the target's parameter
                        // roster, so it cannot choose that interpretation. An
                        // unresolved occurrence of a record binder must not
                        // disappear into a positional Generic placeholder.
                        if arg
                            .type_for_declared_slot()
                            .is_some_and(|ty| contains_source_projection(&ty, scope))
                        {
                            return Err(SourceDeclarationDiagnostic::from("ambiguous generic argument containing a record field requires declared-slot normalization").at(expression.span));
                        }
                        children.push(None);
                    }
                    GenericArg::Binding(binding)
                        if contains_source_projection(&binding.ty, scope) =>
                    {
                        return Err("record field in associated binding has no runtime template representation".into());
                    }
                    _ => children.push(None),
                }
            }
            projection_node(ProjectionShape::Generic, children)
        }
        S::Function {
            params,
            return_type,
            ..
        } => {
            let mut children = params.iter().map(child).collect::<Result<List<_>, _>>()?;
            children.push(child(return_type)?);
            projection_node(ProjectionShape::Function(None), children)
        }
        S::Rank2Function {
            type_params,
            params,
            return_type,
            calling_convention,
            where_clause,
            ..
        } => {
            let scope = shadow_record_binders(scope, type_params);
            let mut children = params
                .iter()
                .map(|ty| source_projection_pattern(ty, &scope, preserve_refs))
                .collect::<Result<List<_>, _>>()?;
            children.push(source_projection_pattern(
                return_type,
                &scope,
                preserve_refs,
            )?);
            if children.iter().any(Option::is_some)
                && (calling_convention.is_some()
                    || where_clause
                        .as_ref()
                        .is_some_and(|clause| !clause.predicates.is_empty())
                    || type_params.iter().any(|parameter| {
                        !matches!(
                        &parameter.kind,
                        GenericParamKind::Type { bounds, default, .. }
                            if bounds.is_empty() && default.is_none() && !parameter.is_implicit)
                    }))
            {
                return Err("dependent locally quantified source type has no runtime template representation".into());
            }
            let count = u16::try_from(type_params.len())
                .map_err(|_| Text::from("rank-2 binder count exceeds VBC range"))?;
            projection_node(ProjectionShape::Function(Some(count)), children)
        }
        S::AssociatedType { base, assoc } => single(
            ProjectionShape::Associated(assoc.name.clone()),
            child(base)?,
        ),
        S::Qualified {
            self_ty,
            assoc_name,
            ..
        } => single(
            ProjectionShape::Associated(assoc_name.name.clone()),
            child(self_ty)?,
        ),
        _ => {
            if contains_source_projection(ty, scope) {
                return Err(
                    "dependent record field source type has no runtime template representation"
                        .into(),
                );
            }
            None
        }
    })
}

fn projection_pattern_matches(
    pattern: Option<&ProjectionPattern>,
    ty: &TypeRef,
    owner: crate::types::TypeId,
) -> bool {
    let Some(pattern) = pattern else {
        return !ty.contains_record_field_projection();
    };
    match pattern {
        ProjectionPattern::Field(ordinal) => {
            matches!(ty, TypeRef::RecordFieldProjection { owner: actual_owner, ordinal: actual_ordinal } if owner == *actual_owner && ordinal == actual_ordinal)
        }
        ProjectionPattern::Node(shape, children) => {
            let mut actual = List::new();
            match (shape, ty) {
                (ProjectionShape::Tuple, TypeRef::Tuple(elements))
                | (ProjectionShape::Generic, TypeRef::Instantiated { args: elements, .. }) => {
                    actual.extend(elements)
                }
                (
                    ProjectionShape::Function(None),
                    TypeRef::Function {
                        params,
                        return_type,
                        ..
                    },
                ) => {
                    actual.extend(params);
                    actual.push(return_type.as_ref());
                }
                (
                    ProjectionShape::Function(Some(expected)),
                    TypeRef::Rank2Function {
                        type_param_count,
                        params,
                        return_type,
                        ..
                    },
                ) if expected == type_param_count => {
                    actual.extend(params);
                    actual.push(return_type.as_ref());
                }
                (
                    ProjectionShape::Reference(expected_mutability, expected_tier),
                    TypeRef::Reference {
                        inner,
                        mutability,
                        tier,
                    },
                ) if expected_mutability == mutability && expected_tier == tier => {
                    actual.push(inner.as_ref())
                }
                (ProjectionShape::Array, TypeRef::Array { element, .. }) => {
                    actual.push(element.as_ref())
                }
                (ProjectionShape::DynamicArray, TypeRef::Instantiated { base, args })
                    if *base == crate::types::TypeId::ARRAY =>
                {
                    actual.extend(args)
                }
                (ProjectionShape::Slice, TypeRef::Slice(inner)) => actual.push(inner.as_ref()),
                (
                    ProjectionShape::Associated(expected),
                    TypeRef::AssociatedProjection { base, qualifier: None, assoc, args },
                ) if expected.as_str() == assoc && args.is_empty() => actual.push(base.as_ref()),
                _ => return false,
            }
            children.len() == actual.len()
                && children
                    .iter()
                    .zip(actual)
                    .all(|(pattern, ty)| projection_pattern_matches(pattern.as_ref(), ty, owner))
        }
    }
}

fn validate_record_projection_correspondence(
    descriptor: &TypeDescriptor,
    fields: &[verum_ast::decl::RecordField],
) -> Result<(), SourceDeclarationDiagnostic> {
    let mut scope = verum_common::Map::new();
    for (ordinal, (actual, source)) in descriptor.fields.iter().zip(fields).enumerate() {
        let pattern = source_projection_pattern(&source.ty, &scope, false)?;
        if !projection_pattern_matches(pattern.as_ref(), &actual.type_ref, descriptor.id) {
            return Err(
                "runtime record field projection differs from its exact source binder occurrence"
                    .into(),
            );
        }
        let pattern = source_projection_pattern(&source.ty, &scope, true)?;
        match &actual.declaration_type {
            Some(ty) if !projection_pattern_matches(pattern.as_ref(), ty, descriptor.id) => return Err("declared record field projection differs from its exact source binder occurrence".into()),
            None if pattern.is_some() => return Err("dependent record field has no declaration type template".into()),
            _ => {}
        }
        if matches!(source.ty.kind, verum_ast::ty::TypeKind::Universe { .. }) {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| Text::from("record field ordinal exceeds VBC range"))?;
            scope.insert(source.name.name.clone(), ordinal);
        }
    }
    Ok(())
}

fn check_fields(
    actual: &[FieldDescriptor],
    expected: impl Iterator<Item = String>,
    strings: &StringTable,
) -> Result<(), Text> {
    let mut actual = actual.iter();
    let mut names = Set::new();
    for name in expected {
        let field = actual
            .next()
            .ok_or_else(|| Text::from("missing runtime field"))?;
        if strings.get(field.name) != Some(name.as_str()) || !names.insert(Text::from(name)) {
            return Err("runtime field name or order differs from declaration".into());
        }
    }
    if actual.next().is_some() {
        return Err("extra runtime field".into());
    }
    Ok(())
}
