//! #48 — THE single authority for the canonical Tier-1 slice cell and
//! every runtime container-shape classification that consumes it.
//!
//! Canonical cell: 24 bytes `{data@0, len@8, elem@16}`; `data`/`len`
//! offsets are back-compatible with the legacy 16-byte fat ref; `elem`
//! is SELF-NORMALISING over the domain {1, 2, 4, 8} (anything else —
//! legacy heap garbage at offset 16, a zeroed cell — degrades to the
//! historic Value-wide stride 8, so a not-yet-migrated producer can
//! never regress a consumer below legacy behaviour).
//!
//! Runtime container classification (`view`) discriminates the three
//! shapes every slice-typed register can hold:
//!   * cell — word0 is a heap DATA pointer (>= platform heap floor);
//!   * stamped Pack — header type_id TUPLE(521) / BYTE_SLICE(528):
//!     `{data@24, len@32}`, byte elements;
//!   * unstamped List — word0 == 0 stamp: canonical layout
//!     `{len@24, cap@32, ptr@40}` (verum_common::layout), 8-byte
//!     Value elements.
//! REAL branches throughout — the shapes have different valid extents,
//! and a `select` executes BOTH arms' loads (the exact OOB class the
//! #48 campaign retired; see the phase-1 commit dae237627).
//!
//! `probe()` discriminates cell-vs-not on the heap-floor compare ALONE
//! — deliberately NOT also on 8-byte alignment of word0 (dropped post
//! T0129/#56 SUBSLICE-AOT-LEN0). For the cell shape, word0 IS the
//! `data` field: a byte-precise pointer into the referent buffer.
//! `SliceSubslice`/`SplitAt` compute it as `source_data + start*elem`
//! (phase-1.6, 4341ed7d0); for a byte-stride source (`elem == 1`) a
//! non-multiple-of-8 `start` legitimately produces a misaligned
//! pointer. Requiring alignment made such a (real, correctly-formed)
//! cell fail the probe on every later re-classification — e.g.
//! `sub.len()` — falling through to the Pack/List arms and reading
//! `len` from the wrong slot (always garbage, observed as 0). Dropping
//! alignment is sound: `lower_pack_typed` zeroes the full 24-byte
//! header before stamping ONLY a small TypeId constant into the low 4
//! bytes (521/528, upper 32 bits left 0), and `lower_new_list[_with_capacity]`
//! memsets the whole header to 0 — word0 for both shapes is always far
//! below ANY platform floor regardless of alignment, so the floor
//! compare alone already carries the full disambiguating weight.
//!
//! The functions here are RAW (LLVM `Context` + `Builder`): usable both
//! from `instruction.rs` (which wraps them for `FunctionContext`
//! call sites) and from `runtime.rs` (`RuntimeLowering`, which has no
//! FunctionContext). One implementation, two consumers, zero drift.

use super::error::{BuildExt, OptionExt, Result};
use verum_llvm::IntPredicate;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::values::{IntValue, PointerValue};

/// Byte size of the canonical cell.
pub const CELL_SIZE: u64 = 24;
/// Field offsets inside the canonical cell.
pub const CELL_DATA_OFF: u64 = 0;
pub const CELL_LEN_OFF: u64 = 8;
pub const CELL_ELEM_OFF: u64 = 16;

/// Stamped-Pack layout (legacy byte views).
pub const PACK_DATA_OFF: u64 = 24;
pub const PACK_LEN_OFF: u64 = 32;

/// Environment for raw emission: the LLVM context plus the
/// target-derived heap floor (see `target_triple::heap_floor` — darwin
/// user heaps live above 4 GiB; elsewhere the conservative page floor).
pub struct CellEnv<'ctx> {
    pub llvm: &'ctx Context,
    pub heap_floor: u64,
}

/// One classified view of a slice-shaped value: `(data, len, elem)` as
/// i64 phi values on the current insert block after the call.
pub struct ContainerView<'ctx> {
    pub data: IntValue<'ctx>,
    pub len: IntValue<'ctx>,
    pub elem: IntValue<'ctx>,
}

impl<'ctx> CellEnv<'ctx> {
    /// Pointer-plausibility probe: `(is_cell, word0)`.
    /// `word0 >= heap_floor` ⇒ canonical cell — see the module-level
    /// doc for why this is NOT also gated on 8-byte alignment of
    /// `word0` (T0129/#56: a byte-stride cell's `data` field is a
    /// byte-precise pointer, not guaranteed aligned).
    pub fn probe(
        &self,
        builder: &Builder<'ctx>,
        base_ptr: PointerValue<'ctx>,
        tag: &str,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i64_ty = self.llvm.i64_type();
        let word0 = builder
            .build_load(i64_ty, base_ptr, &format!("{}_w0", tag))
            .or_llvm_err()?
            .into_int_value();
        let is_cell = builder
            .build_int_compare(
                IntPredicate::UGE,
                word0,
                i64_ty.const_int(self.heap_floor, false),
                &format!("{}_is_cell", tag),
            )
            .or_llvm_err()?;
        Ok((is_cell, word0))
    }

