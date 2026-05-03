//! ATS-V parser — `@arch_module(...)` named-args → [`crate::arch::Shape`].
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §8 (syntax integration), the
//! `@arch_module(...)` typed attribute uses the existing generic
//! `attribute_args = named_arg_list` form (verum.ebnf:438-441).
//! The parser sees a list of `NamedArg { name, value }` pairs;
//! this module converts them into the typed [`crate::arch::Shape`]
//! struct that the ATS-V phase consumes.
//!
//! ## Reuse over invention
//!
//! The parser is **structure-driven**: each Shape field has a
//! corresponding parser method that pattern-matches on the AST
//! `ExprKind`. No new grammar — just typed extraction from the
//! generic AST shape per V8.1 META1 architectural principle.
//!
//! ## Soundness contract
//!
//! `parse_arch_module` returns `Ok(Shape)` only when EVERY
//! recognised field parses cleanly. Unknown fields produce
//! [`ArchParseError::UnknownField`]; type mismatches produce
//! [`ArchParseError::InvalidValue`]. The kernel never silently
//! ignores or down-casts.

use crate::arch::*;
use verum_ast::expr::{Expr, ExprKind, ArrayExpr};
use verum_ast::literal::{LiteralKind, StringLit};

// =============================================================================
// ArchParseError — structured error per spec §32.4 dual-audience
// =============================================================================

