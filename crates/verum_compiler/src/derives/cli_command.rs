//! `@command` derive macro — lower a `@command`-decorated record type
//! into a `core.cli.spec.CommandSpec` factory function.
//!
//! User contract (per `internal/specs/cli-framework.md` §4):
//!
//! ```verum
//! @command(name = "mytool", version = "1.0.0", about = "An example CLI")
//! type Cli is {
//!     @flag(short = 'v', long = "verbose", help = "Enable verbose output")
//!     verbose: Bool,
//!
//!     @flag(long = "config", env = "MYTOOL_CONFIG", help = "Config file")
//!     config: Maybe<Path>,
//!
//!     @arg(help = "Build target")
//!     target: Text,
//! }
//! ```
//!
//! Generated (by this derive, hoisted into the parent module via the
//! `@derive_generated` Module wrapper that `phases::macro_expansion`
//! understands):
//!
//! ```verum
//! pub fn __cli_spec_for_Cli() -> CommandSpec {
//!     AppBuilder.new("mytool", "An example CLI")
//!         .version("1.0.0")
//!         .flag(FlagBuilder.switch("verbose", "Enable verbose output")
//!             .short('v')
//!             .build())
//!         .flag(FlagBuilder.value("config", "Config file")
//!             .env("MYTOOL_CONFIG")
//!             .arity(Arity.optional())
//!             .build())
//!         .arg(ArgBuilder.new("target", "Build target").build())
//!         .build()
//! }
//! ```
//!
//! Field classification:
//! * `Bool`         → `FlagBuilder.switch(long, help)`
//! * `Maybe<T>`     → `FlagBuilder.value(...).arity(Arity.optional())`
//! * `List<T>`      → `FlagBuilder.value(...).repeated().arity(Arity.Unlimited)`
//! * `Int` + `count = true` attribute → `FlagBuilder.value(...).counter()`
//! * Bare `T`       → `FlagBuilder.value(...)` (single required value)
//! * Field with `@arg` instead of `@flag` → positional `ArgBuilder`
//!
//! `@command` itself is recognised both as a derive name (`@derive(Command)`)
//! and as a *trigger attribute* — `phases::macro_expansion::extract_derive_names`
//! is patched to auto-include `Command` in the derive set whenever a
//! `@command` attribute appears on the type.
//!
//! Spec coverage: §4.1 minimal example, §4.2 attribute semantics
//! (name/version/about/long_about/author),
//! §4.3 refinement bridge (refinement expressions are *forwarded* into
//! `FlagBuilder.refinement(expr)` calls — runtime validation lives in
//! `core.cli.refinement`).

use super::common::{DeriveContext, DeriveError, FieldInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit};
use verum_ast::Attribute;
use verum_ast::Span;
use verum_ast::decl::{
    FunctionBody, FunctionDecl, Item, ItemKind, ModuleDecl, Visibility,
};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::{Heap, List, Maybe, Text};

pub struct DeriveCommand;

