//! Bounded, simultaneous substitution with an explicit postorder work stack.
//!
//! Source trees are borrowed. Only binder scopes and results own new data.
//! Application reduction re-enters the same queue with owned intermediate
//! nodes; neither ordinary nesting nor reduction consumes the Rust call stack.
use super::{Substitution, Type, TypeVar};
use std::rc::Rc;
use verum_common::{List, Map, Text};

enum Node<'a> {
    Borrowed(&'a Type),
    Owned(Box<Type>),
}

impl Node<'_> {
    fn ty(&self) -> &Type {
        match self {
            Self::Borrowed(ty) => ty,
            Self::Owned(ty) => ty,
        }
    }
}

#[derive(Clone)]
enum Scope<'a> {
    Borrowed(&'a Substitution),
    Owned(Rc<Substitution>),
    Names(&'a Map<Text, Type>, NameAuthority),
    ScopedNames(Rc<Map<Text, Type>>, NameAuthority),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameAuthority {
    Alias,
    DeclaredSlot,
}

impl Scope<'_> {
    fn substitution(&self) -> Option<&Substitution> {
        match self {
            Self::Borrowed(substitution) => Some(substitution),
            Self::Owned(substitution) => Some(substitution),
            Self::Names(..) | Self::ScopedNames(..) => None,
        }
    }

    fn names(&self) -> Option<(&Map<Text, Type>, NameAuthority)> {
        match self {
            Self::Names(names, authority) => Some((names, *authority)),
            Self::ScopedNames(names, authority) => Some((names, *authority)),
            _ => None,
        }
    }

    fn replacement(&self, ty: &Type) -> Option<&Type> {
        if let Some((names, authority)) = self.names() {
            return match ty {
                Type::Var(variable) if authority == NameAuthority::Alias => {
                    names.get(&Text::from(format!("{variable}")))
                }
                Type::Generic { name, args } if args.is_empty() => names.get(name),
                Type::Named { path, args } if args.is_empty() => {
                    path.as_ident().and_then(|name| names.get(&name.name))
                }
                // A value binder's diagnostic name is never its identity.
                _ => None,
            };
        }
        let substitution = self.substitution()?;
        match ty {
            Type::Var(variable) => substitution.get(variable),
            Type::Meta { value, .. } => value
                .id()
                .and_then(|id| substitution.value_bindings.get(&id)),
            _ => None,
        }
    }

    fn under_binders(
        &self,
        binders: &[TypeVar],
        body: &Type,
        depth: usize,
    ) -> Result<(List<TypeVar>, Self), Text> {
        if let Some((names, authority)) = self.names() {
            // Declaration slots become fresh variables. Only the following
            // simultaneous substitution inserts the caller's actual arguments.
            if authority == NameAuthority::DeclaredSlot {
                return Ok((binders.iter().copied().collect(), self.clone()));
            }
            let (renamed, scoped) =
                Type::named_substitution_under_binders(binders, body, names, depth)?;
            Ok((renamed, Self::ScopedNames(Rc::new(scoped), authority)))
        } else {
            let (renamed, scoped) = body.substitution_under_binders(
                binders,
                self.substitution().expect("variable substitution"),
                depth,
            )?;
            Ok((renamed, Self::Owned(Rc::new(scoped))))
        }
    }

    /// Scope a declaration's named slots over this node. The common child
    /// visitor transports type operands. Separate obligations need their own
    /// checked representation before they may cross this boundary.
    fn enter_declaration(&mut self, ty: &Type) -> Result<(), Text> {
        let Some((names, NameAuthority::DeclaredSlot)) = self.names() else {
            return Ok(());
        };
        match ty {
            Type::Function {
                type_params,
                contexts,
                ..
            } => {
                if type_params.iter().any(|parameter| {
                    parameter.is_meta || !parameter.bounds.is_empty() || parameter.default.is_some()
                }) {
                    return Err("declared function carrier requires checked transport of generic parameter obligations".into());
                }
                if contexts.as_ref().is_some_and(|context| match context {
                    crate::di::requirement::ContextExpr::Variable(_) => true,
                    crate::di::requirement::ContextExpr::Concrete(requirements) => {
                        requirements.iter().any(|context| {
                            !context.type_args.is_empty()
                                || context.condition.is_some()
                                || context
                                    .transforms
                                    .iter()
                                    .any(|transform| !transform.args.is_empty())
                        })
                    }
                }) {
                    return Err("declared function carrier requires checked transport of context dependencies".into());
                }
                if type_params
                    .iter()
                    .any(|local| names.contains_key(&local.name))
                {
                    let mut scoped = names.clone();
                    for local in type_params {
                        scoped.remove(&local.name);
                    }
                    *self = Self::ScopedNames(Rc::new(scoped), NameAuthority::DeclaredSlot);
                }
            }
            Type::Meta { .. }
            | Type::Refined { .. }
            | Type::Named { .. }
            | Type::Generic { .. }
            | Type::Record(_)
            | Type::Tuple(_)
            | Type::Variant(_)
            | Type::Reference { .. }
            | Type::CheckedReference { .. }
            | Type::UnsafeReference { .. }
            | Type::Pointer { .. }
            | Type::VolatilePointer { .. }
            | Type::Future { .. }
            | Type::Array { .. }
            | Type::Slice { .. }
            | Type::Forall { .. }
            | Type::Exists { .. }
            | Type::TypeApp { .. }
            | Type::Unit
            | Type::Never
            | Type::Unknown
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Var(_)
            | Type::Lifetime { .. }
            | Type::Universe { .. }
            | Type::Prop
            | Type::Interval => {}
            _ => return Err("declared carrier has no supported named-slot traversal".into()),
        }
        Ok(())
    }
}

enum Work<'a> {
    Visit {
        node: Node<'a>,
        scope: Scope<'a>,
        depth: usize,
    },
    Finish {
        shape: Box<Type>,
        children: usize,
        depth: usize,
        reduce: bool,
    },
}

