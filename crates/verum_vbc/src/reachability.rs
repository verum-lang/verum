//! Function-level reachability analysis over a VBC module (T0103).
//!

//! An AOT build lowers `module.functions` wholesale — for a hello-world
//! that is the entire baked stdlib (~48K functions), and every
//! unresolved cross-module call inside a *never-executed* archive body
//! degrades to a const-zero stub that the strict-mono gate then counts.
//! The gate can never flip while it counts dead carry.  This module
//! computes the conservative closure of functions reachable from a
//! program's roots so consumers (the unresolved-call report, the
//! strict gate, and — later — the lowering loop itself) can scope
//! themselves to code that can actually run.
//!

//! # Conservatism contract
//!

//! The walk over-approximates, never under-approximates:
//!

//!  * Direct calls (`Call` / `TailCall` / `CallG` / `Spawn` /
//!  `GenCreate` / `NewClosure`) follow the function id; band / stub
//!  ids are chased through `external_function_names` the same way
//!  the runtime does (exact name first).
//!  * `CallM` dispatches by NAME at runtime (with runtime type
//!  switches over every same-name candidate), so every function
//!  whose bare method name matches a called method name is kept —
//!  narrowed by type evidence: a RECORD type's methods are excluded
//!  while no `New`/`NewG` of that type appears in reachable code
//!  (variants, newtypes, primitives, and well-known builtins stay
//!  conservatively included). A type becoming reachable also pulls
//!  its implicit edges — drop glue, clone glue, protocol
//!  dispatch-table methods.
//!  * Function references materialised as VALUES (`Constant::Function`
//!  entries, `FfiExtended::CreateCallback` payloads) are roots —
//!  anything that can flow into an indirect call stays.
//!  * Global ctors / dtors and mount aliases are roots.
//!

//! New fn-id-bearing instructions MUST be added to `visit_function`
//! (the codegen serializer keeps the same list in its remap arm —
//! `codegen/mod.rs`, `Instruction::Call | TailCall | …` — keep the
//! two in sync).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::instruction::{Instruction, SystemSubOpcode};
use crate::module::{Constant, VbcModule};

/// Result of a reachability walk.
#[derive(Debug, Default)]
pub struct Reachability {
    /// Ids of functions reachable from the roots (function-table ids;
    /// band/stub ids are resolved before insertion when possible).
    pub reachable_ids: HashSet<u32>,
    /// Names of reachable functions (interned module strings) — the
    /// report layer keys unresolved-call records by LLVM symbol name,
    /// which for VBC-lowered functions is the VBC function name.
    pub reachable_names: HashSet<String>,
    /// Method names invoked via `CallM` anywhere in reachable code —
    /// used for the conservative by-name candidate closure.
    pub called_method_names: HashSet<String>,
    /// Type ids whose instances are constructed in reachable code
    /// (`New` / `NewG`). Marking a type reachable also pulls its
    /// implicit function edges: `drop_fn`, `clone_fn`, and protocol
    /// dispatch-table methods.
    pub reachable_type_ids: HashSet<u32>,
}

/// Compute the conservative reachable set for `module`.
///

/// Roots: the entry function (named `main`, when present), global
/// ctors/dtors, every `Constant::Function` in the constant pool, and
/// mount-alias targets. Callers with additional roots (e.g. exported
/// functions in library builds) extend via `analyze_with_roots`.
pub fn analyze(module: &VbcModule) -> Reachability {
    let mut roots: Vec<u32> = Vec::new();
    if let Some(main_id) = module.find_function_by_name("main") {
        roots.push(main_id.0);
    }
    analyze_with_roots(module, &roots)
}

