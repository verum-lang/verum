//! Authorize declaration and callable contracts before value conversion.

use super::TypeChecker;
use crate::core_metadata::{TypeDescriptor, TypeDescriptorKind, VariantPayload};
use crate::{Result, Type, TypeError};
use verum_ast::{Span, Visibility, decl::{RecordField, TypeDeclBody, VariantData, Variant}};
use verum_common::{List, Maybe, Text};
use verum_modules::{ModulePath, VisibilityChecker};

#[derive(Clone, Debug)]
pub(super) struct RecordVisibility {
    owner: Text,
    type_name: Text,
    fields: List<(Text, Maybe<Visibility>)>,
}

fn declared_fields<'a>(body: &'a TypeDeclBody, variant: Option<&str>) -> Option<&'a [RecordField]> {
    match (body, variant) {
        (TypeDeclBody::Record(fields), None) => Some(fields),
        (TypeDeclBody::Variant(cases), Some(name)) => {
            let case = cases.iter().find(|case| case.name.name.as_str() == name)?;
            match &case.data {
                Some(VariantData::Record(fields)) => Some(fields),
                _ => None,
            }
        }
        _ => None,
    }
}

impl TypeChecker {
    pub(super) fn register_record_field_visibility(
        &mut self,
        name: &str,
        fields: &[verum_ast::decl::RecordField],
    ) {
        self.source_record_field_visibility.insert(
            Self::field_policy_key(&self.declared_type_key(name), None),
            RecordVisibility {
                owner: self.current_module_path.clone(),
                type_name: name.into(),
                fields: fields
                    .iter()
                    .map(|field| (field.name.name.clone(), Some(field.visibility.clone())))
                    .collect(),
            },
        );
    }

    pub(super) fn register_variant_field_visibility(
        &mut self,
        name: &str,
        variants: &[Variant],
    ) {
        let owner = self.declared_type_key(name);
        for variant in variants {
            let fields = match &variant.data {
                Some(VariantData::Record(fields)) => fields.iter().map(|field|
                    (field.name.name.clone(), Some(field.visibility.clone()))).collect(),
                // A declared unit or tuple case has no named-field policy.
                // Retain that fact instead of confusing it with absent authority.
                _ => List::new(),
            };
            self.source_record_field_visibility.insert(
                Self::field_policy_key(&owner, Some(variant.name.name.as_str())),
                RecordVisibility { owner: self.current_module_path.clone(), type_name: name.into(), fields },
            );
        }
    }

    fn field_policy_key(owner: &Text, variant: Option<&str>) -> (Text, Maybe<Text>) {
        (owner.clone(), variant.map(Text::from))
    }

    pub(super) fn scalar_representation_key(receiver: &Type) -> Option<Text> {
        let name = receiver.primitive_name()?;
        let owner = verum_common::well_known_types::WellKnownType::declaring_source_module(name)?;
        Some(Self::source_type_declaration_key(owner, name))
    }

    fn metadata_field_visibility(
        descriptor: &TypeDescriptor,
        variant: Option<&str>,
    ) -> Option<RecordVisibility> {
        let fields = match (&descriptor.kind, variant) {
            (TypeDescriptorKind::Record { fields }, None) => fields,
            (TypeDescriptorKind::Variant { cases }, Some(name)) => {
                match &cases.iter().find(|case| case.name.as_str() == name)?.payload {
                    Some(VariantPayload::Record(fields)) => fields,
                    _ => return None,
                }
            }
            _ => return None,
        };
        let owner = descriptor.origin_module_path.as_ref()
            .filter(|owner| !owner.is_empty()).unwrap_or(&descriptor.module_path);
        Some(RecordVisibility {
            owner: owner.clone(),
            type_name: descriptor.name.clone(),
            fields: fields.iter().map(|field| {
                (field.name.clone(), field.declared_visibility.clone())
            }).collect(),
        })
    }

