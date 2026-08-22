//! ATS-V parser — `@arch_module(...)` named-args → [`crate::arch::Shape`].
//!
//! ## Architectural role
//!
//! The `@arch_module(...)` typed attribute uses the existing
//! generic `attribute_args = named_arg_list` grammar form — no new
//! grammar production was introduced for ATS-V.  The parser sees a
//! list of `NamedArg { name, value }` pairs and converts them into
//! the typed [`crate::arch::Shape`] struct that the ATS-V phase
//! consumes.
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
    UnknownField {
        /// Offending field name as written in the source.
        name: String,
        /// Levenshtein-style spelling suggestion, if any.
        suggestion: Option<String>,
    },
    /// Field value has wrong AST shape. e.g. `at_tier = 42` where
    /// `at_tier` expects a `Tier` variant.
    InvalidValue {
        /// Field whose value is malformed.
        field: String,
        /// Expected value shape (free-form description).
        expected: &'static str,
    },
    /// Required field missing in strict mode.
    MissingRequired {
        /// Name of the required field that was omitted.
        field: &'static str,
    },
    /// Capability variant references unknown ResourceTag/etc.
    UnknownVariant {
        /// Variant family name (`ResourceTag`, `ExecTarget`, …).
        kind: &'static str,
        /// Unknown variant identifier as written.
        value: String,
    },
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
            "declarations" => {
                shape.declarations = Some(parse_declarations(value)?);
            }
            "cve_closure" => {
                shape.cve_closure = parse_cve_closure(value)?;
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
        "declarations",
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

    // Recognise canonical `Capability.<Variant>(args...)` forms and
    // parse the ARGUMENTS into the variant's real payload. This block
    // used to ignore every inner argument ("placeholder fillers"):
    // all eight arms produced fixed placeholders, so every pin in the
    // corpus — `Read(ResourceTag.Database("ledger"))`,
    // `Network(NetProtocol.Tcp, NetDirection.Outbound)` — parsed to
    // the same handful of stamped values. The judgment against an
    // INFERRED atom could then agree only by accident: the T0834
    // class (a parser blind to the fields its own surface advertises),
    // caught live by the row judgment's clean-twin fixture (T0848).
    let call_args: Vec<&Expr> = match &expr.kind {
        ExprKind::MethodCall { args, .. } => args.iter().collect(),
        ExprKind::Call { args, .. } => args.iter().collect(),
        _ => Vec::new(),
    };
    if matches!(receiver_path.as_deref(), Some("Capability") | None)
        || receiver_path.as_deref().map(|p| p.ends_with(".Capability")).unwrap_or(false)
    {
        if let Some(method) = method_name.as_deref() {
            match method {
                "Read" => {
                    return Ok(Capability::Read {
                        resource: parse_resource_tag_arg(call_args.first().copied()),
                    });
                }
                "Write" => {
                    return Ok(Capability::Write {
                        resource: parse_resource_tag_arg(call_args.first().copied()),
                    });
                }
                "Exec" => {
                    return Ok(Capability::Exec {
                        target: parse_exec_target_arg(call_args.first().copied()),
                    });
                }
                "Escalate" => {
                    return Ok(Capability::Escalate {
                        realm: parse_realm_arg(call_args.first().copied()),
                    });
                }
                "Spawn" => {
                    return Ok(Capability::Spawn {
                        lifetime: parse_lifetime_arg(call_args.first().copied()),
                    });
                }
                "Persist" => {
                    return Ok(Capability::Persist {
                        medium: parse_medium_arg(call_args.first().copied()),
                    });
                }
                "Network" => {
                    return Ok(Capability::Network {
                        protocol: parse_protocol_arg(call_args.first().copied()),
                        direction: parse_direction_arg(call_args.get(1).copied()),
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

/// The `(last_segment, first_string_arg)` of a variant-shaped
/// argument: `ResourceTag.Database("ledger")` → ("Database",
/// Some("ledger")); a bare `ResourceTag.Logger` → ("Logger", None).
/// Unparseable shapes yield ("", None) so every caller falls to its
/// Custom arm with the raw rendering, never silently to a default.
fn variant_last_and_string(expr: Option<&Expr>) -> (String, Option<String>) {
    let Some(expr) = expr else {
        return (String::new(), None);
    };
    let (path_expr, string_arg): (&Expr, Option<String>) = match &expr.kind {
        ExprKind::Call { func, args, .. } => (
            func.as_ref(),
            args.iter().next().and_then(expr_string_literal),
        ),
        ExprKind::MethodCall { .. } => {
            // `ResourceTag.Database("x")` parses as a method call on
            // the `ResourceTag` receiver; the method IS the variant.
            if let ExprKind::MethodCall {
                method, args, ..
            } = &expr.kind
            {
                return (
                    method.name.as_str().to_string(),
                    args.iter().next().and_then(expr_string_literal),
                );
            }
            unreachable!()
        }
        _ => (expr, None),
    };
    let last = parse_path_string(path_expr, "capability-arg")
        .ok()
        .and_then(|p| p.split('.').next_back().map(str::to_string))
        .unwrap_or_default();
    (last, string_arg)
}

fn expr_string_literal(expr: &Expr) -> Option<String> {
    use verum_ast::literal::LiteralKind;
    if let ExprKind::Literal(lit) = &expr.kind {
        if let LiteralKind::Text(t) = &lit.kind {
            // The lexer keeps the surrounding quotes in the token text;
            // the capability payload wants the VALUE.
            let raw = t.to_string();
            let trimmed = raw
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .unwrap_or(&raw);
            return Some(trimmed.to_string());
        }
    }
    None
}

fn parse_resource_tag_arg(arg: Option<&Expr>) -> ResourceTag {
    let (last, sarg) = variant_last_and_string(arg);
    let s = |d: &str| sarg.clone().unwrap_or_else(|| d.to_string());
    match last.as_str() {
        "Database" => ResourceTag::Database { name: s("*") },
        "File" => ResourceTag::File { path_pattern: s("*") },
        "Memory" => ResourceTag::Memory { region: s("*") },
        "Config" => ResourceTag::Config { namespace: s("*") },
        "Logger" => ResourceTag::Logger,
        "Random" => ResourceTag::Random,
        _ => ResourceTag::Custom(if last.is_empty() {
            "<unparsed>".to_string()
        } else {
            last
        }),
    }
}

fn parse_exec_target_arg(arg: Option<&Expr>) -> ExecTarget {
    let (last, sarg) = variant_last_and_string(arg);
    match last.as_str() {
        "Ffi" => ExecTarget::Ffi {
            library: sarg.unwrap_or_else(|| "*".to_string()),
            symbol: "*".to_string(),
        },
        // `Syscall(0)` carries an int literal v1 does not need — the
        // variant identity is the judged fact.
        "Syscall" => ExecTarget::Syscall { number: 0 },
        "Program" => ExecTarget::Program {
            path: sarg.unwrap_or_else(|| "*".to_string()),
        },
        _ => ExecTarget::Custom(if last.is_empty() {
            "<unparsed>".to_string()
        } else {
            last
        }),
    }
}

fn parse_realm_arg(arg: Option<&Expr>) -> PrivilegeRealm {
    let (last, _) = variant_last_and_string(arg);
    match last.as_str() {
        "Admin" => PrivilegeRealm::Admin,
        "Root" => PrivilegeRealm::Root,
        "Audit" => PrivilegeRealm::Audit,
        _ => PrivilegeRealm::Custom(if last.is_empty() {
            "<unparsed>".to_string()
        } else {
            last
        }),
    }
}

fn parse_lifetime_arg(arg: Option<&Expr>) -> TaskLifetime {
    let (last, _) = variant_last_and_string(arg);
    match last.as_str() {
        "ScopedToParent" => TaskLifetime::ScopedToParent,
        "Detached" | "" => TaskLifetime::Detached,
        // Deadlined/AtUnixTime/AfterDuration/OnEvent carry payloads
        // the pin-judgment does not compare in v1 — identity default.
        _ => TaskLifetime::Detached,
    }
}

fn parse_medium_arg(arg: Option<&Expr>) -> PersistenceMedium {
    let (last, sarg) = variant_last_and_string(arg);
    match last.as_str() {
        "Disk" => PersistenceMedium::Disk {
            path: sarg.unwrap_or_else(|| "*".to_string()),
        },
        "Database" | "DatabaseMedium" => PersistenceMedium::Database {
            connection_tag: sarg.unwrap_or_else(|| "*".to_string()),
        },
        "DistributedLog" => PersistenceMedium::DistributedLog {
            topic: sarg.unwrap_or_else(|| "*".to_string()),
        },
        _ => PersistenceMedium::Disk {
            path: "<unparsed>".to_string(),
        },
    }
}

fn parse_protocol_arg(arg: Option<&Expr>) -> NetProtocol {
    let (last, _) = variant_last_and_string(arg);
    match last.as_str() {
        "Udp" => NetProtocol::Udp,
        "Unix" => NetProtocol::Unix,
        "Tls" => NetProtocol::Tls,
        "Quic" => NetProtocol::Quic,
        _ => NetProtocol::Tcp,
    }
}

fn parse_direction_arg(arg: Option<&Expr>) -> NetDirection {
    let (last, _) = variant_last_and_string(arg);
    match last.as_str() {
        "Inbound" => NetDirection::Inbound,
        "Outbound" => NetDirection::Outbound,
        _ => NetDirection::Bidirectional,
    }
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
        let last = path.split('.').next_back().unwrap_or(&path);
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
        let last = path.split('.').next_back().unwrap_or(&path);
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
    let last = path.split('.').next_back().unwrap_or(&path);
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
// declarations: ShapeDeclarations { ... } — CVE-architecture spec declarations
// =============================================================================

/// Split a call-shaped expression into (dotted callee path, first arg).
/// `Maybe.Some(x)` parses as a method call on the path `Maybe`, while
/// `Some(x)` is a plain call — the same two spellings of one concept
/// that `parse_lifecycle` already collapses, factored here so every
/// declarations field treats them identically.
fn callee_view<'e>(expr: &'e Expr, field: &str) -> Option<(String, Option<&'e Expr>)> {
    match &expr.kind {
        ExprKind::Call { func, args, .. } => Some((
            parse_path_string(func, field).ok()?,
            args.iter().next(),
        )),
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => Some((
            format!(
                "{}.{}",
                parse_path_string(receiver, field).ok()?,
                method.name.as_str()
            ),
            args.iter().next(),
        )),
        _ => None,
    }
}

/// `Maybe.Some(inner)` / `Some(inner)` → `Some(parse_inner(inner))`;
/// `Maybe.None` / `None` → `None`.  The declarations fields are all
/// optional, and this is the only shape optionality takes.
fn parse_maybe<T>(
    expr: &Expr,
    field: &'static str,
    parse_inner: impl FnOnce(&Expr) -> Result<T, ArchParseError>,
) -> Result<Option<T>, ArchParseError> {
    if let Ok(path) = parse_path_string(expr, field)
        && path.split('.').next_back() == Some("None")
    {
        return Ok(None);
    }
    if let Some((callee, arg)) = callee_view(expr, field)
        && callee.split('.').next_back() == Some("Some")
    {
        let inner = arg.ok_or(ArchParseError::InvalidValue {
            field: field.to_string(),
            expected: "`Maybe.Some(value)` carrying a value",
        })?;
        return Ok(Some(parse_inner(inner)?));
    }
    Err(ArchParseError::InvalidValue {
        field: field.to_string(),
        expected: "`Maybe.Some(...)` or `Maybe.None`",
    })
}

/// A unit variant of a serde-derived enum, resolved by its last path
/// segment THROUGH the enum's own serde name table.  The enum is the
/// list: a variant added in `arch.rs` is parseable here with no second
/// string table to drift out of sync.
fn parse_unit_variant<T: serde::de::DeserializeOwned>(
    expr: &Expr,
    kind: &'static str,
) -> Result<T, ArchParseError> {
    use serde::de::IntoDeserializer;
    let path = parse_path_string(expr, kind)?;
    let last = path.split('.').next_back().unwrap_or(&path);
    T::deserialize(last.into_deserializer()).map_err(|_: serde::de::value::Error| {
        ArchParseError::UnknownVariant {
            kind,
            value: last.to_string(),
        }
    })
}

/// A text value: a string literal, or the same literal spelled
/// `"...".to_text()` — both appear in the corpus and mean one thing.
fn parse_text_value(expr: &Expr, field: &'static str) -> Result<String, ArchParseError> {
    if let ExprKind::MethodCall {
        receiver, method, ..
    } = &expr.kind
        && method.name.as_str() == "to_text"
    {
        return parse_path_string(receiver, field);
    }
    parse_path_string(expr, field)
}

/// The record fields of an `X { ... }` literal whose type's last path
/// segment is `expected_type`, as (name, value) pairs.  Shorthand
/// fields (`{ x }`) are rejected: a declarations record names what it
/// declares.
fn record_fields<'e>(
    expr: &'e Expr,
    expected_type: &'static str,
) -> Result<Vec<(&'e str, &'e Expr)>, ArchParseError> {
    let ExprKind::Record { path, fields, .. } = &expr.kind else {
        return Err(ArchParseError::InvalidValue {
            field: expected_type.to_string(),
            expected: "record literal `Type { field: value, ... }`",
        });
    };
    let type_name = path
        .segments
        .iter()
        .filter_map(|s| match s {
            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
            _ => None,
        })
        .next_back()
        .unwrap_or("");
    if type_name != expected_type {
        return Err(ArchParseError::UnknownVariant {
            kind: "record type",
            value: type_name.to_string(),
        });
    }
    fields
        .iter()
        .map(|init| match &init.value {
            verum_common::Maybe::Some(v) => Ok((init.name.name.as_str(), v)),
            verum_common::Maybe::None => Err(ArchParseError::InvalidValue {
                field: init.name.name.as_str().to_string(),
                expected: "explicit `field: value` (no shorthand in declarations)",
            }),
        })
        .collect()
}

/// Parse `Purpose { role, k_min, v_min, e_min }` (spec §14.6).
fn parse_purpose(expr: &Expr) -> Result<Purpose, ArchParseError> {
    let mut role: Option<String> = None;
    let mut k_min: Option<CveThresholdK> = None;
    let mut v_min: Option<CveThresholdV> = None;
    let mut e_min: Option<CveThresholdE> = None;
    for (name, value) in record_fields(expr, "Purpose")? {
        match name {
            "role" => role = Some(parse_text_value(value, "role")?),
            "k_min" => k_min = Some(parse_unit_variant(value, "CveThresholdK")?),
            "v_min" => v_min = Some(parse_unit_variant(value, "CveThresholdV")?),
            "e_min" => e_min = Some(parse_unit_variant(value, "CveThresholdE")?),
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                });
            }
        }
    }
    let missing = |field: &'static str| ArchParseError::MissingRequired { field };
    Ok(Purpose {
        role: role.ok_or(missing("role"))?,
        k_min: k_min.ok_or(missing("k_min"))?,
        v_min: v_min.ok_or(missing("v_min"))?,
        e_min: e_min.ok_or(missing("e_min"))?,
    })
}

/// Parse a fixpoint-class value in the surface AP-040 itself suggests:
/// `fixpoint_class_banach()` / `_tarski()` / `_adamek()` /
/// `_custom_fixpoint("citation")` — the in-language smart constructors
/// from `core/architecture/types.vr`, mapped to their kernel twins.
fn parse_fixpoint_class(expr: &Expr) -> Result<FixpointClass, ArchParseError> {
    let Some((callee, arg)) = callee_view(expr, "fixpoint_class") else {
        return Err(ArchParseError::InvalidValue {
            field: "fixpoint_class".to_string(),
            expected: "fixpoint_class_banach() / _tarski() / _adamek() / _custom_fixpoint(\"citation\")",
        });
    };
    let last = callee.split('.').next_back().unwrap_or(&callee);
    match last.trim_start_matches("fixpoint_class_") {
        "banach" => Ok(FixpointClass::banach()),
        "tarski" => Ok(FixpointClass::tarski()),
        "adamek" => Ok(FixpointClass::adamek()),
        "custom_fixpoint" => {
            let citation = arg.ok_or(ArchParseError::MissingRequired { field: "citation" })?;
            Ok(FixpointClass::custom_fixpoint(parse_text_value(
                citation, "citation",
            )?))
        }
        other => Err(ArchParseError::UnknownVariant {
            kind: "FixpointClass",
            value: other.to_string(),
        }),
    }
}

/// Parse `SelfReferenceWitness { operator, fixed_point, fixpoint_class }`
/// (spec §16 — the AP-040 discharge witness).
fn parse_self_reference(expr: &Expr) -> Result<SelfReferenceWitness, ArchParseError> {
    let mut operator: Option<String> = None;
    let mut fixed_point: Option<String> = None;
    let mut fixpoint_class: Option<FixpointClass> = None;
    for (name, value) in record_fields(expr, "SelfReferenceWitness")? {
        match name {
            "operator" => operator = Some(parse_text_value(value, "operator")?),
            "fixed_point" => fixed_point = Some(parse_text_value(value, "fixed_point")?),
            "fixpoint_class" => fixpoint_class = Some(parse_fixpoint_class(value)?),
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                });
            }
        }
    }
    let missing = |field: &'static str| ArchParseError::MissingRequired { field };
    Ok(SelfReferenceWitness {
        operator: operator.ok_or(missing("operator"))?,
        fixed_point: fixed_point.ok_or(missing("fixed_point"))?,
        fixpoint_class: fixpoint_class.ok_or(missing("fixpoint_class"))?,
    })
}

