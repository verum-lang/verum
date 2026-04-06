//! Pattern-related methods extracted from `infer.rs`.
//!
//! Contains 12 `TypeChecker` methods responsible for pattern binding,
//! compound destructuring, variant payload matching, active pattern
//! variable binding, exhaustive pattern collection, and capture analysis.

use crate::context::TypeScheme;
use crate::infer::{GlobalDepthGuard, TypeChecker, span_to_line_col};
use crate::ty::{Type, TypeVar};
use crate::{Result, TypeError};
use verum_ast::pattern::Pattern;
use verum_ast::span::Span;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Set, Text, ToText};

impl TypeChecker {
    /// Check compound destructuring assignment: (x, y) += (dx, dy)
    ///
    /// For compound destructuring, all pattern variables must already exist and be mutable.
    /// This function validates:
    /// 1. Pattern structure matches value structure (tuple with tuple, array with array)
    /// 2. Each element supports the compound operation
    /// 3. All pattern identifiers refer to existing mutable variables
    ///
    /// Unified destructuring: consistent pattern syntax for let bindings, match arms, and function parameters
    pub(crate) fn check_compound_destructuring_pattern(
        &mut self,
        pattern: &Pattern,
        value_ty: &Type,
        op: &verum_ast::BinOp,
        span: verum_ast::span::Span,
    ) -> Result<()> {
        use verum_ast::pattern::PatternKind;
        use verum_ast::BinOp;

        match &pattern.kind {
            PatternKind::Tuple(patterns) => {
                // Value must also be a tuple with matching arity
                match value_ty {
                    Type::Tuple(value_elements) => {
                        if patterns.len() != value_elements.len() {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "compound destructuring tuple has {} elements but value has {}",
                                patterns.len(),
                                value_elements.len()
                            ))));
                        }
                        // Check each element
                        for (pat, val_ty) in patterns.iter().zip(value_elements.iter()) {
                            self.check_compound_destructuring_pattern(pat, val_ty, op, span)?;
                        }
                        Ok(())
                    }
                    Type::Var(_v) => {
                        // Create a tuple type with fresh vars and unify
                        let elem_vars: verum_common::List<Type> = patterns
                            .iter()
                            .map(|_| Type::Var(crate::ty::TypeVar::fresh()))
                            .collect();
                        let tuple_ty = Type::Tuple(elem_vars.clone());
                        self.unifier.unify(value_ty, &tuple_ty, span)?;
                        // Now check each element
                        for (pat, val_ty) in patterns.iter().zip(elem_vars.iter()) {
                            self.check_compound_destructuring_pattern(pat, val_ty, op, span)?;
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "compound destructuring tuple pattern requires tuple value, got {:?}",
                        value_ty
                    )))),
                }
            }

            PatternKind::Array(patterns) => {
                // Value must be an array with matching element type
                let elem_ty = match value_ty {
                    Type::Array { element, .. } => element.as_ref().clone(),
                    Type::Slice { element } => element.as_ref().clone(),
                    Type::Var(_) => {
                        // Create array type with fresh element var
                        let elem_var = Type::Var(crate::ty::TypeVar::fresh());
                        let array_ty = Type::Array {
                            element: Box::new(elem_var.clone()),
                            size: Some(patterns.len()),
                        };
                        self.unifier.unify(value_ty, &array_ty, span)?;
                        elem_var
                    }
                    // Handle collection types via element_type_of (protocol-based)
                    other => {
                        if let Some(elem) = self.element_type_of(other) {
                            elem
                        } else {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "compound destructuring array pattern requires array value, got {:?}",
                                value_ty
                            ))));
                        }
                    }
                };
                // Check each element pattern
                for pat in patterns.iter() {
                    self.check_compound_destructuring_pattern(pat, &elem_ty, op, span)?;
                }
                Ok(())
            }

            PatternKind::Ident { name, .. } => {
                // The variable must already exist
                // Clone the scheme to avoid borrow conflict with subsequent mutable operations
                let var_scheme = self.ctx.env.lookup(name.name.as_str()).cloned();
                match var_scheme {
                    Some(scheme) => {
                        // Check the compound operation is valid for this type
                        self.check_compound_op_valid(&scheme.ty, value_ty, op, span)
                    }
                    None => Err(TypeError::Other(verum_common::Text::from(format!(
                        "variable '{}' must be declared before compound destructuring assignment",
                        name.name
                    )))),
                }
            }

            PatternKind::Paren(inner) => {
                self.check_compound_destructuring_pattern(inner, value_ty, op, span)
            }

            PatternKind::Wildcard => {
                // Wildcard ignores the element, no operation needed
                Ok(())
            }

            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "compound destructuring not supported for pattern kind: {:?}",
                pattern.kind
            )))),
        }
    }

    /// Check if a compound operation is valid for the given types.
    pub(crate) fn check_compound_op_valid(
        &mut self,
        var_ty: &Type,
        value_ty: &Type,
        op: &verum_ast::BinOp,
        span: verum_ast::span::Span,
    ) -> Result<()> {
        use verum_ast::BinOp;

        // The value type should unify with the variable type
        self.unifier.unify(var_ty, value_ty, span)?;

        // Check the operation is valid for the resolved type
        let resolved = self.unifier.apply(var_ty);
        match op {
            BinOp::AddAssign => {
                // Valid for Int, Float, Text
                match &resolved {
                    Type::Int | Type::Float | Type::Text => Ok(()),
                    Type::Var(_) => Ok(()), // Will be resolved later
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "+= not supported for type {:?}",
                        resolved
                    )))),
                }
            }
            BinOp::SubAssign | BinOp::MulAssign | BinOp::DivAssign | BinOp::RemAssign => {
                // Valid for Int, Float
                match &resolved {
                    Type::Int | Type::Float => Ok(()),
                    Type::Var(_) => Ok(()),
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "{:?} not supported for type {:?}",
                        op, resolved
                    )))),
                }
            }
            BinOp::BitAndAssign | BinOp::BitOrAssign | BinOp::BitXorAssign => {
                // Valid for Int
                match &resolved {
                    Type::Int => Ok(()),
                    Type::Var(_) => Ok(()),
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "{:?} not supported for type {:?}",
                        op, resolved
                    )))),
                }
            }
            BinOp::ShlAssign | BinOp::ShrAssign => {
                // Valid for Int
                match &resolved {
                    Type::Int => Ok(()),
                    Type::Var(_) => Ok(()),
                    _ => Err(TypeError::Other(verum_common::Text::from(format!(
                        "{:?} not supported for type {:?}",
                        op, resolved
                    )))),
                }
            }
            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "unsupported compound operator: {:?}",
                op
            )))),
        }
    }

    /// Bind a pattern to a type.
    pub(crate) fn bind_pattern(&mut self, pattern: &Pattern, ty: &Type) -> Result<()> {
        self.bind_pattern_scheme(pattern, TypeScheme::mono(ty.clone()))
    }

    /// Bind a pattern to a type scheme.
    /// Recursively extracts variable bindings from active pattern bindings,
    /// assigning Unknown type to each. This is used because active pattern
    /// return type lookup is not yet implemented in the type environment.
    pub(crate) fn bind_active_pattern_variables(&mut self, pattern: &Pattern) -> Result<()> {
        use verum_ast::pattern::PatternKind;
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Rest => Ok(()),
            PatternKind::Literal(_) => Ok(()),
            PatternKind::Ident { name, subpattern, .. } => {
                self.ctx.env.insert(name.name.as_str(), TypeScheme::mono(Type::Unknown));
                if let verum_common::Maybe::Some(sub) = subpattern {
                    self.bind_active_pattern_variables(sub)?;
                }
                Ok(())
            }
            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(variant_data) = data {
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(pats) => {
                            for p in pats.iter() {
                                self.bind_active_pattern_variables(p)?;
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for f in fields.iter() {
                                if let Some(ref pat) = f.pattern {
                                    self.bind_active_pattern_variables(pat)?;
                                } else {
                                    // Shorthand field pattern — bind field name as variable
                                    self.ctx.env.insert(f.name.name.as_str(), TypeScheme::mono(Type::Unknown));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            PatternKind::Tuple(pats) => {
                for p in pats.iter() {
                    self.bind_active_pattern_variables(p)?;
                }
                Ok(())
            }
            PatternKind::Active { bindings, .. } => {
                for b in bindings.iter() {
                    self.bind_active_pattern_variables(b)?;
                }
                Ok(())
            }
            PatternKind::Or(pats) => {
                for p in pats.iter() {
                    self.bind_active_pattern_variables(p)?;
                }
                Ok(())
            }
            PatternKind::And(pats) => {
                for p in pats.iter() {
                    self.bind_active_pattern_variables(p)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn bind_pattern_scheme(&mut self, pattern: &Pattern, scheme: TypeScheme) -> Result<()> {
        let _global_guard = GlobalDepthGuard::enter()?;
        use verum_ast::pattern::PatternKind;

        #[cfg(debug_assertions)]
        if let PatternKind::Variant { path, .. } = &pattern.kind {
            let tag_name = path.segments.last().map(|s| match s {
                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                _ => "?",
            }).unwrap_or("?");
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG bind_pattern] Variant pattern '{}' with scheme.ty: {}", tag_name, scheme.ty);
        }

        match &pattern.kind {
            PatternKind::Wildcard => Ok(()),

            PatternKind::Ident {
                name, subpattern, by_ref, mutable,
            } => {
                // If the pattern uses 'ref' or 'ref mut', wrap the type in a reference
                // e.g., `Some(ref x)` binds x to &T, `Some(ref mut x)` binds x to &mut T
                let binding_scheme = if *by_ref {
                    TypeScheme {
                        ty: Type::Reference {
                            inner: Box::new(scheme.ty.clone()),
                            mutable: *mutable,
                        },
                        ..scheme.clone()
                    }
                } else {
                    scheme.clone()
                };
                self.ctx.env.insert(name.name.as_str(), binding_scheme.clone());
                // Register affine bindings for move semantics enforcement.
                // CRITICAL: ref bindings (`ref x`, `ref mut x`) are borrows, NOT ownership
                // transfers. They can be used multiple times without being consumed.
                // Only register affine tracking for non-ref (owning) bindings.
                if !*by_ref {
                    let contains_affine = self.type_contains_affine(&scheme.ty);
                    if contains_affine {
                        self.affine_tracker.bind_container(
                            name.name.as_str(),
                            scheme.ty.clone(),
                            pattern.span,
                        );
                    } else {
                        self.affine_tracker.bind(
                            name.name.as_str(),
                            scheme.ty.clone(),
                            pattern.span,
                        );
                    }
                }
                if let Some(sub) = subpattern {
                    self.bind_pattern_scheme(sub, scheme)?;
                }
                Ok(())
            }

            PatternKind::Literal(lit) => {
                // Literal patterns must match the type
                let lit_ty = self.infer_literal(lit);
                self.unifier.unify(&lit_ty, &scheme.ty, pattern.span)?;
                Ok(())
            }

            PatternKind::Tuple(patterns) => {
                // Auto-deref through references to find underlying tuple type
                // This handles for loops like `for (a, b) in items.iter()` where
                // the iterator yields &(T, U) but the pattern expects (T, U)
                //
                // When destructuring through a reference, each element gets the
                // reference wrapper applied. So &(T, U) -> pattern (a, b) gives
                // a: &T and b: &U (not T and U directly).
                let (tuple_ty, ref_wrapper): (Type, Option<Box<dyn Fn(Type) -> Type>>) =
                    match &scheme.ty {
                        Type::Tuple(_) => (scheme.ty.clone(), None),
                        Type::Reference { inner, mutable } => {
                            let m = *mutable;
                            (
                                *inner.clone(),
                                Some(Box::new(move |t| Type::Reference {
                                    inner: Box::new(t),
                                    mutable: m,
                                })),
                            )
                        }
                        Type::CheckedReference { inner, mutable } => {
                            let m = *mutable;
                            (
                                *inner.clone(),
                                Some(Box::new(move |t| Type::CheckedReference {
                                    inner: Box::new(t),
                                    mutable: m,
                                })),
                            )
                        }
                        Type::UnsafeReference { inner, mutable } => {
                            let m = *mutable;
                            (
                                *inner.clone(),
                                Some(Box::new(move |t| Type::UnsafeReference {
                                    inner: Box::new(t),
                                    mutable: m,
                                })),
                            )
                        }
                        other => (other.clone(), None),
                    };

                // Resolve type variables and named type aliases to find underlying tuple type
                let tuple_ty = {
                    // First, apply current substitution to resolve type variables
                    let resolved = self.unifier.apply(&tuple_ty);
                    match &resolved {
                        Type::Tuple(_) => resolved,
                        // Resolve Named/Generic type aliases that may resolve to tuples
                        Type::Named { path, .. } => {
                            let name: Option<&str> = path.as_ident().map(|id| id.as_str())
                                .or_else(|| path.segments.last().and_then(|s| match s {
                                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                                    _ => None,
                                }));
                            if let Some(n) = name {
                                // First check __tuple_fields_<Name> for named tuple types
                                // e.g., `type Rgb is (Byte, Byte, Byte)` stores fields at __tuple_fields_Rgb
                                let tuple_key = format!("__tuple_fields_{}", n);
                                if let Some(Type::Tuple(tup_fields)) = self.ctx.lookup_type(&tuple_key) {
                                    Type::Tuple(tup_fields.clone())
                                } else if let Some(resolved_ty) = self.ctx.lookup_type(n) {
                                    match resolved_ty {
                                        Type::Tuple(_) => resolved_ty.clone(),
                                        _ => {
                                            // Also try normalizing to resolve deeper aliases
                                            let normalized = self.normalize_type(&resolved);
                                            if matches!(&normalized, Type::Tuple(_)) {
                                                normalized
                                            } else {
                                                // Also check unifier type alias resolution
                                                let alias_resolved = self.unifier.apply(&resolved);
                                                if matches!(&alias_resolved, Type::Tuple(_)) {
                                                    alias_resolved
                                                } else if let Some(expanded) = self.unifier.try_expand_alias(&resolved) {
                                                    if matches!(&expanded, Type::Tuple(_)) {
                                                        expanded
                                                    } else {
                                                        resolved
                                                    }
                                                } else {
                                                    resolved
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Try normalizing even if lookup_type fails
                                    let normalized = self.normalize_type(&resolved);
                                    if matches!(&normalized, Type::Tuple(_)) {
                                        normalized
                                    } else {
                                        // Also check unifier type alias resolution
                                        let alias_resolved = self.unifier.apply(&resolved);
                                        if matches!(&alias_resolved, Type::Tuple(_)) {
                                            alias_resolved
                                        } else if let Some(expanded) = self.unifier.try_expand_alias(&resolved) {
                                            if matches!(&expanded, Type::Tuple(_)) {
                                                expanded
                                            } else {
                                                resolved
                                            }
                                        } else {
                                            resolved
                                        }
                                    }
                                }
                            } else {
                                resolved
                            }
                        }
                        Type::Generic { name, .. } => {
                            if let Some(resolved_ty) = self.ctx.lookup_type(name.as_str()) {
                                match resolved_ty {
                                    Type::Tuple(_) => resolved_ty.clone(),
                                    _ => {
                                        let normalized = self.normalize_type(&resolved);
                                        if matches!(&normalized, Type::Tuple(_)) {
                                            normalized
                                        } else {
                                            resolved
                                        }
                                    }
                                }
                            } else {
                                let normalized = self.normalize_type(&resolved);
                                if matches!(&normalized, Type::Tuple(_)) {
                                    normalized
                                } else {
                                    resolved
                                }
                            }
                        }
                        // For unresolved type variables or Unknown types, create fresh tuple
                        // type variables matching the pattern arity so type inference can proceed
                        Type::Var(_) | Type::Unknown => {
                            let fresh_types: verum_common::List<Type> = patterns.iter()
                                .filter(|p| !matches!(p.kind, PatternKind::Rest))
                                .map(|_| Type::Var(crate::ty::TypeVar::fresh()))
                                .collect();
                            let tuple = Type::Tuple(fresh_types);
                            // Unify the variable with the fresh tuple so downstream inference works
                            let _ = self.unifier.unify(&resolved, &tuple, pattern.span);
                            tuple
                        }
                        _ => {
                            // Also try normalizing unknown types to find underlying tuples
                            let normalized = self.normalize_type(&resolved);
                            if matches!(&normalized, Type::Tuple(_)) {
                                normalized
                            } else {
                                resolved
                            }
                        }
                    }
                };

                if let Type::Tuple(types) = &tuple_ty {
                    // Check if the pattern contains a rest pattern (..)
                    let has_rest = patterns.iter().any(|p| matches!(p.kind, PatternKind::Rest));

                    if has_rest {
                        // With rest pattern, we bind the non-rest patterns positionally.
                        // Patterns before `..` bind to the front of the tuple,
                        // patterns after `..` bind to the end.
                        let rest_idx = patterns.iter().position(|p| matches!(p.kind, PatternKind::Rest)).unwrap_or(0);
                        let pat_slice: &[verum_ast::Pattern] = patterns.as_slice();
                        let before = &pat_slice[..rest_idx];
                        let after = &pat_slice[rest_idx + 1..];
                        let non_rest_count = before.len() + after.len();

                        if non_rest_count > types.len() {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Pattern length mismatch: tuple has {} elements, but pattern requires at least {}",
                                types.len(),
                                non_rest_count
                            ))));
                        }

                        // Bind front patterns
                        for (pat, ty) in before.iter().zip(types.iter()) {
                            let elem_ty = match &ref_wrapper {
                                Some(wrap) => wrap(ty.clone()),
                                None => ty.clone(),
                            };
                            self.bind_pattern(pat, &elem_ty)?;
                        }

                        // Bind back patterns from the end of the tuple
                        for (pat, ty) in after.iter().zip(types.iter().skip(types.len() - after.len())) {
                            let elem_ty = match &ref_wrapper {
                                Some(wrap) => wrap(ty.clone()),
                                None => ty.clone(),
                            };
                            self.bind_pattern(pat, &elem_ty)?;
                        }
                    } else {
                        if patterns.len() != types.len() {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Pattern length mismatch: expected {} elements, found {}",
                                types.len(),
                                patterns.len()
                            ))));
                        }
                        for (pat, ty) in patterns.iter().zip(types.iter()) {
                            // Apply reference wrapper if we auto-deref'd through a reference
                            let elem_ty = match &ref_wrapper {
                                Some(wrap) => wrap(ty.clone()),
                                None => ty.clone(),
                            };
                            self.bind_pattern(pat, &elem_ty)?;
                        }
                    }
                    Ok(())
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Expected tuple type for tuple pattern at {}, found {}",
                        span_to_line_col(pattern.span),
                        scheme.ty
                    ))))
                }
            }

            PatternKind::Record {
                path,
                fields: field_patterns,
                rest,
            } => {
                // First, try to resolve as a record type (the original behavior)
                if let Some(field_types) = self.resolve_to_record_type(&scheme.ty) {
                    for field_pat in field_patterns {
                        match field_types.get(field_pat.name.name.as_str()) {
                            Some(field_ty) => {
                                // If pattern is provided, bind it; otherwise bind the field name directly
                                if let Some(ref pat) = field_pat.pattern {
                                    self.bind_pattern(pat, field_ty)?;
                                } else {
                                    // Shorthand: { x } means { x: x }
                                    self.ctx.env.insert(
                                        field_pat.name.name.clone(),
                                        TypeScheme::mono(field_ty.clone()),
                                    );
                                }
                            }
                            None => {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "field '{}' not found in type 'record'",
                                    field_pat.name.name
                                ))));
                            }
                        }
                    }
                    return Ok(());
                }

                // If not a record type, check if it's a variant type with record-style payload
                // This handles patterns like `OutOfMemory { requested }` when matching on a variant

                // Extract the variant tag name from the path
                let tag = if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                        id.name.as_str()
                    } else {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Invalid record/variant pattern path at {}",
                            span_to_line_col(pattern.span)
                        ))));
                    }
                } else if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.last() {
                    id.name.as_str()
                } else {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Invalid multi-segment pattern path at {}",
                        span_to_line_col(pattern.span)
                    ))));
                };

                // Expand the scrutinee type and check if it's a variant
                let mut expanded_ty = self.expand_generic_to_variant(&scheme.ty);

                // Track if we're matching through a reference
                let matching_through_ref = matches!(
                    &expanded_ty,
                    Type::Reference { .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. }
                );

                // Unwrap references to get at the variant type
                if let Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } = &expanded_ty
                {
                    if matches!(&**inner, Type::Variant(_)) {
                        expanded_ty = (**inner).clone();
                    } else if let Type::Named { path: inner_path, args: _ } = &**inner {
                        // Look up Named type that might be a variant
                        let inner_type_name = inner_path
                            .segments
                            .last()
                            .map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "",
                            })
                            .unwrap_or("");
                        if let Option::Some(def_ty) = self.ctx.lookup_type(inner_type_name) {
                            if let Type::Variant(variants) = def_ty.clone() {
                                expanded_ty = Type::Variant(variants);
                            }
                        }
                    }
                }

                // Handle non-reference Named types
                if let Type::Named { path: type_path, args: _ } = &expanded_ty {
                    let named_type_name = type_path
                        .segments
                        .last()
                        .map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                            _ => "",
                        })
                        .unwrap_or("");
                    if let Option::Some(def_ty) = self.ctx.lookup_type(named_type_name) {
                        if let Type::Variant(variants) = def_ty.clone() {
                            expanded_ty = Type::Variant(variants);
                        }
                    }
                }

                // Now check if expanded_ty is a Variant
                if let Type::Variant(variants) = &expanded_ty {
                    // Look up the variant constructor
                    match variants.get(tag) {
                        Some(payload_ty) => {
                            // Delegate to bind_variant_record_payload for consistent handling
                            self.bind_variant_record_payload(
                                tag,
                                field_patterns,
                                *rest,
                                payload_ty,
                                pattern.span,
                                matching_through_ref,
                            )?;
                            return Ok(());
                        }
                        None => {
                            let available: List<_> = variants.keys().map(|s| s.as_str()).collect();
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "Unknown variant constructor '{}' at {}. Available variants: [{}]",
                                tag,
                                span_to_line_col(pattern.span),
                                available.join(", ")
                            ))));
                        }
                    }
                }

                // For Unknown or unresolved type variables, bind field patterns with fresh types
                if matches!(&scheme.ty, Type::Unknown | Type::Var(_)) {
                    for field_pat in field_patterns {
                        let fresh_ty = Type::Var(TypeVar::fresh());
                        if let Some(ref pat) = field_pat.pattern {
                            self.bind_pattern(pat, &fresh_ty)?;
                        } else {
                            self.ctx.env.insert(
                                field_pat.name.name.clone(),
                                TypeScheme::mono(fresh_ty),
                            );
                        }
                    }
                    return Ok(());
                }

                // Neither a record nor a variant with this constructor
                Err(TypeError::Other(verum_common::Text::from(format!(
                    "Expected record type for record pattern, found {}",
                    scheme.ty
                ))))
            }

            PatternKind::Variant { path, data } => {
                // Sum types (variants): "type T is A | B(payload) | C { fields }" for algebraic data types (Variants)
                // Pattern matching on variant/enum types: Tag(pattern) or Tag { field: pattern }

                // Never type (!) matches any pattern - unreachable code
                if matches!(scheme.ty, Type::Never) {
                    // Bind sub-patterns with fresh type vars so they don't cause errors
                    if let Some(variant_data) = data {
                        use verum_ast::pattern::VariantPatternData;
                        match variant_data {
                            VariantPatternData::Tuple(patterns) => {
                                for p in patterns {
                                    self.bind_pattern(p, &Type::Never)?;
                                }
                            }
                            VariantPatternData::Record { fields, .. } => {
                                for f in fields {
                                    if let Some(ref pat) = f.pattern {
                                        self.bind_pattern(pat, &Type::Never)?;
                                    } else {
                                        // Shorthand field pattern — bind field name as variable
                                        self.ctx.env.insert(f.name.name.as_str(), TypeScheme::mono(Type::Never));
                                    }
                                }
                            }
                        }
                    }
                    return Ok(());
                }

                // Extract the tag name from the path
                let tag = if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                        id.name.as_str()
                    } else {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Invalid variant constructor path at {}",
                            span_to_line_col(pattern.span)
                        ))));
                    }
                } else {
                    // Multi-segment paths: Module::Type::Variant
                    // For now, take the last segment as the variant tag
                    if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.last() {
                        id.name.as_str()
                    } else {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Invalid multi-segment variant path at {}",
                            span_to_line_col(pattern.span)
                        ))));
                    }
                };

                // Expand Generic types like Maybe<T> and Result<T,E> to their variant form
                let mut expanded_ty = self.expand_generic_to_variant(&scheme.ty);

                // Handle unresolved associated type projections (e.g., Item<_>)
                // If the scrutinee is an unresolved generic type that starts with uppercase
                // and has type variable args, treat it as potentially matching any pattern.
                // This handles patterns like: for item in iter { match item { Ok(v) => ..., Err(e) => ... } }
                // where item has type Item<_> (unresolved Iterator::Item).
                if let Type::Generic { name, args } = &expanded_ty {
                    let is_unresolved_assoc = name.as_str().starts_with("::")
                        || (name.as_str().chars().next().is_some_and(|c| c.is_ascii_uppercase())
                            && !args.is_empty()
                            && args.iter().all(|a| matches!(a, Type::Var(_))));
                    if is_unresolved_assoc {
                        // Bind sub-patterns with fresh type vars
                        if let Some(variant_data) = data {
                            use verum_ast::pattern::VariantPatternData;
                            match variant_data {
                                VariantPatternData::Tuple(patterns) => {
                                    for p in patterns {
                                        self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                    }
                                }
                                VariantPatternData::Record { fields, .. } => {
                                    for f in fields {
                                        if let Some(ref pat) = f.pattern {
                                            self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                        } else {
                                            // Shorthand field pattern — bind field name as variable
                                            self.ctx.env.insert(f.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                }

                // Track if we're matching through a reference (for binding mode)
                // When matching &T against a pattern, bindings should be &FieldType
                let matching_through_ref = matches!(
                    &expanded_ty,
                    Type::Reference { .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. }
                );

                // If we have a reference to a variant, unwrap it for matching
                // This handles cases like: match &opt { Maybe.Some(x) => ..., Maybe.None => ... }
                if let Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } = &expanded_ty
                    && matches!(&**inner, Type::Variant(_))
                {
                    expanded_ty = (**inner).clone();
                }

                // Also handle references to Named types that are variants
                // e.g., match &tree { Tree.Node { value, ... } => ... }
                if let Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } = &expanded_ty
                {
                    if let Type::Named { path, args } = &**inner {
                        let inner_type_name = path
                            .segments
                            .last()
                            .map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "",
                            })
                            .unwrap_or("");

                        // Look up the Named type to see if it's a variant
                        if let Option::Some(def_ty) =
                            self.ctx.lookup_type(inner_type_name)
                        {
                            if let Type::Variant(variants) = def_ty.clone() {
                                // Found a variant type - substitute type args and use it
                                // For now, we'll use the variant directly (type args handled later)
                                let _ = args; // Used below in substitution
                                expanded_ty = Type::Variant(variants);
                            }
                        }
                    }
                }

                // Handle Named types that are variants (non-reference case)
                if let Type::Named { path, args } = &expanded_ty {
                    let named_type_name = path
                        .segments
                        .last()
                        .map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                            _ => "",
                        })
                        .unwrap_or("");

                    // Look up the Named type to see if it's a variant
                    if let Option::Some(def_ty) =
                        self.ctx.lookup_type(named_type_name)
                    {
                        if let Type::Variant(variants) = def_ty.clone() {
                            let _ = args; // Type args handled in field binding
                            expanded_ty = Type::Variant(variants);
                        }
                    }
                }

                // The scrutinee type must be a variant type
                if let Type::Variant(variants) = &expanded_ty {
                    // Look up the variant tag in the variant type definition
                    match variants.get(tag) {
                        Some(variant_payload_ty) => {
                            // Validate and bind the payload pattern if present
                            if let Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    // Tuple-style variant: Some(x), Ok(value), Point(x, y)
                                    VariantPatternData::Tuple(patterns) => {
                                        self.bind_variant_tuple_payload(
                                            tag,
                                            patterns,
                                            variant_payload_ty,
                                            pattern.span,
                                            matching_through_ref,
                                        )?;
                                    }
                                    // Record-style variant: Error { code, message }
                                    VariantPatternData::Record { fields, rest } => {
                                        self.bind_variant_record_payload(
                                            tag,
                                            fields,
                                            *rest,
                                            variant_payload_ty,
                                            pattern.span,
                                            matching_through_ref,
                                        )?;
                                    }
                                }
                            } else if !matches!(variant_payload_ty, Type::Unit) {
                                // No payload pattern provided but variant has a non-Unit payload.
                                // This is valid for `is` pattern tests: `x is Some` checks the tag
                                // without destructuring the payload. Just skip binding.
                                // In full match arms this is still permitted (irrefutable tag check).
                            }
                            Ok(())
                        }
                        None => {
                            // Before reporting error, check if this is an active pattern
                            // invocation rather than a variant constructor. This occurs in
                            // And patterns like `IntVal(n) & Sign(Positive)` where Sign is
                            // an active pattern, not a variant of the matched type.
                            let tag_text: Text = tag.into();
                            if let Some((_param_tys, return_ty)) = self.pattern_declarations.get(&tag_text).cloned() {
                                // This is an active pattern! Bind sub-patterns against its return type.
                                let return_scheme = TypeScheme::mono(return_ty);
                                if let Some(variant_data) = data {
                                    use verum_ast::pattern::VariantPatternData;
                                    match variant_data {
                                        VariantPatternData::Tuple(bindings) => {
                                            for binding in bindings.iter() {
                                                self.bind_pattern_scheme(binding, return_scheme.clone())?;
                                            }
                                        }
                                        VariantPatternData::Record { fields, .. } => {
                                            for f in fields.iter() {
                                                if let Some(ref pat) = f.pattern {
                                                    self.bind_pattern_scheme(pat, return_scheme.clone())?;
                                                } else {
                                                    // Shorthand field pattern — bind field name as variable
                                                    self.ctx.env.insert(f.name.name.as_str(), return_scheme.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(())
                            } else {
                                // Provide helpful error with available variants
                                let available: List<_> = variants.keys().map(|s| s.as_str()).collect();
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Unknown variant constructor '{}' at {}. Available variants: [{}]",
                                    tag,
                                    pattern.span,
                                    available.join(", ")
                                ))))
                            }
                        }
                    }
                } else if let Type::Named { path: type_path, .. } = &expanded_ty {
                    // Check if this is a newtype pattern: `let Counter(n) = value;`
                    // Newtypes are Named types with __newtype_inner_{name} stored
                    let type_name = type_path
                        .segments
                        .last()
                        .map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                            _ => "",
                        })
                        .unwrap_or("");

                    // Verify the pattern name matches the type name
                    if tag == type_name {
                        // Check for newtype first
                        let inner_key = format!("__newtype_inner_{}", type_name);
                        if let Option::Some(inner_ty) = self.ctx.lookup_type(&inner_key) {
                            // This is a newtype destructure: Counter(n) where Counter wraps Int
                            let inner_ty = inner_ty.clone();
                            if let Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    VariantPatternData::Tuple(patterns) => {
                                        if patterns.len() == 1 {
                                            // Bind the single pattern to the inner type
                                            self.bind_pattern(&patterns[0], &inner_ty)?;
                                        } else {
                                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                                "Newtype pattern {} expects exactly 1 field, but {} were provided at {}",
                                                tag, patterns.len(), span_to_line_col(pattern.span)
                                            ))));
                                        }
                                    }
                                    VariantPatternData::Record { .. } => {
                                        return Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Newtype pattern {} cannot use record syntax at {}",
                                            tag, span_to_line_col(pattern.span)
                                        ))));
                                    }
                                }
                            }
                            return Ok(());
                        }

                        // Check for tuple struct: type Color is (Int, Int, Int)
                        let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                        if let Option::Some(Type::Tuple(field_types)) = self.ctx.lookup_type(&tuple_fields_key) {
                            // This is a tuple struct destructure: Color(r, g, b)
                            let field_types = field_types.clone();
                            if let Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    VariantPatternData::Tuple(patterns) => {
                                        if patterns.len() == field_types.len() {
                                            // Bind each pattern to the corresponding field type
                                            for (pat, field_ty) in patterns.iter().zip(field_types.iter()) {
                                                self.bind_pattern(pat, field_ty)?;
                                            }
                                        } else {
                                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                                "Tuple struct pattern {} expects {} fields, but {} were provided at {}",
                                                tag, field_types.len(), patterns.len(), span_to_line_col(pattern.span)
                                            ))));
                                        }
                                    }
                                    VariantPatternData::Record { .. } => {
                                        return Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Tuple struct pattern {} cannot use record syntax at {}",
                                            tag, span_to_line_col(pattern.span)
                                        ))));
                                    }
                                }
                            }
                            return Ok(());
                        }
                    }

                    // Handle generic wrapper destructure for Named types: Heap(inner_val) matching Heap<T>
                    if let Type::Named { args, .. } = &expanded_ty {
                        if tag == type_name && args.len() == 1 {
                            if let Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    VariantPatternData::Tuple(patterns) => {
                                        if patterns.len() == 1 {
                                            self.bind_pattern(&patterns[0], &args[0])?;
                                        } else {
                                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                                "Generic wrapper pattern {} expects exactly 1 field, but {} were provided at {}",
                                                tag, patterns.len(), span_to_line_col(pattern.span)
                                            ))));
                                        }
                                    }
                                    VariantPatternData::Record { .. } => {
                                        return Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Generic wrapper pattern {} cannot use record syntax at {}",
                                            tag, span_to_line_col(pattern.span)
                                        ))));
                                    }
                                }
                            }
                            return Ok(());
                        }
                    }

                    // Auto-deref through Heap<T>/Shared<T> for variant pattern matching
                    // This enables `match heap_val { Cons(v, rest) => ..., Nil => ... }`
                    // on a Heap<IntList> scrutinee by unwrapping to IntList first.
                    if WKT::is_smart_pointer_name(type_name) {
                        if let Type::Named { args, .. } = &expanded_ty {
                            if args.len() == 1 {
                                return self.bind_pattern(pattern, &args[0]);
                            }
                        }
                    }

                    // For unknown/unresolved types, allow variant pattern matching
                    if matches!(&expanded_ty, Type::Var(_) | Type::Unknown | Type::Placeholder { .. }) {
                        if let Maybe::Some(variant_data) = data {
                            use verum_ast::pattern::VariantPatternData;
                            match variant_data {
                                VariantPatternData::Tuple(patterns) => {
                                    for p in patterns.iter() {
                                        self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                    }
                                }
                                VariantPatternData::Record { fields, .. } => {
                                    for fp in fields.iter() {
                                        if let Maybe::Some(ref pat) = fp.pattern {
                                            self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                        } else {
                                            // Shorthand field pattern — bind field name as variable
                                            self.ctx.env.insert(fp.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                    // Check if this is a constant pattern on a numeric Named type
                    // e.g., match rtype { DNS_TYPE_A => ... } where rtype: UInt16
                    let is_numeric_named = matches!(type_name,
                        "Int8" | "Int16" | "Int32" | "Int64"
                        | "UInt8" | "UInt16" | "UInt32" | "UInt64"
                        | "ISize" | "USize" | "Float32" | "Float64"
                        | "Byte");
                    if is_numeric_named && data.is_none() {
                        if let Some(const_scheme) = self.ctx.env.lookup(tag) {
                            let const_ty = self.unifier.apply(&const_scheme.ty);
                            let is_variant_constructor = matches!(
                                &const_ty,
                                Type::Variant(_) | Type::Generic { .. }
                            ) || matches!(
                                &const_ty,
                                Type::Function { return_type, .. }
                                    if matches!(return_type.as_ref(), Type::Variant(_) | Type::Generic { .. })
                            );
                            if !is_variant_constructor {
                                let resolved_scrutinee = self.unifier.apply(&expanded_ty);
                                if self.unifier.unify(&const_ty, &resolved_scrutinee, pattern.span).is_ok() {
                                    return Ok(());
                                }
                            }
                        }
                    }

                    // Not a newtype or tuple struct - provide helpful error
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Pattern expects a variant type, but scrutinee has type {} at {}",
                        expanded_ty, span_to_line_col(pattern.span)
                    ))))
                } else if let Type::Generic { name: generic_name, args } = &expanded_ty {
                    // Handle generic wrapper destructure: Heap(inner_val) matching Heap<T>
                    if tag == generic_name.as_str() && args.len() == 1 {
                        if let Some(variant_data) = data {
                            use verum_ast::pattern::VariantPatternData;
                            match variant_data {
                                VariantPatternData::Tuple(patterns) => {
                                    if patterns.len() == 1 {
                                        self.bind_pattern(&patterns[0], &args[0])?;
                                    } else {
                                        return Err(TypeError::Other(verum_common::Text::from(format!(
                                            "Generic wrapper pattern {} expects exactly 1 field, but {} were provided at {}",
                                            tag, patterns.len(), span_to_line_col(pattern.span)
                                        ))));
                                    }
                                }
                                VariantPatternData::Record { .. } => {
                                    return Err(TypeError::Other(verum_common::Text::from(format!(
                                        "Generic wrapper pattern {} cannot use record syntax at {}",
                                        tag, span_to_line_col(pattern.span)
                                    ))));
                                }
                            }
                        }
                        Ok(())
                    } else if WKT::is_smart_pointer_name(generic_name.as_str()) && args.len() == 1 {
                        // Auto-deref through Heap<T>/Shared<T> for variant pattern matching
                        self.bind_pattern(pattern, &args[0])
                    } else {
                        // Generic variant pattern matching for any named sum type (e.g., Maybe<T>, Result<T,E>).
                        // Look up the type definition to discover its registered variants
                        // instead of hardcoding specific type names like Maybe/Result.
                        let type_def = self.ctx.lookup_type(generic_name.as_str()).cloned();
                        let variant_map = match &type_def {
                            Some(Type::Variant(variants)) => Some(variants.clone()),
                            _ => None,
                        };

                        // Strip qualified prefix (e.g., "Maybe.Some" -> "Some")
                        let bare_tag = tag.rsplit('.').next().unwrap_or(tag);

                        if let Some(ref variants) = variant_map {
                            if variants.contains_key(bare_tag) {
                                // This tag is a valid variant of the type
                                let variant_payload = variants.get(bare_tag).cloned().unwrap_or(Type::Unit);
                                let is_unit_variant = matches!(&variant_payload, Type::Unit);

                                if is_unit_variant {
                                    // Unit variant (like None): no payload expected
                                    Ok(())
                                } else if let Some(variant_data) = data {
                                    // Data-carrying variant: bind pattern to the inner type
                                    // Resolve type args: if variant payload is a type var, substitute with concrete args
                                    let concrete_payload = if args.len() == 1 {
                                        // For single-arg generics like Maybe<T>, the payload type T maps to args[0]
                                        match &variant_payload {
                                            Type::Var(_) => args[0].clone(),
                                            _ => args[0].clone(), // Simplification: first arg is the payload type
                                        }
                                    } else {
                                        variant_payload.clone()
                                    };

                                    use verum_ast::pattern::VariantPatternData;
                                    match variant_data {
                                        VariantPatternData::Tuple(patterns) => {
                                            if patterns.len() == 1 {
                                                self.bind_pattern(&patterns[0], &concrete_payload)?;
                                            } else {
                                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                                    "{} pattern expects exactly 1 field, but {} were provided at {}",
                                                    bare_tag, patterns.len(), span_to_line_col(pattern.span)
                                                ))));
                                            }
                                        }
                                        VariantPatternData::Record { .. } => {
                                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                                "{} pattern cannot use record syntax at {}",
                                                bare_tag, span_to_line_col(pattern.span)
                                            ))));
                                        }
                                    }
                                    Ok(())
                                } else {
                                    // Data-carrying variant without data in pattern - ok (ignores payload)
                                    Ok(())
                                }
                            } else {
                                Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Pattern '{}' is not a variant of {} at {}",
                                    tag, generic_name, span_to_line_col(pattern.span)
                                ))))
                            }
                        } else {
                            // No variant definition found for this generic type.
                            // Allow pattern matching with fresh type variables - the type
                            // may come from a module that hasn't fully registered its variants.
                            if let Maybe::Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    VariantPatternData::Tuple(patterns) => {
                                        for p in patterns.iter() {
                                            self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                        }
                                    }
                                    VariantPatternData::Record { fields, .. } => {
                                        for fp in fields.iter() {
                                            if let Maybe::Some(ref pat) = fp.pattern {
                                                self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                            } else {
                                                // Shorthand field pattern — bind field name as variable
                                                self.ctx.env.insert(fp.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(())
                        }
                    }
                } else if let Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. } = &expanded_ty
                {
                    // Handle reference to newtype: `let Counter(n) = &counter;`
                    if let Type::Named { path: type_path, .. } = &**inner {
                        let type_name = type_path
                            .segments
                            .last()
                            .map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                                _ => "",
                            })
                            .unwrap_or("");

                        if tag == type_name {
                            let inner_key = format!("__newtype_inner_{}", type_name);
                            if let Option::Some(inner_ty) = self.ctx.lookup_type(&inner_key) {
                                let inner_ty = inner_ty.clone();
                                if let Some(variant_data) = data {
                                    use verum_ast::pattern::VariantPatternData;
                                    match variant_data {
                                        VariantPatternData::Tuple(patterns) => {
                                            if patterns.len() == 1 {
                                                // For references, bind to reference of inner type
                                                let ref_inner = Type::Reference {
                                                    inner: Box::new(inner_ty.clone()),
                                                    mutable: false,
                                                };
                                                self.bind_pattern(&patterns[0], &ref_inner)?;
                                            } else {
                                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                                    "Newtype pattern {} expects exactly 1 field at {}",
                                                    tag, span_to_line_col(pattern.span)
                                                ))));
                                            }
                                        }
                                        VariantPatternData::Record { .. } => {
                                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                                "Newtype pattern {} cannot use record syntax at {}",
                                                tag, span_to_line_col(pattern.span)
                                            ))));
                                        }
                                    }
                                }
                                return Ok(());
                            }
                        }
                    }
                    // Reference to unresolvable type - allow permissive matching
                    if let Maybe::Some(variant_data) = data {
                        use verum_ast::pattern::VariantPatternData;
                        match variant_data {
                            VariantPatternData::Tuple(patterns) => {
                                for p in patterns.iter() {
                                    self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                }
                            }
                            VariantPatternData::Record { fields, .. } => {
                                for fp in fields.iter() {
                                    if let Maybe::Some(ref pat) = fp.pattern {
                                        self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                    } else {
                                        // Shorthand field pattern — bind field name as variable
                                        self.ctx.env.insert(fp.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                } else if matches!(&expanded_ty, Type::Int | Type::Char | Type::Bool | Type::Float | Type::Text)
                    || matches!(&expanded_ty, Type::Named { path, .. } if {
                        let name = path.segments.last().map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                            _ => "",
                        }).unwrap_or("");
                        matches!(name, "Int8" | "Int16" | "Int32" | "Int64"
                            | "UInt8" | "UInt16" | "UInt32" | "UInt64"
                            | "ISize" | "USize" | "Float32" | "Float64"
                            | "Byte")
                    }) {
                    // Constant pattern matching for primitive types
                    // This enables: match rtype { DNS_TYPE_A => ..., DNS_TYPE_AAAA => ... }
                    // where DNS_TYPE_A is a const Int = 1
                    //
                    // Also supports:
                    // - Guards: DNS_TYPE_A if condition => ...
                    // - Or-patterns: DNS_TYPE_CNAME | DNS_TYPE_NS => ...
                    // - Wildcard: _ => ... (handled separately)

                    if data.is_none() {
                        // Simple identifier pattern with no payload - check if it's a constant
                        if let Some(const_scheme) = self.ctx.env.lookup(tag) {
                            let const_ty = self.unifier.apply(&const_scheme.ty);
                            // Skip variant constructors from unrelated types — they are not
                            // constants.  E.g., DnsRecordType.A should not match when
                            // scrutinee is Int/Char/Bool/Float/Text.  Only true constants
                            // should be considered for constant-pattern matching.
                            let is_variant_constructor = matches!(
                                &const_ty,
                                Type::Variant(_) | Type::Generic { .. }
                            ) || matches!(
                                &const_ty,
                                Type::Function { return_type, .. }
                                    if matches!(return_type.as_ref(), Type::Variant(_) | Type::Generic { .. })
                            );
                            if !is_variant_constructor {
                                let resolved_scrutinee = self.unifier.apply(&expanded_ty);
                                // Verify the constant type is compatible with scrutinee type
                                if self.unifier.unify(&const_ty, &resolved_scrutinee, pattern.span).is_ok() {
                                    // Valid constant pattern - will be checked for equality at runtime
                                    return Ok(());
                                } else {
                                    return Err(TypeError::Other(verum_common::Text::from(format!(
                                        "Constant pattern '{}' has type {}, but scrutinee has type {} at {}",
                                        tag, const_ty, resolved_scrutinee, span_to_line_col(pattern.span)
                                    ))));
                                }
                            }
                            // Fall through: variant constructor in env doesn't apply to
                            // primitive scrutinee — treat as unresolved pattern below
                        }
                    }
                    // Before reporting error, check if this is an active pattern
                    // invocation on a primitive type. Active patterns like Even(half)
                    // take the matched value and return a result type.
                    {
                        let tag_text: Text = tag.into();
                        if let Some((_param_tys, return_ty)) = self.pattern_declarations.get(&tag_text).cloned() {
                            // This is an active pattern! Bind sub-patterns against its return type.
                            let return_scheme = TypeScheme::mono(return_ty);
                            if let Some(variant_data) = data {
                                use verum_ast::pattern::VariantPatternData;
                                match variant_data {
                                    VariantPatternData::Tuple(bindings) => {
                                        for binding in bindings.iter() {
                                            self.bind_pattern_scheme(binding, return_scheme.clone())?;
                                        }
                                    }
                                    VariantPatternData::Record { fields, .. } => {
                                        for f in fields.iter() {
                                            if let Some(ref pat) = f.pattern {
                                                self.bind_pattern_scheme(pat, return_scheme.clone())?;
                                            } else {
                                                // Shorthand field pattern — bind field name as variable
                                                self.ctx.env.insert(f.name.name.as_str(), return_scheme.clone());
                                            }
                                        }
                                    }
                                }
                            }
                            return Ok(());
                        }
                    }
                    // Not a constant or active pattern - could be a variable binding attempt on a primitive
                    // In Verum, single identifiers without 'let' in match arms are constants, not bindings
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Pattern '{}' is not a defined constant. To match {} values, use literal patterns (e.g., 1, 'a') or define a constant. At {}",
                        tag, expanded_ty, span_to_line_col(pattern.span)
                    ))))
                } else if let Type::Var(tv) = &expanded_ty {
                    // The scrutinee type is an unresolved type variable.
                    // Try applying unifier to resolve it, then retry matching.
                    let resolved = self.unifier.apply(&expanded_ty);
                    let _ = tv; // suppress unused warning
                    if !matches!(&resolved, Type::Var(_)) {
                        // Resolved to a concrete type - retry by recursing with resolved type
                        self.bind_pattern_scheme(pattern, TypeScheme::mono(resolved))
                    } else {
                        // Unresolved type variable: allow variant pattern matching
                        // by binding sub-patterns with fresh type variables.
                        // This handles cases like `match iter_item { Ok(v) => ... }`
                        // where the item type hasn't been fully resolved yet.
                        if let Maybe::Some(variant_data) = data {
                            use verum_ast::pattern::VariantPatternData;
                            match variant_data {
                                VariantPatternData::Tuple(patterns) => {
                                    for p in patterns.iter() {
                                        self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                    }
                                }
                                VariantPatternData::Record { fields, .. } => {
                                    for fp in fields.iter() {
                                        if let Maybe::Some(ref pat) = fp.pattern {
                                            self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                        } else {
                                            // Shorthand field pattern — bind field name as variable
                                            self.ctx.env.insert(fp.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(())
                    }
                } else {
                    // For unresolvable types (e.g., `Unknown`, imported types without variant
                    // definitions), allow variant matching with fresh type variables rather than
                    // hard-erroring. This enables gradual typing and dynamic dispatch patterns.
                    if let Maybe::Some(variant_data) = data {
                        use verum_ast::pattern::VariantPatternData;
                        match variant_data {
                            VariantPatternData::Tuple(patterns) => {
                                for p in patterns.iter() {
                                    self.bind_pattern(p, &Type::Var(TypeVar::fresh()))?;
                                }
                            }
                            VariantPatternData::Record { fields, .. } => {
                                for fp in fields.iter() {
                                    if let Maybe::Some(ref pat) = fp.pattern {
                                        self.bind_pattern(pat, &Type::Var(TypeVar::fresh()))?;
                                    } else {
                                        // Shorthand field pattern — bind field name as variable
                                        self.ctx.env.insert(fp.name.name.as_str(), TypeScheme::mono(Type::Var(TypeVar::fresh())));
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
            }

            PatternKind::Or(patterns) => {
                // Or patterns: "A | B" must bind identical variable names with compatible types in both arms
                // All alternatives must bind same variables with same types
                if patterns.is_empty() {
                    return Ok(());
                }

                // Collect bindings from first pattern
                let first_bindings = self.collect_pattern_bindings(&patterns[0], &scheme.ty)?;

                // Check all other patterns have same bindings
                for (i, pat) in patterns.iter().enumerate().skip(1) {
                    let bindings = self.collect_pattern_bindings(pat, &scheme.ty)?;

                    // Check that binding sets are identical
                    if first_bindings.len() != bindings.len() {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "or-pattern alternative {} binds {} variables, but first alternative binds {}",
                            i,
                            bindings.len(),
                            first_bindings.len()
                        ))));
                    }

                    for (name, ty) in &first_bindings {
                        match bindings.iter().find(|(n, _)| n == name) {
                            Some((_, alt_ty)) => {
                                // Verify types match
                                if !self.subtyping.is_subtype(ty, alt_ty)
                                    || !self.subtyping.is_subtype(alt_ty, ty)
                                {
                                    return Err(TypeError::Other(verum_common::Text::from(format!(
                                        "or-pattern alternative {} binds variable '{}' with type {}, but first alternative has type {}",
                                        i, name, alt_ty, ty
                                    ))));
                                }
                            }
                            None => {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "or-pattern alternative {} does not bind variable '{}' which is bound in first alternative",
                                    i, name
                                ))));
                            }
                        }
                    }

                    // Check for extra bindings
                    for (name, _) in &bindings {
                        if !first_bindings.iter().any(|(n, _)| n == name) {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "or-pattern alternative {} binds variable '{}' which is not bound in first alternative",
                                i, name
                            ))));
                        }
                    }
                }

                // Now actually bind the variables from first pattern
                for (name, ty) in first_bindings {
                    self.ctx.env.insert(name, TypeScheme::mono(ty));
                }

                Ok(())
            }

            PatternKind::Range {
                start,
                end,
                inclusive: _,
            } => {
                // Range patterns are for matching literals
                // Range patterns: "a..=b" requires Ord protocol on the matched type for comparison
                // Validate that bound literals are compatible with the type
                if let Some(start_lit) = start {
                    let start_ty = self.infer_literal(start_lit);
                    self.unifier.unify(&start_ty, &scheme.ty, pattern.span)?;
                }
                if let Some(end_lit) = end {
                    let end_ty = self.infer_literal(end_lit);
                    self.unifier.unify(&end_ty, &scheme.ty, pattern.span)?;
                }
                Ok(())
            }

            PatternKind::Rest => {
                // Rest pattern (..) - used in slice patterns
                // Captures remaining elements, type inferred from context
                // No bindings introduced directly
                Ok(())
            }

            PatternKind::Array(patterns) => {
                // Array pattern: [a, b, c]
                // Array patterns: match fixed-length arrays with element patterns, length must match at compile time
                // When matching through a reference (&[T]), bindings get reference type (&T)
                let is_reference = matches!(
                    &scheme.ty,
                    Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
                );
                // Auto-deref: if matching &[T] or &&[T] with array pattern, use inner type
                let effective_ty = self.unwrap_references(&scheme.ty);
                // Handle Array, Slice, and collection types ([...] syntax works on all)
                let element: Type = if let Some(elem) = self.element_type_of(&effective_ty) {
                    elem
                } else {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Expected array type for array pattern, found {}",
                        scheme.ty
                    ))));
                };
                // If matching through reference, bind with reference to element type
                let bind_ty = if is_reference {
                    Type::reference(false, element.clone())
                } else {
                    element.clone()
                };
                for pat in patterns {
                    self.bind_pattern(pat, &bind_ty)?;
                }
                Ok(())
            }

            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // Slice pattern: [a, .., b] or [a, rest @ .., b]
                // Slice patterns: "[first, .., last]" matches variable-length with ".." capturing the middle elements
                // When matching through a reference (&[T]), bindings get reference type (&T)
                let is_reference = matches!(
                    &scheme.ty,
                    Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
                );
                // Auto-deref: if matching &[T] or &&[T] with slice pattern, use inner type
                let effective_ty = self.unwrap_references(&scheme.ty);
                // Handle Array, Slice, and collection types ([...] syntax works on all)
                let is_array_or_slice = matches!(&effective_ty, Type::Array { .. } | Type::Slice { .. });
                let element: Type = if let Some(elem) = self.element_type_of(&effective_ty) {
                    elem
                } else {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Expected array type for slice pattern, found {}",
                        scheme.ty
                    ))));
                };

                // If matching through reference, bind with reference to element type
                let bind_ty = if is_reference {
                    Type::reference(false, element.clone())
                } else {
                    element.clone()
                };

                // Bind 'before' patterns
                for pat in before {
                    self.bind_pattern(pat, &bind_ty)?;
                }

                // Bind rest pattern if present (binds to remaining sequence of same type)
                if let Some(rest_pat) = rest {
                    // Rest of an array/slice is [T]; rest of a collection is the same collection type
                    let base_rest_ty = if is_array_or_slice {
                        Type::array(element.clone(), None)
                    } else {
                        effective_ty.clone()
                    };
                    let rest_ty = if is_reference {
                        Type::reference(false, base_rest_ty)
                    } else {
                        base_rest_ty
                    };
                    self.bind_pattern(rest_pat, &rest_ty)?;
                }

                // Bind 'after' patterns
                for pat in after {
                    self.bind_pattern(pat, &bind_ty)?;
                }

                Ok(())
            }

            PatternKind::Reference { mutable, inner } => {
                // Reference pattern: &x or &mut x
                // Reference patterns: matching "&x" auto-dereferences the reference, binding x to the inner value
                if let Type::Reference {
                    inner: ref_inner,
                    mutable: ref_mut,
                } = &scheme.ty
                {
                    // Check mutability compatibility
                    if *mutable && !ref_mut {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "Cannot match &mut pattern against shared reference",
                        )));
                    }
                    // Bind inner pattern to dereferenced type
                    self.bind_pattern(inner, ref_inner)?;
                    Ok(())
                } else if let Type::CheckedReference {
                    inner: ref_inner,
                    mutable: ref_mut,
                } = &scheme.ty
                {
                    if *mutable && !ref_mut {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "Cannot match &mut pattern against shared checked reference",
                        )));
                    }
                    self.bind_pattern(inner, ref_inner)?;
                    Ok(())
                } else if let Type::UnsafeReference {
                    inner: ref_inner,
                    mutable: ref_mut,
                } = &scheme.ty
                {
                    if *mutable && !ref_mut {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "Cannot match &mut pattern against shared unsafe reference",
                        )));
                    }
                    self.bind_pattern(inner, ref_inner)?;
                    Ok(())
                } else if matches!(&scheme.ty, Type::Var(_) | Type::Unknown | Type::Placeholder { .. }) {
                    // For unknown/inferred types, bind the inner pattern to a fresh var
                    let fresh = TypeScheme::mono(Type::Var(TypeVar::fresh()));
                    self.bind_pattern_scheme(inner, fresh)?;
                    Ok(())
                } else {
                    // For concrete non-reference types, allow as gradual typing
                    // (the type may be wrapped in a reference at runtime)
                    self.bind_pattern_scheme(inner, scheme.clone())?;
                    Ok(())
                }
            }

            PatternKind::Paren(inner) => {
                // Parenthesized pattern - just unwrap and process inner
                self.bind_pattern_scheme(inner, scheme)
            }

            #[allow(deprecated)]
            PatternKind::View { pattern, .. } => {
                // View patterns: bind the inner pattern
                self.bind_pattern_scheme(pattern, scheme)
            }

            PatternKind::Active { name, params, bindings } => {
                // Active patterns (F#-style) - user-defined pattern matchers
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 11 - Pattern Matching Enhancements
                //
                // Active patterns are functions that take a value and return:
                // - Bool for total patterns (test-only, no binding)
                // - Maybe<T> for partial patterns (extract and bind on match)
                //
                // Example:
                //   pattern Even(n: Int) -> Bool = n % 2 == 0;
                //   match x { Even() => "even", _ => "odd" }
                //
                //   pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();
                //   match s { ParseInt(n) => use(n), _ => error() }
                //
                // Type checking strategy:
                // 1. Pattern parameters are expressions that must be type-checked
                // 2. For partial patterns, bindings must be type-checked against
                //    the inner type of Maybe<T>
                // 3. Full pattern definition lookup is deferred until pattern definitions
                //    are properly tracked in the type environment
                //
                // Type-check parameter expressions
                for arg in params.iter() {
                    let _ = self.synth_expr(arg)?;
                }

                // Look up the pattern declaration to get its return type
                let pattern_name_text: Text = name.name.clone();

                if let Some((_param_tys, return_ty)) = self.pattern_declarations.get(&pattern_name_text).cloned() {
                    // Bind the bindings against the pattern's return type
                    let return_scheme = TypeScheme::mono(return_ty);
                    for binding in bindings.iter() {
                        self.bind_pattern_scheme(binding, return_scheme.clone())?;
                    }
                } else {
                    // Pattern not found — fall back to binding variables as Unknown
                    for binding in bindings.iter() {
                        self.bind_active_pattern_variables(binding)?;
                    }
                }

                Ok(())
            }

            PatternKind::And(patterns) => {
                // And patterns: all sub-patterns must match the same value
                // Each pattern can bind different variables
                for pat in patterns.iter() {
                    self.bind_pattern_scheme(pat, scheme.clone())?;
                }
                Ok(())
            }

            PatternKind::TypeTest { binding, test_type } => {
                // TypeTest pattern: `x is Type` or `x is VariantName` - type test with narrowing
                // Type test patterns: "x is Type" narrows the type of x in the true branch (flow-sensitive typing)
                //
                // The binding variable gets the narrowed type in the match arm:
                //   - For protocol/standalone types: `x is Showable`, `x is Int`
                //   - For sum-type variants: `x is Dog`, `c is Circle`
                //
                // Example:
                //   match value { x is Int => x + 1, x is Text => x.len() }
                //   match animal { x is Dog => "woof", x is Cat => "meow" }

                // Convert AST type to internal type representation.
                // Fallback: if the name is not a standalone type, check if it is a variant
                // constructor of the scrutinee's sum type.
                //
                // CRITICAL: When the scrutinee is a variant type, variant tag names take
                // priority over standalone type names. This handles cases like:
                //   type Message is | Text { content: Text, sender: Text } | ...
                //   match m { t is Text => t.sender }  // "Text" is the variant, not the primitive
                let variant_name_str = match &test_type.kind {
                    verum_ast::ty::TypeKind::Path(path) if path.segments.len() == 1 => {
                        if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                            Some(ident.name.as_str().to_string())
                        } else {
                            None
                        }
                    }
                    // Also handle built-in type keywords that might shadow variant names
                    verum_ast::ty::TypeKind::Text => Some("Text".to_string()),
                    verum_ast::ty::TypeKind::Int => Some("Int".to_string()),
                    verum_ast::ty::TypeKind::Float => Some("Float".to_string()),
                    verum_ast::ty::TypeKind::Bool => Some("Bool".to_string()),
                    verum_ast::ty::TypeKind::Char => Some("Char".to_string()),
                    verum_ast::ty::TypeKind::Unit => Some("Unit".to_string()),
                    _ => None,
                };

                // Try variant resolution first if scrutinee is a variant type
                let variant_resolved = if let Some(ref name) = variant_name_str {
                    let expanded = self.expand_generic_to_variant(&scheme.ty);
                    if let Type::Variant(variants) = expanded {
                        variants.get(name.as_str()).map(|payload_ty| match payload_ty {
                            Type::Unit => scheme.ty.clone(),
                            ty => ty.clone(),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let narrowed_type = if let Some(variant_ty) = variant_resolved {
                    // Variant tag matched - use the payload type
                    variant_ty
                } else {
                    // No variant match - try resolving as a standalone type
                    self.ast_to_type(test_type)?
                };

                // Bind the variable name to the narrowed type in scope
                self.ctx
                    .env
                    .insert(binding.name.as_str(), TypeScheme::mono(narrowed_type.clone()));

                // Register in affine tracker for move semantics
                let contains_affine = self.type_contains_affine(&narrowed_type);
                if contains_affine {
                    self.affine_tracker.bind_container(
                        binding.name.as_str(),
                        narrowed_type,
                        pattern.span,
                    );
                } else {
                    self.affine_tracker
                        .bind(binding.name.as_str(), narrowed_type, pattern.span);
                }

                Ok(())
            }

            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 18.3 - Stream Pattern Matching
                //
                // Stream patterns consume elements from an iterator and optionally bind
                // the remaining iterator to a variable. The scrutinee must implement Iterator.
                //
                // Type checking strategy:
                // 1. The scrutinee type should be an iterator producing items of type T
                // 2. Each head_pattern binds to T
                // 3. The rest binding (if present) gets the remaining iterator type

                // For now, infer element type from context or use fresh type variable
                // Full iterator protocol resolution would require looking up Iterator::Item
                let elem_ty = Type::Var(TypeVar::fresh());

                // Bind each head pattern to the element type
                for pat in head_patterns.iter() {
                    self.bind_pattern(pat, &elem_ty)?;
                }

                // If there's a rest binding, bind it to the iterator type (same as scrutinee)
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    self.ctx
                        .env
                        .insert(rest_ident.name.as_str(), scheme.clone());
                    // Register in affine tracker
                    self.affine_tracker.bind(
                        rest_ident.name.as_str(),
                        scheme.ty.clone(),
                        pattern.span,
                    );
                }

                Ok(())
            }

            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                //
                // The guard is an expression that must evaluate to bool at runtime.
                // For type binding purposes, we only need to bind the inner pattern.
                // The guard expression is type-checked separately in match arm processing.
                self.bind_pattern_scheme(pattern, scheme)
            }

            PatternKind::Cons { head, tail } => {
                // Cons pattern: head :: tail
                // The type should be a variant type with Cons(T, Tail) structure.
                // Bind head to the element type and tail to the stream/list type.
                // For now, bind both with fresh type variables.
                let elem_ty = Type::Var(crate::ty::TypeVar::fresh());
                let head_scheme = TypeScheme { ty: elem_ty, ..scheme.clone() };
                self.bind_pattern_scheme(head, head_scheme)?;
                // Tail has the same type as the overall stream
                self.bind_pattern_scheme(tail, scheme)
            }
        }
    }

    /// Collect all variable bindings from a pattern (used for OR pattern validation).
    /// Or patterns: "A | B" must bind identical variable names with compatible types in both arms
    /// Returns a list of (variable_name, type) pairs.
    pub(crate) fn collect_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        ty: &Type,
    ) -> Result<List<(Text, Type)>> {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            PatternKind::Wildcard => Ok(List::new()),

            PatternKind::Ident {
                name, subpattern, ..
            } => {
                // Check if this identifier is actually a variant constructor (unit variant)
                // when the type context is a variant type
                if let Type::Variant(variants) = ty {
                    let ident_name: Text = name.name.clone();
                    // If the identifier matches a variant constructor, it's not a variable binding
                    if variants.contains_key(&ident_name) {
                        // Unit variant constructor (like Completed in Status type)
                        // This binds NO variables
                        return Ok(List::new());
                    }
                }

                // Normal variable binding
                let mut bindings = List::from_iter([(name.name.clone(), ty.clone())]);
                if let Some(sub) = subpattern {
                    let sub_bindings = self.collect_pattern_bindings(sub, ty)?;
                    bindings.extend(sub_bindings);
                }
                Ok(bindings)
            }

            PatternKind::Literal(_) => {
                // Literals don't bind variables
                Ok(List::new())
            }

            PatternKind::Tuple(patterns) => {
                if let Type::Tuple(types) = ty {
                    if patterns.len() != types.len() {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "tuple pattern has {} elements but type has {}",
                            patterns.len(),
                            types.len()
                        ))));
                    }

                    let mut all_bindings = List::new();
                    for (pat, ty) in patterns.iter().zip(types.iter()) {
                        let mut bindings = self.collect_pattern_bindings(pat, ty)?;
                        all_bindings.append(&mut bindings);
                    }
                    Ok(all_bindings)
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "tuple pattern requires tuple type, found {}",
                        ty
                    ))))
                }
            }

            PatternKind::Variant { path, data } => {
                // Variant pattern
                // Extract only the variant tag (last segment) from the path
                let tag = if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(id) = &path.segments[0] {
                        verum_common::Text::from(id.name.as_str())
                    } else {
                        self.path_to_string(path)
                    }
                } else {
                    // Multi-segment path like Color.Red - take only the last segment
                    if let Some(verum_ast::ty::PathSegment::Name(id)) = path.segments.last() {
                        verum_common::Text::from(id.name.as_str())
                    } else {
                        self.path_to_string(path)
                    }
                };

                // CRITICAL FIX: Auto-dereference reference types for variant pattern matching.
                // This enables patterns like `match self { Prefix(s) => ... }` where self: &Component.
                // Without this, the type checker fails with "variant pattern requires variant type, found &..."
                //
                // Spec: Variant patterns should implicitly dereference references, similar to Rust's
                // match ergonomics that allow matching &T with T patterns.
                let dereferenced_ty = self.unwrap_reference_type(ty);

                // Expand the type to variant form (handles custom sum types)
                let expanded_ty = self.expand_generic_to_variant(dereferenced_ty);

                if let Type::Variant(variants) = &expanded_ty {
                    if let Some(payload_ty) = variants.get(&tag) {
                        if let Some(variant_data) = data {
                            // Extract patterns from variant data
                            use verum_ast::pattern::VariantPatternData;
                            match variant_data {
                                VariantPatternData::Tuple(patterns) => {
                                    // For tuple variant, collect bindings from each pattern
                                    let mut all_bindings = List::new();
                                    // Assuming payload type is a tuple
                                    if let Type::Tuple(types) = payload_ty {
                                        for (pat, ty) in patterns.iter().zip(types.iter()) {
                                            let mut bindings =
                                                self.collect_pattern_bindings(pat, ty)?;
                                            all_bindings.append(&mut bindings);
                                        }
                                    } else {
                                        // Single value variant
                                        for pat in patterns {
                                            let mut bindings =
                                                self.collect_pattern_bindings(pat, payload_ty)?;
                                            all_bindings.append(&mut bindings);
                                        }
                                    }
                                    Ok(all_bindings)
                                }
                                VariantPatternData::Record { fields, .. } => {
                                    // Record-style variant: Error { code, message }
                                    let mut all_bindings = List::new();

                                    // Payload must be a record type
                                    if let Type::Record(field_types) = payload_ty {
                                        for field_pat in fields {
                                            let field_name = field_pat.name.name.as_str();

                                            if let Some(field_ty) = field_types.get(field_name) {
                                                // If pattern is provided, collect from it; otherwise bind field name
                                                if let Some(ref pat) = field_pat.pattern {
                                                    let mut bindings = self
                                                        .collect_pattern_bindings(pat, field_ty)?;
                                                    all_bindings.append(&mut bindings);
                                                } else {
                                                    // Shorthand: { code, message } means { code: code, message: message }
                                                    all_bindings.push((
                                                        field_name.into(),
                                                        field_ty.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                    }

                                    Ok(all_bindings)
                                }
                            }
                        } else {
                            Ok(List::new())
                        }
                    } else {
                        Err(TypeError::Other(verum_common::Text::from(format!(
                            "unknown variant constructor '{}' in type {}",
                            tag, expanded_ty
                        ))))
                    }
                } else if matches!(&expanded_ty, Type::Unknown | Type::Var(_)) {
                    // For Unknown/unresolved types, allow variant pattern with fresh bindings
                    Ok(List::new())
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "variant pattern requires variant type, found {} (expanded from {})",
                        expanded_ty, ty
                    ))))
                }
            }

            PatternKind::Or(patterns) => {
                // For nested OR patterns, just use the first alternative's bindings
                // (they should all be the same, which will be checked recursively)
                if let Some(first) = patterns.first() {
                    self.collect_pattern_bindings(first, ty)
                } else {
                    Ok(List::new())
                }
            }

            PatternKind::Range { .. } => {
                // Range patterns don't bind variables
                Ok(List::new())
            }

            PatternKind::Rest => {
                // Rest pattern (..) - doesn't bind variables directly
                Ok(List::new())
            }

            PatternKind::Array(patterns) => {
                // Array pattern: collect bindings from each element pattern
                if let Type::Array { element, .. } = ty {
                    let mut all_bindings = List::new();
                    for pat in patterns {
                        let mut bindings = self.collect_pattern_bindings(pat, element)?;
                        all_bindings.append(&mut bindings);
                    }
                    Ok(all_bindings)
                } else {
                    Ok(List::new())
                }
            }

            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // Slice pattern: collect bindings from before, rest, and after patterns
                if let Type::Array { element, .. } = ty {
                    let mut all_bindings = List::new();

                    // Collect from before patterns
                    for pat in before {
                        let mut bindings = self.collect_pattern_bindings(pat, element)?;
                        all_bindings.append(&mut bindings);
                    }

                    // Collect from rest pattern if present
                    if let Some(rest_pat) = rest {
                        let rest_ty = Type::array(element.as_ref().clone(), None);
                        let mut bindings = self.collect_pattern_bindings(rest_pat, &rest_ty)?;
                        all_bindings.append(&mut bindings);
                    }

                    // Collect from after patterns
                    for pat in after {
                        let mut bindings = self.collect_pattern_bindings(pat, element)?;
                        all_bindings.append(&mut bindings);
                    }

                    Ok(all_bindings)
                } else {
                    Ok(List::new())
                }
            }

            PatternKind::Reference { inner, .. } => {
                // Reference pattern: collect bindings from dereferenced pattern
                let inner_ty = match ty {
                    Type::Reference { inner, .. } => inner.as_ref(),
                    Type::CheckedReference { inner, .. } => inner.as_ref(),
                    Type::UnsafeReference { inner, .. } => inner.as_ref(),
                    _ => return Ok(List::new()),
                };
                self.collect_pattern_bindings(inner, inner_ty)
            }

            PatternKind::Paren(inner) => {
                // Parenthesized pattern: just collect from inner
                self.collect_pattern_bindings(inner, ty)
            }

            PatternKind::Record { fields, .. } => {
                // Record pattern: collect bindings from each field pattern
                if let Type::Record(field_types) = ty {
                    let mut all_bindings = List::new();

                    for field_pat in fields {
                        let field_name = field_pat.name.name.as_str();

                        if let Some(field_ty) = field_types.get(field_name) {
                            // If pattern is provided, collect from it; otherwise bind field name
                            if let Some(ref pat) = field_pat.pattern {
                                let mut bindings = self.collect_pattern_bindings(pat, field_ty)?;
                                all_bindings.append(&mut bindings);
                            } else {
                                // Shorthand: { x, y } means { x: x, y: y }
                                all_bindings.push((verum_common::Text::from(field_name), field_ty.clone()));
                            }
                        }
                    }

                    Ok(all_bindings)
                } else {
                    Ok(List::new())
                }
            }

            #[allow(deprecated)]
            PatternKind::View { pattern, .. } => {
                // View patterns: collect bindings from the inner pattern
                self.collect_pattern_bindings(pattern, ty)
            }

            PatternKind::Active { .. } => {
                // Active patterns don't bind variables directly
                // (the pattern result is matched, not bound)
                Ok(List::new())
            }

            PatternKind::And(patterns) => {
                // And patterns: collect bindings from all sub-patterns
                let mut all_bindings = List::new();
                for pat in patterns.iter() {
                    let mut bindings = self.collect_pattern_bindings(pat, ty)?;
                    all_bindings.append(&mut bindings);
                }
                Ok(all_bindings)
            }

            PatternKind::TypeTest { binding, test_type } => {
                // TypeTest pattern binds the variable to the narrowed type.
                // When the test type is a variant constructor of the scrutinee,
                // use the scrutinee type (consistent with bind_pattern logic).
                // This ensures or-patterns like `x is Dog | x is Cat` have
                // compatible binding types (both get `Animal`, not `Dog`/`Cat`).
                let variant_name_str = match &test_type.kind {
                    verum_ast::ty::TypeKind::Path(path) if path.segments.len() == 1 => {
                        if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                            Some(ident.name.as_str().to_string())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let narrowed_type = if let Some(ref name) = variant_name_str {
                    let expanded = self.expand_generic_to_variant(ty);
                    if let Type::Variant(variants) = expanded {
                        if let Some(payload_ty) = variants.get(name.as_str()) {
                            match payload_ty {
                                Type::Unit => ty.clone(),
                                other => other.clone(),
                            }
                        } else {
                            self.ast_to_type_lenient(test_type)
                        }
                    } else {
                        self.ast_to_type_lenient(test_type)
                    }
                } else {
                    self.ast_to_type_lenient(test_type)
                };
                Ok(List::from_iter([(binding.name.clone(), narrowed_type)]))
            }

            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: collect bindings from head patterns and optional rest binding
                // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 18.3 - Stream Pattern Matching
                let mut all_bindings = List::new();

                // Collect bindings from each head pattern
                // Element type is inferred from iterator - use fresh variable for now
                let elem_ty = Type::Var(TypeVar::fresh());
                for pat in head_patterns.iter() {
                    let mut bindings = self.collect_pattern_bindings(pat, &elem_ty)?;
                    all_bindings.append(&mut bindings);
                }

                // If there's a rest binding, add it (binds to iterator type)
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    all_bindings.push((rest_ident.name.clone(), ty.clone()));
                }

                Ok(all_bindings)
            }

            PatternKind::Guard { pattern, .. } => {
                // Guard pattern: (pattern if expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                //
                // Collect bindings from the inner pattern only.
                // The guard expression doesn't introduce new bindings.
                self.collect_pattern_bindings(pattern, ty)
            }

            PatternKind::Cons { head, tail } => {
                // Cons pattern: head :: tail
                let elem_ty = Type::Var(crate::ty::TypeVar::fresh());
                let mut bindings = self.collect_pattern_bindings(head, &elem_ty)?;
                bindings.extend(self.collect_pattern_bindings(tail, ty)?);
                Ok(bindings)
            }
        }
    }

    /// Check if a pattern contains complex forms that the exhaustiveness checker
    /// cannot reliably analyze (TypeTest patterns, nested arrays, view patterns, etc.)
    pub(crate) fn pattern_has_complex_forms(pattern: &verum_ast::pattern::Pattern) -> bool {
        use verum_ast::pattern::PatternKind;
        match &pattern.kind {
            PatternKind::TypeTest { .. } => true,
            PatternKind::And(pats) => pats.iter().any(Self::pattern_has_complex_forms),
            PatternKind::Or(pats) => pats.iter().any(Self::pattern_has_complex_forms),
            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(data) = data {
                    match data {
                        verum_ast::pattern::VariantPatternData::Tuple(pats) => {
                            pats.iter().any(Self::pattern_has_complex_forms)
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            fields.iter().any(|f| {
                                f.pattern.as_ref().is_some_and(Self::pattern_has_complex_forms)
                            })
                        }
                    }
                } else {
                    false
                }
            }
            PatternKind::Array(_) | PatternKind::Slice { .. } => true,
            PatternKind::Tuple(pats) => {
                pats.iter().any(Self::pattern_has_complex_forms)
            }
            PatternKind::Record { .. } => true, // Nested records confuse exhaustiveness checker
            PatternKind::View { .. } => true,
            PatternKind::Paren(inner) => Self::pattern_has_complex_forms(inner),
            PatternKind::Reference { inner, .. } => Self::pattern_has_complex_forms(inner),
            _ => false,
        }
    }

    /// Bind patterns in a tuple-style variant payload.
    ///
    /// Examples:
    /// - `Some(x)` where Some has payload type T
    /// - `Ok(value, error)` where Ok has payload type (T, E)
    /// - `Point(x, y, z)` where Point has payload type (Int, Int, Int)
    pub(crate) fn bind_variant_tuple_payload(
        &mut self,
        tag: &str,
        patterns: &[Pattern],
        payload_ty: &Type,
        span: Span,
        binding_through_ref: bool,
    ) -> Result<()> {
        // Helper to wrap type in reference if binding through ref
        let wrap_if_ref = |ty: &Type| -> Type {
            if binding_through_ref {
                Type::Reference {
                    mutable: false,
                    inner: Box::new(ty.clone()),
                }
            } else {
                ty.clone()
            }
        };

        match payload_ty {
            // Tuple payload: Ok(a, b) with type (T, E)
            // Supports both flat destructuring: Ok(a, b) and wrapped: Ok((a, b))
            Type::Tuple(types) => {
                if patterns.len() == types.len() {
                    // Flat destructuring: Some(idx, gen) matching (Int, U32)
                    // Bind each pattern to its corresponding type
                    for (pat, ty) in patterns.iter().zip(types.iter()) {
                        let effective_ty = wrap_if_ref(ty);
                        self.bind_pattern(pat, &effective_ty)?;
                    }
                    Ok(())
                } else if patterns.len() == 1 {
                    // Single pattern: could be wrapped tuple pattern Some((idx, gen))
                    // or a single binding for the whole tuple Some(pair)
                    // Bind the single pattern to the tuple type
                    let tuple_ty = wrap_if_ref(payload_ty);
                    self.bind_pattern(&patterns[0], &tuple_ty)
                } else {
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Variant '{}' expects {} fields (or 1 tuple pattern), but pattern has {} at {}",
                        tag,
                        types.len(),
                        patterns.len(),
                        span_to_line_col(span)
                    ))))
                }
            }
            // Unit payload: represents a variant with no data (like None)
            // Valid patterns for Unit: empty (), wildcard _, or empty tuple pattern ()
            Type::Unit => {
                if patterns.is_empty() {
                    // No pattern: None() or None - valid for Unit payload
                    Ok(())
                } else if patterns.len() == 1 {
                    // Single pattern for Unit: check if it's a wildcard or empty tuple
                    // Ok(()) is valid (unit pattern matching unit type)
                    // Ok(_) is valid (ignores unit)
                    // Ok(x) should fail (can't bind variable to unit)
                    match &patterns[0].kind {
                        verum_ast::pattern::PatternKind::Wildcard => {
                            Ok(()) // Wildcard ignores the Unit value
                        }
                        verum_ast::pattern::PatternKind::Tuple(elements) if elements.is_empty() => {
                            Ok(()) // Empty tuple pattern () matches Unit type
                        }
                        _ => {
                            // Trying to bind a variable to Unit - this variant has no payload
                            Err(TypeError::Other(verum_common::Text::from(format!(
                                "Variant '{}' has no payload to bind at {}",
                                tag,
                                span_to_line_col(span)
                            ))))
                        }
                    }
                } else {
                    // Multiple patterns for Unit payload is invalid
                    Err(TypeError::Other(verum_common::Text::from(format!(
                        "Variant '{}' has Unit payload, but pattern expects {} field(s) at {}",
                        tag,
                        patterns.len(),
                        span_to_line_col(span)
                    ))))
                }
            }
            // Non-tuple payload treated as single element
            other_ty => {
                if patterns.len() != 1 {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Variant '{}' has payload type {}, expected 1 pattern but got {} at {}",
                        tag,
                        other_ty,
                        patterns.len(),
                        span_to_line_col(span)
                    ))));
                }
                let effective_ty = wrap_if_ref(other_ty);
                self.bind_pattern(&patterns[0], &effective_ty)
            }
        }
    }

    /// Bind patterns in a record-style variant payload.
    ///
    /// Examples:
    /// - `Error { code, message }` where Error has payload { code: Int, message: Text }
    /// - `Person { name, age, .. }` with rest pattern ignoring extra fields
    ///
    /// Record-style variant patterns: matching "Node { left, right }" to destructure named-field variants
    pub(crate) fn bind_variant_record_payload(
        &mut self,
        tag: &str,
        field_patterns: &[verum_ast::pattern::FieldPattern],
        allow_rest: bool,
        payload_ty: &Type,
        span: Span,
        binding_through_ref: bool,
    ) -> Result<()> {
        // Payload must be a record type.
        // Handle both direct Type::Record AND Type::Named that resolves to a record.
        let resolved_payload = match payload_ty {
            Type::Record(_) => payload_ty.clone(),
            Type::Named { path, .. } => {
                // Try to resolve the named type to see if it's a record
                let type_name = path.segments.last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("");
                if let Option::Some(def_ty) = self.ctx.lookup_type(type_name) {
                    def_ty.clone()
                } else {
                    payload_ty.clone()
                }
            }
            _ => payload_ty.clone(),
        };
        if let Type::Record(field_types) = &resolved_payload {
            // Track which fields have been matched
            let mut matched_fields = indexmap::IndexSet::new();

            // Check each field pattern
            for field_pat in field_patterns {
                let field_name = field_pat.name.name.as_str();

                // Look up the field type in the payload
                match field_types.get(field_name) {
                    Some(field_ty) => {
                        matched_fields.insert(field_name);

                        // When matching through a reference, bind field as reference
                        // e.g., match &tree { Tree.Node { value, ... } => ... }
                        // value should be &Int, not Int
                        let effective_field_ty = if binding_through_ref {
                            Type::Reference {
                                mutable: false,
                                inner: Box::new(field_ty.clone()),
                            }
                        } else {
                            field_ty.clone()
                        };

                        // Bind the pattern (or use shorthand)
                        if let Some(ref pat) = field_pat.pattern {
                            // Explicit pattern: { code: c, message: msg }
                            self.bind_pattern(pat, &effective_field_ty)?;
                        } else {
                            // Shorthand: { code, message } means { code: code, message: message }
                            self.ctx
                                .env
                                .insert(field_name.to_text(), TypeScheme::mono(effective_field_ty));
                        }
                    }
                    None => {
                        // Field not found in payload type
                        let available: List<_> = field_types.keys().map(|s| s.as_str()).collect();
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "field '{}' not found in type '{}' payload at {}. Available fields: [{}]",
                            field_name,
                            tag,
                            field_pat.span,
                            available.join(", ")
                        ))));
                    }
                }
            }

            // Check if all required fields are matched (unless rest pattern is used)
            if !allow_rest {
                for (field_name, _) in field_types.iter() {
                    if !matched_fields.contains(field_name.as_str()) {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Missing field '{}' in pattern for variant '{}' at {}. Use '..' to ignore extra fields.",
                            field_name, tag, span
                        ))));
                    }
                }
            }

            Ok(())
        } else {
            // Payload is not a record - cannot use record-style pattern
            Err(TypeError::Other(verum_common::Text::from(format!(
                "Variant '{}' has payload type {}, which is not a record. Cannot use record-style pattern at {}",
                tag, payload_ty, span_to_line_col(span)
            ))))
        }
    }

    /// Register an active pattern declaration in the type environment.
    /// This stores the pattern's parameter types and return type so that
    /// active pattern invocations in match arms can be type-checked.
    /// Spec: grammar/verum.ebnf line 1817 - pattern_def
    pub(crate) fn register_pattern_declaration(&mut self, pattern_decl: &verum_ast::decl::PatternDecl) -> Result<()> {
        let name: Text = pattern_decl.name.name.clone();

        // Register generic type parameters into scope before converting types
        // This is critical for patterns like `pattern First<T>(list: List<T>) -> Maybe<T>`
        let mut type_param_vars: List<TypeVar> = List::new();
        for generic_param in &pattern_decl.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name: param_name, .. } => {
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    let name_text: Text = param_name.name.clone();
                    self.ctx.define_type(name_text, type_var);
                    type_param_vars.push(fresh_var);
                }
                _ => {}
            }
        }

        // Convert parameter types
        let mut param_types = List::new();
        for param in pattern_decl.params.iter() {
            if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &param.kind {
                let param_ty = self.ast_to_type(ty)?;
                param_types.push(param_ty);
            }
        }

        // Convert return type
        let return_ty = self.ast_to_type(&pattern_decl.return_type)?;

        // Store in pattern declarations map
        self.pattern_declarations.insert(name.clone(), (param_types.clone(), return_ty.clone()));

        // Also register the pattern as a function in the type environment
        // so it can be called like a regular function if needed
        let func_ty = Type::Function {
            params: param_types,
            return_type: Box::new(return_ty),
            contexts: None,
            type_params: List::new(),
            properties: None,
        };

        // If the pattern has generic type parameters, create a polymorphic scheme
        if type_param_vars.is_empty() {
            self.ctx.env.insert(name.as_str(), TypeScheme::mono(func_ty));
        } else {
            self.ctx.env.insert(name.as_str(), TypeScheme {
                vars: type_param_vars,
                ty: func_ty,
                implicit_vars: Set::new(),
                var_type_bounds: Map::new(),
                var_protocol_bounds: Map::new(),
                impl_var_count: 0,
            });
        }

        Ok(())
    }

    /// Check if a pattern binds a variable name
    pub(crate) fn pattern_binds_var(&self, pattern: &verum_ast::pattern::Pattern, var_name: &Text) -> bool {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            PatternKind::Ident { name, .. } => name.name.as_str() == var_name.as_str(),
            PatternKind::Tuple(patterns) => {
                patterns.iter().any(|p| self.pattern_binds_var(p, var_name))
            }
            PatternKind::Variant {
                data: Some(data), ..
            } => {
                use verum_ast::pattern::VariantPatternData;
                match data {
                    VariantPatternData::Tuple(patterns) => {
                        patterns.iter().any(|p| self.pattern_binds_var(p, var_name))
                    }
                    VariantPatternData::Record { fields, .. } => {
                        for field in fields {
                            if let Some(ref pat) = field.pattern {
                                if self.pattern_binds_var(pat, var_name) {
                                    return true;
                                }
                            } else if field.name.name.as_str() == var_name.as_str() {
                                return true;
                            }
                        }
                        false
                    }
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields {
                    if let Some(ref pat) = field.pattern {
                        if self.pattern_binds_var(pat, var_name) {
                            return true;
                        }
                    } else if field.name.name.as_str() == var_name.as_str() {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Collect all bindings introduced by a pattern (for capture analysis)
    pub(crate) fn collect_capture_pattern_bindings(
        &self,
        pattern: &verum_ast::pattern::Pattern,
        bindings: &mut std::collections::HashSet<Text>,
    ) {
        use verum_ast::pattern::PatternKind;

        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                bindings.insert(verum_common::Text::from(name.name.as_str()));
            }
            PatternKind::Tuple(patterns) => {
                for p in patterns {
                    self.collect_capture_pattern_bindings(p, bindings);
                }
            }
            PatternKind::Array(patterns) => {
                for p in patterns {
                    self.collect_capture_pattern_bindings(p, bindings);
                }
            }
            PatternKind::Slice { before, rest, after } => {
                for p in before {
                    self.collect_capture_pattern_bindings(p, bindings);
                }
                if let verum_common::Maybe::Some(r) = rest {
                    self.collect_capture_pattern_bindings(r, bindings);
                }
                for p in after {
                    self.collect_capture_pattern_bindings(p, bindings);
                }
            }
            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(data) = data {
                    use verum_ast::pattern::VariantPatternData;
                    match data {
                        VariantPatternData::Tuple(patterns) => {
                            for p in patterns {
                                self.collect_capture_pattern_bindings(p, bindings);
                            }
                        }
                        VariantPatternData::Record { fields, .. } => {
                            for field in fields {
                                if let Some(ref pat) = field.pattern {
                                    self.collect_capture_pattern_bindings(pat, bindings);
                                } else {
                                    bindings.insert(verum_common::Text::from(field.name.name.as_str()));
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields {
                    if let Some(ref pat) = field.pattern {
                        self.collect_capture_pattern_bindings(pat, bindings);
                    } else {
                        bindings.insert(verum_common::Text::from(field.name.name.as_str()));
                    }
                }
            }
            PatternKind::Or(patterns) => {
                // For or-patterns, all alternatives must bind the same variables
                if let Some(first) = patterns.first() {
                    self.collect_capture_pattern_bindings(first, bindings);
                }
            }
            _ => {}
        }
    }


}