    fn unknown_record_visibility(&self, key: &Text, member: &str, span: Span) -> TypeError {
        TypeError::VisibilityError {
            name: format!("{key}.{member}").into(),
            visibility: "of unknown declaration visibility; rebuild the declaring cog".into(),
            module_path: key
                .rsplit_once('.')
                .map(|(owner, _)| Text::from(owner))
                .unwrap_or_else(|| self.current_module_path.clone()),
            span,
            declaration_span: None,
        }
    }

    /// Preserve the nominal head through instantiated aliases and the same
    /// automatic dereferencing used by field lookup. Unknown nominal authority
    /// is a refusal; anonymous structural records have no declaration policy.
    fn declared_field_visibility(
        &self,
        receiver: &Type,
        variant: Option<&str>,
        member: &str,
        span: Span,
    ) -> Result<Option<RecordVisibility>> {
        let mut current = receiver.clone();
        let mut visited: List<Type> = List::new();
        let mut last_key = Text::from("record");
        for _ in 0..64 {
            current = self.unifier.apply(&current);
            current = self.unwrap_reference_type(&current).clone();
            // Associated types retain the nominal declaration selected by the
            // protocol implementation. Resolve it before consulting policy.
            let projected = self.resolve_assoc_projections_deep(&current);
            if projected != current {
                current = projected;
                continue;
            }
            match &current {
                Type::Ownership { inner, .. } => {
                    current = (**inner).clone();
                    continue;
                }
                Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                    current = (**base).clone();
                    continue;
                }
                _ => {}
            }
            if let Some(key) = Self::scalar_representation_key(&current) {
                if let Some(policy) = self.source_record_field_visibility.get(&Self::field_policy_key(&key, variant)) {
                    return Ok(Some(policy.clone()));
                }
                return Ok(self.core_metadata.as_ref()
                    .and_then(|metadata| Self::metadata_type_with_declaring_key(metadata, &key))
                    .and_then(|descriptor| Self::metadata_field_visibility(descriptor, variant)));
            }
            let (key, args) = match &current {
                Type::Named { path, args } => (self.path_to_string(path), args),
                Type::Generic { name, args } => (name.clone(), args),
                _ => return Ok(None),
            };
            last_key = key.clone();
            if visited.contains(&current) {
                return Err(self.unknown_record_visibility(&key, member, span));
            }
            visited.push(current.clone());
            if let Some(policy) = self.source_record_field_visibility.get(&Self::field_policy_key(&key, variant)) {
                return Ok(Some(policy.clone()));
            }
            if let Some(target) = self.ctx.type_aliases.get(&key) {
                current = self.substitute_with_params(key.as_str(), target, args);
                continue;
            }
            if !key.contains(".") {
                // A contextual constructor can retain a lexical sum name
                // while its field policy belongs to the imported declaration.
                // Resolve the registered sum owner before consulting policy;
                // the leaf spelling alone grants no construction authority.
                if variant.is_some()
                    && let Some(declared) = self.declared_sum_key(&key)
                    && declared != key
                {
                    current = Type::Named {
                        path: Self::text_to_path(&declared), args: args.clone(),
                    };
                    continue;
                }
                // A local declaration has exact stored authority even when a
                // stand-alone checker has no ModuleRegistry. Imports have their
                // declaring keys and cannot populate this local owner's entry.
                let local_key = self.declared_type_key(key.as_str());
                if let Some(policy) = self.source_record_field_visibility.get(&Self::field_policy_key(&local_key, variant)) {
                    return Ok(Some(policy.clone()));
                }
                if let Some(target @ (Type::Named { .. } | Type::Generic { .. })) =
                    self.ctx.lookup_type(key.as_str())
                    && target != &current
                {
                    current = self.substitute_with_params(key.as_str(), target, args);
                    continue;
                }
            }
            if let Some((module, declaration)) = self.source_receiver_declaration(&current) {
                if matches!(declaration.body, TypeDeclBody::Alias(_)) {
                    return Err(self.unknown_record_visibility(&key, member, span));
                }
                return Ok(declared_fields(&declaration.body, variant).map(|fields| RecordVisibility {
                    owner: module.path.to_string().into(),
                    type_name: declaration.name.name.clone(),
                    fields: fields.iter()
                        .map(|field| (field.name.name.clone(), Some(field.visibility.clone())))
                        .collect(),
                }));
            }
            if let Some(metadata) = &self.core_metadata
                && let Some(descriptor) = Self::metadata_type_with_declaring_key(metadata, &key)
            {
                if matches!(descriptor.kind, TypeDescriptorKind::Alias { .. }) {
                    return Err(self.unknown_record_visibility(&key, member, span));
                }
                return Ok(Self::metadata_field_visibility(descriptor, variant));
            }
            // A module mount changes the spelling of a nominal path, not its
            // field policy. Follow only the registered owner after exact
            // declaration lookup; a missing member still has no authority.
            if let Some((alias, member_path)) = key.split_once('.')
                && let Some(owner) = self.module_aliases.get(&Text::from(alias))
            {
                let declared: Text = format!("{owner}.{member_path}").into();
                if declared != key {
                    current = match &current {
                        Type::Named { path, args } => Type::Named {
                            path: verum_ast::ty::Path::new(
                                declared
                                    .as_str()
                                    .split('.')
                                    .map(|name| {
                                        verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                                            name, path.span,
                                        ))
                                    })
                                    .collect(),
                                path.span,
                            ),
                            args: args.clone(),
                        },
                        Type::Generic { args, .. } => Type::Generic {
                            name: declared,
                            args: args.clone(),
                        },
                        _ => unreachable!("nominal receiver selected above"),
                    };
                    continue;
                }
            }
            return Err(self.unknown_record_visibility(&key, member, span));
        }
        Err(self.unknown_record_visibility(&last_key, member, span))
    }

    pub(super) fn check_declared_member_visibility(
        &self,
        owner: &Text,
        field: &Text,
        visibility: &Maybe<Visibility>,
        span: Span,
        declaration_span: impl Fn() -> Option<Span>,
    ) -> Result<()> {
        if owner == &self.current_module_path {
            return Ok(());
        }
        let Some(visibility) = visibility else {
            return Err(TypeError::VisibilityError {
                name: field.clone(),
                visibility: "of unknown archived visibility; rebuild the declaring cog".into(),
                module_path: owner.clone(),
                span,
                declaration_span: None,
            });
        };
        let visibility = visibility
            .resolve_declared_scope(owner.as_str())
            .map_err(|message| TypeError::VisibilityError {
                name: field.clone(),
                visibility: message,
                module_path: owner.clone(),
                span,
                declaration_span: declaration_span(),
            })?;
        if VisibilityChecker::new().is_visible(
            visibility.clone(),
            &ModulePath::from_str(owner.as_str()),
            &ModulePath::from_str(self.current_module_path.as_str()),
        ) {
            Ok(())
        } else {
            Err(TypeError::VisibilityError {
                name: field.clone(),
                visibility: format!("{visibility:?}").into(),
                module_path: owner.clone(),
                span,
                declaration_span: declaration_span(),
            })
        }
    }

    fn check_field_policy(
        &self, record: &RecordVisibility, variant: Option<&str>, name: &Text,
        visibility: &Maybe<Visibility>, span: Span,
    ) -> Result<()> {
        self.check_declared_member_visibility(&record.owner, name, visibility, span, || {
            // Archived syntax can carry a FileId allocated to an unrelated editor buffer.
            // Only the selected declaration in the current source snapshot owns this location.
            let (module, declaration) = self.registered_type_declaration(&record.owner, &record.type_name)?;
            let field = declared_fields(&declaration.body, variant)?.iter().find(|field|
                field.name.name == *name && Some(&field.visibility) == visibility.as_ref())?;
            let location = field.name.span;
            let bytes = location.start as usize..location.end as usize;
            (location.file_id == module.file_id && module.source.get(bytes) == Some(name.as_str()))
                .then_some(location)
        })
    }

    /// Inspect a conversion without expanding a declared record into its field
    /// map. Only aliases, projections and reference/ownership wrappers are
    /// transparent here; declarations remain available to the policy lookup.
    pub(super) fn record_conversion_head(&self, ty: &Type) -> Type {
        self.value_conversion_head(ty, true)
    }

    fn value_conversion_head(&self, ty: &Type, dereference: bool) -> Type {
        let mut current = self.unifier.apply(ty);
        let mut visited: List<Type> = List::new();
        for _ in 0..64 {
            if visited.contains(&current) {
                break;
            }
            visited.push(current.clone());
            let projected = self.resolve_assoc_projections_deep(&current);
            if projected != current {
                current = projected;
                continue;
            }
            match &current {
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. }
                | Type::Ownership { inner, .. }
                    if dereference =>
                {
                    current = (**inner).clone();
                    continue;
                }
                Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                    current = (**base).clone();
                    continue;
                }
                _ => {}
            }
            let Some((name, args)) = self.conversion_nominal_head(&current) else {
                break;
            };
            let Some(target) = self.ctx.type_aliases.get(&name) else {
                break;
            };
            let target = self.substitute_with_params(name.as_str(), target, args);
            if target == current {
                break;
            }
            current = self.unifier.apply(&target);
        }
        current
    }

    pub(super) fn conversion_nominal_head<'a>(&self, ty: &'a Type) -> Maybe<(Text, &'a [Type])> {
        match ty {
            Type::Named { path, args } => Some((self.path_to_string(path), args.as_slice())),
            Type::Generic { name, args } => Some((name.clone(), args.as_slice())),
            _ => None,
        }
    }

    /// A conversion may expose a nominal field or strengthen a nested callable
    /// contract without a field expression or call in the source. Check both
    /// obligations before unification; success here does not establish value
    /// compatibility. The ordinary conversion relation still has to succeed.
    pub(super) fn check_value_conversion_obligations(
        &self,
        actual: &Type,
        expected: &Type,
        span: Span,
    ) -> Result<()> {
        self.check_value_conversion_inner(actual, expected, span, &mut List::new())
    }

    fn check_value_conversion_inner(
        &self,
        actual: &Type,
        expected: &Type,
        span: Span,
        visited: &mut List<(Type, Type)>,
    ) -> Result<()> {
        let actual = self.value_conversion_head(actual, false);
        let expected = self.value_conversion_head(expected, false);
        // Transferring the original nominal value grants no field access.
        if actual == expected || actual.is_never() {
            return Ok(());
        }
        let pair = (actual.clone(), expected.clone());
        if visited.contains(&pair) {
            return Ok(());
        }
        if visited.len() >= 64 {
            return Err(TypeError::OtherWithCode {
                code: "E103".into(),
                msg: "Value conversion obligations exceed the supported nesting depth".into(),
            });
        }
        visited.push(pair);
        let result = (|| {
            // Generic arguments may contain the conversion even when the
            // containing declaration stays the same (Carrier<Secret>).
            if let (Some((from, from_args)), Some((to, to_args))) = (
                self.conversion_nominal_head(&actual),
                self.conversion_nominal_head(&expected),
            ) {
                if from == to && from_args.len() == to_args.len() {
                    for (from, to) in from_args.iter().zip(to_args.iter()) {
                        self.check_value_conversion_inner(from, to, span, visited)?;
                    }
                }
                // Different nominal declarations still use ordinary nominal
                // compatibility; equal record shapes do not identify owners.
                return Ok(());
            }
            match (&actual, &expected) {
                (
                    Type::Reference { inner: from, .. }
                    | Type::CheckedReference { inner: from, .. }
                    | Type::UnsafeReference { inner: from, .. }
                    | Type::Ownership { inner: from, .. }
                    | Type::Pointer { inner: from, .. }
                    | Type::VolatilePointer { inner: from, .. },
                    Type::Reference { inner: to, mutable }
                    | Type::CheckedReference { inner: to, mutable }
                    | Type::UnsafeReference { inner: to, mutable }
                    | Type::Ownership { inner: to, mutable }
                    | Type::Pointer { inner: to, mutable }
                    | Type::VolatilePointer { inner: to, mutable },
                ) => {
                    self.check_value_conversion_inner(from, to, span, visited)?;
                    if *mutable {
                        // Writes through the converted reference travel in the
                        // opposite direction. They must preserve contracts too.
                        self.check_value_conversion_inner(to, from, span, visited)?;
                    }
                }
                (
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. }
                    | Type::Ownership { inner, .. },
                    to,
                ) => {
                    self.check_resource_copy(to, span)?;
                    self.check_value_conversion_inner(inner, to, span, visited)?;
                }
                (
                    from,
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. }
                    | Type::Ownership { inner, .. },
                ) => {
                    self.check_value_conversion_inner(from, inner, span, visited)?;
                }
                (Type::Named { .. } | Type::Generic { .. }, Type::Record(fields)) => {
                    for name in fields.keys() {
                        self.check_record_field_visibility(&actual, name.as_str(), span)?;
                    }
                    if self.is_dependent_record(&actual) {
                        return Err(TypeError::Other("dependent record cannot be erased to an independent structural field map".into()));
                    }
                    if let Some(from_fields) = self.resolve_to_record_type(&actual) {
                        for (name, to) in fields {
                            if let Some(from) = from_fields.get(name) {
                                self.check_value_conversion_inner(from, to, span, visited)?;
                            }
                        }
                    }
                }
                (Type::Record(fields), Type::Named { .. } | Type::Generic { .. }) => {
                    self.check_record_construction_visibility(&expected, span)?;
                    if self.is_dependent_record(&expected) {
                        return Err(TypeError::Other(
                            "dependent record requires its checked nominal constructor".into(),
                        ));
                    }
                    if let Some(to_fields) = self.resolve_to_record_type(&expected) {
                        for (name, to) in &to_fields {
                            if let Some(from) = fields.get(name) {
                                self.check_value_conversion_inner(from, to, span, visited)?;
                            }
                        }
                    }
                }
                (
                    Type::Record(from_fields)
                    | Type::ExtensibleRecord {
                        fields: from_fields,
                        ..
                    },
                    Type::Record(to_fields)
                    | Type::ExtensibleRecord {
                        fields: to_fields, ..
                    },
                )
                | (Type::Variant(from_fields), Type::Variant(to_fields)) => {
                    for (name, to) in to_fields {
                        if let Some(from) = from_fields.get(name) {
                            self.check_value_conversion_inner(from, to, span, visited)?;
                        }
                    }
                }
                (Type::Tuple(from), Type::Tuple(to)) if from.len() == to.len() => {
                    for (from, to) in from.iter().zip(to.iter()) {
                        self.check_value_conversion_inner(from, to, span, visited)?;
                    }
                }
                (
                    Type::Function {
                        params: from_params,
                        return_type: from_return,
                        ..
                    },
                    Type::Function {
                        params: to_params,
                        return_type: to_return,
                        ..
                    },
                ) if from_params.len() == to_params.len() => {
                    self.check_callable_header(&actual, &expected, span)?;
                    // The converted callable receives values promised by the
                    // target signature; those flow into the original parameters.
                    for (from, to) in from_params.iter().zip(to_params.iter()) {
                        self.check_value_conversion_inner(to, from, span, visited)?;
                    }
                    self.check_value_conversion_inner(from_return, to_return, span, visited)?;
                }
                (Type::Future { output: from }, Type::Future { output: to })
                | (Type::GenRef { inner: from }, Type::GenRef { inner: to })
                | (Type::Tensor { element: from, .. }, Type::Tensor { element: to, .. })
                | (Type::Quantified { inner: from, .. }, Type::Quantified { inner: to, .. }) => {
                    self.check_value_conversion_inner(from, to, span, visited)?;
                }
                (
                    Type::Generator {
                        yield_ty: from_yield,
                        return_ty: from_return,
                    },
                    Type::Generator {
                        yield_ty: to_yield,
                        return_ty: to_return,
                    },
                ) => {
                    self.check_value_conversion_inner(from_yield, to_yield, span, visited)?;
                    self.check_value_conversion_inner(from_return, to_return, span, visited)?;
                }
                (
                    Type::TypeApp {
                        constructor: from,
                        args: from_args,
                    },
                    Type::TypeApp {
                        constructor: to,
                        args: to_args,
                    },
                ) => {
                    self.check_value_conversion_inner(from, to, span, visited)?;
                    for (from, to) in from_args.iter().zip(to_args) {
                        self.check_value_conversion_inner(from, to, span, visited)?;
                    }
                }
                (
                    Type::Forall {
                        vars: from_vars,
                        body: from,
                    },
                    Type::Forall {
                        vars: to_vars,
                        body: to,
                    },
                ) if from_vars.len() == to_vars.len() => {
                    let mut alpha = crate::Substitution::new();
                    for (from, to) in from_vars.iter().zip(to_vars) {
                        alpha.insert(*to, Type::Var(*from));
                    }
                    let to = to
                        .try_apply_subst(&alpha)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?;
                    self.check_value_conversion_inner(from, &to, span, visited)?;
                }
                (
                    Type::Array { element: from, .. } | Type::Slice { element: from },
                    Type::Array { element: to, .. } | Type::Slice { element: to },
                ) => {
                    self.check_value_conversion_inner(from, to, span, visited)?;
                }
                (Type::Array { element: from, .. } | Type::Slice { element: from }, collection) => {
                    if let Some(to) = self.unifier.array_coercible_element(collection) {
                        self.check_value_conversion_inner(from, to, span, visited)?;
                    }
                }
                (collection, Type::Array { element: to, .. } | Type::Slice { element: to }) => {
                    if let Some(from) = self.unifier.array_coercible_element(collection) {
                        self.check_value_conversion_inner(from, to, span, visited)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        visited.pop();
        result
    }

    /// A nominal record declaration outranks an ambient case with the same
    /// spelling. Payload field maps and their shape cannot establish this.
    pub(super) fn has_declared_record_field_policy(&self, name: &str, span: Span) -> bool {
        let receiver = Type::Named { path: Self::text_to_path(&Text::from(name)), args: List::new() };
        self.declared_field_visibility(&receiver, None, "record construction", span)
            .is_ok_and(|policy| policy.is_some())
    }

    pub(super) fn check_record_field_visibility(
        &self,
        receiver: &Type,
        field: &str,
        span: Span,
    ) -> Result<()> {
        if let Some(record) = self.declared_field_visibility(receiver, None, field, span)?
            && let Some((name, visibility)) = record
                .fields
                .iter()
                .find(|(name, _)| name.as_str() == field)
        {
            self.check_field_policy(&record, None, name, visibility, span)?;
        }
        Ok(())
    }

    /// The selected nominal sum owns each named payload field. Pattern
    /// rest syntax does not access omitted fields; an explicit `_` still does.
    pub(super) fn check_variant_field_visibility(
        &self, receiver: &Type, variant: &str, field: &str, span: Span,
    ) -> Result<()> {
        if let Some(record) = self.declared_field_visibility(receiver, Some(variant), field, span)?
            && let Some((name, visibility)) = record.fields.iter().find(|(name, _)| name.as_str() == field)
        {
            self.check_field_policy(&record, Some(variant), name, visibility, span)?;
        }
        Ok(())
    }

    /// Both named and positional construction supply the whole payload.
    pub(super) fn check_variant_construction_visibility(
        &self, receiver: &Type, variant: &str, span: Span,
    ) -> Result<()> {
        if let Some(record) = self.declared_field_visibility(receiver, Some(variant), "variant construction", span)? {
            for (name, visibility) in &record.fields {
                self.check_field_policy(&record, Some(variant), name, visibility, span)?;
            }
        }
        Ok(())
    }

    /// Creating a complete record also supplies the fields copied from a
    /// spread or supplied by defaults. Every declared field needs authority.
    pub(super) fn check_record_construction_visibility(
        &self,
        receiver: &Type,
        span: Span,
    ) -> Result<()> {
        if let Some(record) = self.declared_field_visibility(receiver, None, "record construction", span)? {
            for (name, visibility) in &record.fields {
                self.check_field_policy(&record, None, name, visibility, span)?;
            }
        }
        Ok(())
    }
}