/// Compute the conservative reachable set starting from `extra_roots`
/// plus the implicit roots (ctors, dtors, constant-pool function refs,
/// mount aliases).
pub fn analyze_with_roots(module: &VbcModule, extra_roots: &[u32]) -> Reachability {
    // Pre-index: id → descriptor position, name → ids, bare method
    // name (last dotted segment) → ids.
    let mut by_id: HashMap<u32, usize> = HashMap::with_capacity(module.functions.len());
    let mut by_bare_name: HashMap<&str, Vec<u32>> = HashMap::new();
    for (pos, f) in module.functions.iter().enumerate() {
        by_id.insert(f.id.0, pos);
        if let Some(name) = module.strings.get(f.name) {
            let bare = name.rsplit('.').next().unwrap_or(name);
            by_bare_name.entry(bare).or_default().push(f.id.0);
        }
    }
    // Band/stub name table: id → qualified name.
    let external_names: HashMap<u32, &str> = module
        .external_function_names
        .iter()
        .filter_map(|(fid, sid)| module.strings.get(*sid).map(|s| (fid.0, s)))
        .collect();

    // Type table index (types are also positionally indexed by the
    // `New`/`NewG` type-table operand, but the descriptor's own id is
    // the authority the rest of the toolchain keys on).
    let type_by_id: HashMap<u32, &crate::types::TypeDescriptor> =
        module.types.iter().map(|t| (t.id.0, t)).collect();

    let mut out = Reachability::default();
    let mut queue: VecDeque<u32> = VecDeque::new();

    let push = |id: u32, out: &mut Reachability, queue: &mut VecDeque<u32>| {
        if out.reachable_ids.insert(id) {
            queue.push_back(id);
        }
    };

    for id in extra_roots {
        push(*id, &mut out, &mut queue);
    }
    for (fid, _prio) in module.global_ctors.iter().chain(module.global_dtors.iter()) {
        push(fid.0, &mut out, &mut queue);
    }
    for c in &module.constants {
        if let Constant::Function(fid) = c {
            push(fid.0, &mut out, &mut queue);
        }
    }
    for (_alias, fid, _canon) in &module.mount_aliases {
        push(fid.0, &mut out, &mut queue);
    }

    // Resolve a callee id the way the runtime would: direct table hit,
    // else name-chase for band/stub ids via external_function_names.
    let resolve = |id: u32| -> Option<u32> {
        if by_id.contains_key(&id) {
            return Some(id);
        }
        if crate::stub_ranges::is_xmod_name_reference(id) || crate::stub_ranges::is_stub_id(id) {
            if let Some(name) = external_names.get(&id) {
                // Ranked discipline (exact → head-strip → qualified
                // suffix) — the same resolver the AOT band-id chase
                // uses, so the walk and the lowering agree on which
                // band calls have concrete targets.
                if let Some(fid) = module.resolve_function_by_name_ranked(name) {
                    return Some(fid.0);
                }
            }
        }
        None
    };

    // Worklist pass 1: direct edges + method-name collection.
    let visit = |id: u32, out: &mut Reachability, queue: &mut VecDeque<u32>| {
        let Some(&pos) = by_id.get(&id) else { return };
        let f = &module.functions[pos];
        if let Some(name) = module.strings.get(f.name) {
            out.reachable_names.insert(name.to_string());
        }
        let Some(instrs) = f.instructions.as_ref() else {
            return;
        };
        for instr in instrs {
            match instr {
                Instruction::Call { func_id, .. }
                | Instruction::TailCall { func_id, .. }
                | Instruction::NewClosure { func_id, .. }
                | Instruction::CallG { func_id, .. }
                | Instruction::GenCreate { func_id, .. }
                | Instruction::Spawn { func_id, .. } => {
                    if let Some(rid) = resolve(*func_id) {
                        if out.reachable_ids.insert(rid) {
                            queue.push_back(rid);
                        }
                    }
                }
                Instruction::New { type_id, .. } | Instruction::NewG { type_id, .. } => {
                    if out.reachable_type_ids.insert(*type_id) {
                        // Implicit function edges of a live type:
                        // drop glue, clone glue, and protocol
                        // dispatch-table methods can all be invoked
                        // without a textual call site.
                        if let Some(td) = type_by_id.get(type_id) {
                            for f in td
                                .drop_fn
                                .iter()
                                .chain(td.clone_fn.iter())
                                .chain(td.protocols.iter().flat_map(|p| p.methods.iter()))
                            {
                                if out.reachable_ids.insert(*f) {
                                    queue.push_back(*f);
                                }
                            }
                        }
                    }
                }
                Instruction::CallM { method_id, .. } => {
                    if let Some(name) = module.strings.get(crate::types::StringId(*method_id)) {
                        // Dispatch tokens like `dyn:Proto.method` and
                        // qualified spellings both floor to the bare
                        // method segment.
                        let bare = name.rsplit(['.', ':']).next().unwrap_or(name);
                        out.called_method_names.insert(bare.to_string());
                    }
                }
                Instruction::FfiExtended { sub_op, operands }
                    if *sub_op == SystemSubOpcode::CreateCallback as u8 =>
                {
                    // Operand layout (see codegen remap arm): dst reg
                    // (1-2 byte varint-reg), fn_id: u32 LE, sig_idx: u32.
                    let mut cursor = 0usize;
                    if let Some(&b0) = operands.first() {
                        cursor += if b0 & 0x80 != 0 { 2 } else { 1 };
                    }
                    if operands.len() >= cursor + 4 {
                        let fn_id = u32::from_le_bytes([
                            operands[cursor],
                            operands[cursor + 1],
                            operands[cursor + 2],
                            operands[cursor + 3],
                        ]);
                        if let Some(rid) = resolve(fn_id) {
                            if out.reachable_ids.insert(rid) {
                                queue.push_back(rid);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    };

    while let Some(id) = queue.pop_front() {
        visit(id, &mut out, &mut queue);
    }

    // Pass 2: by-name candidate closure for CallM dispatch. Any
    // function whose bare name matches a called method name becomes
    // reachable (its body may introduce new direct edges and new
    // method names — iterate to a fixed point).
    // A by-name candidate is kept unless we can PROVE its receiver
    // type cannot exist: methods of RECORD types whose construction
    // (`New`/`NewG`) never appears in reachable code are excluded.
    // Non-record parents (variants materialise via `MakeVariant`
    // with no type operand; newtypes/aliases/primitives via
    // conversions) and well-known builtin containers (instances come
    // from dedicated instructions / runtime helpers) stay
    // conservatively included.
    let candidate_possible = |fid: u32, out: &Reachability| -> bool {
        let Some(&pos) = by_id.get(&fid) else {
            return true;
        };
        match module.functions[pos].parent_type {
            None => true,
            Some(tid) => {
                out.reachable_type_ids.contains(&tid.0)
                    || match type_by_id.get(&tid.0) {
                        Some(td) => {
                            td.kind != crate::types::TypeKind::Record
                                || module
                                    .strings
                                    .get(td.name)
                                    .map(verum_common::well_known_types::WellKnownType::is_well_known)
                                    .unwrap_or(true)
                        }
                        None => true,
                    }
            }
        }
    };
    loop {
        let mut newly: Vec<u32> = Vec::new();
        for (bare, ids) in &by_bare_name {
            if out.called_method_names.contains(*bare) {
                for id in ids {
                    if !out.reachable_ids.contains(id) && candidate_possible(*id, &out) {
                        newly.push(*id);
                    }
                }
            }
        }
        if newly.is_empty() {
            break;
        }
        for id in newly {
            if out.reachable_ids.insert(id) {
                queue.push_back(id);
            }
        }
        while let Some(id) = queue.pop_front() {
            visit(id, &mut out, &mut queue);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{FunctionDescriptor, FunctionId, VbcModule};
    use crate::types::StringId;

    fn module_with(funcs: Vec<(&str, Vec<Instruction>)>) -> VbcModule {
        let mut m = VbcModule::new("reach_test".to_string());
        for (name, instrs) in funcs {
            let sid = m.intern_string(name);
            let id = FunctionId(m.functions.len() as u32);
            let mut d = FunctionDescriptor::new(sid);
            d.id = id;
            d.instructions = Some(instrs);
            m.functions.push(d);
        }
        m
    }

    fn call(target: u32) -> Instruction {
        Instruction::Call {
            dst: crate::instruction::Reg(0),
            func_id: target,
            args: crate::instruction::RegRange {
                start: crate::instruction::Reg(0),
                count: 0,
            },
        }
    }

    #[test]
    fn direct_call_chain_reachable_dead_excluded() {
        let m = module_with(vec![
            ("main", vec![call(1)]),
            ("helper", vec![call(2)]),
            ("leaf", vec![]),
            ("dead", vec![call(2)]),
        ]);
        let r = analyze(&m);
        assert!(r.reachable_ids.contains(&0));
        assert!(r.reachable_ids.contains(&1));
        assert!(r.reachable_ids.contains(&2));
        assert!(
            !r.reachable_ids.contains(&3),
            "dead fn must not be reachable"
        );
    }

    #[test]
    fn callm_by_name_closure_keeps_all_candidates() {
        let mut m = module_with(vec![
            ("main", vec![]),
            ("TypeA.tick", vec![]),
            ("TypeB.tick", vec![call(3)]),
            ("tick_leaf", vec![]),
            ("dead", vec![]),
        ]);
        let mid = m.intern_string("tick");
        if let Some(instrs) = m.functions[0].instructions.as_mut() {
            instrs.push(Instruction::CallM {
                dst: crate::instruction::Reg(0),
                receiver: crate::instruction::Reg(0),
                method_id: mid.0,
                args: crate::instruction::RegRange {
                    start: crate::instruction::Reg(0),
                    count: 0,
                },
            });
        }
        let r = analyze(&m);
        assert!(r.reachable_ids.contains(&1), "candidate A kept");
        assert!(r.reachable_ids.contains(&2), "candidate B kept");
        assert!(
            r.reachable_ids.contains(&3),
            "candidate body edges followed"
        );
        assert!(!r.reachable_ids.contains(&4), "dead stays dead");
    }

    #[test]
    fn record_method_candidates_gated_on_type_construction() {
        use crate::types::{TypeDescriptor, TypeId, TypeKind};
        // main CallM's "tick"; Gadget.tick is a RECORD-parented method.
        // Without a New{Gadget} in reachable code the candidate is
        // excluded; adding the New pulls it in.
        let build = |construct: bool| {
            let mut m = module_with(vec![("main", vec![]), ("Gadget.tick", vec![])]);
            let tname = m.intern_string("Gadget");
            let mut td = TypeDescriptor::default();
            td.id = TypeId(7);
            td.name = tname;
            td.kind = TypeKind::Record;
            m.types.push(td);
            m.functions[1].parent_type = Some(TypeId(7));
            let mid = m.intern_string("tick");
            let instrs = m.functions[0].instructions.as_mut().unwrap();
            if construct {
                instrs.push(Instruction::New {
                    dst: crate::instruction::Reg(0),
                    type_id: 7,
                    field_count: 0,
                });
            }
            instrs.push(Instruction::CallM {
                dst: crate::instruction::Reg(0),
                receiver: crate::instruction::Reg(0),
                method_id: mid.0,
                args: crate::instruction::RegRange {
                    start: crate::instruction::Reg(0),
                    count: 0,
                },
            });
            m
        };
        let without = analyze(&build(false));
        assert!(
            !without.reachable_ids.contains(&1),
            "record method excluded while its type is never constructed"
        );
        let with = analyze(&build(true));
        assert!(
            with.reachable_ids.contains(&1),
            "record method included once New{{Gadget}} is reachable"
        );
    }

    #[test]
    fn reachable_type_pulls_drop_glue() {
        use crate::types::{TypeDescriptor, TypeId, TypeKind};
        let mut m = module_with(vec![
            (
                "main",
                vec![Instruction::New {
                    dst: crate::instruction::Reg(0),
                    type_id: 3,
                    field_count: 0,
                }],
            ),
            ("Gadget.drop", vec![]),
        ]);
        let tname = m.intern_string("Gadget");
        let mut td = TypeDescriptor::default();
        td.id = TypeId(3);
        td.name = tname;
        td.kind = TypeKind::Record;
        td.drop_fn = Some(1);
        m.types.push(td);
        let r = analyze(&m);
        assert!(
            r.reachable_ids.contains(&1),
            "drop glue of a constructed type is an implicit edge"
        );
        assert!(r.reachable_type_ids.contains(&3));
    }

    #[test]
    fn constant_function_refs_are_roots() {
        let mut m = module_with(vec![("main", vec![]), ("via_ref", vec![])]);
        m.constants.push(Constant::Function(FunctionId(1)));
        let r = analyze(&m);
        assert!(r.reachable_ids.contains(&1));
    }

    #[test]
    fn band_id_resolves_via_ranked_suffix() {
        // Recorded name is the SHORT spelling; the table registers the
        // fully-qualified key — the ranked suffix chase must connect
        // them (the `Mutex.new` class from the T0103 measurement).
        let mut m = module_with(vec![
            ("main", vec![call(crate::module::XMOD_CALL_ID_BAND_BASE)]),
            ("core.sync.mutex.Mutex.new", vec![]),
        ]);
        let sid: StringId = m.intern_string("Mutex.new");
        m.external_function_names
            .push((FunctionId(crate::module::XMOD_CALL_ID_BAND_BASE), sid));
        let r = analyze(&m);
        assert!(
            r.reachable_ids.contains(&1),
            "short-spelled band name chases to the qualified body"
        );
    }

    #[test]
    fn band_id_resolves_by_external_name() {
        let mut m = module_with(vec![
            ("main", vec![call(crate::module::XMOD_CALL_ID_BAND_BASE)]),
            ("core.time.Instant.now", vec![]),
        ]);
        let sid: StringId = m.intern_string("core.time.Instant.now");
        m.external_function_names
            .push((FunctionId(crate::module::XMOD_CALL_ID_BAND_BASE), sid));
        let r = analyze(&m);
        assert!(
            r.reachable_ids.contains(&1),
            "band id chases to the named body"
        );
    }
}