/// Parse `cve_closure: CveClosure { constructive, verifiable_strategy,
/// executable }` — the record spelling of the CVE triple that the
/// shape reference documents as the full surface.  The flat fields
/// (`cve_closure_C` / `cve_closure_V_strategy` / `cve_closure_E`)
/// remain the other legal spelling; both feed the same struct.
fn parse_cve_closure(expr: &Expr) -> Result<CveClosure, ArchParseError> {
    let mut closure = CveClosure {
        constructive: None,
        verifiable_strategy: None,
        executable: None,
    };
    for (name, value) in record_fields(expr, "CveClosure")? {
        match name {
            "constructive" => {
                closure.constructive =
                    parse_maybe(value, "constructive", |e| parse_text_value(e, "constructive"))?;
            }
            "verifiable_strategy" => {
                closure.verifiable_strategy =
                    parse_maybe(value, "verifiable_strategy", parse_verify_strategy)?;
            }
            "executable" => {
                closure.executable =
                    parse_maybe(value, "executable", |e| parse_text_value(e, "executable"))?;
            }
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                });
            }
        }
    }
    Ok(closure)
}

/// Parse `declarations: ShapeDeclarations { ... }` — the optional
/// CVE-architecture spec declarations (§14.6 purpose, §1.5 substrate,
/// §4.5 anchoring, §2.3.0 executability sense, §16 self-reference).
///
/// `Shape.declarations` has carried this data since the CVE-AH band
/// landed, the anti-pattern auto-fixes tell authors to WRITE the
/// field, and 74 core/ headers do — but the parser never learned it,
/// so every one of those headers died with `UnknownField` the moment
/// the ATS-V phase ran on a user path (T0834).
fn parse_declarations(expr: &Expr) -> Result<ShapeDeclarations, ArchParseError> {
    let mut decls = ShapeDeclarations::empty();
    for (name, value) in record_fields(expr, "ShapeDeclarations")? {
        match name {
            "purpose" => decls.purpose = parse_maybe(value, "purpose", parse_purpose)?,
            "substrate" => {
                decls.substrate = parse_maybe(value, "substrate", |e| {
                    parse_unit_variant(e, "CognitiveSubstrate")
                })?;
            }
            "anchoring" => {
                decls.anchoring = parse_maybe(value, "anchoring", |e| {
                    parse_unit_variant(e, "FormalAnchoring")
                })?;
            }
            "e_sense" => {
                decls.e_sense = parse_maybe(value, "e_sense", |e| {
                    parse_unit_variant(e, "ExecutabilitySense")
                })?;
            }
            "self_reference" => {
                decls.self_reference =
                    parse_maybe(value, "self_reference", parse_self_reference)?;
            }
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                });
            }
        }
    }
    Ok(decls)
}