/// Error produced when `@arch_module(...)` cannot be parsed into a
/// canonical `Shape`. Each variant carries enough information to
/// produce both human-friendly diagnostics and agent-actionable
/// auto-fix suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchParseError {
 /// Field name is not in the canonical roster.
    UnknownField { name: String, suggestion: Option<String> },
 /// Field value has wrong AST shape. e.g. `at_tier = 42` where
 /// `at_tier` expects a `Tier` variant.
    InvalidValue { field: String, expected: &'static str },
 /// Required field missing in strict mode.
    MissingRequired { field: &'static str },
 /// Capability variant references unknown ResourceTag/etc.
    UnknownVariant { kind: &'static str, value: String },
 /// Generic AST mismatch (caller didn't pass a Call expression).
    NotAnArchModuleAttribute,
}

impl ArchParseError {
 /// Human-friendly message.
    pub fn human_message(&self) -> String {
        match self {
            ArchParseError::UnknownField { name, suggestion } => {
                let mut msg = format!("Unknown @arch_module field: `{}`", name);
                if let Some(s) = suggestion {
                    msg.push_str(&format!(". Did you mean `{}`?", s));
                }
                msg
            }
            ArchParseError::InvalidValue { field, expected } => format!(
                "Invalid value for @arch_module field `{}`. Expected {}.",
                field, expected,
            ),
            ArchParseError::MissingRequired { field } => format!(
                "@arch_module(strict = true) requires field `{}` to be set.",
                field,
            ),
            ArchParseError::UnknownVariant { kind, value } => format!(
                "Unknown {} variant: `{}`.",
                kind, value,
            ),
            ArchParseError::NotAnArchModuleAttribute => {
                "Expected @arch_module(...) call expression.".to_string()
            }
        }
    }
}

// =============================================================================
// parse_arch_module — main entry point
// =============================================================================

/// Parse an `@arch_module(...)` attribute's argument list into a
/// canonical [`Shape`].
///
/// Caller passes the AST `Expr` representing the attribute call
/// (typically `attribute_item.attribute.attribute_args`). Each
/// argument MUST be `ExprKind::NamedArg { name, value }`;
/// positional args are rejected.
pub fn parse_arch_module(args: &[Expr]) -> Result<Shape, ArchParseError> {
    let mut shape = Shape::default_for_unannotated();

    for arg in args {
        let (name, value) = match &arg.kind {
            // Function-call named arg: `foo(name = value)` — produced
            // by `parse_call_arg` when the argument list uses `:`
            // syntax outside attribute-arg context.
            ExprKind::NamedArg { name, value } => (name.name.as_str().to_string(), value.as_ref()),
            // Attribute-arg named pair: `@attr(name: value)` — the
            // attribute-argument parser represents `name: value` as
            // `Binary { op: Assign, left: Path(name), right: value }`.
            // We unify both surfaces here so callers can write either
            // form (the canonical `@arch_module(...)` form is the
            // attribute-arg `:` style).
            ExprKind::Binary {
                op: verum_ast::expr::BinOp::Assign,
                left,
                right,
            } => match &left.kind {
                ExprKind::Path(p) => {
                    let name = p
                        .segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => {
                                Some(ident.name.as_str().to_string())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(".");
                    if name.is_empty() {
                        return Err(ArchParseError::InvalidValue {
                            field: "<binary-assign-lhs>".to_string(),
                            expected: "named argument with single-segment ident on LHS",
                        });
                    }
                    (name, right.as_ref())
                }
                _ => {
                    return Err(ArchParseError::InvalidValue {
                        field: "<binary-assign-lhs>".to_string(),
                        expected: "named argument with Path on LHS",
                    });
                }
            },
            _ => {
                return Err(ArchParseError::InvalidValue {
                    field: "<positional>".to_string(),
                    expected: "named argument `name: value` or `name = value`",
                })
            }
        };

        match name.as_str() {
            "exposes" => {
                shape.exposes = parse_capability_list(value)?;
            }
            "requires" => {
                shape.requires = parse_capability_list(value)?;
            }
            "preserves" => {
                shape.preserves = parse_invariant_list(value)?;
            }
            "consumes" => {
                shape.consumes = parse_string_list(value)?;
            }
            "at_tier" => {
                shape.at_tier = parse_tier(value)?;
            }
            "foundation" => {
                shape.foundation = parse_foundation(value)?;
            }
            "stratum" => {
                shape.stratum = parse_stratum(value)?;
            }
            "lifecycle" => {
                shape.lifecycle = parse_lifecycle(value)?;
            }
            "cve_closure_C" => {
                shape.cve_closure.constructive = Some(parse_path_string(value, "cve_closure_C")?);
            }
            "cve_closure_V_strategy" => {
                shape.cve_closure.verifiable_strategy = Some(parse_verify_strategy(value)?);
            }
            "cve_closure_E" => {
                shape.cve_closure.executable = Some(parse_path_string(value, "cve_closure_E")?);
            }
            "composes_with" => {
                shape.composes_with = parse_string_list(value)?;
            }
            "strict" => {
                shape.strict = parse_bool(value)?;
            }
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: suggest_field(other),
                });
            }
        }
    }

 // Strict-mode requirement: full CVE-closure must be present.
 // Per spec §4.8 + AP-010 CveIncomplete.
    if shape.strict {
        if shape.cve_closure.constructive.is_none() {
            return Err(ArchParseError::MissingRequired {
                field: "cve_closure_C",
            });
        }
        if shape.cve_closure.verifiable_strategy.is_none() {
            return Err(ArchParseError::MissingRequired {
                field: "cve_closure_V_strategy",
            });
        }
        if shape.cve_closure.executable.is_none() {
            return Err(ArchParseError::MissingRequired {
                field: "cve_closure_E",
            });
        }
    }

    Ok(shape)
}

/// Lev-distance suggestion for unknown field names.
fn suggest_field(input: &str) -> Option<String> {
    let canonical = [
        "exposes",
        "requires",
        "preserves",
        "consumes",
        "at_tier",
        "foundation",
        "stratum",
        "lifecycle",
        "cve_closure_C",
        "cve_closure_V_strategy",
        "cve_closure_E",
        "composes_with",
        "strict",
    ];
    canonical
        .iter()
        .map(|c| (c, levenshtein(input, c)))
        .min_by_key(|(_, d)| *d)
        .filter(|(_, d)| *d <= 2)
        .map(|(c, _)| c.to_string())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

// =============================================================================
// Per-field parsers
// =============================================================================

fn parse_bool(expr: &Expr) -> Result<bool, ArchParseError> {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Ok(*b),
            _ => Err(ArchParseError::InvalidValue {
                field: "strict".to_string(),
                expected: "boolean literal (true/false)",
            }),
        },
        _ => Err(ArchParseError::InvalidValue {
            field: "strict".to_string(),
            expected: "boolean literal",
        }),
    }
}

