//! `verum import --from <format>` — read external knowledge-base
//! formats and emit a `.vr` source file with the corresponding typed
//! attributes.
//!
//! V1 (follow-up, Task B5) ships the OWL 2 Functional-Style
//! Syntax importer: `verum import --from owl2-fs <file.ofn>` reads a
//! W3C OWL 2 FS document and emits a `.vr` file populated with
//! `@owl2_class`, `@owl2_subclass_of`, `@owl2_property`,
//! `@owl2_characteristic`, `@owl2_disjoint_with`,
//! `@owl2_equivalent_class`, and `@owl2_has_key` attributes.
//!
//! Round-trips with `verum export --to owl2-fs` for FOAF-shaped
//! ontologies (Pellet / HermiT / Protégé / FaCT++ / ELK / Konclude
//! compatible). Pipeline:
//!
//! 1. Tokenise the `.ofn` source into atoms / parens / strings.
//! 2. Parse into a tree of `OwlSExpr` nodes.
//! 3. Walk the `Ontology(...)` body and populate an `Owl2Graph`.
//! 4. Emit a `.vr` file from the graph, preserving entity ordering.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::owl2::{Owl2Entity, Owl2EntityKind, Owl2Graph};
use crate::error::{CliError, Result};
use std::collections::BTreeSet;
use verum_ast::attr::Owl2Characteristic;
use verum_common::Text;

/// Source format selector for `verum import`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// OWL 2 Functional-Style Syntax (`.ofn`).
    Owl2Fs,
}

impl ImportFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "owl2-fs" | "owl2_fs" | "ofn" => Ok(Self::Owl2Fs),
            other => Err(CliError::InvalidArgument(
                format!(
                    "--from must be one of: owl2-fs / ofn (got '{}')",
                    other
                )
                .into(),
            )),
        }
    }
}

/// Options for `verum import`.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub format: ImportFormat,
    pub input: PathBuf,
    pub output: Option<PathBuf>,
}

/// Entry point for `verum import --from <format> <input>`.
pub fn run(options: ImportOptions) -> Result<()> {
    let bytes = fs::read_to_string(&options.input).map_err(|e| {
        CliError::Custom(
            format!("read {}: {}", options.input.display(), e).into(),
        )
    })?;
    let vr_source = match options.format {
        ImportFormat::Owl2Fs => import_owl2_fs(&bytes)?,
    };
    let out_path = options.output.unwrap_or_else(|| {
        options.input.with_extension("vr")
    });
    fs::write(&out_path, vr_source).map_err(|e| {
        CliError::Custom(
            format!("write {}: {}", out_path.display(), e).into(),
        )
    })?;
    println!("imported → {}", out_path.display());
    Ok(())
}

// =============================================================================
// OWL 2 FS importer (Task B5)
// =============================================================================

/// Read OWL 2 FS source, parse the `Ontology(...)` body, and emit a
/// `.vr` file populated with the corresponding `@owl2_*` typed
/// attributes.
pub fn import_owl2_fs(source: &str) -> Result<String> {
    let tree = parse_owl2_fs(source)?;
    let graph = build_graph_from_tree(&tree)?;
    Ok(emit_vr_from_graph(&graph))
}

/// One node in the parsed OWL 2 FS tree. Functional syntax is a
/// parenthesised language: every form is either an atom (an
/// identifier or a `:LocalName` / `<full-iri>`), a quoted string
/// literal, or a list `Op(arg1 arg2 ... argN)`.
#[derive(Debug, Clone)]
enum OwlSExpr {
    Atom(String),
    Str(String),
    List {
        head: String,
        args: Vec<OwlSExpr>,
    },
}

/// Tokenise + parse OWL 2 FS source into a flat list of top-level
/// `OwlSExpr` forms (Prefix(...), Ontology(...), …).
fn parse_owl2_fs(source: &str) -> Result<Vec<OwlSExpr>> {
    let tokens = tokenize_owl2_fs(source);
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < tokens.len() {
        let expr = parse_one(&tokens, &mut pos)?;
        out.push(expr);
    }
    Ok(out)
}