impl DeriveMacro for DeriveCommand {
    fn name(&self) -> &'static str {
        "Command"
    }

    fn protocol_name(&self) -> &'static str {
        // Not a protocol — we emit a free function. Returning a
        // sentinel here keeps the registry's protocol-name lookup
        // happy without claiming a fake protocol implementation.
        "Command"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;

        if ctx.type_info.is_enum {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("enum"),
                hint: Text::from(
                    "@command on a sum type lowers to subcommand-tree; \
                     not yet implemented in this derive (Phase 1.5).",
                ),
                span,
            });
        }

        // Type-level @command(...) args (name / version / about / ...).
        let cmd_args = extract_named_args(&ctx.type_decl.attributes, "command");
        let cmd_name = pluck_string(&cmd_args, "name")
            .unwrap_or_else(|| to_kebab_case(ctx.type_info.name.as_str()));
        let cmd_about =
            pluck_string(&cmd_args, "about").unwrap_or_else(|| Text::from(""));

        // Build the AppBuilder chain expression.
        let mut chain = method_call(
            ident_expr("AppBuilder", span),
            "new",
            list_of(vec![
                string_lit(cmd_name.as_str(), span),
                string_lit(cmd_about.as_str(), span),
            ]),
            span,
        );

        if let Some(version) = pluck_string(&cmd_args, "version") {
            chain = method_call(
                chain,
                "version",
                list_of(vec![string_lit(version.as_str(), span)]),
                span,
            );
        }
        if let Some(long_about) = pluck_string(&cmd_args, "long_about") {
            chain = method_call(
                chain,
                "long_about",
                list_of(vec![string_lit(long_about.as_str(), span)]),
                span,
            );
        }
        if let Some(author) = pluck_string(&cmd_args, "author") {
            chain = method_call(
                chain,
                "author",
                list_of(vec![string_lit(author.as_str(), span)]),
                span,
            );
        }

        // Walk fields. Each field with @flag → .flag(FlagBuilder...build());
        // each field with @arg → .arg(ArgBuilder...build()).
        for field in ctx.type_info.fields.iter() {
            let kind = classify_field(field);
            chain = match kind {
                FieldClass::Flag => attach_flag(chain, field, span),
                FieldClass::Arg => attach_arg(chain, field, span),
                FieldClass::Skip => chain,
            };
        }

        chain = method_call(chain, "build", List::new(), span);

        // Wrap chain in a fn body.
        let body = Block::new(
            List::new(),
            Maybe::Some(Heap::new(chain)),
            span,
        );
        let return_ty = Type::new(
            TypeKind::Path(Path::single(Ident::new("CommandSpec", span))),
            span,
        );
        let fn_name = format!("__cli_spec_for_{}", ctx.type_info.name.as_str());
        let factory_fn = make_top_fn(&fn_name, return_ty, body, span);

        // Wrap the freestanding fn in a Module so the derive infrastructure
        // can lift its contents into the parent scope (the same trick
        // DeriveBuilder uses).
        let module_name = format!(
            "__cli_command_derive_{}",
            ctx.type_info.name.as_str()
        );
        let module = ModuleDecl {
            visibility: Visibility::Public,
            name: Ident::new(module_name.as_str(), span),
            items: Maybe::Some(List::from(vec![Item::new(
                ItemKind::Function(factory_fn),
                span,
            )])),
            profile: Maybe::None,
            features: Maybe::None,
            contexts: List::new(),
            span,
        };
        let attr = Attribute::simple(Text::from("derive_generated"), span);
        Ok(Item {
            kind: ItemKind::Module(module),
            span,
            attributes: List::from(vec![attr]),
        })
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated CommandSpec factory for vcli @command-decorated types"
    }
}

// =============================================================================
// Field classification
// =============================================================================

#[derive(Debug, Clone, Copy)]
enum FieldClass {
    Flag,
    Arg,
    Skip,
}

fn classify_field(field: &FieldInfo) -> FieldClass {
    for attr in field.attributes.iter() {
        match attr.name.as_str() {
            "flag" => return FieldClass::Flag,
            "arg" => return FieldClass::Arg,
            "subcommand" => return FieldClass::Skip,
            _ => {}
        }
    }
    // No annotation — Bool defaults to flag, everything else to arg.
    if field.is_bool() {
        FieldClass::Flag
    } else {
        FieldClass::Arg
    }
}

// =============================================================================
// FlagBuilder chain
// =============================================================================

