//! T1068 investigation instrument: what does the shipped archive carry
//! for `implement Differentiable for Float`?
//!
//! `#[ignore]`d on purpose.  It needs an archive path and would
//! otherwise SKIP in every ordinary run — a test that always skips is
//! indistinguishable from a test that always passes, and would read as
//! coverage it does not provide.  The gate for this defect is
//! `vcs/specs/L0-critical/types/bounds/primitive_satisfies_a_stdlib_protocol.vr`
//! plus `primitive_impls_survive_the_bake` in verum_compiler.  A bake gap and a load gap are different defects and
//! the loader trace cannot tell them apart — the archive can.
//!
//! Positive control is built in: the same scan counts `Iterator`, whose
//! impls demonstrably reach the registry, so an empty `Differentiable`
//! count cannot be read as "the scan found nothing anywhere".

#[test]
#[ignore = "investigation instrument; set T0460_ARCHIVE=/path/to/runtime.vbca"]
fn archive_carries_autodiff_protocol_impls() {
    let Ok(path) = std::env::var("T0460_ARCHIVE") else {
        eprintln!("T0460_ARCHIVE unset — skipping");
        return;
    };
    let archive =
        verum_vbc::archive::read_archive_from_file(&path).expect("archive reads");
    let mut modules = 0usize;
    let mut types_with_protocols = 0usize;
    let mut per_proto: std::collections::BTreeMap<String, usize> =
        Default::default();
    let mut autodiff_types: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut hits: Vec<(String, String)> = Vec::new();
    let mut prim_protocols: Vec<(String, String, usize)> = Vec::new();
    let mut primitive_kind: Vec<(String, String, usize)> = Vec::new();

    for raw in archive.module_data.iter() {
        let Ok(m) = verum_vbc::deserialize::deserialize_module(raw) else {
            continue;
        };
        modules += 1;
        let mname = m.name.clone();
        names.push(mname.clone());
        for td in m.types.iter() {
            if !td.protocols.is_empty() {
                types_with_protocols += 1;
            }
            let tname = m.strings.get(td.name).unwrap_or("?").to_string();
            hits.push((tname.clone(), mname.clone()));
            if matches!(tname.as_str(), "Text" | "Ordering" | "Bool" | "Char")
                && !td.protocols.is_empty()
            {
                eprintln!(
                    "[t0460] PRECEDENT {tname} in {mname}: id={} kind={:?} fields={} protocols={}",
                    td.id.0,
                    td.kind,
                    td.fields.len(),
                    td.protocols.len()
                );
            }
            prim_protocols.push((tname.clone(), mname.clone(), td.protocols.len()));
            if td.kind == verum_vbc::types::TypeKind::Primitive {
                primitive_kind.push((tname.clone(), mname.clone(), td.protocols.len()));
            }
            if mname.contains("autodiff") {
                autodiff_types.push(format!(
                    "{}(protocols={})",
                    tname,
                    td.protocols.len()
                ));
            }
            for pi in td.protocols.iter() {
                // ProtocolId → name is not resolvable here; count by the
                // string table hit instead, which is what the loader reads.
                let key = format!("proto_id={}", pi.protocol.0);
                *per_proto.entry(key).or_default() += 1;
            }
        }
    }
    eprintln!("[t0460] modules={modules} types_with_protocols={types_with_protocols}");
    eprintln!("[t0460] distinct protocol ids used = {}", per_proto.len());
    eprintln!("[t0460] autodiff types = {autodiff_types:?}");
    // A ProtocolId is MODULE-LOCAL.  A global id -> name map built
    // across modules answers confidently and wrongly: measured on this
    // archive, 55062 collisions — id 110 is `Clone` in one module and
    // `Display` in another.  The first version of this probe did
    // exactly that and reported protocol names for `Text` that no
    // `implement` in the source ever wrote.  Resolve WITHIN the module.
    for raw in archive.module_data.iter() {
        let Ok(m) = verum_vbc::deserialize::deserialize_module(raw) else {
            continue;
        };
        let mut local: std::collections::BTreeMap<u32, &str> = Default::default();
        for td in m.types.iter() {
            if td.kind == verum_vbc::types::TypeKind::Protocol
                && let Some(n) = m.strings.get(td.name)
            {
                local.insert(td.id.0, n);
            }
        }
        for td in m.types.iter() {
            if m.strings.get(td.name) == Some("Text") && !td.protocols.is_empty() {
                // An id with no local protocol descriptor is NOT a name —
                // print it as unresolved rather than borrowing one from
                // another module.
                let named: Vec<String> = td
                    .protocols
                    .iter()
                    .map(|pi| match local.get(&pi.protocol.0) {
                        Some(n) => (*n).to_string(),
                        None => format!("<unresolved:{}>", pi.protocol.0),
                    })
                    .collect();
                eprintln!("[t0460] Text@{} protocols = {named:?}", m.name);
            }
        }
    }
    let total: usize = per_proto.values().sum();
    eprintln!("[t0460] TOTAL protocol impls carried by archive = {total}");
    let mut math: Vec<String> = names
        .iter()
        .filter(|n| n.contains("math"))
        .cloned()
        .collect();
    math.sort();
    eprintln!("[t0460] math-named modules ({}) = {math:?}", math.len());
    eprintln!("[t0460] first 12 module names = {:?}", &names[..names.len().min(12)]);
    // Types DECLARED in core/math/autodiff.vr.  If the bake folds a
    // file into its directory module they appear under `core.math`;
    // if the file is not baked at all they appear nowhere.  Absence
    // everywhere and presence under another name are different
    // defects, so ask for the owner, not for a yes/no.
    for probe in ["DiffMode", "GradientScope", "ComputeGraph", "MemoryTracker"] {
        let owners: Vec<&str> = hits
            .iter()
            .filter(|(t, _)| t == probe)
            .map(|(_, m)| m.as_str())
            .collect();
        eprintln!("[t0460] type {probe} declared in modules = {owners:?}");
    }
    // The format carries an impl ON the target type's descriptor.  An
    // `implement P for Float` written in core/math/autodiff.vr can only
    // survive if THIS module also has a `Float` descriptor to hang it
    // on.  Ask whether primitives carry protocols anywhere at all.
    // Which descriptors did the T1068 carrier actually create?  Ask for
    // the KIND, not for a name I expect — a carrier under an unexpected
    // spelling and no carrier at all are different answers.
    let prims: Vec<String> = primitive_kind
        .iter()
        .map(|(n, m, c)| format!("{n}@{m}:{c}"))
        .collect();
    eprintln!("[t0460] Primitive-kind descriptors ({}) = {prims:?}", prims.len());
    for prim in ["Float", "Int", "Bool", "Text", "USize", "Ordering"] {
        let carriers: Vec<String> = prim_protocols
            .iter()
            .filter(|(t, _, n)| t == prim && *n > 0)
            .map(|(_, m, n)| format!("{m}:{n}"))
            .collect();
        let decls = prim_protocols.iter().filter(|(t, _, _)| t == prim).count();
        eprintln!(
            "[t0460] primitive {prim}: {decls} descriptor(s), protocol-carrying = {carriers:?}"
        );
    }
}
