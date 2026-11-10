//! Declaration facts used by exact protocol proof, independently of dispatch names.
use super::{AssociatedTypeAssignment, ProtocolChecker};
use crate::{Type, unify::Unifier};
use verum_ast::{Ident, Path, Span, decl::TypeDeclBodyKind};
use verum_common::{List, Map, Text, well_known_types::type_names};

#[derive(Clone, Default)]
pub(super) struct TypeAuthority {
    builtins: Map<Text, Text>,
    representatives: Map<Text, Type>,
    declarations: Map<Text, (TypeDeclBodyKind, Option<usize>)>,
    aliases: Map<Text, AssociatedTypeAssignment>,
}

enum HeadIdentity {
    Builtin(Text),
    Nominal,
}

fn nominal_key(ty: &Type) -> Option<(Text, usize)> {
    match ty {
        Type::Named { path, args } => {
            let names: Option<List<_>> = path
                .segments
                .iter()
                .map(|part| match part {
                    verum_ast::ty::PathSegment::Name(name) => Some(name.name.as_str()),
                    _ => None,
                })
                .collect();
            let key: Text = names?.join(".").into();
            (!key.is_empty() && key.as_str().split('.').all(|part| !part.is_empty()))
                .then_some((key, args.len()))
        }
        Type::Generic { name, args }
            if !name.is_empty()
                && !name.starts_with("::")
                && name.as_str().split('.').all(|part| !part.is_empty()) =>
        {
            Some((name.clone(), args.len()))
        }
        _ => None,
    }
}

fn scalar_identity(name: &str) -> Text {
    let identity = type_names::canonical_identity_name(name);
    if identity == type_names::NON_PRIMITIVE_IDENTITY {
        name.into()
    } else {
        identity.into()
    }
}

impl ProtocolChecker {
    /// Called only by the checker that actually declares language builtins.
    /// Canonical scalar identity is shared language authority, not a protocol
    /// spelling heuristic. Ordinary source declarations never call this API.
    pub(crate) fn register_builtin_type_identity(&mut self, ty: &Type) {
        if matches!(ty, Type::Unknown) {
            return;
        }
        let (name, primitive) = if let Some(name) = ty.primitive_name() {
            (Text::from(name), true)
        } else if let Some((key, 0)) = nominal_key(ty) {
            if key.contains(".")
                || type_names::canonical_identity_name(&key) == type_names::NON_PRIMITIVE_IDENTITY
            {
                return;
            }
            (key, false)
        } else {
            return;
        };
        let identity = scalar_identity(&name);
        self.type_authority.builtins.insert(name, identity.clone());
        self.type_authority
            .builtins
            .insert(identity.clone(), identity.clone());
        let representative = if primitive {
            ty.clone()
        } else {
            Type::Named {
                path: Path::single(Ident::new(identity.clone(), Span::default())),
                args: List::new(),
            }
        };
        if primitive || !self.type_authority.representatives.contains_key(&identity) {
            self.type_authority
                .representatives
                .insert(identity, representative);
        }
        self.invalidate_impl_cache();
    }

    /// Source callers publish only after the owning declaration transaction
    /// succeeds. Alias/protocol kinds are retained too, so they cannot borrow a
    /// previous builtin or record fact with the same exact declaration key.
    pub(crate) fn register_type_declaration_kind(
        &mut self,
        key: Text,
        kind: TypeDeclBodyKind,
        arity: Option<usize>,
        alias: Option<AssociatedTypeAssignment>,
    ) {
        self.type_authority.aliases.remove(&key);
        if kind == TypeDeclBodyKind::Alias {
            if let Some(alias) = alias {
                self.type_authority.aliases.insert(key.clone(), alias);
            }
        }
        self.type_authority.declarations.insert(key, (kind, arity));
        self.invalidate_impl_cache();
    }

