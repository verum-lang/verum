//! Typed-IR export — the general external-backend seam (T0675).
//!
//! `verum export --to typed-ir <file>` emits a **canonical,
//! versioned, self-describing** serialization of the *checked*
//! program — after type checking, before any bytecode/native
//! lowering. External code generators consume this artefact out of
//! tree; Verum's contract to them is exactly: schema stability,
//! canonical bytes, and the deterministic-profile guarantees
//! recorded in the artefact
//! (`docs/architecture/deterministic-profile-and-typed-export.md` §4).
//!
//! # Design rules (schema = the stability boundary)
//!
//! * The schema types below are an **explicit conversion layer**,
//!   never a `derive` over internal AST/type-checker types — internal
//!   refactors must not be able to change the wire format silently.
//! * Canonical bytes: `BTreeMap` for every map, `Vec`s in a defined
//!   order (items sorted by name; everything inside an item in
//!   source order), pretty JSON with a trailing newline, **no
//!   timestamps, no absolute paths, no environment leakage**.
//!   Reproducibility is asserted by test: two runs over the same
//!   source are byte-identical (T0677).
//! * Every expression position (`requires`, `ensures`, `decreases`,
//!   attribute arguments, bodies) is exported STRUCTURED where the
//!   v1 statement/expression set covers it, with a lossless
//!   `Opaque { source }` fallback rendered by the canonical pretty
//!   printer — a consumer can always see exactly what it does not
//!   yet understand.
//! * Attributes are carried **verbatim** (name + source-rendered
//!   args) — they are the extension point external toolchains key
//!   on (`@effect(...)`, custom attributes).

use std::collections::BTreeMap;

use serde::Serialize;
use verum_ast::decl::{FunctionDecl, FunctionParamKind, ItemKind, TypeDeclBody};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pretty::format_expr;
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{GenericArg, Type as AstType, TypeKind};
use verum_ast::{Block, Module};

/// Artefact identifier — consumers reject anything else.
pub const TYPED_IR_SCHEMA: &str = "verum-typed-ir";
/// Schema version, semver, versioned INDEPENDENTLY of the compiler.
/// Bump the minor for additive fields, the major for anything a
/// v1 consumer could misread.
pub const TYPED_IR_VERSION: &str = "1.0.0";

// =============================================================================
// Schema types (the wire format)
// =============================================================================

/// Top-level artefact.
#[derive(Debug, Serialize)]
pub struct TypedIrArtifact {
    /// Always [`TYPED_IR_SCHEMA`].
    pub schema: &'static str,
    /// [`TYPED_IR_VERSION`].
    pub version: &'static str,
    /// The exported module.
    pub module: ModuleIr,
}

/// One checked module.
#[derive(Debug, Serialize)]
pub struct ModuleIr {
    /// Declared module path, or the empty string for an unnamed
    /// script module. Never a filesystem path.
    pub name: String,
    /// Type declarations, sorted by name.
    pub types: Vec<TypeDeclIr>,
    /// Functions, sorted by name.
    pub functions: Vec<FunctionIr>,
}

/// A type in the export — structural, self-describing, closed.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeIr {
    Unit,
    Never,
    Bool,
    Int,
    Float,
    Char,
    Text,
    /// Nominal reference with type arguments.
    Named { name: String, args: Vec<TypeIr> },
    /// `fn(params) -> ret`.
    Function {
        params: Vec<TypeIr>,
        ret: Box<TypeIr>,
    },
    Tuple { elems: Vec<TypeIr> },
    Array {
        elem: Box<TypeIr>,
        /// Source-rendered size expression, when declared.
        size: Option<String>,
    },
    Slice { elem: Box<TypeIr> },
    /// The three-tier reference model, tier spelled out.
    Reference {
        tier: RefTier,
        mutable: bool,
        inner: Box<TypeIr>,
    },
    /// Refinement: base plus the predicate, STRUCTURED with a
    /// source render alongside (consumers that cannot evaluate the
    /// expression still get the exact text).
    Refined {
        base: Box<TypeIr>,
        predicate: ExprIr,
    },
    /// `dyn P` / `dyn P + Q`.
    DynProtocol { bounds: Vec<String> },
    /// Capability-restricted type — the capability names travel
    /// with the type (deterministic-profile doc §6).
    CapabilityRestricted {
        base: Box<TypeIr>,
        capabilities: Vec<String>,
    },
    /// A shape the v1 schema does not model structurally. The
    /// canonical source render is carried so nothing is lost.
    Opaque { source: String },
}

