//! VBC module disassembler.
//!
//! Produces human-readable text representation of VBC bytecode modules.
//! Used by `--emit-vbc` to dump compiled bytecode for debugging.

use std::fmt::Write;

use crate::instruction::{Instruction, Reg};
use crate::module::{Constant, VbcModule};
use crate::types::{CbgrTier, Mutability, TypeId, TypeRef};

/// Disassemble a VBC module to human-readable text.
pub fn disassemble_module(module: &VbcModule) -> String {
    let mut out = String::with_capacity(4096);
    write_module(&mut out, module).expect("fmt::Write to String never fails");
    out
}

fn write_module(out: &mut String, module: &VbcModule) -> std::fmt::Result {
    writeln!(out, "; VBC Module: {}", module.name)?;
    writeln!(
        out,
        "; Functions: {}, Constants: {}, Types: {}, Strings: {}",
        module.functions.len(),
        module.constants.len(),
        module.types.len(),
        module.strings.len(),
    )?;
    writeln!(out)?;

    // Constants
    if !module.constants.is_empty() {
        writeln!(out, "; === Constants ===")?;
        for (i, c) in module.constants.iter().enumerate() {
            write!(out, ";   #{}: ", i)?;
            write_constant(out, module, c)?;
            writeln!(out)?;
        }
        writeln!(out)?;
    }

    // Types
    if !module.types.is_empty() {
        writeln!(out, "; === Types ===")?;
        for (i, td) in module.types.iter().enumerate() {
            let tid = i as u32 + TypeId::FIRST_USER;
            let name = module.get_string(td.name).unwrap_or("?");
            writeln!(out, ";   T{}: {} ({:?})", tid, name, td.kind)?;
        }
        writeln!(out)?;
    }

    // Functions
    for (i, func) in module.functions.iter().enumerate() {
        let name = module.get_string(func.name).unwrap_or("?");
        let ret = format_type_ref(module, &func.return_type);

        write!(out, "; fn {}(", name)?;
        for (j, p) in func.params.iter().enumerate() {
            if j > 0 {
                write!(out, ", ")?;
            }
            let pname = module.get_string(p.name).unwrap_or("_");
            let pty = format_type_ref(module, &p.type_ref);
            write!(out, "{}: {}", pname, pty)?;
        }
        writeln!(out, ") -> {}  [id={}, regs={}, locals={}]",
            ret, i, func.register_count, func.locals_count)?;

        if let Some(instructions) = &func.instructions {
            for (j, instr) in instructions.iter().enumerate() {
                write!(out, "  {:04}  ", j)?;
                write_instruction(out, module, instr)?;
                writeln!(out)?;
            }
        } else {
            writeln!(out, "  ; (no decoded instructions)")?;
        }
        writeln!(out)?;
    }

    Ok(())
}

fn write_constant(out: &mut String, module: &VbcModule, c: &Constant) -> std::fmt::Result {
    match c {
        Constant::Int(v) => write!(out, "Int({})", v),
        Constant::Float(v) => write!(out, "Float({:?})", v),
        Constant::String(sid) => {
            let s = module.get_string(*sid).unwrap_or("?");
            if s.len() > 60 {
                // UTF-8-safe character truncation via verum_common.
                let preview = verum_common::text_utf8::truncate_chars(s, 57);
                write!(out, "String(\"{}...\")", preview)
            } else {
                write!(out, "String({:?})", s)
            }
        }
        Constant::Type(tr) => write!(out, "Type({})", format_type_ref(module, tr)),
        Constant::Function(fid) => {
            let name = module
                .get_function(*fid)
                .and_then(|f| module.get_string(f.name))
                .unwrap_or("?");
            write!(out, "Function({}={})", fid.0, name)
        }
        Constant::Protocol(pid) => write!(out, "Protocol({})", pid.0),
        Constant::Array(elems) => write!(out, "Array([{}])", elems.len()),
        Constant::Bytes(b) => write!(out, "Bytes({} bytes)", b.len()),
    }
}

