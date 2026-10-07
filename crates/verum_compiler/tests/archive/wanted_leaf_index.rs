//! T1625: alias candidate pruning must preserve declaration-owned registration.
use super::*;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::archive::{ArchiveBuilder, read_archive, write_archive};
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn source_archive(owner: &str, source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .unwrap();
    round_trip(owner, &module)
}

fn round_trip(owner: &str, module: &VbcModule) -> VbcModule {
    let mut builder = ArchiveBuilder::new();
    builder.add_module(owner, module, &[]).unwrap();
    let mut wire = List::new();
    write_archive(&builder.finish(), &mut wire).unwrap();
    read_archive(std::io::Cursor::new(wire.as_slice()))
        .unwrap()
        .load_module(owner)
        .unwrap()
}

// Keep the legacy HashSet input type: its iteration order is part of the
// existing first-wins walk, although it differs between independent sets.
fn wanted(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn borrowed_leaf_buckets_preserve_the_original_candidate_order() {
    // Each independently seeded set has a different permitted iteration
    // order. The index must preserve whichever one the caller supplies.
    for _ in 0..8 {
        let names = wanted(&[
            "choose",
            "alpha.choose",
            "beta.choose",
            "core.alpha.choose",
            "alpha.other",
            "other",
            "δοκιμή.choose",
        ]);
        let index = wanted_aliases_by_leaf(&names);
        assert!(index.get(&"absent").is_none());
        assert_eq!(index.values().map(List::len).sum::<usize>(), names.len());
        for leaf in ["choose", "other"] {
            let expected: List<_> = names
                .iter()
                .filter(|name| name.rsplit('.').next() == Some(leaf))
                .collect();
            let actual = index.get(&leaf).unwrap();
            assert_eq!(actual.as_slice(), expected.as_slice());
            for (actual, expected) in actual.iter().zip(&expected) {
                assert!(
                    std::ptr::eq(*actual, *expected),
                    "index must borrow the original name"
                );
            }
        }
    }
}

#[test]
fn same_leaf_archive_owners_and_bare_first_wins_survive_both_module_orders() {
    let alpha = source_archive("alpha", "public fn choose()->Int {37}");
    let beta = source_archive("beta", "public fn choose()->Int {99}");
    let mut names = wanted(&["alpha.choose", "beta.choose", "choose", "absent.choose"]);
    for i in 0..256 {
        names.insert(format!("noise.module.other_{i}"));
    }
    for reverse in [false, true] {
        let mut ctx = CodegenContext::new();
        ctx.prefer_existing_functions = true;
        let mut next = 100;
        for (owner, module) in if reverse {
            [("beta", &beta), ("alpha", &alpha)]
        } else {
            [("alpha", &alpha), ("beta", &beta)]
        } {
            register_module_filtered(module, owner, &mut ctx, &names, &mut next);
        }
        let a = ctx.lookup_function("alpha.choose").unwrap().id;
        let b = ctx.lookup_function("beta.choose").unwrap().id;
        assert_ne!(a, b, "archive-local IDs must not conflate declarations");
        assert_eq!(
            ctx.lookup_function("choose").unwrap().id,
            if reverse { b } else { a }
        );
        assert!(ctx.lookup_function("absent.choose").is_none());
    }
}

#[test]
fn alias_prefixes_require_whole_segments_in_both_directions() {
    let module = source_archive("base.primitives", "public fn choose()->Int {37}");
    let names = wanted(&[
        "base.primitives.choose",
        "primitives.choose",
        "core.base.primitives.choose",
        "rimitives.choose",
        "other.primitives.choose",
        "primitives.other",
    ]);
    let mut ctx = CodegenContext::new();
    register_module_filtered(&module, "base.primitives", &mut ctx, &names, &mut 0);
    let id = ctx.lookup_function("base.primitives.choose").unwrap().id;
    for alias in ["primitives.choose", "core.base.primitives.choose"] {
        assert_eq!(ctx.lookup_function(alias).unwrap().id, id, "{alias}");
    }
    for rejected in [
        "rimitives.choose",
        "other.primitives.choose",
        "primitives.other",
    ] {
        assert!(ctx.lookup_function(rejected).is_none(), "{rejected}");
    }
}

#[test]
fn methods_do_not_claim_free_slots_and_descriptor_only_variants_stay_qualified() {
    let module = source_archive(
        "surface",
        r#"
type Token is { value: Int };
implement Token { fn choose(self)->Int {self.value} }
public fn choose()->Int {99}
type Choice is Ready(Int) | Empty;
public fn build()->Choice {Ready(7)}
"#,
    );
    let names = wanted(&[
        "Token",
        "Token.choose",
        "choose",
        "Choice",
        "Ready",
        "Choice.Ready",
        "build",
    ]);
    let mut ctx = CodegenContext::new();
    ctx.prefer_existing_functions = true;
    register_module_filtered(&module, "surface", &mut ctx, &names, &mut 0);
    let method = ctx.lookup_function("Token.choose").unwrap();
    assert!(method.parent_type_name.is_some());
    let free = ctx.lookup_function("choose").unwrap();
    assert!(free.parent_type_name.is_none());
    assert_ne!(free.id, method.id);
    let variant = ctx.lookup_function("Choice.Ready").unwrap();
    assert!(variant.variant_tag.is_some());
    assert_eq!(variant.param_count, 1);
    assert!(
        ctx.lookup_function("Ready").is_none(),
        "descriptor-only constructors deliberately stay qualified"
    );
}

#[test]
fn real_constructor_descriptors_keep_bare_and_qualified_variant_aliases() {
    let mut module = source_archive(
        "surface",
        r#"
type Choice is Ready(Int) | Empty;
public fn make_ready(value: Int)->Choice {Ready(value)}
"#,
    );
    let parent = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Choice"))
        .unwrap()
        .id;
    let index = module
        .functions
        .iter()
        .position(|f| module.get_string(f.name) == Some("surface.make_ready"))
        .unwrap();
    // Historical archives can store a real constructor descriptor, unlike
    // source-only sentinel constructors. Keep the source-generated signature
    // and body, then serialize that constructor identity through the wire.
    let name = module.intern_string("Ready");
    module.functions[index].name = name;
    module.functions[index].parent_type = Some(parent);
    let module = round_trip("surface", &module);
    let names = wanted(&["Ready", "Choice.Ready", "surface.Ready"]);
    let mut ctx = CodegenContext::new();
    register_module_filtered(&module, "surface", &mut ctx, &names, &mut 0);
    let bare = ctx.lookup_function("Ready").unwrap();
    assert!(bare.variant_tag.is_some());
    assert!(
        bare.id.0 < u32::MAX / 2,
        "must use the real descriptor, not a sentinel"
    );
    for alias in ["Choice.Ready", "surface.Ready"] {
        assert_eq!(ctx.lookup_function(alias).unwrap().id, bare.id, "{alias}");
    }
}

#[test]
#[ignore = "bounded registration measurement; run explicitly with --ignored --nocapture"]
fn measure_wanted_registration_scaling() {
    let mut source = Text::new();
    for i in 0..128 {
        source.push_str(&format!("public fn export_{i}()->Int {{{i}}} "));
    }
    let module = source_archive("measurement", source.as_str());
    for count in [256, 1024, 4096] {
        let mut names = wanted(&["measurement"]);
        for i in 0..count {
            names.insert(format!("noise.other_{i}"));
        }
        for i in 0..128 {
            names.insert(format!("measurement.export_{i}"));
        }
        let mut samples = List::new();
        for _ in 0..5 {
            let mut ctx = CodegenContext::new();
            let begin = std::time::Instant::now();
            let (_, registered) =
                register_module_filtered(&module, "measurement", &mut ctx, &names, &mut 0);
            samples.push(begin.elapsed().as_micros());
            assert_eq!(registered.len(), module.functions.len());
            for i in 0..128 {
                assert!(
                    ctx.lookup_function(&format!("measurement.export_{i}"))
                        .is_some()
                );
            }
        }
        samples.sort();
        println!(
            "T1625 isolated registration: descriptors={} source_exports=128 wanted={} median_us={} samples_us={samples:?}",
            module.functions.len(),
            names.len(),
            samples[2]
        );
    }
}