pub(super) fn apply(
    ty: &Type,
    substitution: &Substitution,
    max_depth: usize,
) -> Result<Type, Text> {
    apply_scoped(ty, Scope::Borrowed(substitution), max_depth)
}

pub(super) fn apply_named(
    ty: &Type,
    names: &Map<Text, Type>,
    max_depth: usize,
) -> Result<Type, Text> {
    apply_scoped(ty, Scope::Names(names, NameAuthority::Alias), max_depth)
}

pub(super) fn apply_declared_slots(
    ty: &Type,
    parameters: &indexmap::IndexMap<Text, Type>,
    max_depth: usize,
) -> Result<Type, Text> {
    let mut slots = Map::new();
    let mut arguments = Substitution::new();
    for (name, actual) in parameters {
        let variable = TypeVar::fresh();
        slots.insert(name.clone(), Type::Var(variable));
        arguments.insert(variable, actual.clone());
    }
    let bound = apply_scoped(
        ty,
        Scope::Names(&slots, NameAuthority::DeclaredSlot),
        max_depth,
    )?;
    bound.try_apply_subst(&arguments)
}

fn apply_scoped<'a>(ty: &'a Type, scope: Scope<'a>, max_depth: usize) -> Result<Type, Text> {
    let mut work = List::from_iter([Work::Visit {
        node: Node::Borrowed(ty),
        scope,
        depth: 0,
    }]);
    let mut results = List::new();
    while let Some(task) = work.pop() {
        match task {
            Work::Visit {
                node,
                mut scope,
                depth,
            } => {
                if depth >= max_depth {
                    return Err("type substitution exceeds its structural depth budget".into());
                }
                scope.enter_declaration(node.ty())?;
                // Replacement ranges remain untouched: this is simultaneous
                // substitution, including value-indexed type replacements.
                if let Some(replacement) = scope.replacement(node.ty()) {
                    results.push(replacement.clone());
                    continue;
                }

                let binders = match node.ty() {
                    Type::Forall { vars, body } => Some((vars.as_slice(), body)),
                    Type::Exists { var, body } => Some((std::slice::from_ref(var), body)),
                    _ => None,
                };
                let renamed = if let Some((binders, body)) = binders {
                    let (renamed, scoped) =
                        scope.under_binders(binders, body, max_depth - depth - 1)?;
                    scope = scoped;
                    Some(renamed)
                } else {
                    None
                };

                let mut children = List::new();
                let mut shape = match node {
                    Node::Borrowed(ty) => Box::new(ty.map_children(|child| {
                        children.push(Node::Borrowed(child));
                        Type::Unit
                    })),
                    Node::Owned(mut ty) => {
                        ty.for_each_child_mut(|child| {
                            children
                                .push(Node::Owned(Box::new(std::mem::replace(child, Type::Unit))));
                        });
                        ty
                    }
                };
                apply_node_metadata(&mut shape, scope.substitution(), renamed)?;
                if children.is_empty() {
                    results.push(*shape);
                    continue;
                }
                work.push(Work::Finish {
                    shape,
                    children: children.len(),
                    depth,
                    reduce: scope.substitution().is_some(),
                });
                for node in children.into_iter().rev() {
                    work.push(Work::Visit {
                        node,
                        scope: scope.clone(),
                        depth: depth + 1,
                    });
                }
            }
            Work::Finish {
                mut shape,
                children,
                depth,
                reduce,
            } => {
                let mut values = results.drain(results.len() - children..);
                shape.for_each_child_mut(|child| {
                    *child = values
                        .next()
                        .expect("each child has one substitution result");
                });
                debug_assert!(values.next().is_none());
                drop(values);
                match if reduce {
                    reduce_application(*shape)
                } else {
                    Application::Ready(*shape)
                } {
                    Application::Ready(ty) => results.push(ty),
                    Application::Substitute(ty, substitution) => work.push(Work::Visit {
                        node: Node::Owned(Box::new(ty)),
                        scope: Scope::Owned(Rc::new(substitution)),
                        depth: depth + 1,
                    }),
                }
            }
        }
    }
    debug_assert_eq!(results.len(), 1);
    Ok(results.pop().expect("the root has one substitution result"))
}