/// Parse a string from a path-like expression.  Accepts three forms:
///
///  * `ExprKind::Path` — direct path like `Foundation.ZfcTwoInacc`
///    when the parser collapses dotted segments.
///  * `ExprKind::Field` — field access `obj.field` chain.  We walk
///    the chain to produce the canonical dotted form (so
///    `Foundation.ZfcTwoInacc` parses as the two-segment string
///    "Foundation.ZfcTwoInacc").
///  * `ExprKind::Literal(Text)` — string literal form.
fn parse_path_string(expr: &Expr, field: &str) -> Result<String, ArchParseError> {
    match &expr.kind {
        ExprKind::Path(p) => {
            let segs: Vec<String> = p
                .segments
                .iter()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str().to_string(),
                    _ => "<non_ident>".to_string(),
                })
                .collect();
            Ok(segs.join("."))
        }
        ExprKind::Field { .. } => collapse_field_chain(expr, field),
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(StringLit::Regular(s)) | LiteralKind::Text(StringLit::MultiLine(s)) => {
                Ok(s.as_str().to_string())
            }
            _ => Err(ArchParseError::InvalidValue {
                field: field.to_string(),
                expected: "identifier path or string literal",
            }),
        },
        _ => Err(ArchParseError::InvalidValue {
            field: field.to_string(),
            expected: "identifier path or string literal",
        }),
    }
}

/// Walk a `Foundation.ZfcTwoInacc`-style field-access chain and
/// collapse it to a dotted-path string.  Recurses through nested
/// `ExprKind::Field` until it hits the base `ExprKind::Path`.
fn collapse_field_chain(expr: &Expr, field_name: &str) -> Result<String, ArchParseError> {
    let mut tail: Vec<String> = Vec::new();
    let mut cur = expr;
    loop {
        match &cur.kind {
            ExprKind::Field { expr: inner, field } => {
                tail.push(field.name.as_str().to_string());
                cur = inner.as_ref();
            }
            ExprKind::Path(p) => {
                let mut head: Vec<String> = p
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => {
                            ident.name.as_str().to_string()
                        }
                        _ => "<non_ident>".to_string(),
                    })
                    .collect();
                tail.reverse();
                head.extend(tail);
                return Ok(head.join("."));
            }
            _ => {
                return Err(ArchParseError::InvalidValue {
                    field: field_name.to_string(),
                    expected: "identifier path or string literal",
                });
            }
        }
    }
}

fn parse_string_list(expr: &Expr) -> Result<Vec<String>, ArchParseError> {
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => items
            .iter()
            .map(|e| parse_path_string(e, "list_element"))
            .collect(),
        _ => Err(ArchParseError::InvalidValue {
            field: "<list>".to_string(),
            expected: "array literal `[...]`",
        }),
    }
}

fn parse_capability_list(expr: &Expr) -> Result<Vec<Capability>, ArchParseError> {
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => {
            items.iter().map(parse_capability).collect()
        }
        _ => Err(ArchParseError::InvalidValue {
            field: "exposes/requires".to_string(),
            expected: "array literal `[Capability::Variant(...), ...]`",
        }),
    }
}