/// OWL 2 FS tokens. Comments (`# ... \n`) are stripped during
/// tokenisation.
#[derive(Debug, Clone)]
enum Tok {
    Open,
    Close,
    Atom(String),
    Str(String),
}

fn tokenize_owl2_fs(source: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            // Line comment to next \n.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '(' {
            out.push(Tok::Open);
            i += 1;
            continue;
        }
        if c == ')' {
            out.push(Tok::Close);
            i += 1;
            continue;
        }
        if c == '"' {
            // Quoted literal — consume until next unescaped quote.
            i += 1;
            let mut s = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1] as char;
                    match next {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => {
                            s.push('\\');
                            s.push(other);
                        }
                    }
                    i += 2;
                } else {
                    s.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1; // consume closing quote
            }
            out.push(Tok::Str(s));
            continue;
        }
        // Atom — consume until whitespace or paren.
        let start = i;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'(' || b == b')' || (b as char).is_whitespace() {
                break;
            }
            i += 1;
        }
        let raw = &source[start..i];
        out.push(Tok::Atom(raw.to_string()));
    }
    out
}

fn parse_one(tokens: &[Tok], pos: &mut usize) -> Result<OwlSExpr> {
    if *pos >= tokens.len() {
        return Err(CliError::Custom(
            "owl2-fs: unexpected end of input".into(),
        ));
    }
    match &tokens[*pos] {
        Tok::Atom(a) => {
            let head = a.clone();
            *pos += 1;
            // Functional form: <Atom>(args) — peek for Open. Entity-
            // name atoms (`:LocalName`, `<full-iri>`, prefixed `xsd:int`)
            // never take args; only operator atoms (Class, ObjectProperty,
            // SubClassOf, …) do, so a colon-/angle-prefixed head must
            // be returned as a bare atom even if Open follows.
            let takes_args = !head.starts_with(':')
                && !head.starts_with('<')
                && !head.contains(':');
            if takes_args
                && *pos < tokens.len()
                && matches!(tokens[*pos], Tok::Open)
            {
                *pos += 1;
                let mut args = Vec::new();
                while *pos < tokens.len() && !matches!(tokens[*pos], Tok::Close) {
                    args.push(parse_one(tokens, pos)?);
                }
                if *pos >= tokens.len() {
                    return Err(CliError::Custom(
                        "owl2-fs: unmatched '('".into(),
                    ));
                }
                *pos += 1; // consume Close
                Ok(OwlSExpr::List { head, args })
            } else {
                Ok(OwlSExpr::Atom(head))
            }
        }
        Tok::Str(s) => {
            let v = s.clone();
            *pos += 1;
            Ok(OwlSExpr::Str(v))
        }
        Tok::Open => {
            // Bare list — represent with empty head; caller handles.
            *pos += 1;
            let mut args = Vec::new();
            while *pos < tokens.len() && !matches!(tokens[*pos], Tok::Close) {
                args.push(parse_one(tokens, pos)?);
            }
            if *pos >= tokens.len() {
                return Err(CliError::Custom(
                    "owl2-fs: unmatched '('".into(),
                ));
            }
            *pos += 1;
            Ok(OwlSExpr::List {
                head: String::new(),
                args,
            })
        }
        Tok::Close => Err(CliError::Custom(
            "owl2-fs: unexpected ')'".into(),
        )),
    }
}

/// Walk the parsed tree, find `Ontology(...)`, and populate an
/// `Owl2Graph` from its body.
fn build_graph_from_tree(tree: &[OwlSExpr]) -> Result<Owl2Graph> {
    let mut graph = Owl2Graph::default();
    for top in tree {
        if let OwlSExpr::List { head, args } = top {
            if head == "Ontology" {
                process_ontology_body(args, &mut graph)?;
            }
            // Prefix(...) declarations are accepted but not tracked
            // — Owl2Graph only stores entity-shaped data, not IRI
            // prefix bindings. Round-trip still emits a default
            // prefix block in the exporter.
        }
    }
    Ok(graph)
}