// =============================================================================
// @bridge_tier(from: Tier, to: Tier) — auxiliary typed attribute
// =============================================================================

/// Parse `@bridge_tier(from: Tier.X, to: Tier.Y)` named-args into a
/// typed `BridgeTier`.  Lifts the AP-004 TierMixing ban for the
/// annotated function.  No-op bridges (`from == to`) are accepted
/// at parse time but flagged by the architectural type-checker
/// (use the bare call site instead).
pub fn parse_bridge_tier(args: &[Expr]) -> Result<crate::arch::BridgeTier, ArchParseError> {
    let mut from: Option<Tier> = None;
    let mut to: Option<Tier> = None;
    for arg in args {
        let (name, value) = extract_named_arg(arg)?;
        match name.as_str() {
            "from" => from = Some(parse_tier(value)?),
            "to" => to = Some(parse_tier(value)?),
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                })
            }
        }
    }
    let from = from.ok_or(ArchParseError::MissingRequired { field: "from" })?;
    let to = to.ok_or(ArchParseError::MissingRequired { field: "to" })?;
    Ok(crate::arch::BridgeTier { from, to })
}

// =============================================================================
// @deterministic — marker attribute, no args
// =============================================================================

/// Parse `@deterministic` (no args) into a marker.  Returns `Ok(_)`
/// for a well-formed marker and `Err(InvalidValue)` if the source
/// passed positional or named arguments — the marker is strictly
/// argument-less.
pub fn parse_deterministic(args: &[Expr]) -> Result<crate::arch::DeterministicMarker, ArchParseError> {
    if !args.is_empty() {
        return Err(ArchParseError::InvalidValue {
            field: "<deterministic-args>".to_string(),
            expected: "no arguments — `@deterministic` is a marker attribute",
        });
    }
    Ok(crate::arch::DeterministicMarker)
}

