//! Single-goal proof construction. Tactics produce ordinary kernel terms;
//! neither goal mutation nor a successful tactic is proof authority.
use super::*;
use verum_common::List;

enum Frame {
    Intro { id: BinderId, domain: Box<Ir> },
    Transport { motive: Box<Ir>, path: Box<Ir> },
}

impl Elab {
    pub(super) fn tactics(&mut self, tactic: &TacticExpr, goal: &Ir) -> Result<Ir> {
        let saved = self.stack.len();
        let result = self.tactics_inner(tactic, goal.clone());
        self.stack.truncate(saved);
        result
    }

    fn check_goal(&self, proof: &Ir, goal: &Ir) -> Result<()> {
        k::check(
            &self.context()?,
            &lower(proof, &mut self.frame())?,
            &lower(goal, &mut self.frame())?,
        )
        .map_err(|error| Error::Rejected(format!("{error:?}")))
    }

    fn tactics_inner(&mut self, tactic: &TacticExpr, mut goal: Ir) -> Result<Ir> {
        let mut pending = List::new();
        pending.push(tactic);
        let mut frames = List::new();
        while let Some(tactic) = pending.pop() {
            let proof = match tactic {
                TacticExpr::Seq(steps) => {
                    pending.extend(steps.iter().rev());
                    continue;
                }
                TacticExpr::Intro(names) => {
                    // An unnamed intro introduces exactly one anonymous binder.
                    for index in 0..names.len().max(1) {
                        let Ir::Pi { id, domain, body } = goal else {
                            return Err(unsupported(
                                "intro expects an explicit dependent function goal",
                            ));
                        };
                        if self.frame().contains(&id) {
                            return Err(Error::Scope("introduced binder identity reused".into()));
                        }
                        self.require_type(&domain)?;
                        self.stack.push(Binding {
                            id,
                            name: names.get(index).map(|name| name.as_str().into()),
                            domain: (*domain).clone(),
                            role: if matches!(*domain, Ir::Universe(_)) {
                                Role::Type
                            } else {
                                Role::Value
                            },
                        });
                        frames.push(Frame::Intro { id, domain });
                        goal = *body;
                    }
                    continue;
                }
                TacticExpr::Rewrite {
                    hypothesis,
                    at_target,
                    rev,
                } => {
                    if at_target.is_some() {
                        return Err(unsupported(
                            "rewriting a hypothesis requires dependent context transport",
                        ));
                    }
                    let (next, frame) = self
                        .rewrite(hypothesis, *rev, &goal)
                        .map_err(|error| self.locate(error, hypothesis.span))?;
                    goal = next;
                    frames.push(frame);
                    continue;
                }
                TacticExpr::Exact(expression) => self.expr(expression)?,
                TacticExpr::Reflexivity => {
                    let Ir::Id { left, .. } = &goal else {
                        return Err(unsupported("reflexivity expects an explicit identity goal"));
                    };
                    Ir::Refl {
                        value: left.clone(),
                    }
                }
                TacticExpr::Assumption => {
                    let context = self.context()?;
                    let expected = lower(&goal, &mut self.frame())?;
                    let mut found = None;
                    for binding in self.stack.iter().rev() {
                        let proof = Ir::Var(binding.id);
                        if k::check(&context, &lower(&proof, &mut self.frame())?, &expected).is_ok()
                        {
                            found = Some(proof);
                            break;
                        }
                    }
                    found.ok_or_else(|| {
                        Error::Rejected("no assumption has the requested type".into())
                    })?
                }
                _ => {
                    return Err(unsupported(
                        "supported tactics are intro, rewrite, exact, assumption, reflexivity and sequences",
                    ));
                }
            };
            self.check_goal(&proof, &goal)
                .map_err(|error| match tactic {
                    TacticExpr::Exact(expression) => self.locate(error, expression.span),
                    _ => error,
                })?;
            if !pending.is_empty() {
                return Err(Error::Rejected(
                    "tactic follows an already completed proof".into(),
                ));
            }
            return Ok(frames
                .into_iter()
                .rev()
                .fold(proof, |proof, frame| match frame {
                    Frame::Intro { id, domain } => Ir::Lam {
                        id,
                        domain,
                        body: Box::new(proof),
                    },
                    Frame::Transport { motive, path } => Ir::J {
                        motive,
                        base: Box::new(proof),
                        scrutinee: path,
                    },
                }));
        }
        Err(Error::Rejected(
            "tactic sequence leaves an unsolved goal".into(),
        ))
    }