fn process_ontology_body(args: &[OwlSExpr], graph: &mut Owl2Graph) -> Result<()> {
    for arg in args {
        match arg {
            OwlSExpr::List { head, args } => {
                process_ontology_axiom(head, args, graph)?;
            }
            // First arg of Ontology(...) is typically the ontology
            // IRI as a bare atom or `<...>`; ignore — we don't
            // materialise the ontology IRI back into the .vr output
            // (the exporter regenerates it from the manifest name).
            OwlSExpr::Atom(_) | OwlSExpr::Str(_) => {}
        }
    }
    Ok(())
}

fn process_ontology_axiom(
    head: &str,
    args: &[OwlSExpr],
    graph: &mut Owl2Graph,
) -> Result<()> {
    match head {
        "Declaration" => {
            // Declaration(Class(:Name)) | Declaration(ObjectProperty(:Name))
            if let Some(OwlSExpr::List {
                head: inner_head,
                args: inner_args,
            }) = args.first()
            {
                let name = inner_args
                    .first()
                    .and_then(extract_local_name)
                    .ok_or_else(|| {
                        CliError::Custom(
                            "owl2-fs: Declaration missing entity name".into(),
                        )
                    })?;
                match inner_head.as_str() {
                    "Class" => {
                        graph.add_entity(Owl2Entity::new_class(
                            Text::from(name),
                            None,
                            PathBuf::from("imported.ofn"),
                        ));
                    }
                    "ObjectProperty" => {
                        graph.add_entity(Owl2Entity::new_property(
                            Text::from(name),
                            PathBuf::from("imported.ofn"),
                            None,
                            None,
                            None,
                            BTreeSet::new(),
                        ));
                    }
                    // NamedIndividual / DataProperty / AnnotationProperty
                    // — out of scope for V1 round-trip.
                    _ => {}
                }
            }
        }
        "SubClassOf" => {
            if let (Some(child), Some(parent)) = (
                args.get(0).and_then(extract_local_name),
                args.get(1).and_then(extract_local_name),
            ) {
                graph
                    .subclass_edges
                    .insert((Text::from(child), Text::from(parent)));
            }
        }
        "EquivalentClasses" => {
            // Symmetric pairs — emit (a,b) and (b,a) for every pair
            // of distinct elements in the equivalence list.
            let names: Vec<String> =
                args.iter().filter_map(extract_local_name).collect();
            for i in 0..names.len() {
                for j in (i + 1)..names.len() {
                    let a = Text::from(names[i].clone());
                    let b = Text::from(names[j].clone());
                    graph.equivalence_pairs.insert((a.clone(), b.clone()));
                    graph.equivalence_pairs.insert((b, a));
                }
            }
        }
        "DisjointClasses" => {
            let names: Vec<String> =
                args.iter().filter_map(extract_local_name).collect();
            for i in 0..names.len() {
                for j in (i + 1)..names.len() {
                    let a = Text::from(names[i].clone());
                    let b = Text::from(names[j].clone());
                    graph.disjoint_pairs.insert((a.clone(), b.clone()));
                    graph.disjoint_pairs.insert((b, a));
                }
            }
        }
        "HasKey" => {
            // HasKey(Class () (op1 op2)) — class then two parens
            // groups (object-properties + data-properties); we
            // collect any local names in the trailing groups as
            // a single key tuple.
            if let Some(class_name) = args.first().and_then(extract_local_name) {
                let mut key: Vec<Text> = Vec::new();
                for arg in args.iter().skip(1) {
                    if let OwlSExpr::List {
                        head: _, args: inner,
                    } = arg
                    {
                        for a in inner {
                            if let Some(n) = extract_local_name(a) {
                                key.push(Text::from(n));
                            }
                        }
                    } else if let Some(n) = extract_local_name(arg) {
                        key.push(Text::from(n));
                    }
                }
                if !key.is_empty() {
                    let class_key = Text::from(class_name);
                    let entry = graph
                        .entities
                        .entry(class_key.clone())
                        .or_insert_with(|| {
                            Owl2Entity::new_class(
                                class_key,
                                None,
                                PathBuf::from("imported.ofn"),
                            )
                        });
                    entry.keys.push(key);
                }
            }
        }
        "ObjectPropertyDomain" => {
            if let (Some(prop), Some(cls)) = (
                args.get(0).and_then(extract_local_name),
                args.get(1).and_then(extract_local_name),
            ) {
                let key = Text::from(prop);
                let entry = graph.entities.entry(key.clone()).or_insert_with(|| {
                    Owl2Entity::new_property(
                        key,
                        PathBuf::from("imported.ofn"),
                        None,
                        None,
                        None,
                        BTreeSet::new(),
                    )
                });
                entry.property_domain = Some(Text::from(cls));
            }
        }
        "ObjectPropertyRange" => {
            if let (Some(prop), Some(cls)) = (
                args.get(0).and_then(extract_local_name),
                args.get(1).and_then(extract_local_name),
            ) {
                let key = Text::from(prop);
                let entry = graph.entities.entry(key.clone()).or_insert_with(|| {
                    Owl2Entity::new_property(
                        key,
                        PathBuf::from("imported.ofn"),
                        None,
                        None,
                        None,
                        BTreeSet::new(),
                    )
                });
                entry.property_range = Some(Text::from(cls));
            }
        }
        "InverseObjectProperties" => {
            if let (Some(p), Some(q)) = (
                args.get(0).and_then(extract_local_name),
                args.get(1).and_then(extract_local_name),
            ) {
                let p_text = Text::from(p);
                let q_text = Text::from(q);
                let entry = graph
                    .entities
                    .entry(p_text.clone())
                    .or_insert_with(|| {
                        Owl2Entity::new_property(
                            p_text,
                            PathBuf::from("imported.ofn"),
                            None,
                            None,
                            None,
                            BTreeSet::new(),
                        )
                    });
                entry.property_inverse_of = Some(q_text);
            }
        }
        // Characteristic flags — Shkotin Table 6 / W3C §9.2.
        "TransitiveObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Transitive);
        }
        "SymmetricObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Symmetric);
        }
        "AsymmetricObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Asymmetric);
        }
        "ReflexiveObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Reflexive);
        }
        "IrreflexiveObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Irreflexive);
        }
        "FunctionalObjectProperty" => {
            attach_characteristic(graph, args, Owl2Characteristic::Functional);
        }
        "InverseFunctionalObjectProperty" => {
            attach_characteristic(
                graph,
                args,
                Owl2Characteristic::InverseFunctional,
            );
        }
        // Out-of-scope axioms (DataProperty*, Annotation*, etc.) are
        // accepted silently — round-trip preserves the in-scope
        // surface; the rest is ignored at this V1 stage.
        _ => {}
    }
    Ok(())
}