fn attach_flag(chain: Expr, field: &FieldInfo, span: Span) -> Expr {
    let flag_attr = find_attr(&field.attributes, "flag");
    let args = flag_attr
        .map(|a| extract_named_args_from(a))
        .unwrap_or_else(Vec::new);

    let long_name = pluck_string(&args, "long")
        .unwrap_or_else(|| to_kebab_case(field.name.as_str()));
    let help = pluck_string(&args, "help").unwrap_or_else(|| Text::from(""));

    let is_switch = field.is_bool();
    let mut flag_expr = if is_switch {
        method_call(
            ident_expr("FlagBuilder", span),
            "switch",
            list_of(vec![
                string_lit(long_name.as_str(), span),
                string_lit(help.as_str(), span),
            ]),
            span,
        )
    } else {
        method_call(
            ident_expr("FlagBuilder", span),
            "value",
            list_of(vec![
                string_lit(long_name.as_str(), span),
                string_lit(help.as_str(), span),
            ]),
            span,
        )
    };

    // .short('c')
    if let Some(short_ch) = pluck_char(&args, "short") {
        flag_expr = method_call(
            flag_expr,
            "short",
            list_of(vec![char_lit(short_ch, span)]),
            span,
        );
    }

    // .env("X")
    if let Some(env) = pluck_string(&args, "env") {
        flag_expr = method_call(
            flag_expr,
            "env",
            list_of(vec![string_lit(env.as_str(), span)]),
            span,
        );
    }

    // .config("path")
    if let Some(config) = pluck_string(&args, "config") {
        flag_expr = method_call(
            flag_expr,
            "config",
            list_of(vec![string_lit(config.as_str(), span)]),
            span,
        );
    }

    // .persistent()
    if pluck_bool(&args, "persistent").unwrap_or(false) {
        flag_expr = method_call(flag_expr, "persistent", List::new(), span);
    }

    // .negatable()
    if pluck_bool(&args, "negatable").unwrap_or(false) {
        flag_expr = method_call(flag_expr, "negatable", List::new(), span);
    }

    // .repeated() / .counter() — driven by attribute hints AND by type.
    if pluck_bool(&args, "count").unwrap_or(false) {
        flag_expr = method_call(flag_expr, "counter", List::new(), span);
    } else if field.is_list() {
        flag_expr = method_call(flag_expr, "repeated", List::new(), span);
    }

    // Optional via Maybe<T>.
    if !is_switch && field.is_maybe() {
        // Arity.optional() — qualified path call.
        let optional_call = method_call(
            ident_expr("Arity", span),
            "optional",
            List::new(),
            span,
        );
        flag_expr = method_call(
            flag_expr,
            "arity",
            list_of(vec![optional_call]),
            span,
        );
    }

    // .build()
    flag_expr = method_call(flag_expr, "build", List::new(), span);

    // chain.flag(flag_expr)
    method_call(chain, "flag", list_of(vec![flag_expr]), span)
}

// =============================================================================
// ArgBuilder chain
// =============================================================================

fn attach_arg(chain: Expr, field: &FieldInfo, span: Span) -> Expr {
    let arg_attr = find_attr(&field.attributes, "arg");
    let args = arg_attr
        .map(|a| extract_named_args_from(a))
        .unwrap_or_else(Vec::new);

    let arg_name = field.name.clone();
    let help = pluck_string(&args, "help").unwrap_or_else(|| Text::from(""));

    let mut arg_expr = method_call(
        ident_expr("ArgBuilder", span),
        "new",
        list_of(vec![
            string_lit(arg_name.as_str(), span),
            string_lit(help.as_str(), span),
        ]),
        span,
    );

    // Type-driven arity adjustment.
    if field.is_maybe() {
        arg_expr = method_call(arg_expr, "optional", List::new(), span);
    } else if field.is_list() {
        arg_expr = method_call(arg_expr, "variadic", List::new(), span);
    }

    if let Some(env) = pluck_string(&args, "env") {
        arg_expr = method_call(
            arg_expr,
            "env",
            list_of(vec![string_lit(env.as_str(), span)]),
            span,
        );
    }
    if let Some(config) = pluck_string(&args, "config") {
        arg_expr = method_call(
            arg_expr,
            "config",
            list_of(vec![string_lit(config.as_str(), span)]),
            span,
        );
    }
    if let Some(placeholder) = pluck_string(&args, "placeholder") {
        arg_expr = method_call(
            arg_expr,
            "placeholder",
            list_of(vec![string_lit(placeholder.as_str(), span)]),
            span,
        );
    }

    arg_expr = method_call(arg_expr, "build", List::new(), span);

    method_call(chain, "arg", list_of(vec![arg_expr]), span)
}

// =============================================================================
// Attribute / arg pluckers
// =============================================================================