fn format_type_ref(module: &VbcModule, tr: &TypeRef) -> String {
    match tr {
        TypeRef::Concrete(tid) => format_type_id(module, *tid),
        TypeRef::Generic(p) => format!("T{}", p.0),
        TypeRef::Instantiated { base, args } => {
            let base_name = format_type_id(module, *base);
            let args_str: Vec<String> = args.iter().map(|a| format_type_ref(module, a)).collect();
            format!("{}<{}>", base_name, args_str.join(", "))
        }
        TypeRef::Function { params, return_type, .. } => {
            let params_str: Vec<String> = params.iter().map(|p| format_type_ref(module, p)).collect();
            format!("fn({}) -> {}", params_str.join(", "), format_type_ref(module, return_type))
        }
        TypeRef::Rank2Function { type_param_count, params, return_type, .. } => {
            let params_str: Vec<String> = params.iter().map(|p| format_type_ref(module, p)).collect();
            format!("fn<{}>({}) -> {}", type_param_count, params_str.join(", "), format_type_ref(module, return_type))
        }
        TypeRef::Reference { inner, mutability, tier } => {
            let m = match mutability {
                Mutability::Immutable => "&",
                Mutability::Mutable => "&mut ",
            };
            let t = match tier {
                CbgrTier::Tier0 => "",
                CbgrTier::Tier1 => "checked ",
                CbgrTier::Tier2 => "unsafe ",
            };
            format!("{}{}{}", m, t, format_type_ref(module, inner))
        }
        TypeRef::Tuple(elems) => {
            let elems_str: Vec<String> = elems.iter().map(|e| format_type_ref(module, e)).collect();
            format!("({})", elems_str.join(", "))
        }
        TypeRef::Array { element, length } => {
            format!("[{}; {}]", format_type_ref(module, element), length)
        }
        TypeRef::Slice(inner) => {
            format!("[{}]", format_type_ref(module, inner))
        }
    }
}

fn format_type_id(module: &VbcModule, tid: TypeId) -> String {
    match tid {
        TypeId::UNIT => "()".to_string(),
        TypeId::BOOL => "Bool".to_string(),
        TypeId::INT => "Int".to_string(),
        TypeId::FLOAT => "Float".to_string(),
        TypeId::TEXT => "Text".to_string(),
        TypeId::NEVER => "Never".to_string(),
        TypeId::U8 => "U8".to_string(),
        TypeId::U16 => "U16".to_string(),
        TypeId::U32 => "U32".to_string(),
        TypeId::U64 => "U64".to_string(),
        TypeId::I8 => "I8".to_string(),
        TypeId::I16 => "I16".to_string(),
        TypeId::I32 => "I32".to_string(),
        TypeId::F32 => "F32".to_string(),
        TypeId::PTR => "Ptr".to_string(),
        TypeId::LIST => "List".to_string(),
        TypeId::MAP => "Map".to_string(),
        TypeId::SET => "Set".to_string(),
        TypeId::MAYBE => "Maybe".to_string(),
        TypeId::RESULT => "Result".to_string(),
        TypeId::RANGE => "Range".to_string(),
        TypeId::ARRAY => "Array".to_string(),
        TypeId::HEAP => "Heap".to_string(),
        TypeId::SHARED => "Shared".to_string(),
        TypeId::TUPLE => "Tuple".to_string(),
        TypeId::DEQUE => "Deque".to_string(),
        TypeId::CHANNEL => "Channel".to_string(),
        _ => {
            // User-defined type — look up name
            module
                .get_type_name(tid)
                .unwrap_or_else(|| format!("T{}", tid.0))
        }
    }
}

fn r(reg: &Reg) -> String {
    format!("r{}", reg.0)
}

fn func_name(module: &VbcModule, func_id: u32) -> String {
    module
        .get_function(crate::module::FunctionId(func_id))
        .and_then(|f| module.get_string(f.name))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("func#{}", func_id))
}

fn str_name(module: &VbcModule, str_id: u32) -> String {
    module
        .get_string(crate::types::StringId(str_id))
        .map(|s| {
            if s.len() > 40 {
                let preview = verum_common::text_utf8::truncate_chars(s, 37);
                format!("{:?}...", preview)
            } else {
                format!("{:?}", s)
            }
        })
        .unwrap_or_else(|| format!("str#{}", str_id))
}

fn reg_range(args: &crate::instruction::RegRange) -> String {
    if args.count == 0 {
        "[]".to_string()
    } else if args.count == 1 {
        format!("[r{}]", args.start.0)
    } else {
        format!("[r{}..r{}]", args.start.0, args.start.0 + args.count as u16 - 1)
    }
}

