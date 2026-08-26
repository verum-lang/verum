//! What does a module actually export?
//!
//! `mount demo.lib.{Clock}` reported "cannot find `Clock`" with a note
//! listing three exports, all of them types — while the file plainly
//! declares `public context Clock`. The export walk HAS a context arm
//! and the parser HAS the visibility, so the question is which table the
//! mount reads and what is in it.
//!
//! Usage: `dump_module_exports <file.vr>`

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: dump_module_exports <file.vr>")?;
    let source = std::fs::read_to_string(&path)?;

    let mut parser = verum_fast_parser::Parser::new(&source);
    let module = parser
        .parse_module()
        .map_err(|e| format!("parse failed: {e:?}"))?;

    println!("top-level items:");
    for item in &module.items {
        let d = match &item.kind {
            verum_ast::ItemKind::Context(c) => {
                format!("context {} (visibility {:?})", c.name.name, c.visibility)
            }
            verum_ast::ItemKind::Type(t) => {
                format!("type {} (visibility {:?})", t.name.name, t.visibility)
            }
            verum_ast::ItemKind::Function(f) => {
                format!("fn {} (visibility {:?})", f.name.name, f.visibility)
            }
            verum_ast::ItemKind::Module(m) => format!(
                "module {} [{} items]",
                m.name.name,
                m.items.as_ref().map_or(0, |v| v.len())
            ),
            other => format!("{other:?}").chars().take(30).collect(),
        };
        println!("  {d}");
    }

    let table = verum_modules::exports::extract_exports_from_module(
        &module,
        verum_modules::ModuleId::new(0),
        &verum_modules::path::ModulePath::from_str("probe"),
    )
    .map_err(|e| format!("export extraction failed: {e:?}"))?;

    println!("export table: {} entries total", table.len());
    for (name, e) in table.all_exports() {
        println!("  {} :: {:?} visibility={:?}", name, e.kind, e.visibility);
    }
    Ok(())
}