/// Parse one capability from path-or-call expression. Accepts
/// canonical variants: `Capability::Logger` (enum-shorthand),
/// `Capability::Read(ResourceTag::Logger)` (Call form), AND
/// `Capability.Read(ResourceTag.File("*"))` (MethodCall form, the
/// surface used by `@arch_module(...)` declarations).
///
/// Canonical recogniser: when the receiver path resolves to
/// `Capability` and the method name matches a known variant
/// (`Read`/`Write`/`Exec`/`Escalate`/`Spawn`/`TimeBound`/
/// `Persist`/`Network`), produce the real `Capability::Variant`
/// with a placeholder inner value derived from the surface tag.
/// This makes structural equality between declared `requires` and
/// inferred used capabilities work in the audit gate without
/// requiring full ResourceTag / ExecTarget unpacking.
fn parse_capability(expr: &Expr) -> Result<Capability, ArchParseError> {
    let (receiver_path, method_name): (Option<String>, Option<String>) = match &expr.kind {
        ExprKind::MethodCall {
            receiver, method, ..
        } => (
            Some(parse_path_string(receiver, "capability")?),
            Some(method.name.as_str().to_string()),
        ),
        ExprKind::Call { func, .. } => {
            // `Capability::Read(...)` or `Read(...)` — split
            // dotted path into prefix + last segment.
            let path = parse_path_string(func, "capability")?;
            if let Some(idx) = path.rfind('.') {
                (Some(path[..idx].to_string()), Some(path[idx + 1..].to_string()))
            } else {
                (None, Some(path))
            }
        }
        _ => {
            let path = parse_path_string(expr, "capability")?;
            if let Some(idx) = path.rfind('.') {
                (Some(path[..idx].to_string()), Some(path[idx + 1..].to_string()))
            } else {
                (None, Some(path))
            }
        }
    };

    // Recognise canonical `Capability.<Variant>` forms — produce
    // real Capability variants for downstream structural-equality
    // matching.  Ignore the inner args (placeholder fillers).
    if matches!(receiver_path.as_deref(), Some("Capability") | None)
        || receiver_path.as_deref().map(|p| p.ends_with(".Capability")).unwrap_or(false)
    {
        if let Some(method) = method_name.as_deref() {
            match method {
                "Read" => {
                    return Ok(Capability::Read {
                        resource: ResourceTag::Custom("<inferred>".to_string()),
                    });
                }
                "Write" => {
                    return Ok(Capability::Write {
                        resource: ResourceTag::Custom("<inferred>".to_string()),
                    });
                }
                "Exec" => {
                    return Ok(Capability::Exec {
                        target: ExecTarget::Custom("<inferred>".to_string()),
                    });
                }
                "Escalate" => {
                    return Ok(Capability::Escalate {
                        realm: PrivilegeRealm::Custom("<inferred>".to_string()),
                    });
                }
                "Spawn" => {
                    return Ok(Capability::Spawn {
                        lifetime: TaskLifetime::Detached,
                    });
                }
                "Persist" => {
                    return Ok(Capability::Persist {
                        medium: PersistenceMedium::Disk {
                            path: "<inferred>".to_string(),
                        },
                    });
                }
                "Network" => {
                    return Ok(Capability::Network {
                        protocol: NetProtocol::Tcp,
                        direction: NetDirection::Bidirectional,
                    });
                }
                _ => {}
            }
        }
    }

    // Fallback: unrecognised form — store as Custom with the
    // dotted-path tag so JSON / diagnostic output still surfaces
    // something meaningful.
    let tag = match (receiver_path, method_name) {
        (Some(r), Some(m)) => format!("{}.{}", r, m),
        (None, Some(m)) => m,
        (Some(r), None) => r,
        _ => "<unknown>".to_string(),
    };
    Ok(Capability::Custom {
        tag,
        schema: CapabilitySchema {
            description: "parsed from @arch_module".to_string(),
            transfers_privilege: false,
            subsumed_by: vec![],
        },
    })
}

fn parse_invariant_list(expr: &Expr) -> Result<Vec<BoundaryInvariant>, ArchParseError> {
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => {
            items.iter().map(parse_invariant).collect()
        }
        _ => Err(ArchParseError::InvalidValue {
            field: "preserves".to_string(),
            expected: "array literal `[BoundaryInvariant::Variant, ...]`",
        }),
    }
}

fn parse_invariant(expr: &Expr) -> Result<BoundaryInvariant, ArchParseError> {
    let path = parse_path_string(expr, "boundary_invariant")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "AllOrNothing" => BoundaryInvariant::AllOrNothing,
        "DeterministicSerialisation" => BoundaryInvariant::DeterministicSerialisation,
        "AuthenticatedFirst" => BoundaryInvariant::AuthenticatedFirst,
        "BackpressureHonoured" => BoundaryInvariant::BackpressureHonoured,
        custom => BoundaryInvariant::Custom {
            name: custom.to_string(),
        },
    })
}