// =============================================================================
// @mtac_decision { point, by_observer, proposition, modality }
// =============================================================================

/// Parse `@mtac_decision { point: TimePoint.X, by_observer: Observer.Y,
/// proposition: ArchProposition.Z, modality: ModalAssertion.W }`.
/// The four fields are required; missing any raises
/// `MissingRequired`.
pub fn parse_mtac_decision(args: &[Expr]) -> Result<crate::arch::MtacDecisionAttr, ArchParseError> {
    let mut point: Option<crate::arch_mtac::TimePoint> = None;
    let mut by_observer: Option<crate::arch_mtac::Observer> = None;
    let mut proposition: Option<crate::arch_mtac::ArchProposition> = None;
    let mut modality: Option<crate::arch::MtacModality> = None;
    for arg in args {
        let (name, value) = extract_named_arg(arg)?;
        match name.as_str() {
            "point" => point = Some(parse_time_point(value)?),
            "by_observer" => by_observer = Some(parse_observer(value)?),
            "proposition" => proposition = Some(parse_arch_proposition(value)?),
            "modality" => modality = Some(parse_mtac_modality(value)?),
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                })
            }
        }
    }
    Ok(crate::arch::MtacDecisionAttr {
        point: point.ok_or(ArchParseError::MissingRequired { field: "point" })?,
        by_observer: by_observer
            .ok_or(ArchParseError::MissingRequired { field: "by_observer" })?,
        proposition: proposition
            .ok_or(ArchParseError::MissingRequired { field: "proposition" })?,
        modality: modality.ok_or(ArchParseError::MissingRequired { field: "modality" })?,
    })
}