    /// Self-normalising elem load from a cell base pointer (@16).
    pub fn elem_width(
        &self,
        builder: &Builder<'ctx>,
        base_ptr: PointerValue<'ctx>,
        tag: &str,
    ) -> Result<IntValue<'ctx>> {
        let i64_ty = self.llvm.i64_type();
        let i8_ty = self.llvm.i8_type();
        // SAFETY: fixed offset 16 inside the 24-byte canonical cell.
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(CELL_ELEM_OFF, false)],
                    &format!("{}_elem_slot", tag),
                )
                .or_llvm_err()?
        };
        let raw = builder
            .build_load(i64_ty, slot, &format!("{}_elem_raw", tag))
            .or_llvm_err()?
            .into_int_value();
        self.normalise_elem(builder, raw, tag)
    }

    /// Domain-normalise a stride value: {1,2,4,8} pass, else 8.
    pub fn normalise_elem(
        &self,
        builder: &Builder<'ctx>,
        raw: IntValue<'ctx>,
        tag: &str,
    ) -> Result<IntValue<'ctx>> {
        let i64_ty = self.llvm.i64_type();
        let mut dom = builder
            .build_int_compare(
                IntPredicate::EQ,
                raw,
                i64_ty.const_int(1, false),
                &format!("{}_e1", tag),
            )
            .or_llvm_err()?;
        for w in [2u64, 4, 8] {
            let c = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    raw,
                    i64_ty.const_int(w, false),
                    &format!("{}_ew", tag),
                )
                .or_llvm_err()?;
            dom = builder
                .build_or(dom, c, &format!("{}_edom", tag))
                .or_llvm_err()?;
        }
        Ok(builder
            .build_select(
                dom,
                raw,
                i64_ty.const_int(8, false),
                &format!("{}_elem", tag),
            )
            .or_llvm_err()?
            .into_int_value())
    }

    /// Element address: `data + index * elem` (byte GEP).
    pub fn elem_addr(
        &self,
        builder: &Builder<'ctx>,
        data: IntValue<'ctx>,
        elem: IntValue<'ctx>,
        index: IntValue<'ctx>,
        tag: &str,
    ) -> Result<(IntValue<'ctx>, PointerValue<'ctx>)> {
        let ptr_ty = self.llvm.ptr_type(verum_llvm::AddressSpace::default());
        let off = builder
            .build_int_mul(index, elem, &format!("{}_off", tag))
            .or_llvm_err()?;
        let addr = builder
            .build_int_add(data, off, &format!("{}_addr", tag))
            .or_llvm_err()?;
        let eptr = builder
            .build_int_to_ptr(addr, ptr_ty, &format!("{}_eptr", tag))
            .or_llvm_err()?;
        Ok((addr, eptr))
    }

    /// Width-switched element LOAD (1/2/4 zext, 8 direct) → i64 phi.
    pub fn elem_load(
        &self,
        builder: &Builder<'ctx>,
        elem: IntValue<'ctx>,
        eptr: PointerValue<'ctx>,
        tag: &str,
    ) -> Result<IntValue<'ctx>> {
        let i64_ty = self.llvm.i64_type();
        let cur = builder
            .get_insert_block()
            .or_internal("slice_cell::elem_load: no insert block")?;
        let func = cur
            .get_parent()
            .or_internal("slice_cell::elem_load: block has no parent")?;
        let w1 = self.llvm.append_basic_block(func, &format!("{}_lw1", tag));
        let w2 = self.llvm.append_basic_block(func, &format!("{}_lw2", tag));
        let w4 = self.llvm.append_basic_block(func, &format!("{}_lw4", tag));
        let w8 = self.llvm.append_basic_block(func, &format!("{}_lw8", tag));
        let merge = self.llvm.append_basic_block(func, &format!("{}_lmerge", tag));
        builder
            .build_switch(
                elem,
                w8,
                &[
                    (i64_ty.const_int(1, false), w1),
                    (i64_ty.const_int(2, false), w2),
                    (i64_ty.const_int(4, false), w4),
                ],
            )
            .or_llvm_err()?;
        let mut loads: Vec<(IntValue<'ctx>, _)> = Vec::new();
        for (bb, bits) in [(w1, 8u32), (w2, 16), (w4, 32)] {
            builder.position_at_end(bb);
            let ity = self.llvm.custom_width_int_type(bits);
            let v = builder
                .build_load(ity, eptr, &format!("{}_lv", tag))
                .or_llvm_err()?
                .into_int_value();
            let z = builder
                .build_int_z_extend(v, i64_ty, &format!("{}_lz", tag))
                .or_llvm_err()?;
            builder.build_unconditional_branch(merge).or_llvm_err()?;
            loads.push((z, builder.get_insert_block().unwrap()));
        }
        builder.position_at_end(w8);
        let v8 = builder
            .build_load(i64_ty, eptr, &format!("{}_lv8", tag))
            .or_llvm_err()?
            .into_int_value();
        builder.build_unconditional_branch(merge).or_llvm_err()?;
        loads.push((v8, builder.get_insert_block().unwrap()));
        builder.position_at_end(merge);
        let phi = builder
            .build_phi(i64_ty, &format!("{}_lval", tag))
            .or_llvm_err()?;
        for (v, bb) in &loads {
            phi.add_incoming(&[(&(*v), *bb)]);
        }
        Ok(phi.as_basic_value().into_int_value())
    }

    /// Width-switched element STORE (1/2/4 trunc, 8 direct).
    pub fn elem_store(
        &self,
        builder: &Builder<'ctx>,
        elem: IntValue<'ctx>,
        eptr: PointerValue<'ctx>,
        val_i64: IntValue<'ctx>,
        tag: &str,
    ) -> Result<()> {
        let i64_ty = self.llvm.i64_type();
        let cur = builder
            .get_insert_block()
            .or_internal("slice_cell::elem_store: no insert block")?;
        let func = cur
            .get_parent()
            .or_internal("slice_cell::elem_store: block has no parent")?;
        let w1 = self.llvm.append_basic_block(func, &format!("{}_sw1", tag));
        let w2 = self.llvm.append_basic_block(func, &format!("{}_sw2", tag));
        let w4 = self.llvm.append_basic_block(func, &format!("{}_sw4", tag));
        let w8 = self.llvm.append_basic_block(func, &format!("{}_sw8", tag));
        let merge = self.llvm.append_basic_block(func, &format!("{}_smerge", tag));
        builder
            .build_switch(
                elem,
                w8,
                &[
                    (i64_ty.const_int(1, false), w1),
                    (i64_ty.const_int(2, false), w2),
                    (i64_ty.const_int(4, false), w4),
                ],
            )
            .or_llvm_err()?;
        for (bb, bits) in [(w1, 8u32), (w2, 16), (w4, 32)] {
            builder.position_at_end(bb);
            let ity = self.llvm.custom_width_int_type(bits);
            let t = builder
                .build_int_truncate(val_i64, ity, &format!("{}_st", tag))
                .or_llvm_err()?;
            builder.build_store(eptr, t).or_llvm_err()?;
            builder.build_unconditional_branch(merge).or_llvm_err()?;
        }
        builder.position_at_end(w8);
        builder.build_store(eptr, val_i64).or_llvm_err()?;
        builder.build_unconditional_branch(merge).or_llvm_err()?;
        builder.position_at_end(merge);
        Ok(())
    }

    /// The 3-arm classifier: {cell | stamped Pack | unstamped List} →
    /// `(data, len, elem)` phis. `tuple_tid`/`byte_slice_tid` are the
    /// Pack stamps (TypeId::TUPLE / TypeId::BYTE_SLICE — passed in so
    /// this crate-local module needs no verum_vbc type dependency);
    /// `list_len_off`/`list_ptr_off` are the canonical List offsets
    /// (verum_common::layout::LIST_{LEN,PTR}_OFFSET).
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        builder: &Builder<'ctx>,
        base_ptr: PointerValue<'ctx>,
        tuple_tid: u64,
        byte_slice_tid: u64,
        list_len_off: u64,
        list_ptr_off: u64,
        tag: &str,
    ) -> Result<ContainerView<'ctx>> {
        let i64_ty = self.llvm.i64_type();
        let i8_ty = self.llvm.i8_type();
        let i32_ty = self.llvm.i32_type();
        let cur = builder
            .get_insert_block()
            .or_internal("slice_cell::view: no insert block")?;
        let func = cur
            .get_parent()
            .or_internal("slice_cell::view: block has no parent")?;
        let bb_cell = self.llvm.append_basic_block(func, &format!("{}_cv_cell", tag));
        let bb_obj = self.llvm.append_basic_block(func, &format!("{}_cv_obj", tag));
        let bb_pack = self.llvm.append_basic_block(func, &format!("{}_cv_pack", tag));
        let bb_list = self.llvm.append_basic_block(func, &format!("{}_cv_list", tag));
        let bb_done = self.llvm.append_basic_block(func, &format!("{}_cv_done", tag));

        let (is_cell, w0) = self.probe(builder, base_ptr, tag)?;
        builder
            .build_conditional_branch(is_cell, bb_cell, bb_obj)
            .or_llvm_err()?;

        builder.position_at_end(bb_cell);
        // SAFETY: fixed offsets inside the 24-byte cell.
        let cell_len_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(CELL_LEN_OFF, false)],
                    &format!("{}_cv_len_slot", tag),
                )
                .or_llvm_err()?
        };
        let cell_len = builder
            .build_load(i64_ty, cell_len_slot, &format!("{}_cv_cell_len", tag))
            .or_llvm_err()?
            .into_int_value();
        let cell_elem = self.elem_width(builder, base_ptr, &format!("{}_cv", tag))?;
        builder.build_unconditional_branch(bb_done).or_llvm_err()?;

        builder.position_at_end(bb_obj);
        let tid = builder
            .build_load(i32_ty, base_ptr, &format!("{}_cv_tid", tag))
            .or_llvm_err()?
            .into_int_value();
        let is_tuple = builder
            .build_int_compare(
                IntPredicate::EQ,
                tid,
                i32_ty.const_int(tuple_tid, false),
                &format!("{}_cv_is_tuple", tag),
            )
            .or_llvm_err()?;
        let is_bslice = builder
            .build_int_compare(
                IntPredicate::EQ,
                tid,
                i32_ty.const_int(byte_slice_tid, false),
                &format!("{}_cv_is_bslice", tag),
            )
            .or_llvm_err()?;
        let is_pack = builder
            .build_or(is_tuple, is_bslice, &format!("{}_cv_is_pack", tag))
            .or_llvm_err()?;
        builder
            .build_conditional_branch(is_pack, bb_pack, bb_list)
            .or_llvm_err()?;

        builder.position_at_end(bb_pack);
        // SAFETY: real stamped heap object ≥ 48 bytes.
        let pk_data_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(PACK_DATA_OFF, false)],
                    &format!("{}_cv_pk_data_slot", tag),
                )
                .or_llvm_err()?
        };
        let pk_data = builder
            .build_load(i64_ty, pk_data_slot, &format!("{}_cv_pk_data", tag))
            .or_llvm_err()?
            .into_int_value();
        let pk_len_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(PACK_LEN_OFF, false)],
                    &format!("{}_cv_pk_len_slot", tag),
                )
                .or_llvm_err()?
        };
        let pk_len = builder
            .build_load(i64_ty, pk_len_slot, &format!("{}_cv_pk_len", tag))
            .or_llvm_err()?
            .into_int_value();
        builder.build_unconditional_branch(bb_done).or_llvm_err()?;

        builder.position_at_end(bb_list);
        // SAFETY: real (unstamped) List object — canonical layout.
        let ls_data_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(list_ptr_off, false)],
                    &format!("{}_cv_ls_data_slot", tag),
                )
                .or_llvm_err()?
        };
        let ls_data = builder
            .build_load(i64_ty, ls_data_slot, &format!("{}_cv_ls_data", tag))
            .or_llvm_err()?
            .into_int_value();
        let ls_len_slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i8_ty,
                    base_ptr,
                    &[i64_ty.const_int(list_len_off, false)],
                    &format!("{}_cv_ls_len_slot", tag),
                )
                .or_llvm_err()?
        };
        let ls_len = builder
            .build_load(i64_ty, ls_len_slot, &format!("{}_cv_ls_len", tag))
            .or_llvm_err()?
            .into_int_value();
        builder.build_unconditional_branch(bb_done).or_llvm_err()?;

        builder.position_at_end(bb_done);
        let one = i64_ty.const_int(1, false);
        let eight = i64_ty.const_int(8, false);
        let data_phi = builder
            .build_phi(i64_ty, &format!("{}_cv_data", tag))
            .or_llvm_err()?;
        data_phi.add_incoming(&[(&w0, bb_cell), (&pk_data, bb_pack), (&ls_data, bb_list)]);
        let len_phi = builder
            .build_phi(i64_ty, &format!("{}_cv_len", tag))
            .or_llvm_err()?;
        len_phi.add_incoming(&[(&cell_len, bb_cell), (&pk_len, bb_pack), (&ls_len, bb_list)]);
        let elem_phi = builder
            .build_phi(i64_ty, &format!("{}_cv_elem", tag))
            .or_llvm_err()?;
        elem_phi.add_incoming(&[(&cell_elem, bb_cell), (&one, bb_pack), (&eight, bb_list)]);
        Ok(ContainerView {
            data: data_phi.as_basic_value().into_int_value(),
            len: len_phi.as_basic_value().into_int_value(),
            elem: elem_phi.as_basic_value().into_int_value(),
        })
    }
}