fn attach_characteristic(
    graph: &mut Owl2Graph,
    args: &[OwlSExpr],
    flag: Owl2Characteristic,
) {
    if let Some(name) = args.first().and_then(extract_local_name) {
        let key = Text::from(name);
        let entry = graph.entities.entry(key.clone()).or_insert_with(|| {
            Owl2Entity::new_property(
                key,
                PathBuf::from("imported.ofn"),
                None,
                None,
                None,
                BTreeSet::new(),
            )
        });
        entry.property_characteristics.insert(flag);
    }
}

/// Extract a local name from an `:Name` atom, a `<#Name>` IRI, or a
/// nested `ObjectInverseOf(:p)` / `Class(:Name)` form (taking the
/// first nameable atom).
fn extract_local_name(expr: &OwlSExpr) -> Option<String> {
    match expr {
        OwlSExpr::Atom(a) => Some(strip_iri_decoration(a)),
        OwlSExpr::List { args, .. } => {
            args.first().and_then(extract_local_name)
        }
        OwlSExpr::Str(_) => None,
    }
}

fn strip_iri_decoration(raw: &str) -> String {
    let s = raw.trim();
    // Leading colon indicates default-prefix local name.
    if let Some(rest) = s.strip_prefix(':') {
        return rest.to_string();
    }
    // Angle-bracket IRI — strip and use last fragment after '#' or '/'.
    if let Some(rest) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        let fragment = rest
            .rsplit(['#', '/'])
            .next()
            .unwrap_or(rest);
        return fragment.to_string();
    }
    // Prefixed name `prefix:LocalName` — keep local part.
    if let Some((_, local)) = s.rsplit_once(':') {
        return local.to_string();
    }
    s.to_string()
}