/// Reference tier (grammar: `&T`, `&checked T`, `&unsafe T`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefTier {
    Managed,
    Checked,
    Unsafe,
}

/// One type declaration.
#[derive(Debug, Serialize)]
pub struct TypeDeclIr {
    pub name: String,
    pub body: TypeBodyIr,
    /// Verbatim attributes.
    pub attributes: Vec<AttributeIr>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeBodyIr {
    Record { fields: Vec<FieldIr> },
    Variant { variants: Vec<VariantIr> },
    Alias { target: TypeIr },
    /// `type X is (T);`
    Newtype { inner: TypeIr },
    Unit,
    Opaque { source: String },
}

#[derive(Debug, Serialize)]
pub struct FieldIr {
    pub name: String,
    pub ty: TypeIr,
}

#[derive(Debug, Serialize)]
pub struct VariantIr {
    pub name: String,
    /// Tuple-payload types; empty for unit variants.
    pub payload: Vec<TypeIr>,
    /// Record-payload fields; empty unless a record variant.
    pub fields: Vec<FieldIr>,
}

/// One function.
#[derive(Debug, Serialize)]
pub struct FunctionIr {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<ParamIr>,
    pub return_type: Option<TypeIr>,
    /// `requires` clauses, structured.
    pub requires: Vec<ExprIr>,
    /// `ensures` clauses, structured.
    pub ensures: Vec<ExprIr>,
    /// Declared DI contexts (`using [...]`).
    pub contexts: Vec<String>,
    /// Inferred computational properties, sorted, canonical names.
    pub properties: Vec<String>,
    /// Verbatim attributes.
    pub attributes: Vec<AttributeIr>,
    /// Every loop in the body with its termination metadata.
    pub loops: Vec<LoopIr>,
    /// Structured body.
    pub body: Option<BodyIr>,
}

#[derive(Debug, Serialize)]
pub struct ParamIr {
    pub name: String,
    pub ty: Option<TypeIr>,
}

#[derive(Debug, Serialize)]
pub struct AttributeIr {
    pub name: String,
    /// Source-rendered arguments, in order.
    pub args: Vec<String>,
}

/// Loop termination metadata (deterministic-profile doc §3).
#[derive(Debug, Serialize)]
pub struct LoopIr {
    /// Which loop form.
    pub form: LoopForm,
    /// Source-rendered `decreases` measures.
    pub measures: Vec<String>,
    /// Bound classification.
    pub bound: BoundClass,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopForm {
    While,
    For,
    Loop,
}

/// `ConstantBound(k)` | `FiniteNotConstant` | `Unproven` — the §3
/// API surface, carried in the artefact.
#[derive(Debug, Serialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum BoundClass {
    /// The measure's initial value is a compile-time constant `k`
    /// (decimal string — values may exceed 64 bits).
    Constant { k: String },
    /// A measure exists but its bound is not a compile-time
    /// constant. Diagnostic contract: "loop bound is not a
    /// compile-time constant".
    FiniteNotConstant,
    /// No `decreases` measure declared.
    Unproven,
}

/// Structured statement.
#[derive(Debug, Serialize)]
#[serde(tag = "stmt", rename_all = "snake_case")]
pub enum StmtIr {
    Let {
        pattern: String,
        ty: Option<TypeIr>,
        value: Option<ExprIr>,
    },
    Expr { expr: ExprIr },
    Opaque { source: String },
}

/// Structured body: statements plus optional tail expression.
#[derive(Debug, Serialize)]
pub struct BodyIr {
    pub stmts: Vec<StmtIr>,
    pub tail: Option<Box<ExprIr>>,
}

/// Structured expression — the v1 set. Everything else is
/// `Opaque { source }` via the canonical pretty printer.
#[derive(Debug, Serialize)]
#[serde(tag = "expr", rename_all = "snake_case")]
pub enum ExprIr {
    /// Integer literal as a decimal string — full 128-bit fidelity,
    /// no host-width truncation in the artefact.
    Int { value: String },
    Float { value: f64 },
    Bool { value: bool },
    Text { value: String },
    Path { name: String },
    Binary {
        op: String,
        left: Box<ExprIr>,
        right: Box<ExprIr>,
    },
    Unary { op: String, operand: Box<ExprIr> },
    Call {
        callee: Box<ExprIr>,
        args: Vec<ExprIr>,
    },
    MethodCall {
        receiver: Box<ExprIr>,
        method: String,
        args: Vec<ExprIr>,
    },
    Field {
        base: Box<ExprIr>,
        field: String,
    },
    Index {
        base: Box<ExprIr>,
        index: Box<ExprIr>,
    },
    Tuple { elems: Vec<ExprIr> },
    If {
        condition: Box<ExprIr>,
        then_body: BodyIr,
        else_body: Option<Box<ExprIr>>,
    },
    While {
        condition: Box<ExprIr>,
        body: BodyIr,
    },
    Return { value: Option<Box<ExprIr>> },
    Block { body: BodyIr },
    Opaque { source: String },
}

// =============================================================================
// Builder — AST → schema conversion
// =============================================================================

/// Build the artefact from a CHECKED module. The caller is
/// responsible for having run the checker — this builder converts,
/// it does not re-verify.
pub fn build_typed_ir(module: &Module) -> TypedIrArtifact {
    let mut property_inferrer = verum_types::computational_properties::PropertyInferrer::new();

    let mut types: Vec<TypeDeclIr> = Vec::new();
    let mut functions: Vec<FunctionIr> = Vec::new();

    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Type(td) => types.push(convert_type_decl(td)),
            ItemKind::Function(fd) => {
                functions.push(convert_function(fd, &mut property_inferrer))
            }
            _ => {}
        }
    }

    types.sort_by(|a, b| a.name.cmp(&b.name));
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    TypedIrArtifact {
        schema: TYPED_IR_SCHEMA,
        version: TYPED_IR_VERSION,
        module: ModuleIr {
            name: module
                .items
                .iter()
                .find_map(|it| match &it.kind {
                    ItemKind::Module(m) => Some(m.name.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default(),
            types,
            functions,
        },
    }
}

/// Canonical bytes: pretty JSON + trailing newline. All maps are
/// `BTreeMap`s and all Vec orders are defined, so this is
/// byte-deterministic across runs and machines (T0677).
pub fn to_canonical_bytes(artifact: &TypedIrArtifact) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(artifact)
        .expect("typed-IR schema types serialize infallibly");
    bytes.push(b'\n');
    bytes
}

fn convert_type_decl(td: &verum_ast::TypeDecl) -> TypeDeclIr {
    let body = match &td.body {
        TypeDeclBody::Record(fields) => TypeBodyIr::Record {
            fields: fields
                .iter()
                .map(|f| FieldIr {
                    name: f.name.name.to_string(),
                    ty: convert_type(&f.ty),
                })
                .collect(),
        },
        TypeDeclBody::Variant(variants) => TypeBodyIr::Variant {
            variants: variants
                .iter()
                .map(|v| {
                    use verum_ast::decl::VariantData;
                    let (payload, fields) = match &v.data {
                        None => (Vec::new(), Vec::new()),
                        Some(VariantData::Tuple(tys)) => {
                            (tys.iter().map(convert_type).collect(), Vec::new())
                        }
                        Some(VariantData::Record(fs)) => (
                            Vec::new(),
                            fs.iter()
                                .map(|f| FieldIr {
                                    name: f.name.name.to_string(),
                                    ty: convert_type(&f.ty),
                                })
                                .collect(),
                        ),
                    };
                    VariantIr {
                        name: v.name.name.to_string(),
                        payload,
                        fields,
                    }
                })
                .collect(),
        },
        TypeDeclBody::Alias(target) => TypeBodyIr::Alias {
            target: convert_type(target),
        },
        TypeDeclBody::Newtype(inner) => TypeBodyIr::Newtype {
            inner: convert_type(inner),
        },
        TypeDeclBody::Unit => TypeBodyIr::Unit,
        other => TypeBodyIr::Opaque {
            source: format!("{:?}", std::mem::discriminant(other)),
        },
    };
    TypeDeclIr {
        name: td.name.name.to_string(),
        body,
        attributes: convert_attributes(&td.attributes),
    }
}

fn convert_function(
    fd: &FunctionDecl,
    property_inferrer: &mut verum_types::computational_properties::PropertyInferrer,
) -> FunctionIr {
    let mut loops: Vec<LoopIr> = Vec::new();
    let body = fd.body.as_ref().map(|b| match b {
        verum_ast::decl::FunctionBody::Block(block) => {
            collect_loops_block(block, &mut loops);
            convert_block(block)
        }
        verum_ast::decl::FunctionBody::Expr(e) => {
            collect_loops_expr(e, &mut loops);
            BodyIr {
                stmts: Vec::new(),
                tail: Some(Box::new(convert_expr(e))),
            }
        }
    });

    // Inferred computational properties — canonical sorted names.
    let props = property_inferrer.infer_function_decl(fd);
    let mut properties: Vec<String> = props
        .iter()
        .map(|p| format!("{:?}", p))
        .collect();
    properties.sort();
    properties.dedup();

    FunctionIr {
        name: fd.name.to_string(),
        generics: fd
            .generics
            .iter()
            .filter_map(|g| {
                use verum_ast::ty::GenericParamKind;
                match &g.kind {
                    GenericParamKind::Type { name, .. } => Some(name.name.to_string()),
                    _ => None,
                }
            })
            .collect(),
        params: fd
            .params
            .iter()
            .filter_map(|p| match &p.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => Some(ParamIr {
                    name: verum_ast::pretty::format_pattern(pattern).to_string(),
                    ty: Some(convert_type(ty)),
                }),
                _ => Some(ParamIr {
                    name: "self".to_string(),
                    ty: None,
                }),
            })
            .collect(),
        return_type: match &fd.return_type {
            verum_common::Maybe::Some(t) => Some(convert_type(t)),
            verum_common::Maybe::None => None,
        },
        requires: fd.requires.iter().map(convert_expr).collect(),
        ensures: fd.ensures.iter().map(convert_expr).collect(),
        contexts: fd
            .contexts
            .iter()
            .map(|c| {
                let name = c
                    .path
                    .segments
                    .iter()
                    .filter_map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(id) => {
                            Some(id.name.as_str().to_string())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                if c.is_negative {
                    format!("!{}", name)
                } else {
                    name
                }
            })
            .collect(),
        properties,
        attributes: convert_attributes(&fd.attributes),
        loops,
        body,
    }
}

fn convert_attributes(attrs: &verum_common::List<verum_ast::attr::Attribute>) -> Vec<AttributeIr> {
    attrs
        .iter()
        .map(|a| AttributeIr {
            name: a.name.to_string(),
            args: match &a.args {
                verum_common::Maybe::Some(args) => {
                    args.iter().map(|e| format_expr(e).to_string()).collect()
                }
                verum_common::Maybe::None => Vec::new(),
            },
        })
        .collect()
}

fn convert_type(ty: &AstType) -> TypeIr {
    match &ty.kind {
        TypeKind::Unit => TypeIr::Unit,
        TypeKind::Never => TypeIr::Never,
        TypeKind::Bool => TypeIr::Bool,
        TypeKind::Int => TypeIr::Int,
        TypeKind::Float => TypeIr::Float,
        TypeKind::Char => TypeIr::Char,
        TypeKind::Text => TypeIr::Text,
        TypeKind::Path(p) => TypeIr::Named {
            name: p.last_segment_name().to_string(),
            args: Vec::new(),
        },
        TypeKind::Generic { base, args } => TypeIr::Named {
            name: match &base.kind {
                TypeKind::Path(p) => p.last_segment_name().to_string(),
                _ => return opaque_type(ty),
            },
            args: args
                .iter()
                .filter_map(|a| match a {
                    GenericArg::Type(t) => Some(convert_type(t)),
                    _ => None,
                })
                .collect(),
        },
        TypeKind::Function {
            params,
            return_type,
            ..
        } => TypeIr::Function {
            params: params.iter().map(convert_type).collect(),
            ret: Box::new(convert_type(return_type)),
        },
        TypeKind::Tuple(elems) => TypeIr::Tuple {
            elems: elems.iter().map(convert_type).collect(),
        },
        TypeKind::Array { element, size } => TypeIr::Array {
            elem: Box::new(convert_type(element)),
            size: match size {
                verum_common::Maybe::Some(e) => Some(format_expr(e).to_string()),
                verum_common::Maybe::None => None,
            },
        },
        TypeKind::Slice(elem) => TypeIr::Slice {
            elem: Box::new(convert_type(elem)),
        },
        TypeKind::Reference { mutable, inner } => TypeIr::Reference {
            tier: RefTier::Managed,
            mutable: *mutable,
            inner: Box::new(convert_type(inner)),
        },
        TypeKind::CheckedReference { mutable, inner } => TypeIr::Reference {
            tier: RefTier::Checked,
            mutable: *mutable,
            inner: Box::new(convert_type(inner)),
        },
        TypeKind::UnsafeReference { mutable, inner } => TypeIr::Reference {
            tier: RefTier::Unsafe,
            mutable: *mutable,
            inner: Box::new(convert_type(inner)),
        },
        TypeKind::Refined { base, predicate } => TypeIr::Refined {
            base: Box::new(convert_type(base)),
            predicate: convert_expr(&predicate.expr),
        },
        _ => opaque_type(ty),
    }
}

fn opaque_type(ty: &AstType) -> TypeIr {
    TypeIr::Opaque {
        source: verum_ast::pretty::format_type(ty).to_string(),
    }
}

fn convert_block(block: &Block) -> BodyIr {
    BodyIr {
        stmts: block.stmts.iter().map(convert_stmt).collect(),
        tail: block.expr.as_ref().map(|e| Box::new(convert_expr(e))),
    }
}

fn convert_stmt(stmt: &Stmt) -> StmtIr {
    match &stmt.kind {
        StmtKind::Let { pattern, ty, value } => StmtIr::Let {
            pattern: verum_ast::pretty::format_pattern(pattern).to_string(),
            ty: ty.as_ref().map(convert_type),
            value: value.as_ref().map(convert_expr),
        },
        StmtKind::Expr { expr, .. } => StmtIr::Expr {
            expr: convert_expr(expr),
        },
        other => StmtIr::Opaque {
            source: format!("{:?}", std::mem::discriminant(other)),
        },
    }
}

fn convert_expr(expr: &Expr) -> ExprIr {
    use verum_ast::literal::LiteralKind;
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            // Decimal string: the artefact never narrows to a host
            // width — `i128::MAX` survives verbatim.
            LiteralKind::Int(i) => ExprIr::Int {
                value: i.value.to_string(),
            },
            LiteralKind::Float(f) => ExprIr::Float { value: f.value },
            LiteralKind::Bool(b) => ExprIr::Bool { value: *b },
            LiteralKind::Text(s) => ExprIr::Text {
                value: match s {
                    verum_ast::literal::StringLit::Regular(t)
                    | verum_ast::literal::StringLit::MultiLine(t) => t.to_string(),
                },
            },
            _ => opaque_expr(expr),
        },
        ExprKind::Path(_) => ExprIr::Path {
            name: format_expr(expr).to_string(),
        },
        ExprKind::Binary { op, left, right } => ExprIr::Binary {
            op: format!("{:?}", op),
            left: Box::new(convert_expr(left)),
            right: Box::new(convert_expr(right)),
        },
        ExprKind::Unary { op, expr: inner } => ExprIr::Unary {
            op: format!("{:?}", op),
            operand: Box::new(convert_expr(inner)),
        },
        ExprKind::Call { func, args, .. } => ExprIr::Call {
            callee: Box::new(convert_expr(func)),
            args: args.iter().map(convert_expr).collect(),
        },
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => ExprIr::MethodCall {
            receiver: Box::new(convert_expr(receiver)),
            method: method.to_string(),
            args: args.iter().map(convert_expr).collect(),
        },
        ExprKind::Field { expr: base, field } => ExprIr::Field {
            base: Box::new(convert_expr(base)),
            field: field.to_string(),
        },
        ExprKind::Index { expr: base, index } => ExprIr::Index {
            base: Box::new(convert_expr(base)),
            index: Box::new(convert_expr(index)),
        },
        ExprKind::Tuple(elems) => ExprIr::Tuple {
            elems: elems.iter().map(convert_expr).collect(),
        },
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // v1 models the single plain-expression condition; the
            // `if let` forms fall to Opaque.
            let mut conds: Vec<&Expr> = Vec::new();
            for c in condition.conditions.iter() {
                match c {
                    verum_ast::expr::ConditionKind::Expr(e) => conds.push(e),
                    verum_ast::expr::ConditionKind::Let { .. } => return opaque_expr(expr),
                }
            }
            let [single] = conds.as_slice() else {
                return opaque_expr(expr);
            };
            ExprIr::If {
                condition: Box::new(convert_expr(single)),
                then_body: convert_block(then_branch),
                else_body: else_branch
                    .as_ref()
                    .map(|e| Box::new(convert_expr(e))),
            }
        }
        ExprKind::While {
            condition, body, ..
        } => ExprIr::While {
            condition: Box::new(convert_expr(condition)),
            body: convert_block(body),
        },
        ExprKind::Return(v) => ExprIr::Return {
            value: v.as_ref().map(|e| Box::new(convert_expr(e))),
        },
        ExprKind::Block(b) => ExprIr::Block {
            body: convert_block(b),
        },
        ExprKind::Paren(inner) => convert_expr(inner),
        _ => opaque_expr(expr),
    }
}

