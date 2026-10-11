//! Authorize record fields before nominal declarations become structural maps.

use super::TypeChecker;
use crate::core_metadata::TypeDescriptorKind;
use crate::{Result, Type, TypeError};
use verum_ast::{Span, Visibility, decl::TypeDeclBody};
use verum_common::{List, Maybe, Text};
use verum_modules::{ModulePath, VisibilityChecker};

#[derive(Clone, Debug)]
pub(super) struct RecordVisibility {
    owner: Text,
    fields: List<(Text, Maybe<Visibility>)>,
}

impl TypeChecker {
    pub(super) fn register_record_field_visibility(
        &mut self,
        name: &str,
        fields: &[verum_ast::decl::RecordField],
    ) {
        self.source_record_field_visibility.insert(
            self.declared_type_key(name),
            RecordVisibility {
                owner: self.current_module_path.clone(),
                fields: fields
                    .iter()
                    .map(|field| (field.name.name.clone(), Some(field.visibility.clone())))
                    .collect(),
            },
        );
    }

    fn unknown_record_visibility(&self, key: &Text, member: &str, span: Span) -> TypeError {
        TypeError::VisibilityError {
            name: member.into(),
            visibility: "of unknown declaration visibility; rebuild the declaring cog".into(),
            module_path: key
                .rsplit_once('.')
                .map(|(owner, _)| Text::from(owner))
                .unwrap_or_else(|| self.current_module_path.clone()),
            span,
        }
    }

