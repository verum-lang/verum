//! T1536: lazy archive replay preserves declarations and genuine reexports.
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn lazy_replay_keeps_exact_declaration_owner_in_both_entry_orders() {
    for reverse in [false, true] {
        let ast = Parser::new("public fn original()->Int {7} public fn choose()->Int {37}")
            .parse_module()
            .unwrap();
        let mut alpha = VbcCodegen::with_config(CodegenConfig::new("alpha"))
            .compile_module(&ast)
            .unwrap();
        let original = alpha
            .functions
            .iter()
            .find(|f| alpha.get_string(f.name) == Some("alpha.original"))
            .unwrap()
            .id;
        let target = alpha.intern_string("alpha.original");
        for alias in ["alpha.choose", "alpha.renamed"] {
            let name = alpha.intern_string(alias);
            // Historical stale row and genuine reexport share the same target.
            alpha.mount_aliases.push((name, original, target));
        }
        let ast = Parser::new("public fn choose()->Int {99}")
            .parse_module()
            .unwrap();
        let beta = VbcCodegen::with_config(CodegenConfig::new("beta"))
            .compile_module(&ast)
            .unwrap();
        let mut modules = List::from_iter([("alpha", alpha), ("beta", beta)]);
        if reverse {
            modules.reverse();
        }
        let mut builder = verum_vbc::archive::ArchiveBuilder::new();
        for (name, module) in &modules {
            builder.add_module(name, module, &[]).unwrap();
        }
        let archive = builder.finish();
        let ast = Parser::new("fn main()->Int { alpha.choose()+alpha.renamed()+beta.choose() }")
            .parse_module()
            .unwrap();
        let mut codegen = VbcCodegen::new();
        let cache = verum_compiler::archive_ctx_loader::ArchiveCtxCache::new();
        let (functions, _) = cache.apply_lazy_with_types(&archive, &mut codegen, &ast, &[]);
        assert!(
            functions >= 2,
            "both source entries must reach the public loader"
        );
        let ctx = codegen.ctx_mut();
        let chosen = ctx.lookup_function("alpha.choose").unwrap().id;
        let original = ctx.lookup_function("alpha.original").unwrap().id;
        assert_ne!(chosen, original, "stale alias replaced the declared body");
        assert_eq!(ctx.lookup_function("alpha.renamed").unwrap().id, original);
        assert_ne!(ctx.lookup_function("beta.choose").unwrap().id, chosen);
    }
}

#[test]
fn foreign_reexport_cannot_select_an_unrelated_same_number_local_function() {
    for reverse in [false, true] {
        let mut modules = List::new();
        for (owner, source) in [
            ("alpha", "public fn unrelated()->Int {99}"),
            ("beta", "public fn chosen()->Int {7}"),
        ] {
            let ast = Parser::new(source).parse_module().unwrap();
            modules.push(
                VbcCodegen::with_config(CodegenConfig::new(owner))
                    .compile_module(&ast)
                    .unwrap(),
            );
        }
        let foreign_id = modules[1].functions[0].id;
        assert_eq!(
            modules[0].functions[0].id, foreign_id,
            "control needs a real per-entry ID collision"
        );
        let alias = modules[0].intern_string("alpha.reexport");
        let target = modules[0].intern_string("beta.chosen");
        modules[0].mount_aliases.push((alias, foreign_id, target));
        let mut builder = verum_vbc::archive::ArchiveBuilder::new();
        for i in if reverse { [1, 0] } else { [0, 1] } {
            builder
                .add_module(if i == 0 { "alpha" } else { "beta" }, &modules[i], &[])
                .unwrap();
        }
        let ast =
            Parser::new("fn main()->Int { alpha.unrelated()+alpha.reexport()+beta.chosen() }")
                .parse_module()
                .unwrap();
        let mut codegen = VbcCodegen::new();
        verum_compiler::archive_ctx_loader::ArchiveCtxCache::new().apply_lazy_with_types(
            &builder.finish(),
            &mut codegen,
            &ast,
            &[],
        );
        let ctx = codegen.ctx_mut();
        let actual = ctx.lookup_function("alpha.reexport").unwrap().id;
        assert_eq!(actual, ctx.lookup_function("beta.chosen").unwrap().id);
        assert_ne!(actual, ctx.lookup_function("alpha.unrelated").unwrap().id);
    }
}