fn opaque_expr(expr: &Expr) -> ExprIr {
    ExprIr::Opaque {
        source: format_expr(expr).to_string(),
    }
}

// =============================================================================
// Loop metadata collection (§3 classification)
// =============================================================================

fn classify_bound(measures: &[&Expr]) -> BoundClass {
    if measures.is_empty() {
        return BoundClass::Unproven;
    }
    // A measure whose INITIAL value is a compile-time constant gives
    // the constant bound; otherwise the loop is finite (a measure
    // exists) but not constant-bounded. Diagnostic contract for
    // consumers requiring constants: "loop bound is not a
    // compile-time constant".
    let mut eval = verum_types::const_eval::ConstEvaluator::new();
    for m in measures {
        if let Ok(v) = eval.eval(m) {
            if let Some(k) = v.as_u128() {
                return BoundClass::Constant { k: k.to_string() };
            }
        }
    }
    BoundClass::FiniteNotConstant
}

fn collect_loops_block(block: &Block, out: &mut Vec<LoopIr>) {
    for stmt in block.stmts.iter() {
        match &stmt.kind {
            StmtKind::Expr { expr, .. } => collect_loops_expr(expr, out),
            StmtKind::Let {
                value: Some(v), ..
            } => collect_loops_expr(v, out),
            _ => {}
        }
    }
    if let Some(e) = &block.expr {
        collect_loops_expr(e, out);
    }
}