    fn head_identity(&self, ty: &Type) -> Option<HeadIdentity> {
        if !matches!(ty, Type::Unknown)
            && let Some(name) = ty.primitive_name()
        {
            return Some(HeadIdentity::Builtin(scalar_identity(name)));
        }
        let (key, arity) = nominal_key(ty)?;
        if let Some((kind, declared_arity)) = self.type_authority.declarations.get(&key) {
            return (*declared_arity == Some(arity)
                && (kind.is_product_type()
                    || kind.is_sum_type()
                    || matches!(kind, TypeDeclBodyKind::Newtype | TypeDeclBodyKind::Unit)))
            .then_some(HeadIdentity::Nominal);
        }
        if arity != 0 {
            return None;
        }
        self.type_authority
            .builtins
            .get(&key)
            .cloned()
            .map(HeadIdentity::Builtin)
    }

    pub(super) fn has_declared_alias_head(&self, ty: &Type) -> bool {
        nominal_key(ty).is_some_and(|(key, _)| {
            self.type_authority
                .declarations
                .get(&key)
                .is_some_and(|(kind, _)| *kind == TypeDeclBodyKind::Alias)
        })
    }

    /// Only a completed source declaration owns this telescope and body.
    /// An unsupported role, malformed application or cycle supplies no proof.
    pub(super) fn expand_declared_alias_head(&self, ty: &Type) -> Option<Type> {
        let mut current = ty.clone();
        let mut visited = verum_common::Set::new();
        let mut changed = false;
        while self.has_declared_alias_head(&current) {
            let (key, _) = nominal_key(&current)?;
            if !visited.insert(key.clone()) {
                return None;
            }
            let args = match &current {
                Type::Named { args, .. } | Type::Generic { args, .. } => args,
                _ => return None,
            };
            current = self
                .type_authority
                .aliases
                .get(&key)?
                .instantiate(args, &crate::ty::Substitution::new())
                .ok()?;
            changed = true;
        }
        changed.then_some(current)
    }

    pub(crate) fn builtin_source_receiver_name(&self, ty: &Type) -> Option<Text> {
        match self.head_identity(ty)? {
            HeadIdentity::Builtin(identity) => Some(identity),
            HeadIdentity::Nominal => None,
        }
    }

    pub(super) fn requires_unfiltered_type_candidates(&self, ty: &Type) -> bool {
        self.has_declared_alias_head(ty)
            || matches!(self.head_identity(ty), Some(HeadIdentity::Builtin(_)))
    }

    pub(super) fn builtin_types_equal(&self, left: &Type, right: &Type) -> Option<bool> {
        match (self.head_identity(left), self.head_identity(right)) {
            (Some(HeadIdentity::Builtin(left)), Some(HeadIdentity::Builtin(right))) => {
                Some(left == right)
            }
            _ => None,
        }
    }

    pub(super) fn declared_heads_are_disjoint(&self, left: &Type, right: &Type) -> bool {
        match (self.head_identity(left), self.head_identity(right)) {
            (Some(HeadIdentity::Builtin(left)), Some(HeadIdentity::Builtin(right))) => {
                left != right
            }
            (Some(HeadIdentity::Nominal), Some(HeadIdentity::Builtin(_)))
            | (Some(HeadIdentity::Builtin(_)), Some(HeadIdentity::Nominal)) => true,
            _ => false,
        }
    }

    /// Reuse the unifier's existing recursive transparent-alias substitution.
    /// This changes only a private proof matcher, not method dispatch identity.
    pub(super) fn install_builtin_proof_aliases(&self, matcher: &mut Unifier) {
        for (key, alias) in &self.type_authority.aliases {
            if let Some(body) = alias.ordinary_body() {
                matcher.register_type_alias(key.clone(), body.clone());
            }
        }
        for (name, identity) in &self.type_authority.builtins {
            if self.type_authority.declarations.contains_key(name) {
                continue;
            }
            let Some(target) = self.type_authority.representatives.get(identity) else {
                continue;
            };
            if let Some((key, _)) = nominal_key(target) {
                if &key == name || self.type_authority.declarations.contains_key(&key) {
                    continue;
                }
            }
            matcher.register_type_alias(name.clone(), target.clone());
        }
    }
}