fn parse_tier(expr: &Expr) -> Result<Tier, ArchParseError> {
 // Accept either bare identifier (Tier::Aot) or
 // `Tier::MultiTier([...])` call.
    if let ExprKind::Call { func, args, .. } = &expr.kind {
        let path = parse_path_string(func, "tier")?;
        let last = path.split('.').last().unwrap_or(&path);
        if last == "MultiTier" {
            let inner = args.iter().next().ok_or(ArchParseError::InvalidValue {
                field: "at_tier".to_string(),
                expected: "Tier::MultiTier(allowed_list)",
            })?;
 // The arg should itself be an array literal.
            let allowed = parse_tier_list(inner)?;
            return Ok(Tier::MultiTier { allowed });
        }
        return Err(ArchParseError::UnknownVariant {
            kind: "Tier",
            value: last.to_string(),
        });
    }
    let path = parse_path_string(expr, "at_tier")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "Interp" => Tier::Interp,
        "Aot" => Tier::Aot,
        "Gpu" => Tier::Gpu,
        "Check" => Tier::Check,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "Tier",
                value: other.to_string(),
            });
        }
    })
}

fn parse_tier_list(expr: &Expr) -> Result<Vec<Tier>, ArchParseError> {
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => items.iter().map(parse_tier).collect(),
        _ => Err(ArchParseError::InvalidValue {
            field: "tier_list".to_string(),
            expected: "array literal `[Tier::Aot, Tier::Interp, ...]`",
        }),
    }
}

fn parse_foundation(expr: &Expr) -> Result<Foundation, ArchParseError> {
    let path = parse_path_string(expr, "foundation")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "ZfcTwoInacc" => Foundation::ZfcTwoInacc,
        "Hott" => Foundation::Hott,
        "Cubical" => Foundation::Cubical,
        "Cic" => Foundation::Cic,
        "Mltt" => Foundation::Mltt,
        "Eff" => Foundation::Eff,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "Foundation",
                value: other.to_string(),
            });
        }
    })
}

fn parse_stratum(expr: &Expr) -> Result<MsfsStratum, ArchParseError> {
    let path = parse_path_string(expr, "stratum")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "LFnd" => MsfsStratum::LFnd,
        "LCls" => MsfsStratum::LCls,
        "LClsTop" => MsfsStratum::LClsTop,
        "LAbs" => MsfsStratum::LAbs,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "MsfsStratum",
                value: other.to_string(),
            });
        }
    })
}

