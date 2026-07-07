//! Drive the two engines: fixture source → [`MetaFunction`] → one outcome
//! per engine, each run under `catch_unwind`.
//!

//! Every run constructs a **fresh** engine (`VbcExecutor::new()` /
//! `MetaContext::new()`): after a caught panic an engine's internal state
//! must be considered poisoned, and fixture isolation keeps verdicts
//! order-independent.

use std::panic::{catch_unwind, AssertUnwindSafe};

use verum_ast::{decl::ItemKind, FileId, MetaValue};
use verum_common::{Maybe, Text};
use verum_compiler::meta::vbc_executor::VbcExecutor;
use verum_compiler::{MetaContext, MetaFunction, MetaRegistry};
use verum_fast_parser::VerumParser;
use verum_vbc::value::Value;

use crate::compare::EngineOutcome;
use crate::extractor;

/// Module path fixtures are registered under.
const FIXTURE_MODULE: &str = "meta_engines_harness";

/// A fixture argument — restricted to scalars representable in **both**
/// value models (the VBC side takes NaN-boxed `i64`/`f64`/bool arguments;
/// Text/collection arguments would need a pre-populated interpreter heap,
/// which `VbcExecutor` does not expose). Fixtures that need Text build it
/// from literals inside the function body instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    /// Integer argument (`i64`: the VBC-representable range).
    Int(i64),
    /// Float argument.
    Float(f64),
    /// Bool argument.
    Bool(bool),
}

impl Arg {
    /// The VBC-side NaN-boxed value.
    pub fn to_vbc(self) -> Value {
        match self {
            Arg::Int(i) => Value::from_i64(i),
            Arg::Float(f) => Value::from_f64(f),
            Arg::Bool(b) => Value::from_bool(b),
        }
    }

    /// The tree-walk-side `ConstValue` (= [`MetaValue`]).
    pub fn to_tree(self) -> MetaValue {
        match self {
            Arg::Int(i) => MetaValue::Int(i128::from(i)),
            Arg::Float(f) => MetaValue::Float(f),
            Arg::Bool(b) => MetaValue::Bool(b),
        }
    }
}

impl std::fmt::Display for Arg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arg::Int(i) => write!(f, "{i}"),
            Arg::Float(x) => write!(f, "{x:?}"),
            Arg::Bool(b) => write!(f, "{b}"),
        }
    }
}

/// Parse a fixture source (one or more Verum `fn` items) and build the
/// [`MetaFunction`] for `fn_name` through the **production** registration
/// path (`MetaRegistry::register_meta_function`), so body/param/return-type
/// conversion matches what the compiler does for real meta functions.
pub fn build_meta_function(source: &str, fn_name: &str) -> Result<MetaFunction, String> {
    let parser = VerumParser::new();
    let module = parser
        .parse_module_str(source, FileId::new(0))
        .map_err(|errs| format!("parse error in fixture: {:?}", errs.iter().next()))?;

    let module_path = Text::from(FIXTURE_MODULE);
    let mut registry = MetaRegistry::new();
    let mut found = false;
    for item in module.items.iter() {
        if let ItemKind::Function(decl) = &item.kind {
            registry
                .register_meta_function(&module_path, decl)
                .map_err(|e| format!("failed to register fixture fn: {e:?}"))?;
            if decl.name.as_str() == fn_name {
                found = true;
            }
        }
    }
    if !found {
        return Err(format!("fixture source has no fn named '{fn_name}'"));
    }

    match registry.resolve_meta_call(&module_path, &Text::from(fn_name)) {
        Maybe::Some(f) => Ok(f),
        Maybe::None => Err(format!("registered fixture fn '{fn_name}' did not resolve")),
    }
}

/// Run one fixture through engine A — the VBC executor (compile the meta fn
/// to bytecode via `VbcCodegen`, execute on the Tier-0 interpreter, decode
/// the raw result value against the interpreter heap).
pub fn run_vbc(meta_fn: &MetaFunction, args: &[Arg]) -> EngineOutcome {
    let vbc_args: Vec<Value> = args.iter().map(|a| a.to_vbc()).collect();
    let mf = meta_fn.clone();
    let result = catch_unwind(AssertUnwindSafe(move || {
        let mut executor = VbcExecutor::new();
        executor
            .execute_raw(&mf, &vbc_args)
            // Decode while the interpreter (and its heap) is alive.
            .map(|raw| extractor::from_vbc(&raw.interpreter.state, raw.value))
    }));
    match result {
        Ok(Ok(value)) => EngineOutcome::Value(value),
        Ok(Err(e)) => EngineOutcome::Error(e.to_string()),
        Err(payload) => EngineOutcome::Panic(panic_message(payload)),
    }
}

/// Run one fixture through engine B — the tree-walk evaluator
/// (`MetaContext::execute_user_meta_fn`).
pub fn run_tree_walk(meta_fn: &MetaFunction, args: &[Arg]) -> EngineOutcome {
    let tree_args: Vec<MetaValue> = args.iter().map(|a| a.to_tree()).collect();
    let mf = meta_fn.clone();
    let result = catch_unwind(AssertUnwindSafe(move || {
        let mut ctx = MetaContext::new();
        ctx.execute_user_meta_fn(&mf, tree_args)
            .map(|v| extractor::from_tree_walk(&v))
    }));
    match result {
        Ok(Ok(value)) => EngineOutcome::Value(value),
        Ok(Err(e)) => EngineOutcome::Error(format!("{e:?}")),
        Err(payload) => EngineOutcome::Panic(panic_message(payload)),
    }
}

/// Extract a printable message from a `catch_unwind` payload.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Run `f` with panic output suppressed (the expected-panic fixtures would
/// otherwise spray backtraces), restoring the previous hook afterwards.
///

/// The panic hook is process-global: callers must not run this concurrently
/// with code that relies on panic *reporting* (pass/fail of concurrent
/// `catch_unwind`-based code is unaffected). The bin is single-threaded and
/// the test suite funnels every engine run through one entry point.
pub fn with_quiet_panics<T>(f: impl FnOnce() -> T) -> T {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(previous);
    match result {
        Ok(v) => v,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