fn apply_node_metadata(
    ty: &mut Type,
    substitution: Option<&Substitution>,
    renamed: Option<List<TypeVar>>,
) -> Result<(), Text> {
    match ty {
        Type::Exists { var, .. } => *var = renamed.expect("existential binder")[0],
        Type::Forall { vars, .. } => *vars = renamed.expect("universal binders"),
        Type::RecordField { level, .. }
        | Type::Universe { level }
        | Type::Inductive {
            universe: level, ..
        } => {
            if let Some(substitution) = substitution {
                *level = substitution.universes.apply(level)?;
            }
        }
        Type::ExtensibleRecord { row_var, .. } => {
            if let Some(Type::Var(replacement)) = row_var.and_then(|v| substitution?.get(&v)) {
                *row_var = Some(*replacement);
            }
        }
        _ => {}
    }
    Ok(())
}

enum Application {
    Ready(Type),
    Substitute(Type, Substitution),
}

fn reduce_application(ty: Type) -> Application {
    let Type::TypeApp { constructor, args } = ty else {
        return Application::Ready(ty);
    };
    let parameters = match &*constructor {
        Type::Named { args, .. } => Some(args),
        Type::Generic { name, args } if !name.as_str().starts_with("::") => Some(args),
        _ => None,
    };
    if let Some(parameters) = parameters {
        let mut substitution = Substitution::new();
        for (parameter, argument) in parameters.iter().zip(args.iter()) {
            if let Type::Var(variable) = parameter {
                substitution.insert(*variable, argument.clone());
            }
        }
        if !substitution.is_empty() {
            return Application::Substitute(*constructor, substitution);
        }
        if args.is_empty() {
            return Application::Ready(*constructor);
        }
    }
    Application::Ready(Type::TypeApp { constructor, args })
}
