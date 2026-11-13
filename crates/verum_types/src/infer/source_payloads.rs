//! Source field and constructor dependencies are admitted on a worklist. A
//! recursive nominal head is provisional until all bodies in its transaction
//! pass. Transparent aliases cannot supply such a head.
use super::{TypeChecker, source_declarations::SourceTypeDeclaration};
use crate::{Result, TypeError};
use verum_ast::{
    Span, TypeDecl,
    decl::{TypeDeclBody, VariantData},
    ty::{GenericArg, GenericParam, GenericParamKind, Path, PathSegment, TypeKind},
};
use verum_common::{List, Map, Set, Shared, Text};

struct Declaration {
    source: SourceTypeDeclaration,
    owner: Text,
    key: Text,
    span: Span,
}

/// An occurrence inside F<G<T>> stores T only if both selected parameter
/// slots store their argument. References and signatures need no inline edge.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Slot(Text, usize);

#[derive(Clone, Default)]
struct Storage {
    indirect: bool,
    required: List<Slot>,
}

impl Storage {
    fn indirect() -> Self {
        Self {
            indirect: true,
            required: List::new(),
        }
    }

    fn inline(&self, stored: &Set<Slot>) -> bool {
        !self.indirect
            && self
                .required
                .iter()
                .all(|parameter| stored.contains(parameter))
    }
}

struct FieldEdge {
    target: Text,
    span: Span,
    storage: Storage,
}

#[derive(Default)]
struct Payload {
    dependencies: List<FieldEdge>,
    parameters: List<(Slot, Storage)>,
    transparent: bool,
}

fn parameter_name(parameter: &GenericParam) -> Option<&Text> {
    match &parameter.kind {
        GenericParamKind::Type { name, .. }
        | GenericParamKind::HigherKinded { name, .. }
        | GenericParamKind::KindAnnotated { name, .. }
        | GenericParamKind::Const { name, .. }
        | GenericParamKind::Meta { name, .. }
        | GenericParamKind::Context { name }
        | GenericParamKind::Level { name } => Some(&name.name),
        GenericParamKind::Lifetime { .. } => None,
    }
}

fn cycle_error(path: &[Text], back: &Text, span: Span) -> TypeError {
    let mut cycle: List<_> = path.iter().cloned().collect();
    cycle.push(back.clone());
    TypeError::TypeCycle {
        cycle_path: cycle
            .iter()
            .map(Text::as_str)
            .collect::<Vec<_>>()
            .join(" -> ")
            .into(),
        types_in_cycle: cycle,
        span,
    }
}

