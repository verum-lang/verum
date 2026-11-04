#![cfg(feature = "codegen")]
//! Actual terminal span declarations; interface/wire qualification only, not UI execution.
use verum_ast::{ItemKind, Module};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    types::TypeRef,
};
fn source(relative: &str, names: &[&str]) -> Module {
    let core = std::env::var_os("VERUM_TEST_TERMINAL_CORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    let mut m = Parser::new(&std::fs::read_to_string(core.join(relative)).unwrap())
        .parse_module()
        .unwrap();
    m.items.retain(|i| {
        matches!(&i.kind, ItemKind::Module(_) | ItemKind::Mount(_))
            || matches!(&i.kind,ItemKind::Type(d)if names.contains(&d.name.as_str()))
    });
    assert_eq!(
        m.items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::Type(_)))
            .count(),
        names.len()
    );
    m
}
#[test]
fn hyperlink_payload_retains_actual_text_span_owner_on_wire() {
    let files = [
        ("core.base", source("base/maybe.vr", &["Maybe"])),
        (
            "core.term.style",
            source("term/style/color.vr", &["Color", "Rgb", "Hsl"]),
        ),
        (
            "core.term.style",
            source("term/style/modifier.vr", &["Modifier"]),
        ),
        ("core.term.style", source("term/style/style.vr", &["Style"])),
        (
            "core.term.widget",
            source("term/widget/paragraph.vr", &["TextSpan"]),
        ),
        (
            "core.term.style",
            source("term/style/hyperlink.vr", &["HyperlinkSpan"]),
        ),
    ];
    let owned: Vec<_> = files.iter().map(|(p, m)| (*p, m)).collect();
    let m = VbcCodegen::with_config(CodegenConfig::new("terminal_signature"))
        .collect_owned_declaration_interface(&owned)
        .unwrap();
    let wire = verum_vbc::serialize::serialize_module(&m).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for m in [m, restored] {
        let lookup = |owner, name| {
            m.types
                .iter()
                .find(|d| {
                    m.get_string(d.name) == Some(name)
                        && d.origin_module.and_then(|x| m.get_string(x)) == Some(owner)
                })
                .unwrap()
        };
        let span = lookup("core.term.widget.paragraph", "TextSpan");
        let hyperlink = lookup("core.term.style.hyperlink", "HyperlinkSpan");
        let field = hyperlink
            .fields
            .iter()
            .find(|f| m.get_string(f.name) == Some("span"))
            .unwrap();
        assert_eq!(field.type_ref, TypeRef::Concrete(span.id));
    }
}