    fn rewrite(&mut self, hypothesis: &Expr, reverse: bool, goal: &Ir) -> Result<(Ir, Frame)> {
        let path = self.expr(hypothesis)?;
        let path_type = self.infer(&path)?;
        let Ir::Id {
            carrier,
            left,
            right,
        } = self.raise(&path_type, &mut self.frame())?
        else {
            return Err(Error::Rejected("rewrite expects an identity proof".into()));
        };
        let (from, to) = if reverse {
            (&right, &left)
        } else {
            (&left, &right)
        };
        let id = self.fresh();
        let (body, changed) = replace(goal, from, &Ir::Var(id));
        if !changed {
            return Err(Error::Rejected(
                "rewrite endpoint does not occur in the goal".into(),
            ));
        }
        let motive = Ir::Lam {
            id,
            domain: carrier.clone(),
            body: Box::new(body),
        };
        // A textual replacement can invalidate dependent arguments. Reject the
        // family itself before accepting any evidence for the rewritten goal.
        self.infer(&motive)?;
        let (next, _) = replace(goal, from, to);
        self.require_type(&next)?;

        // Rewriting C(a) to C(b) requires C(b) -> C(a). Obtain the
        // reverse path by transport as well, without a symmetry axiom.
        let path = if reverse {
            path
        } else {
            let reverse_id = self.fresh();
            Ir::J {
                motive: Box::new(Ir::Lam {
                    id: reverse_id,
                    domain: carrier.clone(),
                    body: Box::new(Ir::Id {
                        carrier,
                        left: Box::new(Ir::Var(reverse_id)),
                        right: left.clone(),
                    }),
                }),
                base: Box::new(Ir::Refl { value: left }),
                scrutinee: Box::new(path),
            }
        };
        Ok((
            next,
            Frame::Transport {
                motive: Box::new(motive),
                path: Box::new(path),
            },
        ))
    }
}

/// Replace all structurally identical subterms. Stable binder identities make
/// this capture-free even beneath shadowing binders. We do not substitute by
/// source spelling or assume that unchanged dependent arguments stay typed.
fn replace(term: &Ir, from: &Ir, to: &Ir) -> (Ir, bool) {
    if term == from {
        return (to.clone(), true);
    }
    let mut changed = false;
    let mut child = |term: &Ir| {
        let (result, did_change) = replace(term, from, to);
        changed |= did_change;
        Box::new(result)
    };
    let result = match term {
        Ir::Var(_) | Ir::Universe(_) => term.clone(),
        Ir::Pi { id, domain, body } => Ir::Pi {
            id: *id,
            domain: child(domain),
            body: child(body),
        },
        Ir::Lam { id, domain, body } => Ir::Lam {
            id: *id,
            domain: child(domain),
            body: child(body),
        },
        Ir::App(a, b) => Ir::App(child(a), child(b)),
        Ir::Id {
            carrier,
            left,
            right,
        } => Ir::Id {
            carrier: child(carrier),
            left: child(left),
            right: child(right),
        },
        Ir::Refl { value } => Ir::Refl {
            value: child(value),
        },
        Ir::J {
            motive,
            base,
            scrutinee,
        } => Ir::J {
            motive: child(motive),
            base: child(base),
            scrutinee: child(scrutinee),
        },
    };
    (result, changed)
}
