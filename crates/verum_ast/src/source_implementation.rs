//! A bounded, immutable source authority for an implementation's declaration.
//! Equality and hashing use its canonical encoding; no unhashable AST field is
//! omitted from identity. Decoding remains typed and validated before replay.
use crate::decl::{ImplDecl, ImplItemKind, ImplKind};
use serde::{Deserialize, Deserializer, Serialize};
use verum_common::{List, Map, Text};

const MAX_BYTES: usize = 1024 * 1024;

/// A structural product has no nominal declaration. Its method catalogue
/// groups headers by arity; the header still selects the complete receiver.
pub fn product_method_catalogue(arity: usize) -> Text {
    let mut name = Text::from("(");
    for index in 0..arity {
        if index > 0 { name.push_str(", "); }
        name.push('_');
    }
    if arity == 1 { name.push(','); }
    name.push(')');
    name
}

pub fn product_method_arity(name: &str) -> Option<usize> {
    let parts = name.strip_prefix('(')?.strip_suffix(')')?;
    if parts == "_," { return Some(1); }
    let mut arity = 0;
    for part in parts.split(", ") {
        if part != "_" { return None; }
        arity += 1;
    }
    (arity >= 2).then_some(arity)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct AssociatedImplementationSource {
    /// The empty path is the explicitly selected anonymous source root. This
    /// required field is distinct from absent declaration ownership on the wire.
    origin_module: Text,
    declaration: Text,
    /// Rebuilt from the validated source on every decode.
    #[serde(skip)]
    unconditional_members: List<Text>,
}

fn parse(payload: &str) -> Result<ImplDecl, Text> {
    if payload.len() > MAX_BYTES {
        return Err("associated implementation source exceeds its byte budget".into());
    }
    // Leave serde_json's recursion limit enabled before allocating the AST.
    let declaration: ImplDecl = serde_json::from_str(payload).map_err(|error| {
        Text::from(format!("invalid associated implementation source: {error}"))
    })?;
    crate::source_signature::validate_source_associated_implementation(&declaration)?;
    Ok(declaration)
}

impl AssociatedImplementationSource {
    /// Capture the real implementation header and declarations, removing method
    /// bodies. Constants and proof obligations remain exact source; this is
    /// declaration evidence, not a conformance or proof certificate.
    pub fn new(origin_module: Text, declaration: &ImplDecl) -> Result<Self, Text> {
        let declaration = crate::source_signature::source_associated_implementation(declaration)?;
        Self::from_retained(origin_module, &declaration)
    }

    pub fn new_with_items(
        origin_module: Text,
        declaration: &ImplDecl,
        items: &[&crate::decl::ImplItem],
    ) -> Result<Self, Text> {
        let retained =
            crate::source_signature::source_associated_implementation_items(declaration, items)?;
        Self::from_retained(origin_module, &retained)
    }

    fn from_retained(origin_module: Text, declaration: &ImplDecl) -> Result<Self, Text> {
        if origin_module.len() > MAX_BYTES {
            return Err("associated implementation source owner exceeds its byte budget".into());
        }
        if verum_common::qualified_name::has_empty_module_segment(&origin_module) {
            return Err("associated implementation source has a malformed module owner".into());
        }
        let payload =
            serde_json::to_string(declaration).map_err(|error| Text::from(error.to_string()))?;
        // Successful writers must produce payloads accepted by bounded readers.
        parse(&payload)?;
        Ok(Self {
            origin_module,
            declaration: payload.into(),
            unconditional_members: Self::unconditional_members(declaration),
        })
    }

    pub fn origin_module(&self) -> &str {
        self.origin_module.as_str()
    }

    pub fn declaration(&self) -> Result<ImplDecl, Text> {
        parse(self.declaration.as_str())
    }

    /// The owner-scoped runtime table can use a plain assignment without
    /// reopening the source AST at each element of a generic iterator.
    pub fn has_unconditional_member(&self, name: &str) -> bool {
        self.unconditional_members.binary_search_by(|member| member.as_str().cmp(name)).is_ok()
    }

    fn unconditional_members(declaration: &ImplDecl) -> List<Text> {
        use crate::ty::{GenericParamKind, PathSegment, TypeKind};
        let ImplKind::Protocol { protocol_args, for_type, .. } = &declaration.kind else { return List::new(); };
        if !protocol_args.is_empty() || declaration.specialize_attr.is_some()
            || declaration.generic_where_clause.iter().chain(&declaration.meta_where_clause)
                .any(|clause| !clause.predicates.is_empty())
        { return List::new(); }
        let mut parameters = verum_common::Set::new();
        for parameter in &declaration.generics {
            match &parameter.kind {
                GenericParamKind::Type { name, bounds, .. } if bounds.is_empty() => {
                    if !parameters.insert(name.name.clone()) { return List::new(); }
                }
                GenericParamKind::Lifetime { .. } => {}
                _ => return List::new(),
            }
        }
        // The receiver must cover every argument of the nominal owner.
        // A repeated parameter or a concrete specialization covers only part.
        match &for_type.kind {
            TypeKind::Path(_) if parameters.is_empty() => {}
            TypeKind::Generic { base, args } if matches!(base.kind, TypeKind::Path(_)) => {
                for argument in args {
                    let Some(argument) = argument.type_for_declared_slot() else { return List::new(); };
                    let TypeKind::Path(path) = argument.kind else { return List::new(); };
                    let [PathSegment::Name(name)] = path.segments.as_slice() else { return List::new(); };
                    if !parameters.remove(&name.name) { return List::new(); }
                }
                if !parameters.is_empty() { return List::new(); }
            }
            _ => return List::new(),
        }
        let mut plain = Map::new();
        for item in &declaration.items {
            if let ImplItemKind::Type { name, type_params, .. } = &item.kind {
                plain.entry(name.name.clone()).and_modify(|unique| *unique = false)
                    .or_insert(type_params.is_empty());
            }
        }
        let mut members: List<_> = plain.into_iter().filter_map(|(name, unique)| unique.then_some(name)).collect();
        members.sort();
        members
    }
}

impl<'de> Deserialize<'de> for AssociatedImplementationSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Payload {
            origin_module: Text,
            declaration: Text,
        }
        let Payload {
            origin_module,
            declaration,
        } = Payload::deserialize(deserializer)?;
        let parsed = parse(declaration.as_str()).map_err(serde::de::Error::custom)?;
        // Normalize the encoding, retaining every typed field. Whitespace or key
        // ordering must not manufacture a distinct implementation identity.
        Self::from_retained(origin_module, &parsed).map_err(serde::de::Error::custom)
    }
}
