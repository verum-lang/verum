//! Bounded, simultaneous substitution with an explicit postorder work stack.
//!
//! Source trees are borrowed. Only binder scopes and results own new data.
//! Application reduction re-enters the same queue with owned intermediate
//! nodes; neither ordinary nesting nor reduction consumes the Rust call stack.
use super::{Substitution, Type, TypeVar};
use std::rc::Rc;
use verum_common::{List, Text};

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
}

impl Scope<'_> {
    fn substitution(&self) -> &Substitution {
        match self {
            Self::Borrowed(substitution) => substitution,
            Self::Owned(substitution) => substitution,
        }
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
    },
}

pub(super) fn apply(
    ty: &Type,
    substitution: &Substitution,
    max_depth: usize,
) -> Result<Type, Text> {
    let mut work = vec![Work::Visit {
        node: Node::Borrowed(ty),
        scope: Scope::Borrowed(substitution),
        depth: 0,
    }];
    let mut results = Vec::new();
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
                // Replacement ranges remain untouched: this is simultaneous
                // substitution, including value-indexed type replacements.
                let replacement = match node.ty() {
                    Type::Var(variable) => scope.substitution().get(variable),
                    Type::Meta { value, .. } => value
                        .id()
                        .and_then(|id| scope.substitution().value_bindings.get(&id)),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    results.push(replacement.clone());
                    continue;
                }

                let binders = match node.ty() {
                    Type::Forall { vars, body } => Some((vars.as_slice(), body)),
                    Type::Exists { var, body } => Some((std::slice::from_ref(var), body)),
                    _ => None,
                };
                let renamed = if let Some((binders, body)) = binders {
                    let (renamed, scoped) = body.substitution_under_binders(
                        binders,
                        scope.substitution(),
                        max_depth - depth - 1,
                    )?;
                    scope = Scope::Owned(Rc::new(scoped));
                    Some(renamed)
                } else {
                    None
                };

                let mut children = Vec::new();
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
            } => {
                let mut values = results.drain(results.len() - children..);
                shape.for_each_child_mut(|child| {
                    *child = values
                        .next()
                        .expect("each child has one substitution result");
                });
                debug_assert!(values.next().is_none());
                drop(values);
                match reduce_application(*shape) {
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
    substitution: &Substitution,
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
            *level = substitution.universes.apply(level)?;
        }
        Type::ExtensibleRecord { row_var, .. } => {
            if let Some(Type::Var(replacement)) = row_var.and_then(|v| substitution.get(&v)) {
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
