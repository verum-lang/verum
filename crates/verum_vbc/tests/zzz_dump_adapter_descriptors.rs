//! Throwaway inspection harness (T0701 zip-class third landing).
//!
//! Dumps the baked FunctionDescriptors for the adapter-layer methods
//! whose impl/method generic-name collisions drive the zip class
//! (`ZipIter.map`, `MappedIter.map`, `MappedIter.reduce`, …) straight
//! from a `.vbca` on disk — no bake, no CLI (see the
//! inspect-baked-vbca recipe).  Path comes from `VERUM_DUMP_VBCA`;
//! the filter is `VERUM_DUMP_FN` (substring match on the qualified
//! name).  Ignored by default so `--tests` runs stay quiet.
//!
//! cargo test -p verum_vbc --test zzz_dump_adapter_descriptors -- \
//!   --ignored --nocapture

#[test]
#[ignore = "manual inspection harness — needs VERUM_DUMP_VBCA"]
fn dump_adapter_descriptors() {
    let Ok(path) = std::env::var("VERUM_DUMP_VBCA") else {
        eprintln!("set VERUM_DUMP_VBCA=<path to .vbca>");
        return;
    };
    let filter = std::env::var("VERUM_DUMP_FN")
        .unwrap_or_else(|_| "ZipIter.map".to_string());
    let needles: Vec<&str> = filter.split(',').collect();

    let archive = verum_vbc::archive::read_archive_from_file(&path)
        .expect("read_archive_from_file");
    let mut shown = 0usize;
    for raw in archive.module_data.iter() {
        let Ok(m) = verum_vbc::deserialize::deserialize_module(raw) else {
            continue;
        };
        for fd in m.functions.iter() {
            let Some(name) = m.strings.get(fd.name) else { continue };
            if !needles.iter().any(|n| name.contains(n)) {
                continue;
            }
            shown += 1;
            let tps: Vec<String> = fd
                .type_params
                .iter()
                .map(|tp| {
                    // Print carried fn-bounds too — their CONTENT was the
                    // blind spot that hid the b122 census poison (the
                    // descriptor looked byte-correct while type_bounds
                    // carried shifted slots).
                    let tbs: Vec<String> = tp
                        .type_bounds
                        .iter()
                        .map(|tb| format!("{:?}", tb))
                        .collect();
                    let tb_suffix = if tbs.is_empty() {
                        String::new()
                    } else {
                        format!(" bounds=[{}]", tbs.join("; "))
                    };
                    format!(
                        "{}#{}{}",
                        m.strings.get(tp.name).unwrap_or("<?>"),
                        tp.id.0,
                        tb_suffix
                    )
                })
                .collect();
            let params: Vec<String> = fd
                .params
                .iter()
                .map(|p| {
                    let pname = m.strings.get(p.name).unwrap_or("<?>");
                    let carried = if p.type_name == verum_vbc::StringId::EMPTY {
                        "<no-carry>".to_string()
                    } else {
                        m.strings
                            .get(p.type_name)
                            .unwrap_or("<?>")
                            .to_string()
                    };
                    format!("{}: {} [{:?}]", pname, carried, p.type_ref)
                })
                .collect();
            let ret_carry = fd
                .return_type_name
                .and_then(|sid| m.strings.get(sid))
                .unwrap_or("<none>");
            println!(
                "fn {} (fid={}) parent={:?}\n  type_params: [{}]\n  params:\n    {}\n  return: {:?}  carry='{}'\n  body_len={}",
                name,
                fd.id.0,
                fd.parent_type,
                tps.join(", "),
                params.join("\n    "),
                fd.return_type,
                ret_carry,
                fd.bytecode_length,
            );
        }
    }
    println!("== {} descriptors matched '{}'", shown, filter);

    // `VERUM_DUMP_TABLES=1` — the NAME TABLES beside the descriptors.
    //
    // A descriptor dump answers "is there a body called X". It cannot
    // answer "who recorded the name X", and those are different
    // questions the moment a name has no body: a string in a `.vbca`
    // is equally consistent with a pruned function and with a name
    // table pointing at one. Measured 2026-09-12 (T1461) on a
    // three-file stdlib: `the_leaf` has exactly ONE descriptor,
    // `core.probe_decl.the_leaf`, while the archive also carries the
    // string `core.probe_user.the_leaf` — the CONSUMER's path. Which
    // table holds it decides where the defect is, and nothing here
    // could say.
    if std::env::var_os("VERUM_DUMP_TABLES").is_some() {
        for raw in archive.module_data.iter() {
            let Ok(m) = verum_vbc::deserialize::deserialize_module(raw) else {
                continue;
            };
            let hit = |n: &str| needles.iter().any(|x| n.contains(x));
            for (fid, sid) in m.external_function_names.iter() {
                if let Some(n) = m.strings.get(*sid)
                    && hit(n)
                {
                    println!("external_function_names[{}] = '{}'  (module '{}')", fid.0, n, m.name);
                }
            }
            for (alias_sid, fid, canon_sid) in m.mount_aliases.iter() {
                let a = m.strings.get(*alias_sid).unwrap_or("<?>");
                let c = m.strings.get(*canon_sid).unwrap_or("<none>");
                if hit(a) || hit(c) {
                    println!("mount_aliases: '{}' -> fid {} (canonical '{}')  (module '{}')", a, fid.0, c, m.name);
                }
            }
        }
    }
}
