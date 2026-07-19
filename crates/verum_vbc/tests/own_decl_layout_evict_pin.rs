//! OWN-DECL-LAYOUT-EVICT-1 (T0125) — pin for the vcs #143 register
//! entry ("Waker.drop NPE" / stdlib-cache codegen bug).
//!
//! **Defect shape**: the stdlib bake compiles each module in a FRESH
//! `VbcCodegen`, seeding it with earlier modules' record layouts
//! **TypeId-free** (`import_type_layouts`, D2b) and with their
//! function registries (`import_functions` — including sum-type
//! VARIANT constructors, registered under the variant's bare name).
//! When an earlier module's record layout occupies a simple name that
//! a LATER module declares as its OWN record type, the pre-fix
//! `claim_user_type_name` skipped the stale-layout eviction (it was
//! gated on a shadowed `type_name_to_id` binding, which TypeId-free
//! seeds never create). The module's own record literal then failed
//! `compile_record`'s field-superset guard against the FOREIGN
//! layout, fell through to the variant-constructor chain, and an
//! imported same-named VARIANT ctor hijacked the literal.
//!
//! Live instance: `core.theory_interop.congruence_closure`'s own
//! `Term { id, symbol, args, arity }` literal baked as
//! `MakeVariant { tag: 10 }` — `Signal.Term` (SIGTERM!) from
//! `core/sys/signal.vr` — because `core.cog.resolve.Term`'s seeded
//! layout `[package, range, positive]` held the simple key. Every
//! Term/EquationSet round-trip through the embedded archive came
//! back field-scrambled; the L2 spec
//! `vcs/specs/L2-standard/math/theory_interop/congruence_closure_correctness.vr`
//! died inside `term_signature` ("`Text.push_str` not found on
//! receiver of runtime kind `<unknown-tag>`").
//!
//! **Contract pinned here** (lexical scoping): a module's own type
//! declaration owns its simple name inside that module's bodies —
//! regardless of what layouts/functions were seeded first. The own
//! record literal must compile to a record allocation (`New`) with
//! declared-order field indices, never to a foreign `MakeVariant`.

#![cfg(feature = "codegen")]

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::VbcModule;

fn parse(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{}", e)).collect();
            panic!("parse failed:\n{}", msgs.join("\n"))
        })
}

fn compile(module_name: &str, source: &str) -> (VbcCodegen, VbcModule) {
    let ast = parse(source);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(module_name));
    let vbc = codegen
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile of `{}` failed: {}", module_name, e));
    (codegen, vbc)
}

/// Decode the bytecode of the function whose resolved name ends with
/// `suffix`.
fn decoded_fn(module: &VbcModule, suffix: &str) -> Vec<Instruction> {
    let func = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n == suffix || n.ends_with(&format!(".{suffix}")))
        })
        .unwrap_or_else(|| panic!("function `{}` not found in module", suffix));
    let start = func.bytecode_offset as usize;
    let end = start + func.bytecode_length as usize;
    decode_instructions(&module.bytecode[start..end])
        .unwrap_or_else(|e| panic!("decode of `{}` bytecode failed: {:?}", suffix, e))
}

/// Bake-shaped repro: seed a fresh codegen exactly like
/// `compile_core_module_from_ast` does (foreign same-named record
/// layout, TypeId-free + foreign function registry carrying a
/// same-named VARIANT constructor), then compile a module declaring
/// its OWN same-named record and constructing it.
#[test]
fn own_record_literal_survives_typeid_free_foreign_layout_seed() {
    // Earlier-baked module 1 (the `core.cog.resolve` analogue): a
    // record also named `Term`, DIFFERENT fields.
    let (cog_codegen, _cog_vbc) = compile(
        "c",
        "module c;\n\
         public type Term is { package: Int, range: Int, positive: Bool };\n",
    );

    // Earlier-baked module 2 (the `core.sys.signal` analogue): a sum
    // type with a unit variant named `Term` — its bare-name variant
    // constructor lands in the exported function registry.
    let (sig_codegen, _sig_vbc) = compile(
        "s",
        "module s;\n\
         public type Sig is Hup | Intr | Term | Usr1;\n",
    );

    // The declaring module (the `core.theory_interop` analogue):
    // fresh codegen, seeded the same way the stdlib bake seeds it.
    let theory_src = "module t;\n\
         public type Term is { id: Int, symbol: Int, args: Int, arity: Int };\n\
         public fn mk() -> Term {\n\
             Term { id: 1, symbol: 2, args: 3, arity: 4 }\n\
         }\n";
    let ast = parse(theory_src);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("t"));
    codegen.import_functions(&cog_codegen.export_functions());
    codegen.import_functions(&sig_codegen.export_functions());
    codegen.import_type_layouts(&cog_codegen.export_type_layouts());
    codegen.import_type_layouts(&sig_codegen.export_type_layouts());
    let vbc = codegen
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile of `t` failed: {}", e));

    let instrs = decoded_fn(&vbc, "mk");

    // The own record literal must NOT be hijacked by the imported
    // variant constructor of the same bare name.
    let variant_hits: Vec<&Instruction> = instrs
        .iter()
        .filter(|i| {
            matches!(
                i,
                Instruction::MakeVariant { .. } | Instruction::MakeVariantTyped { .. }
            )
        })
        .collect();
    assert!(
        variant_hits.is_empty(),
        "own `Term {{ … }}` record literal compiled as a foreign variant \
         construction: {:?}\nfull body: {:#?}",
        variant_hits,
        instrs
    );

    // …and it must be a record allocation with declared-order field
    // writes: id→0, symbol→1, args→2, arity→3.
    assert!(
        instrs.iter().any(|i| matches!(i, Instruction::New { .. })),
        "own record literal emitted no `New` record allocation.\nbody: {:#?}",
        instrs
    );
    let setf_indices: Vec<u32> = instrs
        .iter()
        .filter_map(|i| match i {
            Instruction::SetF { field_idx, .. } => Some(*field_idx),
            _ => None,
        })
        .collect();
    assert_eq!(
        setf_indices,
        vec![0, 1, 2, 3],
        "field writes must land at the DECLARED positions (id, symbol, \
         args, arity) — the foreign seeded layout must not drive them.\n\
         body: {:#?}",
        instrs
    );
}