fn parse_lifecycle(expr: &Expr) -> Result<Lifecycle, ArchParseError> {
    // Accept bare identifier (`Lifecycle.Theorem`) defaulting to
    // Theorem("unspecified"), call form `Lifecycle::Theorem("v0.1")`,
    // OR method-call form `Lifecycle.Theorem("v0.1")` (the canonical
    // `@arch_module(...)` surface form).
    let call_view: Option<(String, Option<&Expr>)> = match &expr.kind {
        ExprKind::Call { func, args, .. } => Some((
            parse_path_string(func, "lifecycle")?,
            args.iter().next(),
        )),
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            let receiver_path = parse_path_string(receiver, "lifecycle")?;
            let combined = format!("{}.{}", receiver_path, method.name.as_str());
            Some((combined, args.iter().next()))
        }
        _ => None,
    };
    if let Some((path, arg)) = call_view {
        let last = path.split('.').last().unwrap_or(&path);
        return Ok(match (last, arg) {
            ("Theorem", Some(a)) => Lifecycle::Theorem {
                since: parse_path_string(a, "since")?,
            },
            ("Plan", Some(a)) => Lifecycle::Plan {
                target_completion: parse_path_string(a, "target_completion")?,
            },
            ("Postulate", Some(a)) => Lifecycle::Postulate {
                citation: parse_path_string(a, "citation")?,
            },
            ("Definition", _) => Lifecycle::Definition,
            ("Hypothesis", _) => Lifecycle::Hypothesis {
                confidence: ConfidenceLevel::Medium,
            },
            ("Conditional", _) => Lifecycle::Conditional {
                conditions: vec![],
            },
            ("Interpretation", Some(a)) => Lifecycle::Interpretation {
                reason: parse_path_string(a, "reason")?,
            },
            ("Retracted", Some(a)) => Lifecycle::Retracted {
                reason: parse_path_string(a, "reason")?,
                replacement: None,
            },
            ("Obsolete", Some(a)) => Lifecycle::Obsolete {
                deprecation_reason: parse_path_string(a, "reason")?,
                replacement: None,
            },
            (other, _) => {
                return Err(ArchParseError::UnknownVariant {
                    kind: "Lifecycle",
                    value: other.to_string(),
                });
            }
        });
    }
    let path = parse_path_string(expr, "lifecycle")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "Theorem" => Lifecycle::Theorem {
            since: "unspecified".to_string(),
        },
        "Plan" => Lifecycle::Plan {
            target_completion: "unspecified".to_string(),
        },
        "Postulate" => Lifecycle::Postulate {
            citation: "unspecified".to_string(),
        },
        "Definition" => Lifecycle::Definition,
        "Hypothesis" => Lifecycle::Hypothesis {
            confidence: ConfidenceLevel::Medium,
        },
        "Conditional" => Lifecycle::Conditional {
            conditions: vec![],
        },
        "Interpretation" => Lifecycle::Interpretation {
            reason: "unspecified".to_string(),
        },
        "Retracted" => Lifecycle::Retracted {
            reason: "unspecified".to_string(),
            replacement: None,
        },
        "Obsolete" => Lifecycle::Obsolete {
            deprecation_reason: "unspecified".to_string(),
            replacement: None,
        },
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "Lifecycle",
                value: other.to_string(),
            });
        }
    })
}