// =============================================================================
// Owl2Graph → .vr emitter
// =============================================================================

fn emit_vr_from_graph(graph: &Owl2Graph) -> String {
    let mut out = String::new();
    out.push_str(
        "// Imported from OWL 2 Functional Syntax via `verum import \
         --from owl2-fs` (/ B5).\n",
    );
    out.push_str(
        "// Round-trip target for `verum export --to owl2-fs`. \
         Entities are emitted in BTreeMap-sorted order for \
         deterministic CI diffs.\n\n",
    );

    // Entities: classes first, then properties; alphabetical within
    // each group via the underlying BTreeMap.
    for (name, e) in &graph.entities {
        match e.kind {
            Owl2EntityKind::Class => emit_class(&mut out, name, e, graph),
            Owl2EntityKind::Property => emit_property(&mut out, name, e),
        }
    }

    // Stand-alone subclass edges that don't bind to a declared class
    // entity — we emit a synthetic stub so the relation isn't lost.
    for (child, parent) in &graph.subclass_edges {
        if !graph.entities.contains_key(child) {
            out.push_str(&format!(
                "@owl2_subclass_of({})\npublic type {} is {{}};\n\n",
                parent.as_str(),
                child.as_str(),
            ));
        }
    }

    out
}

fn emit_class(out: &mut String, name: &Text, e: &Owl2Entity, graph: &Owl2Graph) {
    // @owl2_class — open-world flag if explicitly OpenWorld.
    if matches!(e.semantics, Some(verum_ast::attr::Owl2Semantics::OpenWorld)) {
        out.push_str("@owl2_class(open_world)\n");
    } else {
        out.push_str("@owl2_class\n");
    }

    // @owl2_subclass_of for every direct parent.
    let parents: Vec<&Text> = graph
        .subclass_edges
        .iter()
        .filter(|(c, _)| c == name)
        .map(|(_, p)| p)
        .collect();
    for parent in &parents {
        out.push_str(&format!("@owl2_subclass_of({})\n", parent.as_str()));
    }

    // @owl2_equivalent_class — emit for unordered representative
    // pairs only (avoid duplicating the symmetric mirror).
    let mut seen_eq: BTreeSet<&Text> = BTreeSet::new();
    for (a, b) in &graph.equivalence_pairs {
        if a == name && !seen_eq.contains(b) && a < b {
            out.push_str(&format!("@owl2_equivalent_class({})\n", b.as_str()));
            seen_eq.insert(b);
        }
    }

    // @owl2_disjoint_with — same dedup rule.
    let mut seen_dj: BTreeSet<&Text> = BTreeSet::new();
    for (a, b) in &graph.disjoint_pairs {
        if a == name && !seen_dj.contains(b) && a < b {
            out.push_str(&format!("@owl2_disjoint_with({})\n", b.as_str()));
            seen_dj.insert(b);
        }
    }

    // @owl2_has_key for every key tuple.
    for key in &e.keys {
        let parts: Vec<String> =
            key.iter().map(|t| t.as_str().to_string()).collect();
        out.push_str(&format!("@owl2_has_key({})\n", parts.join(", ")));
    }

    out.push_str(&format!("public type {} is {{}};\n\n", name.as_str()));
}