fn parse_time_point(expr: &Expr) -> Result<crate::arch_mtac::TimePoint, ArchParseError> {
    let path = parse_path_string(expr, "point")?;
    let last = path.split('.').next_back().unwrap_or(&path);
    Ok(match last {
        "Now" => crate::arch_mtac::TimePoint::Now,
        // Past / Future / Counterfactual carry payloads — keep the
        // decision-attribute surface to bare-name forms; payloads
        // can be encoded via the generic parse_arch_module path or
        // future v2 of the @mtac_decision parser.
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "TimePoint",
                value: other.to_string(),
            });
        }
    })
}

fn parse_observer(expr: &Expr) -> Result<crate::arch_mtac::Observer, ArchParseError> {
    let path = parse_path_string(expr, "by_observer")?;
    let last = path.split('.').next_back().unwrap_or(&path);
    Ok(match last {
        "EndUser" => crate::arch_mtac::Observer::EndUser {
            kind: "default".into(),
        },
        "PeerCog" => crate::arch_mtac::Observer::PeerCog {
            module_path: "<any>".into(),
        },
        "Stakeholder" => crate::arch_mtac::Observer::Stakeholder {
            role: "operator".into(),
        },
        "Auditor" => crate::arch_mtac::Observer::Auditor {
            audit_kind: "compliance".into(),
        },
        "Adversary" => crate::arch_mtac::Observer::Adversary {
            threat_model: "external".into(),
        },
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "Observer",
                value: other.to_string(),
            });
        }
    })
}