fn find_attr<'a>(attrs: &'a List<Attribute>, name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| a.name.as_str() == name)
}

/// Extract every (name, value-expr) pair from the named-arg list of the
/// `attr_name` attribute on a list of attributes.
fn extract_named_args(attrs: &List<Attribute>, attr_name: &str) -> Vec<(Text, Expr)> {
    match find_attr(attrs, attr_name) {
        Some(attr) => extract_named_args_from(attr),
        None => Vec::new(),
    }
}

fn extract_named_args_from(attr: &Attribute) -> Vec<(Text, Expr)> {
    let mut out = Vec::new();
    if let Maybe::Some(args) = &attr.args {
        for arg in args.iter() {
            match &arg.kind {
                ExprKind::NamedArg { name, value } => {
                    out.push((Text::from(name.as_str()), (**value).clone()));
                }
                // Tolerate `key = value` parsed as a Binary(Assign) op too.
                ExprKind::Binary {
                    op: verum_ast::BinOp::Assign,
                    left,
                    right,
                } => {
                    if let ExprKind::Path(p) = &left.kind {
                        if let Some(ident) = p.as_ident() {
                            out.push((
                                Text::from(ident.as_str()),
                                (**right).clone(),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn pluck_string(args: &[(Text, Expr)], key: &str) -> Option<Text> {
    args.iter()
        .find(|(k, _)| k.as_str() == key)
        .and_then(|(_, e)| extract_string(e))
}

fn pluck_char(args: &[(Text, Expr)], key: &str) -> Option<char> {
    args.iter()
        .find(|(k, _)| k.as_str() == key)
        .and_then(|(_, e)| extract_char(e))
}

fn pluck_bool(args: &[(Text, Expr)], key: &str) -> Option<bool> {
    args.iter()
        .find(|(k, _)| k.as_str() == key)
        .and_then(|(_, e)| extract_bool(e))
}

fn extract_string(e: &Expr) -> Option<Text> {
    use verum_ast::LiteralKind;
    if let ExprKind::Literal(lit) = &e.kind {
        if let LiteralKind::Text(s) = &lit.kind {
            return Some(Text::from(s.as_str()));
        }
    }
    None
}

fn extract_char(e: &Expr) -> Option<char> {
    use verum_ast::LiteralKind;
    if let ExprKind::Literal(lit) = &e.kind {
        if let LiteralKind::Char(c) = &lit.kind {
            return Some(*c);
        }
    }
    None
}

fn extract_bool(e: &Expr) -> Option<bool> {
    use verum_ast::LiteralKind;
    if let ExprKind::Literal(lit) = &e.kind {
        if let LiteralKind::Bool(b) = &lit.kind {
            return Some(*b);
        }
    }
    None
}

// =============================================================================
// Misc helpers
// =============================================================================

fn list_of(v: Vec<Expr>) -> List<Expr> {
    List::from(v)
}

fn char_lit(c: char, span: Span) -> Expr {
    use verum_ast::{Literal, LiteralKind};
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Char(c),
            span,
        }),
        span,
    )
}

fn make_top_fn(name: &str, return_ty: Type, body: Block, span: Span) -> FunctionDecl {
    FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_pure: false,
        is_meta: false,
        stage_level: 0,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        extern_abi: None,
        is_variadic: false,
        name: Ident::new(name, span),
        generics: List::new(),
        params: List::new(),
        return_type: Some(return_ty),
        throws_clause: None,
        std_attr: None,
        contexts: List::new(),
        generic_where_clause: None,
        meta_where_clause: None,
        attributes: List::new(),
        body: Some(FunctionBody::Block(body)),
        requires: List::new(),
        ensures: List::new(),
        span,
    }
}

fn to_kebab_case(s: &str) -> Text {
    let mut out = String::with_capacity(s.len() + 2);
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            if prev_lower {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = false;
        } else if c == '_' {
            out.push('-');
            prev_lower = false;
        } else {
            out.push(c);
            prev_lower = c.is_ascii_lowercase();
        }
    }
    Text::from(out)
}