#[allow(clippy::too_many_lines)]
fn write_instruction(out: &mut String, module: &VbcModule, instr: &Instruction) -> std::fmt::Result {
    use Instruction::*;
    match instr {
        // ── Data Movement ──
        Mov { dst, src } => write!(out, "MOV       {}, {}", r(dst), r(src)),
        LoadK { dst, const_id } => {
            let cval = module
                .get_constant(crate::module::ConstId(*const_id))
                .map(|c| {
                    let mut s = String::new();
                    write_constant(&mut s, module, c).ok();
                    s
                })
                .unwrap_or_else(|| format!("#{}", const_id));
            write!(out, "LOAD_K    {}, {}", r(dst), cval)
        }
        LoadI { dst, value } => write!(out, "LOAD_I    {}, {}", r(dst), value),
        LoadF { dst, value } => write!(out, "LOAD_F    {}, {:?}", r(dst), value),
        LoadTrue { dst } => write!(out, "LOAD_TRUE {}", r(dst)),
        LoadFalse { dst } => write!(out, "LOAD_FALSE {}", r(dst)),
        LoadUnit { dst } => write!(out, "LOAD_UNIT {}", r(dst)),
        LoadT { dst, type_ref } => write!(out, "LOAD_T    {}, {}", r(dst), format_type_ref(module, type_ref)),
        LoadSmallI { dst, value } => write!(out, "LOAD_SI   {}, {}", r(dst), value),
        LoadNil { dst } => write!(out, "LOAD_NIL  {}", r(dst)),

        // ── Type Conversions ──
        CvtIF { dst, src } => write!(out, "CVT_IF    {}, {}", r(dst), r(src)),
        CvtFI { mode, dst, src } => write!(out, "CVT_FI    {}, {} ({:?})", r(dst), r(src), mode),
        CvtIC { dst, src } => write!(out, "CVT_IC    {}, {}", r(dst), r(src)),
        CvtCI { dst, src } => write!(out, "CVT_CI    {}, {}", r(dst), r(src)),
        CvtBI { dst, src } => write!(out, "CVT_BI    {}, {}", r(dst), r(src)),
        CvtToI { dst, src } => write!(out, "CVT_TO_I  {}, {}", r(dst), r(src)),
        CvtToF { dst, src } => write!(out, "CVT_TO_F  {}, {}", r(dst), r(src)),

        // ── Arithmetic ──
        BinaryI { op, dst, a, b } => write!(out, "{:<10}{}, {}, {}", format!("{:?}", op), r(dst), r(a), r(b)),
        BinaryF { op, dst, a, b } => write!(out, "{:<10}{}, {}, {}", format!("{:?}F", op), r(dst), r(a), r(b)),
        BinaryG { op, dst, a, b, protocol_id } =>
            write!(out, "{:<10}{}, {}, {} (proto={})", format!("{:?}G", op), r(dst), r(a), r(b), protocol_id),
        UnaryI { op, dst, src } => write!(out, "{:<10}{}, {}", format!("{:?}", op), r(dst), r(src)),
        UnaryF { op, dst, src } => write!(out, "{:<10}{}, {}", format!("{:?}F", op), r(dst), r(src)),
        Not { dst, src } => write!(out, "NOT       {}, {}", r(dst), r(src)),
        Bitwise { op, dst, a, b } => write!(out, "{:<10}{}, {}, {}", format!("{:?}", op), r(dst), r(a), r(b)),

        // ── Comparison ──
        CmpI { op, dst, a, b } => write!(out, "CMP_I     {}, {}, {} ({:?})", r(dst), r(a), r(b), op),
        CmpF { op, dst, a, b } => write!(out, "CMP_F     {}, {}, {} ({:?})", r(dst), r(a), r(b), op),
        CmpG { eq, dst, a, b, protocol_id } =>
            write!(out, "CMP_G     {}, {}, {} (eq={}, proto={})", r(dst), r(a), r(b), eq, protocol_id),
        CmpU { sub_op, dst, a, b } =>
            write!(out, "CMP_U     {}, {}, {} ({:?})", r(dst), r(a), r(b), sub_op),

        // ── Control Flow ──
        Nop => write!(out, "NOP"),
        Jmp { offset } => write!(out, "JMP       {:+}", offset),
        JmpIf { cond, offset } => write!(out, "JMP_IF    {}, {:+}", r(cond), offset),
        JmpNot { cond, offset } => write!(out, "JMP_NOT   {}, {:+}", r(cond), offset),
        JmpCmp { op, a, b, offset } =>
            write!(out, "JMP_CMP   {}, {}, {:+} ({:?})", r(a), r(b), offset, op),
        Ret { value } => write!(out, "RET       {}", r(value)),
        RetV => write!(out, "RET_V"),
        Call { dst, func_id, args } =>
            write!(out, "CALL      {}, {} {}", r(dst), func_name(module, *func_id), reg_range(args)),
        TailCall { func_id, args } =>
            write!(out, "TAIL_CALL {} {}", func_name(module, *func_id), reg_range(args)),
        CallM { dst, receiver, method_id, args } =>
            write!(out, "CALL_M    {}, {}.{} {}", r(dst), r(receiver), func_name(module, *method_id), reg_range(args)),
        CallClosure { dst, closure, args } =>
            write!(out, "CALL_CLS  {}, {} {}", r(dst), r(closure), reg_range(args)),
        CallR { dst, func, argc } =>
            write!(out, "CALL_R    {}, {} (argc={})", r(dst), r(func), argc),
        NewClosure { dst, func_id, captures } => {
            let caps: Vec<String> = captures.iter().map(r).collect();
            write!(out, "NEW_CLS   {}, {} [{}]", r(dst), func_name(module, *func_id), caps.join(", "))
        }

        // ── Memory / Collections ──
        New { dst, type_id, field_count } => {
            let tname = format_type_id(module, TypeId(*type_id));
            write!(out, "NEW       {}, {} (fields={})", r(dst), tname, field_count)
        }
        NewG { dst, type_id, type_args } => {
            let tname = format_type_id(module, TypeId(*type_id));
            let targs: Vec<String> = type_args.iter().map(r).collect();
            write!(out, "NEW_G     {}, {}<{}> ", r(dst), tname, targs.join(", "))
        }
        GetF { dst, obj, field_idx } => write!(out, "GET_F     {}, {}.{}", r(dst), r(obj), field_idx),
        SetF { obj, field_idx, value } => write!(out, "SET_F     {}.{}, {}", r(obj), field_idx, r(value)),
        GetE { dst, arr, idx } => write!(out, "GET_E     {}, {}[{}]", r(dst), r(arr), r(idx)),
        SetE { arr, idx, value } => write!(out, "SET_E     {}[{}], {}", r(arr), r(idx), r(value)),
        Len { dst, arr, type_hint: _ } => write!(out, "LEN       {}, {}", r(dst), r(arr)),
        NewList { dst } => write!(out, "NEW_LIST  {}", r(dst)),
        ListPush { list, val } => write!(out, "LIST_PUSH {}, {}", r(list), r(val)),
        ListPop { dst, list } => write!(out, "LIST_POP  {}, {}", r(dst), r(list)),
        NewMap { dst } => write!(out, "NEW_MAP   {}", r(dst)),
        MapGet { dst, map, key } => write!(out, "MAP_GET   {}, {}[{}]", r(dst), r(map), r(key)),
        MapSet { map, key, val } => write!(out, "MAP_SET   {}[{}], {}", r(map), r(key), r(val)),
        MapContains { dst, map, key } => write!(out, "MAP_HAS   {}, {}[{}]", r(dst), r(map), r(key)),
        Clone { dst, src } => write!(out, "CLONE     {}, {}", r(dst), r(src)),
        MakeList { dst, len } => write!(out, "MAKE_LIST {}, len={}", r(dst), len),
        MakeMap { dst, capacity } => write!(out, "MAKE_MAP  {}, cap={}", r(dst), capacity),
        MakeSet { dst, capacity } => write!(out, "MAKE_SET  {}, cap={}", r(dst), capacity),
        MapInsert { map, key, value } => write!(out, "MAP_INS   {}[{}], {}", r(map), r(key), r(value)),
        NewSet { dst } => write!(out, "NEW_SET   {}", r(dst)),
        SetInsert { set, elem } => write!(out, "SET_INS   {}, {}", r(set), r(elem)),
        SetContains { dst, set, elem } => write!(out, "SET_HAS   {}, {}, {}", r(dst), r(set), r(elem)),
        SetRemove { set, elem } => write!(out, "SET_REM   {}, {}", r(set), r(elem)),
        NewRange { dst, start, end, inclusive } =>
            write!(out, "NEW_RANGE {}, {}..{}{}", r(dst), r(start), if *inclusive { "=" } else { "" }, r(end)),

        // ── Iterators ──
        IterNew { dst, iterable } => write!(out, "ITER_NEW  {}, {}", r(dst), r(iterable)),
        IterNext { dst, has_next, iter } => write!(out, "ITER_NEXT {}, {}, {}", r(dst), r(has_next), r(iter)),
        Iter { dst, iterable } => write!(out, "ITER      {}, {}", r(dst), r(iterable)),

        // ── CBGR ──
        Ref { dst, src } => write!(out, "REF       {}, {}", r(dst), r(src)),
        RefMut { dst, src } => write!(out, "REF_MUT   {}, {}", r(dst), r(src)),
        Deref { dst, ref_reg } => write!(out, "DEREF     {}, {}", r(dst), r(ref_reg)),
        DerefMut { ref_reg, value } => write!(out, "DEREF_MUT {}, {}", r(ref_reg), r(value)),
        ChkRef { ref_reg } => write!(out, "CHK_REF   {}", r(ref_reg)),
        RefChecked { dst, src } => write!(out, "REF_CHK   {}, {}", r(dst), r(src)),
        RefUnsafe { dst, src } => write!(out, "REF_UNSAFE {}, {}", r(dst), r(src)),
        DropRef { src } => write!(out, "DROP_REF  {}", r(src)),

        // ── Pattern Matching + Variants ──
        IsVar { dst, value, tag } => write!(out, "IS_VAR    {}, {}, tag={}", r(dst), r(value), tag),
        AsVar { dst, value, tag } => write!(out, "AS_VAR    {}, {}, tag={}", r(dst), r(value), tag),
        Unpack { dst_start, tuple, count } =>
            write!(out, "UNPACK    r{}..r{}, {} (count={})", dst_start.0, dst_start.0 + *count as u16 - 1, r(tuple), count),
        Pack { dst, src_start, count } =>
            write!(out, "PACK      {}, r{}..r{} (count={})", r(dst), src_start.0, src_start.0 + *count as u16 - 1, count),
        Switch { value, default_offset, cases } => {
            write!(out, "SWITCH    {}, default={:+}, {} cases", r(value), default_offset, cases.len())?;
            for (cv, off) in cases.iter().take(5) {
                write!(out, " [{}→{:+}]", cv, off)?;
            }
            if cases.len() > 5 {
                write!(out, " ...")?;
            }
            Ok(())
        }
        GetTag { dst, variant } => write!(out, "GET_TAG   {}, {}", r(dst), r(variant)),
        MakeVariant { dst, tag, field_count } =>
            write!(out, "MK_VAR    {}, tag={}, fields={}", r(dst), tag, field_count),
        SetVariantData { variant, field, value } =>
            write!(out, "SET_VDATA {}.{}, {}", r(variant), field, r(value)),
        GetVariantData { dst, variant, field } =>
            write!(out, "GET_VDATA {}, {}.{}", r(dst), r(variant), field),
        GetVariantDataRef { dst, variant, field } =>
            write!(out, "GET_VDREF {}, {}.{}", r(dst), r(variant), field),

        // ── String ──
        ToString { dst, src } => write!(out, "TO_STR    {}, {}", r(dst), r(src)),
        Concat { dst, a, b } => write!(out, "CONCAT    {}, {}, {}", r(dst), r(a), r(b)),
        CharToStr { dst, src } => write!(out, "CHR2STR   {}, {}", r(dst), r(src)),

        // ── Generator ──
        GenCreate { dst, func_id, args } =>
            write!(out, "GEN_NEW   {}, {} {}", r(dst), func_name(module, *func_id), reg_range(args)),
        GenNext { dst, generator } => write!(out, "GEN_NEXT  {}, {}", r(dst), r(generator)),
        GenHasNext { dst, generator } => write!(out, "GEN_HAS   {}, {}", r(dst), r(generator)),

        // ── Async ──
        Spawn { dst, func_id, args } =>
            write!(out, "SPAWN     {}, {} {}", r(dst), func_name(module, *func_id), reg_range(args)),
        Await { dst, task } => write!(out, "AWAIT     {}, {}", r(dst), r(task)),
        Yield { value } => write!(out, "YIELD     {}", r(value)),
        Select { dst, futures, handlers } => {
            let futs: Vec<String> = futures.iter().map(r).collect();
            write!(out, "SELECT    {}, [{}], {} handlers", r(dst), futs.join(", "), handlers.len())
        }
        FutureReady { dst, future } => write!(out, "FUT_READY {}, {}", r(dst), r(future)),
        FutureGet { dst, future } => write!(out, "FUT_GET   {}, {}", r(dst), r(future)),
        AsyncNext { dst, iter } => write!(out, "ASYNC_NXT {}, {}", r(dst), r(iter)),

        // ── Nursery ──
        NurseryInit { dst } => write!(out, "NURS_INIT {}", r(dst)),
        NurserySpawn { dst, nursery, task } =>
            write!(out, "NURS_SPAWN {}, {}, {}", r(dst), r(nursery), r(task)),
        NurseryAwaitAll { nursery, success } =>
            write!(out, "NURS_WAIT {}, {}", r(nursery), r(success)),
        NurseryCancel { nursery } => write!(out, "NURS_CNCL {}", r(nursery)),

        // ── Context ──
        CtxGet { dst, ctx_type } => write!(out, "CTX_GET   {}, ctx={}", r(dst), ctx_type),
        CtxProvide { ctx_type, value, body_offset } =>
            write!(out, "CTX_PROV  ctx={}, {}, body={:+}", ctx_type, r(value), body_offset),
        CtxEnd => write!(out, "CTX_END"),
        CtxCheckNegative { ctx_type, func_name } =>
            write!(out, "CTX_CHK_NEG ctx={}, func={}", ctx_type, func_name),
        PushContext { name, handler } => write!(out, "PUSH_CTX  name={}, {}", name, r(handler)),
        PopContext { name } => write!(out, "POP_CTX   name={}", name),
        Attenuate { dst, context, capabilities } =>
            write!(out, "ATTENUATE {}, {}, caps={}", r(dst), r(context), capabilities),

        // ── Debug / Verify ──
        Spec { reg, expected_type } => {
            let tname = format_type_id(module, TypeId(*expected_type));
            write!(out, "SPEC      {}, {}", r(reg), tname)
        }
        Guard { reg, expected_type, deopt_offset } => {
            let tname = format_type_id(module, TypeId(*expected_type));
            write!(out, "GUARD     {}, {}, deopt={:+}", r(reg), tname, deopt_offset)
        }
        Assert { cond, message_id } =>
            write!(out, "ASSERT    {}, {}", r(cond), str_name(module, *message_id)),
        Panic { message_id } => write!(out, "PANIC     {}", str_name(module, *message_id)),
        Unreachable => write!(out, "UNREACHABLE"),
        DebugPrint { value } => write!(out, "DBG_PRINT {}", r(value)),
        // ── Exception Handling ──
        Throw { error } => write!(out, "THROW     {}", r(error)),
        TryBegin { handler_offset } => write!(out, "TRY_BEGIN handler={:+}", handler_offset),
        TryEnd => write!(out, "TRY_END"),
        GetException { dst } => write!(out, "GET_EXC   {}", r(dst)),

        // ── Stack ──
        Push { src } => write!(out, "PUSH      {}", r(src)),
        Pop { dst } => write!(out, "POP       {}", r(dst)),

        // ── Generic Calls ──
        CallG { dst, func_id, type_args, args } => {
            let targs: Vec<String> = type_args.iter().map(r).collect();
            write!(out, "CALL_G    {}, {}<{}> {}", r(dst), func_name(module, *func_id), targs.join(", "), reg_range(args))
        }
        CallV { dst, receiver, vtable_slot, args } =>
            write!(out, "CALL_V    {}, {}.vt[{}] {}", r(dst), r(receiver), vtable_slot, reg_range(args)),
        CallC { dst, cache_id, args } =>
            write!(out, "CALL_C    {}, cache={} {}", r(dst), cache_id, reg_range(args)),
        // ── Autodiff ──
        GradBegin { scope_id, mode, wrt } => {
            let wregs: Vec<String> = wrt.iter().map(r).collect();
            write!(out, "GRAD_BEGIN scope={}, {:?}, wrt=[{}]", scope_id, mode, wregs.join(", "))
        }
        GradEnd { scope_id, output, grad_out, grad_regs } => {
            let gregs: Vec<String> = grad_regs.iter().map(r).collect();
            write!(out, "GRAD_END  scope={}, out={}, grad_out={}, grads=[{}]",
                scope_id, r(output), r(grad_out), gregs.join(", "))
        }
        GradCheckpoint { id, tensors } => {
            let tregs: Vec<String> = tensors.iter().map(r).collect();
            write!(out, "GRAD_CKPT id={}, [{}]", id, tregs.join(", "))
        }
        GradAccumulate { dst, src } => write!(out, "GRAD_ACC  {}, {}", r(dst), r(src)),
        GradStop { dst, src } => write!(out, "GRAD_STOP {}, {}", r(dst), r(src)),

        // ── Meta ──
        MetaQuote { dst, bytes_const_id } =>
            write!(out, "META_QUOTE {}, const={}", r(dst), bytes_const_id),
        MetaSplice { src } => write!(out, "META_SPLICE {}", r(src)),
        MetaEval { dst, expr } => write!(out, "META_EVAL {}, {}", r(dst), r(expr)),
        MetaReflect { dst, type_id } => {
            let tname = format_type_id(module, TypeId(*type_id));
            write!(out, "META_REFL {}, {}", r(dst), tname)
        }

        // ── Tensor (common) ──
        TensorNew { dst, dtype, dims } => {
            let dregs: Vec<String> = dims.iter().map(r).collect();
            write!(out, "T_NEW     {}, {:?}, dims=[{}]", r(dst), dtype, dregs.join(", "))
        }
        TensorBinop { op, dst, a, b } =>
            write!(out, "T_BINOP   {}, {}, {} ({:?})", r(dst), r(a), r(b), op),
        TensorUnop { op, dst, src } =>
            write!(out, "T_UNOP    {}, {} ({:?})", r(dst), r(src), op),
        TensorMatmul { dst, a, b } =>
            write!(out, "T_MATMUL  {}, {}, {}", r(dst), r(a), r(b)),
        TensorReduce { op, dst, src, axes, keepdim } =>
            write!(out, "T_REDUCE  {}, {} ({:?}, axes={:?}, keep={})", r(dst), r(src), op, axes, keepdim),
        TensorReshape { dst, src, shape } => {
            let sregs: Vec<String> = shape.iter().map(r).collect();
            write!(out, "T_RESHAPE {}, {}, [{}]", r(dst), r(src), sregs.join(", "))
        }
        TensorTranspose { dst, src, perm } =>
            write!(out, "T_TRANSP  {}, {}, perm={:?}", r(dst), r(src), perm),
        TensorSlice { dst, src, starts, ends } => {
            let s: Vec<String> = starts.iter().map(r).collect();
            let e: Vec<String> = ends.iter().map(r).collect();
            write!(out, "T_SLICE   {}, {}, [{}]..[{}]", r(dst), r(src), s.join(","), e.join(","))
        }
        TensorClone { dst, src } => write!(out, "T_CLONE   {}, {}", r(dst), r(src)),

        // ── GPU (common) ──
        GpuSync { stream } => write!(out, "GPU_SYNC  {}", r(stream)),
        GpuMemcpy { dst, src, direction } =>
            write!(out, "GPU_CPY   {}, {}, dir={}", r(dst), r(src), direction),
        GpuAlloc { dst, size, device } =>
            write!(out, "GPU_ALLOC {}, size={}, dev={}", r(dst), r(size), r(device)),
        GpuDeviceSync => write!(out, "GPU_DEV_SYNC"),

        // ── Fallback for all remaining exotic instructions ──
        // GPU, tensor, system, extended opcodes — use Debug format
        other => {
            let dbg = format!("{:?}", other);
            // Truncate very long debug strings
            if dbg.len() > 120 {
                write!(out, "{:.120}...", dbg)
            } else {
                write!(out, "{}", dbg)
            }
        }
    }
}
