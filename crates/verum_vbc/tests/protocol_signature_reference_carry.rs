//! T1033 (ARCHIVE-DROPS-REFERENCE-QUALIFIERS-1) — a protocol method's
//! parameter types must keep the reference qualifier the source wrote.
//!
//! The protocol's method signatures are stored as `VariantDescriptor`
//! payloads (`TypeRef::Function`), rendered to text at bake time and
//! re-parsed on load.  Anything erased here is erased from EVERY
//! archive consumer's view of the protocol, so
//! `fn poll(&mut self, cx: &mut Context)` arrived as `fn(Context)` — a
//! by-value parameter where the language declared a mutable borrow.
//!
//! Both directions are pinned:
//!   * the three reference tiers survive with their mutability, and
//!   * a by-value parameter is NOT wrapped (without this the assertions
//!     above would pass under a change that wraps everything), and
//!   * the LAYOUT path still erases, because a raw reference IS a
//!     machine word there (T0846: `&unsafe Byte` recorded as U8 made
//!     the AOT parameter byte-wide and the caller truncated a pointer).
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::types::{CbgrTier, Mutability, TypeRef};

const SOURCE: &str = r#"
module probe.sigref;

public type Ctx is { n: Int };

public type Poller is protocol {
    fn poll(&mut self, cx: &mut Ctx) -> Int;
    fn peek(&self, cx: &Ctx) -> Int;
    fn raw(&self, cx: &unsafe Ctx) -> Int;
    fn plain(&self, cx: Ctx) -> Int;
};

public fn layout_control(cx: &unsafe Ctx) -> Int {
    0
}
"#;

/// Compile the probe and return, for every protocol method, its
/// parameter `TypeRef`s — plus the free function's parameter refs,
/// which answer the layout question rather than the signature one.
fn compile_probe() -> (Vec<(String, Vec<TypeRef>)>, Vec<TypeRef>) {
    let mut parser = Parser::new(SOURCE);
    let module_ast = parser
        .parse_module()
        .unwrap_or_else(|e| panic!("parse failed: {e:?}"));

    let config = CodegenConfig::new("probe_sigref.vr").with_validation();
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module_with_mounts(&module_ast, "probe_sigref.vr", ".")
        .unwrap_or_else(|e| panic!("compile failed: {e:?}"));
    let module = codegen
        .finalize_module()
        .unwrap_or_else(|e| panic!("finalize failed: {e:?}"));

    let poller = module
        .types
        .iter()
        .find(|t| module.strings.get(t.name).map(|n| n == "Poller").unwrap_or(false))
        .expect("Poller TypeDescriptor present");

    let methods: Vec<(String, Vec<TypeRef>)> = poller
        .variants
        .iter()
        .filter_map(|v| {
            let name = module.strings.get(v.name)?.to_string();
            match v.payload.as_ref()? {
                TypeRef::Function { params, .. } => Some((name, params.clone())),
                _ => None,
            }
        })
        .collect();

    // Match by suffix: a module-qualified name is still this function, and
    // a silent `unwrap_or_default()` here would report "0 parameters" for a
    // function that was never found — a green-or-misleading outcome either
    // way. Name what was searched instead.
    let layout_fn = module.functions.iter().find(|f| {
        module
            .strings
            .get(f.name)
            .map(|n| n == "layout_control" || n.ends_with(".layout_control"))
            .unwrap_or(false)
    });
    let layout: Vec<TypeRef> = match layout_fn {
        Some(f) => f.params.iter().map(|p| p.type_ref.clone()).collect(),
        None => panic!(
            "`layout_control` not among the module's functions: {:?}",
            module
                .functions
                .iter()
                .filter_map(|f| module.strings.get(f.name))
                .collect::<Vec<_>>()
        ),
    };

    (methods, layout)
}

fn params_of<'a>(methods: &'a [(String, Vec<TypeRef>)], name: &str) -> &'a [TypeRef] {
    methods
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, p)| p.as_slice())
        .unwrap_or_else(|| {
            panic!(
                "protocol method `{name}` has no Function payload; methods present: {:?}",
                methods.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
            )
        })
}

#[test]
fn a_mutable_borrow_parameter_is_not_recorded_by_value() {
    let (methods, _) = compile_probe();
    let params = params_of(&methods, "poll");
    assert_eq!(params.len(), 1, "poll takes one non-self parameter");
    match &params[0] {
        TypeRef::Reference {
            mutability, tier, ..
        } => {
            assert_eq!(*mutability, Mutability::Mutable, "`&mut Ctx` is mutable");
            assert_eq!(*tier, CbgrTier::Tier0, "a plain `&` is Tier 0");
        }
        other => panic!(
            "`cx: &mut Ctx` must survive as a reference, got {other:?} — \
             every archive consumer would read a by-value parameter"
        ),
    }
}

#[test]
fn a_shared_borrow_parameter_keeps_its_immutability() {
    let (methods, _) = compile_probe();
    match &params_of(&methods, "peek")[0] {
        TypeRef::Reference {
            mutability, tier, ..
        } => {
            assert_eq!(*mutability, Mutability::Immutable);
            assert_eq!(*tier, CbgrTier::Tier0);
        }
        other => panic!("`cx: &Ctx` must survive as a reference, got {other:?}"),
    }
}

#[test]
fn an_unsafe_borrow_parameter_keeps_its_tier() {
    let (methods, _) = compile_probe();
    match &params_of(&methods, "raw")[0] {
        TypeRef::Reference { tier, .. } => {
            assert_eq!(
                *tier,
                CbgrTier::Tier2,
                "`&unsafe T` is Tier 2 — the tier is part of the declared signature"
            );
        }
        other => panic!("`cx: &unsafe Ctx` must survive as a reference, got {other:?}"),
    }
}

/// The differentiator.  Without it, a change that wrapped EVERY
/// parameter in a reference would satisfy all three assertions above
/// while making the signature just as wrong in the other direction.
#[test]
fn a_by_value_parameter_is_not_wrapped_in_a_reference() {
    let (methods, _) = compile_probe();
    let param = &params_of(&methods, "plain")[0];
    assert!(
        !matches!(param, TypeRef::Reference { .. }),
        "`cx: Ctx` is declared by value and must not gain a reference: {param:?}"
    );
}

/// The layout path must keep erasing: the descriptor of an ordinary
/// function parameter answers "what IS this at runtime", and a raw
/// reference is a machine word (T0846).  Preserving references for
/// SIGNATURES must not leak into it.
#[test]
fn the_layout_path_still_erases_a_raw_reference_to_a_pointer() {
    let (_, layout) = compile_probe();
    assert_eq!(layout.len(), 1, "layout_control takes one parameter");
    assert!(
        !matches!(layout[0], TypeRef::Reference { .. }),
        "a function parameter descriptor answers the layout question and \
         must stay reference-free (T0846); got {:?}",
        layout[0]
    );
}