fn emit_property(out: &mut String, name: &Text, e: &Owl2Entity) {
    // Build @owl2_property(domain = ..., range = ..., inverse_of = ...)
    let mut prop_parts: Vec<String> = Vec::new();
    if let Some(d) = &e.property_domain {
        prop_parts.push(format!("domain = {}", d.as_str()));
    }
    if let Some(r) = &e.property_range {
        prop_parts.push(format!("range = {}", r.as_str()));
    }
    if let Some(inv) = &e.property_inverse_of {
        prop_parts.push(format!("inverse_of = {}", inv.as_str()));
    }
    if prop_parts.is_empty() {
        out.push_str("@owl2_property\n");
    } else {
        out.push_str(&format!("@owl2_property({})\n", prop_parts.join(", ")));
    }

    // Characteristic flags — emit each as a separate
    // @owl2_characteristic(<flag>) attribute.
    for c in &e.property_characteristics {
        let token = match c {
            Owl2Characteristic::Transitive => "transitive",
            Owl2Characteristic::Symmetric => "symmetric",
            Owl2Characteristic::Asymmetric => "asymmetric",
            Owl2Characteristic::Reflexive => "reflexive",
            Owl2Characteristic::Irreflexive => "irreflexive",
            Owl2Characteristic::Functional => "functional",
            Owl2Characteristic::InverseFunctional => "inverse_functional",
        };
        out.push_str(&format!("@owl2_characteristic({})\n", token));
    }

    out.push_str(&format!("public fn {}() {{}}\n\n", name.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_render(input: &str) -> String {
        let tree = parse_owl2_fs(input).expect("parse");
        let graph = build_graph_from_tree(&tree).expect("build");
        emit_vr_from_graph(&graph)
    }

    #[test]
    fn import_minimal_class_declaration() {
        let src = r#"
            Prefix(:=<http://example.org/foo#>)
            Ontology(<http://example.org/foo>
                Declaration(Class(:Person))
            )
        "#;
        let vr = parse_and_render(src);
        assert!(vr.contains("@owl2_class"));
        assert!(vr.contains("public type Person is"));
    }

    #[test]
    fn import_subclass_axiom() {
        let src = r#"
            Ontology(<x>
                Declaration(Class(:Person))
                Declaration(Class(:Animal))
                SubClassOf(:Person :Animal)
            )
        "#;
        let vr = parse_and_render(src);
        assert!(
            vr.contains("@owl2_subclass_of(Animal)"),
            "expected subclass attr; got:\n{}",
            vr
        );
    }

    #[test]
    fn import_object_property_with_domain_range() {
        let src = r#"
            Ontology(<x>
                Declaration(ObjectProperty(:knows))
                Declaration(Class(:Person))
                ObjectPropertyDomain(:knows :Person)
                ObjectPropertyRange(:knows :Person)
                SymmetricObjectProperty(:knows)
                TransitiveObjectProperty(:knows)
            )
        "#;
        let vr = parse_and_render(src);
        assert!(
            vr.contains("@owl2_property(domain = Person, range = Person)"),
            "expected property domain/range; got:\n{}",
            vr
        );
        assert!(vr.contains("@owl2_characteristic(symmetric)"));
        assert!(vr.contains("@owl2_characteristic(transitive)"));
    }

    #[test]
    fn import_haskey_axiom() {
        let src = r#"
            Ontology(<x>
                Declaration(Class(:Order))
                Declaration(ObjectProperty(:hasOrderId))
                HasKey(:Order () (:hasOrderId))
            )
        "#;
        let vr = parse_and_render(src);
        assert!(
            vr.contains("@owl2_has_key(hasOrderId)"),
            "expected hasKey attr; got:\n{}",
            vr
        );
    }

    #[test]
    fn import_disjoint_and_equivalent_classes() {
        let src = r#"
            Ontology(<x>
                Declaration(Class(:Cat))
                Declaration(Class(:Dog))
                Declaration(Class(:Feline))
                DisjointClasses(:Cat :Dog)
                EquivalentClasses(:Cat :Feline)
            )
        "#;
        let vr = parse_and_render(src);
        assert!(
            vr.contains("@owl2_disjoint_with(Dog)"),
            "expected disjoint attr; got:\n{}",
            vr
        );
        assert!(
            vr.contains("@owl2_equivalent_class(Feline)"),
            "expected equiv attr; got:\n{}",
            vr
        );
    }

    #[test]
    fn import_iri_form_strips_to_local_name() {
        let src = r#"
            Ontology(<x>
                Declaration(Class(<http://example.org/foo#Person>))
            )
        "#;
        let vr = parse_and_render(src);
        assert!(
            vr.contains("public type Person is"),
            "IRI didn't strip to local name; got:\n{}",
            vr
        );
    }

    #[test]
    fn import_skips_comments_and_whitespace() {
        let src = r#"
            # leading comment
            Prefix(:=<http://x#>)
            # mid comment
            Ontology(<x>
                # body comment
                Declaration(Class(:A))
            )
        "#;
        let vr = parse_and_render(src);
        assert!(vr.contains("public type A is"));
    }
}