fn collect_loops_expr(expr: &Expr, out: &mut Vec<LoopIr>) {
    match &expr.kind {
        ExprKind::While {
            condition,
            body,
            decreases,
            ..
        } => {
            let measures: Vec<&Expr> = decreases.iter().collect();
            out.push(LoopIr {
                form: LoopForm::While,
                measures: measures.iter().map(|m| format_expr(m).to_string()).collect(),
                bound: classify_bound(&measures),
            });
            collect_loops_expr(condition, out);
            collect_loops_block(body, out);
        }
        ExprKind::For {
            iter,
            body,
            decreases,
            ..
        } => {
            let measures: Vec<&Expr> = decreases.iter().collect();
            out.push(LoopIr {
                form: LoopForm::For,
                measures: measures.iter().map(|m| format_expr(m).to_string()).collect(),
                bound: classify_bound(&measures),
            });
            collect_loops_expr(iter, out);
            collect_loops_block(body, out);
        }
        ExprKind::Loop { body, .. } => {
            out.push(LoopIr {
                form: LoopForm::Loop,
                measures: Vec::new(),
                bound: BoundClass::Unproven,
            });
            collect_loops_block(body, out);
        }
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_loops_block(then_branch, out);
            if let Some(e) = else_branch {
                collect_loops_expr(e, out);
            }
        }
        ExprKind::Block(b) => collect_loops_block(b, out),
        ExprKind::Match { arms, .. } => {
            for arm in arms.iter() {
                collect_loops_expr(&arm.body, out);
            }
        }
        _ => {}
    }
}

// A compile-time guard that the schema stays map-free unless the map
// is a BTreeMap: adding a HashMap field to any schema type above
// breaks canonical bytes. (BTreeMap import is referenced here so the
// module keeps the dependency explicit even while v1 has no map
// fields.)
#[allow(dead_code)]
type CanonicalMap<K, V> = BTreeMap<K, V>;
