//! ARCH-P4 step (ii): the VBC-engine evaluation surface for meta
//! EXPRESSIONS — the convergence bridge from the tree-walk evaluator
//! to the single (VBC) meta engine.
//!
//! `vbc_eval_meta_expr` wraps an arbitrary meta expression into a
//! synthetic zero-arg [`MetaFunction`], executes it on the Tier-0
//! interpreter via [`VbcMetaExecutor::execute_raw`], and decodes the
//! raw result into a [`ConstValue`] (scalars + Text exactly; the
//! decode mirrors `script_engine::extract_owned`, the documented
//! step-(ii) donor — see vcs/differential/meta_engines/src/extractor.rs).
//!
//! Engine selection: `VERUM_META_ENGINE=vbc` routes the gated call
//! sites here; the default remains the tree-walk until the corpus
//! gate (the meta-engines differential harness) holds 0 NEW
//! divergences across a full release cycle — the ratchet discipline
//! from tier-coherence-pillars.md Pillar 4.

use verum_ast::{Expr, Span, Type};
use verum_common::{List, Text};

use super::context::ConstValue;
use super::registry::MetaFunction;
use super::vbc_executor::VbcExecutor;

/// Is the VBC engine selected for meta-expression evaluation?
pub fn vbc_meta_engine_selected() -> bool {
    std::env::var("VERUM_META_ENGINE")
        .map(|v| v.eq_ignore_ascii_case("vbc"))
        .unwrap_or(false)
}

/// Evaluate a meta expression on the VBC engine, decoding the raw
/// interpreter value into a `ConstValue`.
///
/// Fail-closed contract: any executor error, panic-free non-scalar
/// result the decoder cannot faithfully represent, or wrapper
/// mismatch returns `Err` — callers fall back to the tree-walk (the
/// hybrid never silently changes semantics; divergences surface in
/// the differential harness instead).
pub fn vbc_eval_meta_expr(
    expr: &Expr,
    span: Span,
    context_items: &[verum_ast::Item],
) -> Result<ConstValue, String> {
    let synthetic = MetaFunction {
        name: Text::from("__vbc_meta_eval"),
        module: Text::from("__synthetic"),
        params: List::new(),
        return_type: Type::inferred(span),
        body: expr.clone(),
        contexts: List::new(),
        is_async: false,
        is_transparent: false,
        stage_level: 0,
        span,
    };
    let mut executor = VbcExecutor::new();
    let raw = executor
        .execute_raw_with_items(&synthetic, context_items, &[])
        .map_err(|e| format!("vbc meta engine: {e:?}"))?;
    decode_const(&raw.interpreter.state, raw.value)
}

/// Decode a raw interpreter `Value` into `ConstValue` — the scalar
/// + Text subset, exactly (mirrors `script_engine::extract_owned`'s
/// discrimination order: unit → bool → int → float → text).
fn decode_const(
    state: &verum_vbc::interpreter::InterpreterState,
    value: verum_vbc::Value,
) -> Result<ConstValue, String> {
    if value.is_unit() || value.is_nil() {
        Ok(ConstValue::Unit)
    } else if value.is_bool() {
        Ok(ConstValue::Bool(value.as_bool()))
    } else if value.is_int() {
        Ok(ConstValue::Int(i128::from(value.as_i64())))
    } else if value.is_float() || value.is_nan_float() {
        Ok(ConstValue::Float(value.as_f64()))
    } else if let Some(s) = state.read_text(value) {
        Ok(ConstValue::Text(Text::from(s.as_str())))
    } else if let Some(elems) = state.list_elements(value) {
        // Structural decode, step-(iii) increment: lists decode
        // recursively (depth-bounded — meta values are finite by
        // construction; a cycle would already have hung the
        // interpreter run that produced them).
        let mut out: List<ConstValue> = List::new();
        for e in elems {
            out.push(decode_const(state, e)?);
        }
        Ok(ConstValue::Array(out))
    } else if let Some(entries) = state.map_entries(value) {
        // Maps decode with TEXT keys (the meta value model's map is
        // Text-keyed — OrderedMap<Text, MetaValue>); non-text keys
        // fail closed.
        let mut out: verum_common::OrderedMap<Text, ConstValue> =
            verum_common::OrderedMap::new();
        for (k, v) in entries {
            let key = state.read_text(k).ok_or_else(|| {
                "vbc meta engine: non-Text map key — tree-walk fallback"
                    .to_string()
            })?;
            out.insert(Text::from(key.as_str()), decode_const(state, v)?);
        }
        Ok(ConstValue::Map(out))
    } else if let Some(fields) = state.record_named_fields(value) {
        // Record leg: the stamped header names the module TypeDescriptor,
        // whose declared field order drives the walk. Decoded as a TUPLE
        // of field values BY DESIGN — the tree-walk evaluator's record
        // arm does exactly that (evaluator.rs MetaExpr::Record: "Records
        // become tuples of field values"), and step-(ii/iii) convergence
        // means matching the observable engine surface. Once the
        // tree-walk retires, this is the single place to upgrade records
        // to a name-preserving MetaValue (the names are already here).
        let mut out: List<ConstValue> = List::new();
        for (_name, v) in fields {
            out.push(decode_const(state, v)?);
        }
        Ok(ConstValue::Tuple(out))
    } else {
        Err(
            "vbc meta engine: undecodable result (sum-variants/opaque) — \
             tree-walk fallback"
                .to_string(),
        )
    }
}