fn parse_arch_proposition(
    expr: &Expr,
) -> Result<crate::arch_mtac::ArchProposition, ArchParseError> {
    let path = parse_path_string(expr, "proposition")?;
    let last = path.split('.').next_back().unwrap_or(&path);
    Ok(match last {
        "FoundationStable" => crate::arch_mtac::ArchProposition::FoundationStable,
        "PublicApiUnchanged" => crate::arch_mtac::ArchProposition::PublicApiUnchanged,
        // HasCapability / Custom carry payloads — bare-name path
        // restricts the attribute surface to the parametric-free
        // arms; payloads via the generic parse_arch_module surface.
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "ArchProposition",
                value: other.to_string(),
            });
        }
    })
}

fn parse_mtac_modality(expr: &Expr) -> Result<crate::arch::MtacModality, ArchParseError> {
    let path = parse_path_string(expr, "modality")?;
    let last = path.split('.').next_back().unwrap_or(&path);
    Ok(match last {
        "Necessity" => crate::arch::MtacModality::Necessity,
        "Possibility" => crate::arch::MtacModality::Possibility,
        "Eventually" => crate::arch::MtacModality::Eventually,
        "Always" => crate::arch::MtacModality::Always,
        "Until" => crate::arch::MtacModality::Until,
        // `Counterfactual` is the bare-name form; the kernel-side
        // ModalAssertion::Counterfactual is the structured arm.
        "Counterfactual" => crate::arch::MtacModality::Counterfactual,
        // Aliases recognised at parse time.
        "CounterfactualImpl" => crate::arch::MtacModality::Counterfactual,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "MtacModality",
                value: other.to_string(),
            });
        }
    })
}

// =============================================================================
// @arch_corpus(invariants: [...], foundation_bridges: [...])
// =============================================================================

/// Parse `@arch_corpus(invariants: [CorpusInvariant.X, ...],
/// foundation_bridges: [...])` into a typed `ArchCorpusAttr`.
/// Both fields are optional; missing fields default to "use the
/// canonical 4-roster" / "no bridges declared".
pub fn parse_arch_corpus(args: &[Expr]) -> Result<crate::arch::ArchCorpusAttr, ArchParseError> {
    let mut out = crate::arch::ArchCorpusAttr::default();
    for arg in args {
        let (name, value) = extract_named_arg(arg)?;
        match name.as_str() {
            "invariants" => out.invariants = parse_corpus_invariant_list(value)?,
            "foundation_bridges" => out.foundation_bridges = parse_string_pair_list(value)?,
            other => {
                return Err(ArchParseError::UnknownField {
                    name: other.to_string(),
                    suggestion: None,
                });
            }
        }
    }
    Ok(out)
}

fn parse_corpus_invariant_list(
    expr: &Expr,
) -> Result<Vec<crate::arch_corpus::CorpusInvariant>, ArchParseError> {
    use verum_ast::expr::ArrayExpr;
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => {
            items.iter().map(parse_corpus_invariant).collect()
        }
        _ => Err(ArchParseError::InvalidValue {
            field: "invariants".to_string(),
            expected: "array literal `[CorpusInvariant.X, ...]`",
        }),
    }
}