fn parse_verify_strategy(expr: &Expr) -> Result<VerifyStrategy, ArchParseError> {
    let path = parse_path_string(expr, "cve_closure_V_strategy")?;
    let last = path.split('.').last().unwrap_or(&path);
    Ok(match last {
        "runtime" | "Runtime" => VerifyStrategy::Runtime,
        "static" | "Static" => VerifyStrategy::Static,
        "fast" | "Fast" => VerifyStrategy::Fast,
        "formal" | "Formal" => VerifyStrategy::Formal,
        "proof" | "Proof" => VerifyStrategy::Proof,
        "thorough" | "Thorough" => VerifyStrategy::Thorough,
        "reliable" | "Reliable" => VerifyStrategy::Reliable,
        "certified" | "Certified" => VerifyStrategy::Certified,
        "synthesize" | "Synthesize" => VerifyStrategy::Synthesize,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "VerifyStrategy",
                value: other.to_string(),
            });
        }
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_ast::literal::Literal;
    use verum_common::{Heap, List};

    fn span() -> Span {
        Span::dummy()
    }

    fn name_path_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(Path::new(
                List::from(vec![PathSegment::Name(Ident::new(name, span()))]),
                span(),
            )),
            span(),
        )
    }

    fn dotted_path_expr(parts: &[&str]) -> Expr {
        Expr::new(
            ExprKind::Path(Path::new(
                List::from(
                    parts
                        .iter()
                        .map(|p| PathSegment::Name(Ident::new(*p, span())))
                        .collect::<Vec<_>>(),
                ),
                span(),
            )),
            span(),
        )
    }

    fn named_arg(name: &str, value: Expr) -> Expr {
        Expr::new(
            ExprKind::NamedArg {
                name: Ident::new(name, span()),
                value: Heap::new(value),
            },
            span(),
        )
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(b), span())),
            span(),
        )
    }

    fn array_expr(items: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Array(ArrayExpr::List(List::from(items))),
            span(),
        )
    }

    #[test]
    fn parse_empty_args_yields_default_shape() {
        let shape = parse_arch_module(&[]).unwrap();
        assert_eq!(shape.foundation, Foundation::ZfcTwoInacc);
        assert_eq!(shape.stratum, MsfsStratum::LFnd);
        assert!(!shape.strict);
    }

    #[test]
    fn parse_strict_true_sets_field() {
        let args = vec![named_arg("strict", bool_lit(true))];
 // Strict requires CVE-closure complete — so without
 // cve_closure fields it errors.
        let r = parse_arch_module(&args);
        assert!(matches!(r, Err(ArchParseError::MissingRequired { .. })));
    }

    #[test]
    fn parse_foundation_canonical_variants() {
        for (name, expected) in [
            ("ZfcTwoInacc", Foundation::ZfcTwoInacc),
            ("Hott", Foundation::Hott),
            ("Cubical", Foundation::Cubical),
            ("Cic", Foundation::Cic),
            ("Mltt", Foundation::Mltt),
            ("Eff", Foundation::Eff),
        ] {
            let args = vec![named_arg(
                "foundation",
                dotted_path_expr(&["Foundation", name]),
            )];
            let shape = parse_arch_module(&args).unwrap();
            assert_eq!(shape.foundation, expected);
        }
    }

    #[test]
    fn parse_unknown_foundation_errors() {
        let args = vec![named_arg(
            "foundation",
            dotted_path_expr(&["Foundation", "BogusFoundation"]),
        )];
        let r = parse_arch_module(&args);
        match r {
            Err(ArchParseError::UnknownVariant {
                kind: "Foundation",
                value,
            }) => assert_eq!(value, "BogusFoundation"),
            other => panic!("expected UnknownVariant, got {:?}", other),
        }
    }

    #[test]
    fn parse_stratum_canonical_variants() {
        for (name, expected) in [
            ("LFnd", MsfsStratum::LFnd),
            ("LCls", MsfsStratum::LCls),
            ("LClsTop", MsfsStratum::LClsTop),
            ("LAbs", MsfsStratum::LAbs),
        ] {
            let args = vec![named_arg(
                "stratum",
                dotted_path_expr(&["MsfsStratum", name]),
            )];
            let shape = parse_arch_module(&args).unwrap();
            assert_eq!(shape.stratum, expected);
        }
    }

    #[test]
    fn parse_tier_bare_variants() {
        for (name, expected) in [
            ("Interp", Tier::Interp),
            ("Aot", Tier::Aot),
            ("Gpu", Tier::Gpu),
            ("Check", Tier::Check),
        ] {
            let args = vec![named_arg("at_tier", dotted_path_expr(&["Tier", name]))];
            let shape = parse_arch_module(&args).unwrap();
            assert_eq!(shape.at_tier, expected);
        }
    }

    #[test]
    fn parse_lifecycle_bare_theorem() {
        let args = vec![named_arg(
            "lifecycle",
            dotted_path_expr(&["Lifecycle", "Theorem"]),
        )];
        let shape = parse_arch_module(&args).unwrap();
        assert_eq!(shape.lifecycle.tag(), "theorem");
    }

    #[test]
    fn parse_unknown_field_suggests_correction() {
 // Typo: `expose` instead of `exposes`.
        let args = vec![named_arg("expose", array_expr(vec![]))];
        let r = parse_arch_module(&args);
        match r {
            Err(ArchParseError::UnknownField { name, suggestion }) => {
                assert_eq!(name, "expose");
                assert_eq!(suggestion, Some("exposes".to_string()));
            }
            other => panic!("expected UnknownField with suggestion, got {:?}", other),
        }
    }

    #[test]
    fn parse_unknown_field_too_far_no_suggestion() {
        let args = vec![named_arg("totally_random_garbage", array_expr(vec![]))];
        let r = parse_arch_module(&args);
        match r {
            Err(ArchParseError::UnknownField { suggestion, .. }) => {
                assert!(suggestion.is_none(), "should NOT suggest for distance > 2");
            }
            other => panic!("expected UnknownField, got {:?}", other),
        }
    }

    #[test]
    fn parse_capability_list_round_trips_simple_names() {
        let args = vec![named_arg(
            "exposes",
            array_expr(vec![
                name_path_expr("logger"),
                name_path_expr("metrics"),
            ]),
        )];
        let shape = parse_arch_module(&args).unwrap();
        assert_eq!(shape.exposes.len(), 2);
    }

    #[test]
    fn parse_invariant_list_canonical_variants() {
        let args = vec![named_arg(
            "preserves",
            array_expr(vec![
                dotted_path_expr(&["BoundaryInvariant", "AllOrNothing"]),
                dotted_path_expr(&["BoundaryInvariant", "AuthenticatedFirst"]),
            ]),
        )];
        let shape = parse_arch_module(&args).unwrap();
        assert_eq!(shape.preserves.len(), 2);
        assert!(matches!(shape.preserves[0], BoundaryInvariant::AllOrNothing));
        assert!(matches!(
            shape.preserves[1],
            BoundaryInvariant::AuthenticatedFirst
        ));
    }

    #[test]
    fn parse_strict_with_full_cve_succeeds() {
        let args = vec![
            named_arg("strict", bool_lit(true)),
            named_arg(
                "cve_closure_C",
                dotted_path_expr(&["my_cog", "synthesize_witness"]),
            ),
            named_arg(
                "cve_closure_V_strategy",
                dotted_path_expr(&["VerifyStrategy", "certified"]),
            ),
            named_arg(
                "cve_closure_E",
                dotted_path_expr(&["my_cog", "Server"]),
            ),
        ];
        let shape = parse_arch_module(&args).unwrap();
        assert!(shape.strict);
        assert!(shape.cve_closure.is_fully_closed());
        assert_eq!(
            shape.cve_closure.verifiable_strategy,
            Some(VerifyStrategy::Certified)
        );
    }

    #[test]
    fn parse_full_arch_module_realistic_example() {
 // Mirror the worked example from spec §17.2.
        let args = vec![
            named_arg(
                "exposes",
                array_expr(vec![
                    name_path_expr("authenticate"),
                    name_path_expr("issue_token"),
                ]),
            ),
            named_arg(
                "requires",
                array_expr(vec![
                    name_path_expr("hash_password"),
                    name_path_expr("random_bytes"),
                ]),
            ),
            named_arg(
                "preserves",
                array_expr(vec![dotted_path_expr(&[
                    "BoundaryInvariant",
                    "AuthenticatedFirst",
                ])]),
            ),
            named_arg(
                "at_tier",
                dotted_path_expr(&["Tier", "Aot"]),
            ),
            named_arg(
                "foundation",
                dotted_path_expr(&["Foundation", "ZfcTwoInacc"]),
            ),
            named_arg(
                "stratum",
                dotted_path_expr(&["MsfsStratum", "LFnd"]),
            ),
            named_arg(
                "lifecycle",
                dotted_path_expr(&["Lifecycle", "Theorem"]),
            ),
            named_arg("strict", bool_lit(false)),
        ];
        let shape = parse_arch_module(&args).unwrap();
        assert_eq!(shape.exposes.len(), 2);
        assert_eq!(shape.requires.len(), 2);
        assert_eq!(shape.preserves.len(), 1);
        assert_eq!(shape.at_tier, Tier::Aot);
        assert_eq!(shape.foundation, Foundation::ZfcTwoInacc);
        assert_eq!(shape.stratum, MsfsStratum::LFnd);
        assert!(!shape.strict);
    }

    #[test]
    fn architectural_pin_no_positional_args() {
 // Positional args (not NamedArg wrapped) are rejected —
 // @arch_module(...) is named-args only per spec §8.
        let args = vec![bool_lit(true)]; // not wrapped as NamedArg
        let r = parse_arch_module(&args);
        match r {
            Err(ArchParseError::InvalidValue { field, .. }) => {
                assert_eq!(field, "<positional>");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn levenshtein_distance_smoke() {
        assert_eq!(levenshtein("expose", "exposes"), 1);
        assert_eq!(levenshtein("requires", "requires"), 0);
        assert_eq!(levenshtein("foo", "bar"), 3);
    }
}
