//! AST formatting utilities for LSP
//!
//! This module provides formatting functions specific to verum_ast types,
//! used across hover.rs, completion.rs, and other LSP modules.
//!
//! Formatting utilities shared across LSP modules (hover, completion, etc.).

use verum_ast::LiteralKind;
use verum_ast::decl::{FunctionParam, FunctionParamKind, TypeDeclBody, VariantData};
use verum_ast::pattern::VariantPatternData;
use verum_ast::ty::{PathSegment, TypeKind};

/// Format function parameters for display
///
/// # Examples
/// ```ignore
/// let params = vec![/* function params */];
/// let formatted = format_params(&params);
/// // Output: "x: Int, y: Float"
/// ```
pub fn format_params(params: &[FunctionParam]) -> String {
    params
        .iter()
        .filter_map(|p| match &p.kind {
            FunctionParamKind::Regular { pattern, ty, .. } => {
                Some(format!("{}: {}", format_pattern(pattern), format_type(ty)))
            }
            FunctionParamKind::SelfValue => Some("self".to_string()),
            FunctionParamKind::SelfValueMut => Some("mut self".to_string()),
            FunctionParamKind::SelfRef => Some("&self".to_string()),
            FunctionParamKind::SelfRefMut => Some("&mut self".to_string()),
            FunctionParamKind::SelfOwn => Some("%self".to_string()),
            FunctionParamKind::SelfOwnMut => Some("%mut self".to_string()),
            FunctionParamKind::SelfRefChecked => Some("&checked self".to_string()),
            FunctionParamKind::SelfRefCheckedMut => Some("&checked mut self".to_string()),
            FunctionParamKind::SelfRefUnsafe => Some("&unsafe self".to_string()),
            FunctionParamKind::SelfRefUnsafeMut => Some("&unsafe mut self".to_string()),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format a pattern for display
///
/// Handles all pattern kinds including identifiers, wildcards, tuples, variants,
/// literals, or-patterns, rest patterns, ranges, slices, and more.
///
/// # Examples
/// ```ignore
/// let pattern = /* pattern */;
/// let formatted = format_pattern(&pattern);
/// // Output: "x" or "_" or "Some(value)" or "(a, b, c)"
/// ```
pub fn format_pattern(pattern: &verum_ast::Pattern) -> String {
    use verum_ast::PatternKind;
    use verum_common::Maybe;
    match &pattern.kind {
        PatternKind::Ident {
            name,
            mutable,
            by_ref,
            subpattern,
        } => {
            let ref_str = if *by_ref { "ref " } else { "" };
            let mut_str = if *mutable { "mut " } else { "" };
            let base = format!("{}{}{}", ref_str, mut_str, name.as_str());
            match subpattern.as_ref() {
                Maybe::Some(sub) => format!("{} @ {}", base, format_pattern(sub)),
                Maybe::None => base,
            }
        }
        PatternKind::Wildcard => "_".to_string(),
        PatternKind::Tuple(patterns) => {
            let formatted = patterns
                .iter()
                .map(format_pattern)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", formatted)
        }
        PatternKind::Array(patterns) => {
            let formatted = patterns
                .iter()
                .map(format_pattern)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", formatted)
        }
        PatternKind::Variant { path, data } => {
            let path_str = format_path(path);
            match data.as_ref() {
                Maybe::None => path_str,
                Maybe::Some(inner) => match inner {
                    VariantPatternData::Tuple(patterns) => {
                        let formatted = patterns
                            .iter()
                            .map(format_pattern)
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{}({})", path_str, formatted)
                    }
                    VariantPatternData::Record { fields, .. } => {
                        let formatted = fields
                            .iter()
                            .filter_map(|f| {
                                f.pattern
                                    .as_ref()
                                    .map(|p| format!("{}: {}", f.name.as_str(), format_pattern(p)))
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{} {{ {} }}", path_str, formatted)
                    }
                },
            }
        }
        PatternKind::Record { path, fields, rest } => {
            let path_str = format_path(path);
            let formatted = fields
                .iter()
                .filter_map(|f| match f.pattern.as_ref() {
                    Maybe::Some(p) => Some(format!("{}: {}", f.name.as_str(), format_pattern(p))),
                    Maybe::None => Some(f.name.as_str().to_string()),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let rest_str = if *rest { ", .." } else { "" };
            format!("{} {{ {}{} }}", path_str, formatted, rest_str)
        }
        PatternKind::Literal(lit) => format_literal(lit),
        PatternKind::Or(patterns) => patterns
            .iter()
            .map(format_pattern)
            .collect::<Vec<_>>()
            .join(" | "),
        PatternKind::Rest => "..".to_string(),
        PatternKind::Range {
            start,
            end,
            inclusive,
        } => {
            let start_str = match start.as_ref() {
                Maybe::Some(l) => format_literal(l),
                Maybe::None => String::new(),
            };
            let end_str = match end.as_ref() {
                Maybe::Some(l) => format_literal(l),
                Maybe::None => String::new(),
            };
            if *inclusive {
                format!("{}..={}", start_str, end_str)
            } else {
                format!("{}..{}", start_str, end_str)
            }
        }
        PatternKind::Slice {
            before,
            rest,
            after,
        } => {
            let before_str: Vec<String> = before.iter().map(format_pattern).collect();
            let after_str: Vec<String> = after.iter().map(format_pattern).collect();
            let rest_str = match rest.as_ref() {
                Maybe::Some(r) => format_pattern(r),
                Maybe::None => "..".to_string(),
            };
            let mut parts = before_str;
            parts.push(rest_str);
            parts.extend(after_str);
            format!("[{}]", parts.join(", "))
        }
        PatternKind::Reference { mutable, inner } => {
            if *mutable {
                format!("&mut {}", format_pattern(inner))
            } else {
                format!("&{}", format_pattern(inner))
            }
        }
        PatternKind::Paren(inner) => format!("({})", format_pattern(inner)),
        _ => "...".to_string(),
    }
}

/// Format a type for display
///
/// # Examples
/// ```ignore
/// let ty = /* type */;
/// let formatted = format_type(&ty);
/// // Output: "List<Int>" or "Map<Text, Float>"
/// ```
pub fn format_type(ty: &verum_ast::Type) -> String {
    if let Some(name) = ty.kind.primitive_name() {
        return name.to_string();
    }
    match &ty.kind {
        TypeKind::Path(path) => {
            // Simple path formatting
            format_path(path)
        }
        TypeKind::Tuple(types) => {
            format!(
                "({})",
                types.iter().map(format_type).collect::<Vec<_>>().join(", ")
            )
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            format!(
                "fn({}) -> {}",
                params
                    .iter()
                    .map(format_type)
                    .collect::<Vec<_>>()
                    .join(", "),
                format_type(return_type)
            )
        }
        TypeKind::Reference { inner, mutable } => {
            if *mutable {
                format!("&mut {}", format_type(inner))
            } else {
                format!("&{}", format_type(inner))
            }
        }
        TypeKind::Array { element, size } => {
            use verum_common::Maybe;
            match size.as_ref() {
                Maybe::Some(s) => format!("[{}; {}]", format_type(element), format_expr(s)),
                Maybe::None => format!("[{}; _]", format_type(element)),
            }
        }
        TypeKind::Slice(inner) => {
            format!("[{}]", format_type(inner))
        }
        TypeKind::Inferred => "_".to_string(),
        _ => "...".to_string(),
    }
}

/// Format a path for display
fn format_path(path: &verum_ast::Path) -> String {
    path.segments
        .iter()
        .map(|seg| match seg {
            PathSegment::Name(ident) => ident.as_str().to_string(),
            PathSegment::SelfValue => "self".to_string(),
            PathSegment::Super => "super".to_string(),
            PathSegment::Cog => "cog".to_string(),
            PathSegment::Relative => ".".to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Format an expression for display
///
/// Handles all common expression kinds including binary, unary, call, field access,
/// index, tuple, array, block, if, match, for, while, loop, closure, and more.
fn format_expr(expr: &verum_ast::Expr) -> String {
    use verum_ast::ExprKind;
    use verum_ast::expr::{ArrayExpr, BinOp, UnOp};

    match &expr.kind {
        ExprKind::Literal(lit) => format_literal(lit),
        ExprKind::Path(path) => format_path(path),
        ExprKind::Binary { left, op, right } => {
            let op_str = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Rem => "%",
                BinOp::And => "&&",
                BinOp::Or => "||",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
                BinOp::BitXor => "^",
                BinOp::Shl => "<<",
                BinOp::Shr => ">>",
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                _ => "??",
            };
            format!("{} {} {}", format_expr(left), op_str, format_expr(right))
        }
        ExprKind::Unary { op, expr: inner } => {
            let op_str = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
                UnOp::Ref => "&",
                UnOp::RefMut => "&mut ",
                UnOp::Deref => "*",
                _ => "?",
            };
            format!("{}{}", op_str, format_expr(inner))
        }
        ExprKind::Call { func, args, .. } => {
            let args_str = args.iter().map(format_expr).collect::<Vec<_>>().join(", ");
            format!("{}({})", format_expr(func), args_str)
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            let args_str = args.iter().map(format_expr).collect::<Vec<_>>().join(", ");
            format!(
                "{}.{}({})",
                format_expr(receiver),
                method.as_str(),
                args_str
            )
        }
        ExprKind::Field { expr: inner, field } => {
            format!("{}.{}", format_expr(inner), field.as_str())
        }
        ExprKind::Index { expr: base, index } => {
            format!("{}[{}]", format_expr(base), format_expr(index))
        }
        ExprKind::Tuple(elements) => {
            let elems = elements
                .iter()
                .map(format_expr)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", elems)
        }
        ExprKind::Array(array_expr) => match array_expr {
            ArrayExpr::List(elements) => {
                let elems = elements
                    .iter()
                    .map(format_expr)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{}]", elems)
            }
            ArrayExpr::Repeat { value, count } => {
                format!("[{}; {}]", format_expr(value), format_expr(count))
            }
        },
        ExprKind::Cast { expr: inner, ty } => {
            format!("{} as {}", format_expr(inner), format_type(ty))
        }
        ExprKind::Block(block) => {
            use verum_common::Maybe;
            if block.stmts.is_empty() {
                match block.expr.as_ref() {
                    Maybe::Some(tail) => format!("{{ {} }}", format_expr(tail)),
                    Maybe::None => "{ }".to_string(),
                }
            } else {
                "{ ... }".to_string()
            }
        }
        ExprKind::If {
            condition,
            then_branch: _,
            else_branch,
        } => {
            use verum_common::Maybe;
            let cond_str = format_if_condition(condition);
            let else_str = match else_branch.as_ref() {
                Maybe::Some(e) => format!(" else {}", format_expr(e)),
                Maybe::None => String::new(),
            };
            format!("if {} {{ ... }}{}", cond_str, else_str)
        }
        ExprKind::Match {
            expr: scrutinee, ..
        } => {
            format!("match {} {{ ... }}", format_expr(scrutinee))
        }
        ExprKind::For { pattern, iter, .. } => {
            format!(
                "for {} in {} {{ ... }}",
                format_pattern(pattern),
                format_expr(iter)
            )
        }
        ExprKind::While { condition, .. } => {
            format!("while {} {{ ... }}", format_expr(condition))
        }
        ExprKind::Loop { .. } => "loop { ... }".to_string(),
        ExprKind::Return(inner) => {
            use verum_common::Maybe;
            match inner.as_ref() {
                Maybe::Some(e) => format!("return {}", format_expr(e)),
                Maybe::None => "return".to_string(),
            }
        }
        ExprKind::Break { label, value } => {
            use verum_common::Maybe;
            let label_str = match label.as_ref() {
                Maybe::Some(l) => format!("'{} ", l.as_str()),
                Maybe::None => String::new(),
            };
            let value_str = match value.as_ref() {
                Maybe::Some(v) => format!(" {}", format_expr(v)),
                Maybe::None => String::new(),
            };
            format!("break{}{}", label_str, value_str)
        }
        ExprKind::Continue { label } => {
            use verum_common::Maybe;
            match label.as_ref() {
                Maybe::Some(l) => format!("continue '{}", l.as_str()),
                Maybe::None => "continue".to_string(),
            }
        }
        ExprKind::Closure {
            params,
            body,
            return_type,
            move_,
            ..
        } => {
            use verum_common::Maybe;
            let move_str = if *move_ { "move " } else { "" };
            let params_str = params
                .iter()
                .map(|p| format_pattern(&p.pattern))
                .collect::<Vec<_>>()
                .join(", ");
            let ret_str = match return_type.as_ref() {
                Maybe::Some(t) => format!(" -> {}", format_type(t)),
                Maybe::None => String::new(),
            };
            format!(
                "{}|{}|{} {}",
                move_str,
                params_str,
                ret_str,
                format_expr(body)
            )
        }
        ExprKind::Await(inner) => format!("{}.await", format_expr(inner)),
        ExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            use verum_common::Maybe;
            let start_str = match start.as_ref() {
                Maybe::Some(s) => format_expr(s),
                Maybe::None => String::new(),
            };
            let end_str = match end.as_ref() {
                Maybe::Some(e) => format_expr(e),
                Maybe::None => String::new(),
            };
            if *inclusive {
                format!("{}..={}", start_str, end_str)
            } else {
                format!("{}..{}", start_str, end_str)
            }
        }
        ExprKind::Record { path, fields, base } => {
            use verum_common::Maybe;
            let path_str = format_path(path);
            let fields_str = fields
                .iter()
                .map(|f| match f.value.as_ref() {
                    Maybe::Some(val) => format!("{}: {}", f.name.as_str(), format_expr(val)),
                    Maybe::None => f.name.as_str().to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let base_str = match base.as_ref() {
                Maybe::Some(b) => format!(", ..{}", format_expr(b)),
                Maybe::None => String::new(),
            };
            format!("{} {{ {}{} }}", path_str, fields_str, base_str)
        }
        ExprKind::Try(inner) => format!("{}?", format_expr(inner)),
        _ => "...".to_string(),
    }
}

/// Format an if condition for display
fn format_if_condition(condition: &verum_ast::expr::IfCondition) -> String {
    condition
        .conditions
        .iter()
        .map(|c| match c {
            verum_ast::expr::ConditionKind::Expr(e) => format_expr(e),
            verum_ast::expr::ConditionKind::Let { pattern, value } => {
                format!("let {} = {}", format_pattern(pattern), format_expr(value))
            }
        })
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Format a literal for display
fn format_literal(lit: &verum_ast::Literal) -> String {
    match &lit.kind {
        LiteralKind::Int(int_lit) => int_lit.value.to_string(),
        LiteralKind::Float(float_lit) => float_lit.value.to_string(),
        LiteralKind::Bool(b) => b.to_string(),
        LiteralKind::Char(c) => format!("'{}'", c),
        LiteralKind::ByteChar(b) => format!("b'{}'", *b as char),
        LiteralKind::ByteString(bytes) => {
            let escaped: String = bytes.iter().map(|b| format!("\\x{:02x}", b)).collect();
            format!("b\"{}\"", escaped)
        }
        LiteralKind::Text(s) => format!("\"{}\"", s.as_str()),
        LiteralKind::Tagged { tag, content } => format!("{}#\"{}\"", tag, content),
        LiteralKind::InterpolatedString(s) => format!("{}\"{}\"", s.prefix, s.content),
        LiteralKind::Contract(c) => format!("contract#\"{}\"", c),
        LiteralKind::Composite(c) => format!("{}#\"{}\"", c.tag, c.content),
        LiteralKind::ContextAdaptive(c) => c.raw.to_string(),
    }
}

/// Format a function signature for hover/completion
///
/// # Examples
/// ```ignore
/// let func = /* function decl */;
/// let signature = format_function_signature(func);
/// // Output: "fn add(x: Int, y: Int) -> Int"
/// ```
pub fn format_function_signature(func: &verum_ast::FunctionDecl) -> String {
    let params_str = format_params(&func.params);

    // Format throws clause if present
    let throws_str = func.throws_clause.as_ref().map(|clause| {
        let types: Vec<String> = clause.error_types.iter().map(format_type).collect();
        if types.len() == 1 {
            format!(" throws({})", types[0])
        } else {
            format!(" throws({})", types.join(" | "))
        }
    });

    let return_type = func
        .return_type
        .as_ref()
        .map(format_type)
        .unwrap_or_else(|| "()".to_string());

    format!(
        "fn {}({}){}-> {}",
        func.name.as_str(),
        params_str,
        throws_str.unwrap_or_default(),
        return_type
    )
}

/// Format a type declaration for hover/completion
///
/// # Examples
/// ```ignore
/// let type_decl = /* type declaration */;
/// let formatted = format_type_decl(type_decl);
/// ```
pub fn format_type_decl(type_decl: &verum_ast::TypeDecl) -> String {
    let mut result = String::new();

    match &type_decl.body {
        TypeDeclBody::Alias(ty) => {
            result.push_str(&format!(
                "type {} = {}",
                type_decl.name.as_str(),
                format_type(ty)
            ));
        }
        TypeDeclBody::Record(fields) => {
            result.push_str(&format!("type {} is {{\n", type_decl.name.as_str()));
            for field in fields {
                result.push_str(&format!(
                    "    {}: {},\n",
                    field.name.as_str(),
                    format_type(&field.ty)
                ));
            }
            result.push('}');
        }
        TypeDeclBody::Variant(variants) => {
            result.push_str(&format!("type {} is ", type_decl.name.as_str()));
            let variant_strs: Vec<String> = variants
                .iter()
                .map(|v| {
                    let data_opt: Option<&_> = v.data.as_ref();
                    match data_opt {
                        None => v.name.as_str().to_string(),
                        Some(VariantData::Tuple(types)) => {
                            let field_types =
                                types.iter().map(format_type).collect::<Vec<_>>().join(", ");
                            format!("{}({})", v.name.as_str(), field_types)
                        }
                        Some(VariantData::Record(fields)) => {
                            let field_strs = fields
                                .iter()
                                .map(|f| format!("{}: {}", f.name.as_str(), format_type(&f.ty)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("{} {{ {} }}", v.name.as_str(), field_strs)
                        }
                    }
                })
                .collect();
            result.push_str(&variant_strs.join(" | "));
        }
        TypeDeclBody::Newtype(ty) => {
            result.push_str(&format!(
                "type {} = {}",
                type_decl.name.as_str(),
                format_type(ty)
            ));
        }
        _ => {
            result.push_str(&format!("type {}", type_decl.name.as_str()));
        }
    }

    result
}

/// Format a protocol declaration for hover/completion
pub fn format_protocol_decl(protocol: &verum_ast::ProtocolDecl) -> String {
    format!("protocol {}", protocol.name.as_str())
}

/// Get information about built-in types and keywords
pub fn get_builtin_info(symbol: &str) -> Option<String> {
    let info = match symbol {
        // Types
        "Int" => "**Integer** - Arbitrary-precision signed integer",
        "Float" => "**Float** - 64-bit floating point number (IEEE 754)",
        "Bool" => "**Boolean** - `true` or `false`",
        "Char" => "**Character** - Unicode scalar value",
        "Text" => "**Text** - UTF-8 encoded string",
        "Unit" => "**Unit** - Empty type `()`",
        "List" => "**List<T>** - Dynamic array (heap-allocated)",
        "Map" => "**Map<K, V>** - Hash map",
        "Set" => "**Set<T>** - Hash set",
        "Maybe" => "**Maybe<T>** - Optional value (`Some(T)` or `None`)",
        "Result" => "**Result<T, E>** - Result type (`Ok(T)` or `Err(E)`)",
        "Heap" => "**Heap<T>** - Heap-allocated reference (CBGR-tracked)",
        "Shared" => "**Shared<T>** - Thread-safe shared reference",

        // Keywords
        "fn" => "**fn** - Function declaration keyword",
        "let" => "**let** - Variable binding keyword",
        "mut" => "**mut** - Mutable reference modifier",
        "if" => "**if** - Conditional expression",
        "else" => "**else** - Alternative branch",
        "match" => "**match** - Pattern matching expression",
        "for" => "**for** - For loop",
        "while" => "**while** - While loop",
        "loop" => "**loop** - Infinite loop",
        "break" => "**break** - Exit from loop",
        "continue" => "**continue** - Skip to next iteration",
        "return" => "**return** - Return from function",
        "type" => "**type** - Type alias declaration",
        "struct" => "**struct** - Struct declaration",
        "enum" => "**enum** - Enum declaration",
        "protocol" => "**protocol** - Protocol (trait) declaration",
        "impl" => "**impl** - Implementation block",
        "mod" => "**mod** - Module declaration",
        "use" => "**use** - Import statement",
        "pub" => "**pub** - Public visibility modifier",
        "async" => "**async** - Async function/block",
        "await" => "**await** - Await async expression",
        "defer" => "**defer** - Defer statement execution (runs at scope exit)",
        "errdefer" => "**errdefer** - Error-path-only deferred execution (runs when scope exits via error)",
        "stream" => "**stream** - Stream comprehension",
        "verify" => "**verify** - Verification annotation",
        "requires" => "**requires** - Precondition specification",
        "ensures" => "**ensures** - Postcondition specification",
        "invariant" => "**invariant** - Loop/type invariant",
        "assert" => "**assert** - Runtime assertion",
        "assume" => "**assume** - Verification assumption",
        "ref" => "**ref** - CBGR-managed reference",
        "checked" => "**checked** - Runtime bounds-checked reference",
        "unsafe" => "**unsafe** - Unchecked reference (zero-cost)",
        "true" => "**true** - Boolean true value",
        "false" => "**false** - Boolean false value",
        "null" => "**null** - Null value",
        "self" => "**self** - Current instance reference",

        _ => return None,
    };

    Some(info.to_string())
}