fn parse_corpus_invariant(
    expr: &Expr,
) -> Result<crate::arch_corpus::CorpusInvariant, ArchParseError> {
    let path = parse_path_string(expr, "invariants")?;
    let last = path.split('.').next_back().unwrap_or(&path);
    Ok(match last {
        "NoCircularDependencies" => crate::arch_corpus::CorpusInvariant::NoCircularDependencies,
        "FoundationConsistency" => crate::arch_corpus::CorpusInvariant::FoundationConsistency,
        "NoLAbsClaim" => crate::arch_corpus::CorpusInvariant::NoLAbsClaim,
        "CapabilityClosure" => crate::arch_corpus::CorpusInvariant::CapabilityClosure,
        other => {
            return Err(ArchParseError::UnknownVariant {
                kind: "CorpusInvariant",
                value: other.to_string(),
            });
        }
    })
}

fn parse_string_pair_list(expr: &Expr) -> Result<Vec<(String, String)>, ArchParseError> {
    use verum_ast::expr::ArrayExpr;
    match &expr.kind {
        ExprKind::Array(ArrayExpr::List(items)) => items.iter().map(parse_string_pair).collect(),
        _ => Err(ArchParseError::InvalidValue {
            field: "foundation_bridges".to_string(),
            expected: "array literal `[(\"peer\", \"corpus_label\"), ...]`",
        }),
    }
}

fn parse_string_pair(expr: &Expr) -> Result<(String, String), ArchParseError> {
    use verum_ast::expr::ArrayExpr;
    match &expr.kind {
        ExprKind::Tuple(items) | ExprKind::Array(ArrayExpr::List(items)) if items.len() == 2 => {
            let a = parse_path_string(&items[0], "foundation_bridges[0]")?;
            let b = parse_path_string(&items[1], "foundation_bridges[1]")?;
            Ok((a, b))
        }
        _ => Err(ArchParseError::InvalidValue {
            field: "foundation_bridges_pair".to_string(),
            expected: "(peer_cog, corpus_label) two-element tuple",
        }),
    }
}

// =============================================================================
// Helper — extract NamedArg / Binary{Assign} unifying surface
// =============================================================================