enum Edges<'a> {
    All,
    Aliases,
    Inline(&'a Set<Slot>),
    Ready(&'a Set<Text>),
}

/// The same iterative DFS orders admission and checks each independent cycle
/// projection. A reference along one path never excuses an inline sibling.
fn visit_payloads(
    checker: &TypeChecker,
    graph: &Map<Text, Payload>,
    nodes: &[Text],
    mode: Edges,
) -> Result<(List<Text>, Set<Text>)> {
    let mut done = Set::new();
    let mut order = List::new();
    let mut forwards = Set::new();
    let mut active = Map::new();
    let mut path = List::new();
    for start in nodes {
        if done.contains(start) {
            continue;
        }
        let mut work = List::from_iter([(start.clone(), 0usize)]);
        active.insert(start.clone(), 0);
        path.push(start.clone());
        while let Some((key, position)) = work.last_mut() {
            checker.check_analysis_control()?;
            let payload = &graph[&*key];
            let Some(edge) = payload.dependencies.get(*position) else {
                let key = path.pop().expect("current DFS frame");
                work.pop();
                active.remove(&key);
                done.insert(key.clone());
                order.push(key);
                continue;
            };
            *position += 1;
            let Some(target) = graph.get(&edge.target) else {
                continue;
            };
            let follow = match mode {
                Edges::All => true,
                Edges::Inline(stored) => edge.storage.inline(stored),
                Edges::Ready(forwards) => !forwards.contains(&edge.target),
                Edges::Aliases => payload.transparent && target.transparent,
            };
            if !follow {
                continue;
            }
            if let Some(&start) = active.get(&edge.target) {
                if matches!(mode, Edges::All) {
                    // An alias cannot be a provisional nominal head. The
                    // second ordering pass cuts only actual nominal targets.
                    let head = path.as_slice()[start..]
                        .iter()
                        .find(|name| !graph[*name].transparent);
                    let Some(head) = head else {
                        return Err(cycle_error(
                            &path.as_slice()[start..],
                            &edge.target,
                            edge.span,
                        ));
                    };
                    forwards.insert(head.clone());
                    continue;
                }
                return Err(cycle_error(
                    &path.as_slice()[start..],
                    &edge.target,
                    edge.span,
                ));
            }
            if done.contains(&edge.target) {
                continue;
            }
            active.insert(edge.target.clone(), path.len());
            path.push(edge.target.clone());
            work.push((edge.target.clone(), 0));
        }
    }
    Ok((order, forwards))
}

/// Least fixed point over declared parameter positions. Each new stored slot
/// wakes only the rules that depend on it. A phantom parameter has no rule.
fn stored_parameters(checker: &TypeChecker, graph: &Map<Text, Payload>) -> Result<Set<Slot>> {
    let mut unlocks: Map<&Slot, List<usize>> = Map::new();
    let mut rules = List::new();
    let mut ready = List::new();
    for node in graph.values() {
        checker.check_analysis_control()?;
        for (slot, condition) in &node.parameters {
            if condition.indirect {
                continue;
            }
            if condition.required.is_empty() {
                ready.push(slot);
                continue;
            }
            let rule = rules.len();
            rules.push((slot, condition.required.len()));
            for input in &condition.required {
                unlocks.entry(input).or_default().push(rule);
            }
        }
    }
    let mut stored = Set::new();
    while let Some(slot) = ready.pop() {
        checker.check_analysis_control()?;
        if !stored.insert(slot.clone()) {
            continue;
        }
        if let Some(dependents) = unlocks.get(&slot) {
            for &rule in dependents {
                let (result, remaining) = &mut rules[rule];
                *remaining -= 1;
                if *remaining == 0 {
                    ready.push(*result);
                }
            }
        }
    }
    Ok(stored)
}

impl TypeChecker {
    fn source_payload_target(&self, path: &Path, span: Span) -> Result<Option<Declaration>> {
        let selected = if let Some(ident) = path.as_ident() {
            self.declaring_source_type_declaration(&ident.name)
        } else {
            self.qualified_source_declaration(&self.path_to_string(path), span)?
                .and_then(|(_, source)| source)
        };
        Ok(selected.map(|(source, owner)| Declaration {
            key: Self::source_type_declaration_key(&owner, &source.name.name),
            source,
            owner,
            span,
        }))
    }

    fn source_payload(
        &self,
        declaration: &Declaration,
        mut discover: impl FnMut(Declaration),
    ) -> Result<Payload> {
        let source = &declaration.source;
        let mut result = Payload::default();
        let alias = self.variant_decl_alias_target(source);
        let slots: Map<_, _> = source
            .generics
            .iter()
            .filter_map(parameter_name)
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect();
        let mut work = List::new();
        let mut field = |ty, bound, storage| work.push((ty, bound, storage));
        let empty: &[GenericParam] = &[];
        match (&source.body, alias.as_ref()) {
            (_, Some(ty)) | (TypeDeclBody::Alias(ty), None) => {
                result.transparent = true;
                field(ty, empty, Storage::default());
            }
            (TypeDeclBody::Newtype(ty) | TypeDeclBody::Quotient { base: ty, .. }, _) => {
                field(ty, empty, Storage::default())
            }
            (TypeDeclBody::Record(fields), _) => {
                for member in fields.iter().rev() {
                    field(&member.ty, empty, Storage::default());
                }
            }
            (TypeDeclBody::Tuple(types) | TypeDeclBody::SigmaTuple(types), _) => {
                for ty in types.iter().rev() {
                    field(ty, empty, Storage::default());
                }
            }
            (TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants), _) => {
                for variant in variants.iter().rev() {
                    let locals = variant.generic_params.as_slice();
                    // An explicit inductive is subject to the existing strict
                    // positivity check. Its recursive constructor is not an
                    // inline nominal record-layout dependency.
                    let storage = if matches!(source.body, TypeDeclBody::Inductive(_)) {
                        Storage::indirect()
                    } else {
                        Storage::default()
                    };
                    match &variant.data {
                        Some(VariantData::Tuple(types)) => {
                            for ty in types.iter().rev() {
                                field(ty, locals, storage.clone());
                            }
                        }
                        Some(VariantData::Record(fields)) => {
                            for member in fields.iter().rev() {
                                field(&member.ty, locals, storage.clone());
                            }
                        }
                        None => {}
                    }
                }
            }
            _ => {}
        }
        while let Some((ty, locals, storage)) = work.pop() {
            self.check_analysis_control()?;
            let lexical = |path: &Path| -> Option<Option<usize>> {
                let PathSegment::Name(first) = path.segments.first()? else {
                    return None;
                };
                if locals
                    .iter()
                    .filter_map(parameter_name)
                    .any(|name| name == &first.name)
                {
                    return Some(None);
                }
                slots.get(&first.name.as_str()).copied().map(Some)
            };
            match &ty.kind {
                TypeKind::Path(path) => {
                    if let Some(parameter) = lexical(path) {
                        if path.is_single()
                            && let Some(index) = parameter
                        {
                            result
                                .parameters
                                .push((Slot(declaration.key.clone(), index), storage));
                        }
                    } else if let Some(target) = self.source_payload_target(path, ty.span)? {
                        result.dependencies.push(FieldEdge {
                            target: target.key.clone(),
                            span: ty.span,
                            storage,
                        });
                        discover(target);
                    }
                }
                TypeKind::Generic { base, args } => {
                    // Bind type arguments to the selected head's declared slots.
                    // A missing source layout cannot establish indirection.
                    let head = match &base.kind {
                        TypeKind::Path(path) if lexical(path).is_none() => {
                            self.source_payload_target(path, base.span)?
                        }
                        _ => None,
                    };
                    let mut formals = head.as_ref().into_iter().flat_map(|node| {
                        node.source
                            .generics
                            .iter()
                            .filter(|param| parameter_name(param).is_some())
                            .enumerate()
                            .map(|(index, formal)| (&node.key, index, formal))
                    });
                    for argument in args {
                        match argument {
                            GenericArg::Lifetime(_) => {}
                            GenericArg::Binding(binding) => {
                                work.push((&binding.ty, locals, Storage::indirect()));
                            }
                            GenericArg::Const(_) => {
                                formals.next();
                            }
                            GenericArg::Type(arg) => {
                                let mut condition = storage.clone();
                                if let Some((owner, index, formal)) = formals.next() {
                                    if matches!(
                                        formal.kind,
                                        GenericParamKind::Meta { .. }
                                            | GenericParamKind::Const { .. }
                                            | GenericParamKind::Level { .. }
                                            | GenericParamKind::Context { .. }
                                    ) {
                                        continue;
                                    }
                                    if !condition.indirect {
                                        condition.required.push(Slot(owner.clone(), index));
                                    }
                                }
                                work.push((arg, locals, condition));
                            }
                        }
                    }
                    if let Some(head) = head {
                        result.dependencies.push(FieldEdge {
                            target: head.key.clone(),
                            span: base.span,
                            storage,
                        });
                        discover(head);
                    } else {
                        work.push((base, locals, storage));
                    }
                }
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. }
                | TypeKind::Pointer { inner, .. }
                | TypeKind::VolatilePointer { inner, .. }
                | TypeKind::GenRef { inner }
                | TypeKind::Slice(inner) => work.push((inner, locals, Storage::indirect())),
                TypeKind::Tuple(elements) => work.extend(
                    elements
                        .iter()
                        .rev()
                        .map(|ty| (ty, locals, storage.clone())),
                ),
                TypeKind::Array { element, .. } | TypeKind::Tensor { element, .. } => {
                    work.push((element, locals, storage))
                }
                TypeKind::Record { fields, .. } => work.extend(
                    fields
                        .iter()
                        .rev()
                        .map(|field| (&field.ty, locals, storage.clone())),
                ),
                TypeKind::Function {
                    params,
                    return_type,
                    ..
                } => {
                    work.push((return_type, locals, Storage::indirect()));
                    work.extend(
                        params
                            .iter()
                            .rev()
                            .map(|ty| (ty, locals, Storage::indirect())),
                    );
                }
                TypeKind::DependentApp { carrier: base, .. }
                | TypeKind::Bounded { base, .. }
                | TypeKind::Refined { base, .. }
                | TypeKind::CapabilityRestricted { base, .. }
                | TypeKind::Ownership { inner: base, .. } => work.push((base, locals, storage)),
                // Projections, quantified binders and value expressions still
                // resolve through the declaration checker's semantic scope.
                _ => {}
            }
        }
        Ok(result)
    }

    pub(super) fn register_source_payloads(
        &mut self,
        source: &TypeDecl,
        owner: &Text,
        root: &Text,
        span: Span,
    ) -> Result<(crate::Type, Text)> {
        let mut pending = List::from_iter([Declaration {
            key: root.clone(),
            owner: owner.clone(),
            span,
            source: SourceTypeDeclaration::Owned(Shared::new(source.clone())),
        }]);
        let mut graph = Map::new();
        let mut sources = Map::new();
        let mut names = List::new();
        while let Some(node) = pending.pop() {
            self.check_analysis_control()?;
            if graph.contains_key(&node.key) {
                continue;
            }
            let already_registered = self.source_type_registration.declaration(&node.key);
            if node.key != *root
                && let Some(existing) = already_registered
            {
                if existing != &*node.source {
                    return Err(TypeError::UniverseBinding {
                        message: format!(
                            "conflicting source declarations while loading {}",
                            node.key
                        )
                        .into(),
                        span: node.span,
                    });
                }
                self.require_source_dependencies(&node.key, node.span)?;
                // Completed generic declarations can still own an inline slot
                // in this traversal. Preserve their actual layout evidence.
                if node.source.generics.is_empty() {
                    continue;
                }
            }
            let payload = self.with_source_signature_scope(&node.owner, |checker| {
                checker.source_payload(&node, |child| pending.push(child))
            })?;
            names.push(node.key.clone());
            graph.insert(node.key.clone(), payload);
            sources.insert(node.key.clone(), node);
        }
        let (mut order, forwards) = visit_payloads(self, &graph, &names, Edges::All)?;
        if !forwards.is_empty() {
            visit_payloads(self, &graph, &names, Edges::Aliases)?;
            visit_payloads(
                self,
                &graph,
                &names,
                Edges::Inline(&stored_parameters(self, &graph)?),
            )?;
            // Within a recursive group, load aliases against an actual nominal
            // head before elaborating the body that consumes the alias.
            order = visit_payloads(self, &graph, &names, Edges::Ready(&forwards))?.0;
        }
        let installed: List<_> = forwards
            .into_iter()
            .filter(|key| {
                !self
                    .source_type_registration
                    .forward_payloads
                    .contains_key(key)
            })
            .collect();
        for key in &installed {
            self.source_type_registration
                .forward_payloads
                .insert(key.clone(), sources[key].source.clone());
        }
        let registration = (|| {
            for key in order {
                self.check_analysis_control()?;
                let node = &sources[&key];
                if key != *root && self.source_type_registration.declaration(&key).is_some() {
                    continue;
                }
                self.register_source_declaration_in_scope(
                    &node.source,
                    &node.owner,
                    &key,
                    node.span,
                )?;
            }
            self.ctx
                .lookup_type(root.as_str())
                .cloned()
                .map(|ty| (ty, root.clone()))
                .ok_or_else(|| TypeError::TypeNotFound {
                    name: root.clone(),
                    span,
                })
        })();
        for key in installed {
            self.source_type_registration.forward_payloads.remove(&key);
        }
        registration
    }
}