    /// Preserve the nominal head through instantiated aliases and the same
    /// automatic dereferencing used by field lookup. Unknown nominal authority
    /// is a refusal; anonymous structural records have no declaration policy.
    fn record_visibility(
        &self,
        receiver: &Type,
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
            if let Some(policy) = self.source_record_field_visibility.get(&key) {
                return Ok(Some(policy.clone()));
            }
            if let Some(target) = self.ctx.type_aliases.get(&key) {
                current = self.substitute_with_params(key.as_str(), target, args);
                continue;
            }
            if !key.contains(".") {
                // A local declaration has exact stored authority even when a
                // stand-alone checker has no ModuleRegistry. Imports have their
                // declaring keys and cannot populate this local owner's entry.
                let local_key = self.declared_type_key(key.as_str());
                if let Some(policy) = self.source_record_field_visibility.get(&local_key) {
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
                match declaration.body {
                    TypeDeclBody::Record(fields) => {
                        return Ok(Some(RecordVisibility {
                            owner: module.path.to_string().into(),
                            fields: fields
                                .into_iter()
                                .map(|field| (field.name.name, Some(field.visibility)))
                                .collect(),
                        }));
                    }
                    TypeDeclBody::Alias(_) => {
                        return Err(self.unknown_record_visibility(&key, member, span));
                    }
                    _ => return Ok(None),
                }
            }
            if let Some(metadata) = &self.core_metadata
                && let Some(descriptor) = Self::metadata_type_with_declaring_key(metadata, &key)
            {
                match &descriptor.kind {
                    TypeDescriptorKind::Record { fields } => {
                        let owner = descriptor
                            .origin_module_path
                            .as_ref()
                            .filter(|owner| !owner.is_empty())
                            .unwrap_or(&descriptor.module_path);
                        return Ok(Some(RecordVisibility {
                            owner: owner.clone(),
                            fields: fields
                                .iter()
                                .map(|field| {
                                    (field.name.clone(), field.declared_visibility.clone())
                                })
                                .collect(),
                        }));
                    }
                    TypeDescriptorKind::Alias { .. } => {
                        return Err(self.unknown_record_visibility(&key, member, span));
                    }
                    _ => return Ok(None),
                }
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

    fn check_declared_field_visibility(
        &self,
        owner: &Text,
        field: &Text,
        visibility: &Maybe<Visibility>,
        span: Span,
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
            });
        };
        let visibility = visibility
            .resolve_declared_scope(owner.as_str())
            .map_err(|message| TypeError::VisibilityError {
                name: field.clone(),
                visibility: message,
                module_path: owner.clone(),
                span,
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
            })
        }
    }

    /// Inspect a conversion without expanding a declared record into its field
    /// map. Only aliases, projections and reference/ownership wrappers are
    /// transparent here; declarations remain available to the policy lookup.
    fn record_conversion_head(&self, ty: &Type) -> Type {
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
                | Type::Ownership { inner, .. } => {
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

    fn conversion_nominal_head<'a>(&self, ty: &'a Type) -> Maybe<(Text, &'a [Type])> {
        match ty {
            Type::Named { path, args } => Some((self.path_to_string(path), args.as_slice())),
            Type::Generic { name, args } => Some((name.clone(), args.as_slice())),
            _ => None,
        }
    }

    /// A type conversion can expose fields or construct a nominal value even
    /// when no field expression or record literal appears in the source.
    /// Authorize that change before unification forgets the declaration.
    pub(super) fn check_record_conversion_visibility(
        &self,
        actual: &Type,
        expected: &Type,
        span: Span,
    ) -> Result<()> {
        self.check_record_conversion_inner(actual, expected, span, &mut List::new())
    }

    fn check_record_conversion_inner(
        &self,
        actual: &Type,
        expected: &Type,
        span: Span,
        visited: &mut List<(Type, Type)>,
    ) -> Result<()> {
        let actual = self.record_conversion_head(actual);
        let expected = self.record_conversion_head(expected);
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
                msg: "Record conversion visibility exceeds the supported nesting depth".into(),
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
                        self.check_record_conversion_inner(from, to, span, visited)?;
                    }
                }
                // Different nominal declarations still use ordinary nominal
                // compatibility; equal record shapes do not identify owners.
                return Ok(());
            }
            match (&actual, &expected) {
                (Type::Named { .. } | Type::Generic { .. }, Type::Record(fields)) => {
                    for name in fields.keys() {
                        self.check_record_field_visibility(&actual, name.as_str(), span)?;
                    }
                    if let Some(from_fields) = self.resolve_to_record_type(&actual) {
                        for (name, to) in fields {
                            if let Some(from) = from_fields.get(name) {
                                self.check_record_conversion_inner(from, to, span, visited)?;
                            }
                        }
                    }
                }
                (Type::Record(fields), Type::Named { .. } | Type::Generic { .. }) => {
                    self.check_record_construction_visibility(&expected, span)?;
                    if let Some(to_fields) = self.resolve_to_record_type(&expected) {
                        for (name, to) in &to_fields {
                            if let Some(from) = fields.get(name) {
                                self.check_record_conversion_inner(from, to, span, visited)?;
                            }
                        }
                    }
                }
                (Type::Record(from_fields), Type::Record(to_fields)) => {
                    for (name, to) in to_fields {
                        if let Some(from) = from_fields.get(name) {
                            self.check_record_conversion_inner(from, to, span, visited)?;
                        }
                    }
                }
                (Type::Tuple(from), Type::Tuple(to)) if from.len() == to.len() => {
                    for (from, to) in from.iter().zip(to.iter()) {
                        self.check_record_conversion_inner(from, to, span, visited)?;
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
                    // The converted callable receives values promised by the
                    // target signature; those flow into the original parameters.
                    for (from, to) in from_params.iter().zip(to_params.iter()) {
                        self.check_record_conversion_inner(to, from, span, visited)?;
                    }
                    self.check_record_conversion_inner(from_return, to_return, span, visited)?;
                }
                (
                    Type::Array { element: from, .. } | Type::Slice { element: from },
                    Type::Array { element: to, .. } | Type::Slice { element: to },
                ) => {
                    self.check_record_conversion_inner(from, to, span, visited)?;
                }
                (
                    Type::Array { element: from, .. } | Type::Slice { element: from },
                    collection,
                ) => {
                    if let Some(to) = self.unifier.array_coercible_element(collection) {
                        self.check_record_conversion_inner(from, to, span, visited)?;
                    }
                }
                (
                    collection,
                    Type::Array { element: to, .. } | Type::Slice { element: to },
                ) => {
                    if let Some(from) = self.unifier.array_coercible_element(collection) {
                        self.check_record_conversion_inner(from, to, span, visited)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        visited.pop();
        result
    }

    pub(super) fn check_record_field_visibility(
        &self,
        receiver: &Type,
        field: &str,
        span: Span,
    ) -> Result<()> {
        if let Some(record) = self.record_visibility(receiver, field, span)?
            && let Some((name, visibility)) = record
                .fields
                .iter()
                .find(|(name, _)| name.as_str() == field)
        {
            self.check_declared_field_visibility(&record.owner, name, visibility, span)?;
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
        if let Some(record) = self.record_visibility(receiver, "record construction", span)? {
            for (name, visibility) in &record.fields {
                self.check_declared_field_visibility(&record.owner, name, visibility, span)?;
            }
        }
        Ok(())
    }
}