fn extract_named_arg(arg: &Expr) -> Result<(String, &Expr), ArchParseError> {
    match &arg.kind {
        ExprKind::NamedArg { name, value } => Ok((name.name.as_str().to_string(), value.as_ref())),
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
                Ok((name, right.as_ref()))
            }
            _ => Err(ArchParseError::InvalidValue {
                field: "<binary-assign-lhs>".to_string(),
                expected: "named argument with Path on LHS",
            }),
        },
        _ => Err(ArchParseError::InvalidValue {
            field: "<positional>".to_string(),
            expected: "named argument `name: value` or `name = value`",
        }),
    }
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

    fn string_lit(s: &str) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Text(StringLit::Regular(s.into())),
                span(),
            )),
            span(),
        )
    }

    fn method_call(receiver: Expr, method: &str, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(receiver),
                method: Ident::new(method, span()),
                type_args: List::new(),
                args: List::from(args),
            },
            span(),
        )
    }

    fn record_expr(type_name: &str, fields: Vec<(&str, Expr)>) -> Expr {
        let inits = fields
            .into_iter()
            .map(|(name, value)| verum_ast::expr::FieldInit {
                attributes: List::new(),
                name: Ident::new(name, span()),
                value: verum_common::Maybe::Some(value),
                span: span(),
            })
            .collect::<Vec<_>>();
        Expr::new(
            ExprKind::Record {
                path: Path::new(
                    List::from(vec![PathSegment::Name(Ident::new(type_name, span()))]),
                    span(),
                ),
                fields: List::from(inits),
                base: verum_common::Maybe::None,
            },
            span(),
        )
    }

    /// The exact shape 74 core/ headers write: this is the corpus
    /// surface, in miniature, and the parser must take ALL of it.
    fn corpus_declarations_expr() -> Expr {
        let some = |inner: Expr| method_call(name_path_expr("Maybe"), "Some", vec![inner]);
        record_expr(
            "ShapeDeclarations",
            vec![
                (
                    "purpose",
                    some(record_expr(
                        "Purpose",
                        vec![
                            // The corpus spells the role both bare and
                            // with `.to_text()`; use the wrapped form
                            // here so the harder spelling is the one
                            // pinned.
                            (
                                "role",
                                method_call(string_lit("cache protocol"), "to_text", vec![]),
                            ),
                            ("k_min", dotted_path_expr(&["CveThresholdK", "FullWitness"])),
                            (
                                "v_min",
                                dotted_path_expr(&["CveThresholdV", "NamedCertification"]),
                            ),
                            (
                                "e_min",
                                dotted_path_expr(&["CveThresholdE", "StructurallyReady"]),
                            ),
                        ],
                    )),
                ),
                (
                    "substrate",
                    some(dotted_path_expr(&[
                        "CognitiveSubstrate",
                        "AnalyticDecompositional",
                    ])),
                ),
                (
                    "anchoring",
                    some(dotted_path_expr(&["FormalAnchoring", "CurryHowardLawvere"])),
                ),
                (
                    "e_sense",
                    some(dotted_path_expr(&[
                        "ExecutabilitySense",
                        "StructuralReadiness",
                    ])),
                ),
                (
                    "self_reference",
                    dotted_path_expr(&["Maybe", "None"]),
                ),
            ],
        )
    }

    #[test]
    fn parse_declarations_takes_the_corpus_surface() {
        let args = vec![named_arg("declarations", corpus_declarations_expr())];
        let shape = parse_arch_module(&args).expect(
            "the declarations field the auto-fixes promote and 74 core/ headers write must parse",
        );
        let decls = shape.declarations.expect("declarations carried onto Shape");
        let purpose = decls.purpose.expect("purpose");
        assert_eq!(purpose.role, "cache protocol");
        assert_eq!(purpose.k_min, CveThresholdK::FullWitness);
        assert_eq!(purpose.v_min, CveThresholdV::NamedCertification);
        assert_eq!(purpose.e_min, CveThresholdE::StructurallyReady);
        assert_eq!(
            decls.substrate,
            Some(CognitiveSubstrate::AnalyticDecompositional)
        );
        assert_eq!(decls.anchoring, Some(FormalAnchoring::CurryHowardLawvere));
        assert_eq!(decls.e_sense, Some(ExecutabilitySense::StructuralReadiness));
        assert_eq!(decls.self_reference, None);
    }

    #[test]
    fn parse_cve_closure_record_spelling_matches_the_documented_surface() {
        // The shape reference documents `cve_closure: CveClosure { ... }`
        // as the full surface; a user copying that example must not die
        // with UnknownField("cve_closure") while the flat spellings
        // (cve_closure_C / _V_strategy / _E) keep working beside it.
        let some = |inner: Expr| method_call(name_path_expr("Maybe"), "Some", vec![inner]);
        let record = record_expr(
            "CveClosure",
            vec![
                ("constructive", some(string_lit("explicit_ctor"))),
                (
                    "verifiable_strategy",
                    some(dotted_path_expr(&["VerifyStrategy", "Certified"])),
                ),
                ("executable", some(string_lit("verum extract"))),
            ],
        );
        let shape = parse_arch_module(&[named_arg("cve_closure", record)])
            .expect("the documented record spelling must parse");
        assert_eq!(shape.cve_closure.constructive.as_deref(), Some("explicit_ctor"));
        assert_eq!(
            shape.cve_closure.verifiable_strategy,
            Some(VerifyStrategy::Certified)
        );
        assert_eq!(shape.cve_closure.executable.as_deref(), Some("verum extract"));
        assert!(shape.cve_closure.is_fully_closed());
    }

    #[test]
    fn parse_declarations_rejects_unknown_record_field() {
        let expr = record_expr("ShapeDeclarations", vec![("porpoise", string_lit("x"))]);
        let err = parse_arch_module(&[named_arg("declarations", expr)]).unwrap_err();
        assert!(
            matches!(err, ArchParseError::UnknownField { ref name, .. } if name == "porpoise"),
            "unknown declaration fields stay errors, got {err:?}",
        );
    }

    #[test]
    fn parse_declarations_rejects_unknown_variant() {
        let expr = record_expr(
            "ShapeDeclarations",
            vec![(
                "substrate",
                method_call(
                    name_path_expr("Maybe"),
                    "Some",
                    vec![dotted_path_expr(&["CognitiveSubstrate", "Nonexistent"])],
                ),
            )],
        );
        let err = parse_arch_module(&[named_arg("declarations", expr)]).unwrap_err();
        assert!(
            matches!(err, ArchParseError::UnknownVariant { kind, ref value } if kind == "CognitiveSubstrate" && value == "Nonexistent"),
            "an unknown enum variant is an error, not a default, got {err:?}",
        );
    }

    #[test]
    fn parse_declarations_takes_a_self_reference_witness() {
        let expr = record_expr(
            "ShapeDeclarations",
            vec![(
                "self_reference",
                method_call(
                    name_path_expr("Maybe"),
                    "Some",
                    vec![record_expr(
                        "SelfReferenceWitness",
                        vec![
                            ("operator", string_lit("core.meta.op")),
                            ("fixed_point", string_lit("core.meta.fix")),
                            (
                                "fixpoint_class",
                                Expr::new(
                                    ExprKind::Call {
                                        func: Heap::new(name_path_expr("fixpoint_class_banach")),
                                        type_args: List::new(),
                                        args: List::new(),
                                    },
                                    span(),
                                ),
                            ),
                        ],
                    )],
                ),
            )],
        );
        let shape = parse_arch_module(&[named_arg("declarations", expr)])
            .expect("the AP-040 discharge surface must parse");
        let witness = shape
            .declarations
            .and_then(|d| d.self_reference)
            .expect("witness carried");
        assert_eq!(witness.operator, "core.meta.op");
        assert_eq!(witness.fixed_point, "core.meta.fix");
        assert_eq!(witness.fixpoint_class, FixpointClass::banach());
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
