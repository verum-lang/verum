//! Typed copying uses a heap-backed work stack. No value graph edge becomes a
//! native call to this helper; recursive source types only revisit plan nodes.
use super::*;
use verum_common::List;
use verum_llvm::{basic_block::BasicBlock, builder::Builder, context::Context, module::Linkage};
use verum_vbc::copy_plan::{CopyFailure, CopyNode, CopyPlan, NodeId};

const FRAME_BYTES: u64 = 96;
const JOURNAL_BYTES: u64 = 32;
struct Emitter<'a, 'ctx> {
    context: &'ctx Context,
    module: &'a Module<'ctx>,
    builder: Builder<'ctx>,
    function: FunctionValue<'ctx>,
    stack: PointerValue<'ctx>,
    capacity: PointerValue<'ctx>,
    depth: PointerValue<'ctx>,
    active: PointerValue<'ctx>,
    active_capacity: PointerValue<'ctx>,
    active_len: PointerValue<'ctx>,
    work: PointerValue<'ctx>,
    bytes: PointerValue<'ctx>,
    top: BasicBlock<'ctx>,
    finish: BasicBlock<'ctx>,
    fail: BasicBlock<'ctx>,
    failure: PointerValue<'ctx>,
    declarations: &'a verum_vbc::VbcModule,
    journal: PointerValue<'ctx>,
    journal_capacity: PointerValue<'ctx>,
    journal_len: PointerValue<'ctx>,
    max_work: IntValue<'ctx>,
    counters: Option<(FunctionValue<'ctx>, FunctionValue<'ctx>)>,
}
impl<'a, 'ctx> Emitter<'a, 'ctx> {
    fn integer(&self, n: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(n, false)
    }
    fn block(&self, name: &str) -> BasicBlock<'ctx> {
        self.context.append_basic_block(self.function, name)
    }
    fn pointer(&self, word: IntValue<'ctx>) -> Result<PointerValue<'ctx>> {
        self.builder
            .build_int_to_ptr(
                word,
                self.context.ptr_type(verum_llvm::AddressSpace::default()),
                "copy_address",
            )
            .or_llvm_err()
    }
    fn word(&self, pointer: PointerValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_ptr_to_int(pointer, self.context.i64_type(), "copy_address_word")
            .or_llvm_err()
    }
    fn address(
        &self,
        base: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        // SAFETY: all callers bound the complete object/frame extent first.
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), base, &[offset], "copy_slot")
                .or_llvm_err()
        }
    }
    fn load(&self, base: PointerValue<'ctx>, offset: u64) -> Result<IntValue<'ctx>> {
        self.builder
            .build_load(
                self.context.i64_type(),
                self.address(base, self.integer(offset))?,
                "copy_word",
            )
            .or_llvm_err()
            .map(|v| v.into_int_value())
    }
    fn store(&self, base: PointerValue<'ctx>, offset: u64, value: IntValue<'ctx>) -> Result<()> {
        self.builder
            .build_store(self.address(base, self.integer(offset))?, value)
            .or_llvm_err()
            .map(|_| ())
    }
    fn guard(&self, predicate: IntValue<'ctx>, label: &str) -> Result<()> {
        self.guard_reason(predicate, label, CopyFailure::InvalidStorage)
    }
    fn guard_reason(
        &self,
        predicate: IntValue<'ctx>,
        label: &str,
        reason: CopyFailure,
    ) -> Result<()> {
        let next = self.block(label);
        let refused = self.block("copy_guard_refused");
        self.builder
            .build_conditional_branch(predicate, next, refused)
            .or_llvm_err()?;
        self.builder.position_at_end(refused);
        self.builder
            .build_store(
                self.failure,
                self.context.i32_type().const_int(reason as u64, false),
            )
            .or_llvm_err()?;
        self.jump(self.fail)?;
        self.builder.position_at_end(next);
        Ok(())
    }
    fn equal(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_int_compare(IntPredicate::EQ, a, b, "copy_equal")
            .or_llvm_err()
    }
    fn bounded(&self, a: IntValue<'ctx>, bound: u64, label: &str) -> Result<()> {
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::ULE, a, self.integer(bound), "copy_bounded")
                .or_llvm_err()?,
            label,
        )
    }
    fn charge(&self, cell: PointerValue<'ctx>, amount: IntValue<'ctx>, label: &str) -> Result<()> {
        let remaining = self.load(cell, 0)?;
        self.guard_reason(
            self.builder
                .build_int_compare(IntPredicate::ULE, amount, remaining, "copy_budget")
                .or_llvm_err()?,
            label,
            if cell == self.work {
                CopyFailure::WorkLimit
            } else {
                CopyFailure::AllocationLimit
            },
        )?;
        self.store(
            cell,
            0,
            self.builder
                .build_int_sub(remaining, amount, "copy_remaining")
                .or_llvm_err()?,
        )
    }
    fn jump(&self, block: BasicBlock<'ctx>) -> Result<()> {
        self.builder
            .build_unconditional_branch(block)
            .or_llvm_err()
            .map(|_| ())
    }
    fn memcpy(
        &self,
        target: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<()> {
        self.builder
            .build_call(
                RuntimeLowering::new(self.context).get_or_declare_memcpy(self.module),
                &[target.into(), source.into(), size.into()],
                "",
            )
            .or_llvm_err()
            .map(|_| ())
    }
    fn malloc(&self, size: IntValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let f = super::super::error::get_or_declare_function(
            self.module,
            "verum_os_alloc",
            self.context
                .ptr_type(Default::default())
                .fn_type(&[self.context.i64_type().into()], false),
        );
        let pointer = self
            .builder
            .build_call(f, &[size.into()], "copy_storage")
            .or_llvm_err()?
            .basic_value_or("copy allocator")?
            .into_pointer_value();
        self.guard_reason(
            self.builder
                .build_is_not_null(pointer, "copy_allocated")
                .or_llvm_err()?,
            "copy_allocation_ready",
            CopyFailure::AllocationFailure,
        )?;
        Ok(pointer)
    }
    fn release(&self, value: PointerValue<'ctx>, size: IntValue<'ctx>) -> Result<()> {
        let f = super::super::error::get_or_declare_function(
            self.module,
            "verum_os_free",
            self.context.void_type().fn_type(
                &[value.get_type().into(), self.context.i64_type().into()],
                false,
            ),
        );
        self.builder
            .build_call(f, &[value.into(), size.into()], "")
            .or_llvm_err()
            .map(|_| ())
    }
    fn workspace(
        &self,
        cell: PointerValue<'ctx>,
        capacity: PointerValue<'ctx>,
        needed: IntValue<'ctx>,
        stride: u64,
        limit: IntValue<'ctx>,
    ) -> Result<()> {
        self.guard_reason(
            self.builder
                .build_int_compare(IntPredicate::ULE, needed, limit, "copy_workspace_bound")
                .or_llvm_err()?,
            "copy_workspace_allowed",
            CopyFailure::WorkLimit,
        )?;
        let old_capacity = self.load(capacity, 0)?;
        let grow = self.block("copy_grow_workspace");
        let ready = self.block("copy_workspace_ready");
        self.builder
            .build_conditional_branch(
                self.builder
                    .build_int_compare(
                        IntPredicate::UGT,
                        needed,
                        old_capacity,
                        "copy_workspace_full",
                    )
                    .or_llvm_err()?,
                grow,
                ready,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(grow);
        let doubled = self
            .builder
            .build_int_mul(old_capacity, self.integer(2), "copy_workspace_double")
            .or_llvm_err()?;
        let enlarged = self
            .builder
            .build_select(
                self.equal(old_capacity, self.integer(0))?,
                self.integer(8),
                doubled,
                "copy_workspace_enlarged",
            )
            .or_llvm_err()?
            .into_int_value();
        let count = self
            .builder
            .build_select(
                self.builder
                    .build_int_compare(IntPredicate::UGT, enlarged, limit, "copy_workspace_clamped")
                    .or_llvm_err()?,
                limit,
                enlarged,
                "copy_workspace_capacity",
            )
            .or_llvm_err()?
            .into_int_value();
        let allocation = self.malloc(
            self.builder
                .build_int_mul(count, self.integer(stride), "copy_workspace_bytes")
                .or_llvm_err()?,
        )?;
        let old = self.pointer(self.load(cell, 0)?)?;
        let old_bytes = self
            .builder
            .build_int_mul(
                old_capacity,
                self.integer(stride),
                "copy_old_workspace_bytes",
            )
            .or_llvm_err()?;
        let copy = self.block("copy_old_workspace");
        let publish = self.block("copy_workspace_publish");
        self.builder
            .build_conditional_branch(self.equal(old_capacity, self.integer(0))?, publish, copy)
            .or_llvm_err()?;
        self.builder.position_at_end(copy);
        self.memcpy(allocation, old, old_bytes)?;
        self.release(old, old_bytes)?;
        self.jump(publish)?;
        self.builder.position_at_end(publish);
        self.store(cell, 0, self.word(allocation)?)?;
        self.store(capacity, 0, count)?;
        self.jump(ready)?;
        self.builder.position_at_end(ready);
        Ok(())
    }
    fn journal_room(&self) -> Result<()> {
        let count = self
            .builder
            .build_int_add(
                self.load(self.journal_len, 0)?,
                self.integer(1),
                "copy_journal_needed",
            )
            .or_llvm_err()?;
        let limit = self
            .builder
            .build_int_mul(self.max_work, self.integer(2), "copy_journal_limit")
            .or_llvm_err()?;
        self.workspace(
            self.journal,
            self.journal_capacity,
            count,
            JOURNAL_BYTES,
            limit,
        )
    }
    fn journal_entry(&self, index: IntValue<'ctx>) -> Result<PointerValue<'ctx>> {
        self.address(
            self.pointer(self.load(self.journal, 0)?)?,
            self.builder
                .build_int_mul(index, self.integer(JOURNAL_BYTES), "copy_journal_offset")
                .or_llvm_err()?,
        )
    }
    fn record_allocation(
        &self,
        kind: u64,
        pointer: PointerValue<'ctx>,
        bytes: IntValue<'ctx>,
        counter: IntValue<'ctx>,
    ) -> Result<()> {
        let index = self.load(self.journal_len, 0)?;
        let entry = self.journal_entry(index)?;
        self.store(entry, 0, self.integer(kind))?;
        self.store(entry, 8, self.word(pointer)?)?;
        self.store(entry, 16, bytes)?;
        self.store(entry, 24, counter)?;
        self.store(
            self.journal_len,
            0,
            self.builder
                .build_int_add(index, self.integer(1), "copy_journal_count")
                .or_llvm_err()?,
        )
    }
    fn new_object(
        &self,
        owner: IntValue<'ctx>,
        bytes: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let payload = self
            .builder
            .build_int_sub(
                bytes,
                self.integer(RuntimeLowering::OBJECT_HEADER_SIZE),
                "copy_payload_bytes",
            )
            .or_llvm_err()?;
        self.charge(self.bytes, payload, "copy_allocation_budget")?;
        self.journal_room()?;
        let pointer = self.malloc(bytes)?;
        // Reserve-before-allocate makes this journal publication infallible.
        self.record_allocation(0, pointer, bytes, self.integer(0))?;
        let runtime = RuntimeLowering::new(self.context);
        self.builder
            .build_call(
                runtime.get_or_declare_memset(self.module)?,
                &[
                    pointer.into(),
                    self.context.i32_type().const_zero().into(),
                    bytes.into(),
                ],
                "",
            )
            .or_llvm_err()?;
        self.builder.build_store(pointer, owner).or_llvm_err()?;
        let size = self
            .builder
            .build_int_truncate(bytes, self.context.i32_type(), "copy_header_size")
            .or_llvm_err()?;
        self.builder
            .build_store(
                self.address(
                    pointer,
                    self.integer(verum_common::layout::OBJECT_HEADER_SIZE_OFFSET),
                )?,
                size,
            )
            .or_llvm_err()?;
        Ok(pointer)
    }
    fn raw(&self, bytes: IntValue<'ctx>) -> Result<PointerValue<'ctx>> {
        self.charge(self.bytes, bytes, "copy_backing_budget")?;
        self.journal_room()?;
        let allocate = super::super::error::get_or_declare_function(
            self.module,
            "verum_try_cbgr_allocate_aligned",
            self.context.ptr_type(Default::default()).fn_type(
                &[
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                ],
                false,
            ),
        );
        let pointer = self
            .builder
            .build_call(
                allocate,
                &[bytes.into(), self.integer(8).into()],
                "copy_owned_backing",
            )
            .or_llvm_err()?
            .basic_value_or("copy backing allocation")?
            .into_pointer_value();
        self.guard_reason(
            self.builder
                .build_is_not_null(pointer, "copy_backing_allocated")
                .or_llvm_err()?,
            "copy_backing_ready",
            CopyFailure::AllocationFailure,
        )?;
        self.record_allocation(1, pointer, bytes, self.integer(0))?;
        Ok(pointer)
    }
    fn workspace_release(
        &self,
        cell: PointerValue<'ctx>,
        capacity: PointerValue<'ctx>,
        stride: u64,
    ) -> Result<()> {
        let pointer = self.pointer(self.load(cell, 0)?)?;
        let release = self.block("copy_workspace_release");
        let done = self.block("copy_workspace_released");
        self.builder
            .build_conditional_branch(
                self.builder
                    .build_is_null(pointer, "copy_no_workspace")
                    .or_llvm_err()?,
                done,
                release,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(release);
        self.release(
            pointer,
            self.builder
                .build_int_mul(
                    self.load(capacity, 0)?,
                    self.integer(stride),
                    "copy_workspace_extent",
                )
                .or_llvm_err()?,
        )?;
        self.jump(done)?;
        self.builder.position_at_end(done);
        Ok(())
    }
    fn commit(&self) -> Result<()> {
        let top = self.block("copy_commit");
        let probe = self.block("copy_commit_entry");
        let acquire = self.block("copy_commit_retain");
        let acquired = self.block("copy_commit_acquired");
        let next = self.block("copy_commit_next");
        let done = self.block("copy_committed");
        let entry = self
            .builder
            .get_insert_block()
            .or_internal("copy commit predecessor")?;
        self.jump(top)?;
        self.builder.position_at_end(top);
        let index = self
            .builder
            .build_phi(self.context.i64_type(), "copy_commit_index")
            .or_llvm_err()?;
        index.add_incoming(&[(&self.integer(0), entry)]);
        let selected = index.as_basic_value().into_int_value();
        self.builder
            .build_conditional_branch(
                self.equal(selected, self.load(self.journal_len, 0)?)?,
                done,
                probe,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(probe);
        let row = self.journal_entry(selected)?;
        self.builder
            .build_conditional_branch(
                self.equal(self.load(row, 0)?, self.integer(2))?,
                acquire,
                next,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(acquire);
        if let Some((retain, _)) = self.counters {
            let counter = self.pointer(self.load(row, 24)?)?;
            let success = self
                .builder
                .build_call(retain, &[counter.into()], "copy_retained")
                .or_llvm_err()?
                .basic_value_or("checked retain")?
                .into_int_value();
            self.guard_reason(success, "copy_retain_accepted", CopyFailure::RetainRefused)?;
            self.jump(acquired)?;
        } else {
            self.jump(self.fail)?;
        }
        self.builder.position_at_end(acquired);
        self.store(row, 0, self.integer(3))?;
        self.jump(next)?;
        self.builder.position_at_end(next);
        let after = self
            .builder
            .build_int_add(selected, self.integer(1), "copy_commit_after")
            .or_llvm_err()?;
        self.jump(top)?;
        index.add_incoming(&[(&after, next)]);
        self.builder.position_at_end(done);
        Ok(())
    }
    fn rollback(&self) -> Result<IntValue<'ctx>> {
        let valid = self
            .builder
            .build_alloca(self.context.bool_type(), "copy_rollback_valid")
            .or_llvm_err()?;
        self.builder
            .build_store(valid, self.context.bool_type().const_int(1, false))
            .or_llvm_err()?;
        let top = self.block("copy_rollback");
        let probe = self.block("copy_rollback_entry");
        let undo = self.block("copy_rollback_retain");
        let storage = self.block("copy_rollback_storage");
        let raw = self.block("copy_rollback_raw");
        let object = self.block("copy_rollback_object");
        let next = self.block("copy_rollback_next");
        let done = self.block("copy_rolled_back");
        self.jump(top)?;
        self.builder.position_at_end(top);
        let count = self.load(self.journal_len, 0)?;
        self.builder
            .build_conditional_branch(self.equal(count, self.integer(0))?, done, probe)
            .or_llvm_err()?;
        self.builder.position_at_end(probe);
        let index = self
            .builder
            .build_int_sub(count, self.integer(1), "copy_rollback_index")
            .or_llvm_err()?;
        self.store(self.journal_len, 0, index)?;
        let row = self.journal_entry(index)?;
        let kind = self.load(row, 0)?;
        let pointer = self.pointer(self.load(row, 8)?)?;
        self.builder
            .build_conditional_branch(self.equal(kind, self.integer(3))?, undo, storage)
            .or_llvm_err()?;
        self.builder.position_at_end(undo);
        if let Some((_, release)) = self.counters {
            let counter = self.pointer(self.load(row, 24)?)?;
            // The operation still owns the original: this subtracts exactly
            // one successful local retain, never overwrites a previous count.
            let success = self
                .builder
                .build_call(release, &[counter.into()], "copy_retain_released")
                .or_llvm_err()?
                .basic_value_or("rollback result")?
                .into_int_value();
            let previous = self
                .builder
                .build_load(self.context.bool_type(), valid, "copy_rollback_so_far")
                .or_llvm_err()?
                .into_int_value();
            self.builder
                .build_store(
                    valid,
                    self.builder
                        .build_and(previous, success, "copy_rollback_complete")
                        .or_llvm_err()?,
                )
                .or_llvm_err()?;
        } else {
            self.builder
                .build_store(valid, self.context.bool_type().const_zero())
                .or_llvm_err()?;
        }
        self.jump(storage)?;
        self.builder.position_at_end(storage);
        self.builder
            .build_conditional_branch(self.equal(kind, self.integer(1))?, raw, object)
            .or_llvm_err()?;
        self.builder.position_at_end(raw);
        let release = super::super::error::get_or_declare_function(
            self.module,
            "verum_cbgr_deallocate",
            self.context
                .void_type()
                .fn_type(&[self.context.ptr_type(Default::default()).into()], false),
        );
        self.builder
            .build_call(release, &[pointer.into()], "")
            .or_llvm_err()?;
        self.jump(next)?;
        self.builder.position_at_end(object);
        self.release(pointer, self.load(row, 16)?)?;
        self.jump(next)?;
        self.builder.position_at_end(next);
        self.jump(top)?;
        self.builder.position_at_end(done);
        Ok(self
            .builder
            .build_load(self.context.bool_type(), valid, "copy_rollback_succeeded")
            .or_llvm_err()?
            .into_int_value())
    }
    fn frame(&self, index: IntValue<'ctx>) -> Result<PointerValue<'ctx>> {
        let base = self.pointer(self.load(self.stack, 0)?)?;
        self.address(
            base,
            self.builder
                .build_int_mul(index, self.integer(FRAME_BYTES), "copy_frame_offset")
                .or_llvm_err()?,
        )
    }
    fn initialize(
        &self,
        frame: PointerValue<'ctx>,
        source: IntValue<'ctx>,
        target: PointerValue<'ctx>,
        node: NodeId,
    ) -> Result<()> {
        self.store(frame, 0, source)?;
        self.store(frame, 8, self.word(target)?)?;
        self.store(frame, 16, self.integer(node.0 as u64))?;
        self.store(frame, 24, self.integer(u64::MAX))?;
        self.store(frame, 80, self.integer(0))?;
        self.store(frame, 88, self.integer(0))
    }
    fn child(
        &self,
        frame: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        target: PointerValue<'ctx>,
        node: NodeId,
    ) -> Result<()> {
        let value = self.load(source, 0)?;
        let index = self.load(frame, 24)?;
        self.store(
            frame,
            24,
            self.builder
                .build_int_add(index, self.integer(1), "copy_next_child")
                .or_llvm_err()?,
        )?;
        let depth = self.load(self.depth, 0)?;
        let next = self
            .builder
            .build_int_add(depth, self.integer(1), "copy_push_depth")
            .or_llvm_err()?;
        self.workspace(self.stack, self.capacity, next, FRAME_BYTES, self.max_work)?;
        self.initialize(self.frame(depth)?, value, target, node)?;
        self.store(self.depth, 0, next)?;
        self.jump(self.top)
    }
    fn owned(&self, frame: PointerValue<'ctx>, owner: TypeId) -> Result<PointerValue<'ctx>> {
        let source = self.load(frame, 0)?;
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::NE, source, self.integer(0), "copy_nonnull")
                .or_llvm_err()?,
            "copy_live",
        )?;
        let pointer = self.pointer(source)?;
        let id = self
            .builder
            .build_load(self.context.i32_type(), pointer, "copy_type")
            .or_llvm_err()?
            .into_int_value();
        self.guard(
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    id,
                    self.context.i32_type().const_int(u64::from(owner.0), false),
                    "copy_owner",
                )
                .or_llvm_err()?,
            "copy_owner_valid",
        )?;
        self.acyclic(frame, source)?;
        Ok(pointer)
    }
    fn active_bucket(
        &self,
        table: PointerValue<'ctx>,
        capacity: IntValue<'ctx>,
        source: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        // Mix all address bits: native object allocations can be page-aligned.
        let mut hash = source;
        for factor in [0xff51afd7ed558ccdu64, 0xc4ceb9fe1a85ec53] {
            let shifted = self
                .builder
                .build_right_shift(hash, self.integer(33), false, "copy_address_shift")
                .or_llvm_err()?;
            hash = self
                .builder
                .build_xor(hash, shifted, "copy_address_mix")
                .or_llvm_err()?;
            hash = self
                .builder
                .build_int_mul(hash, self.integer(factor), "copy_address_hash")
                .or_llvm_err()?;
        }
        let shifted = self
            .builder
            .build_right_shift(hash, self.integer(33), false, "copy_address_shift")
            .or_llvm_err()?;
        hash = self
            .builder
            .build_xor(hash, shifted, "copy_address_mix")
            .or_llvm_err()?;
        let mask = self
            .builder
            .build_int_sub(capacity, self.integer(1), "copy_active_mask")
            .or_llvm_err()?;
        let bucket = self
            .builder
            .build_and(hash, mask, "copy_active_bucket")
            .or_llvm_err()?;
        let offset = self
            .builder
            .build_int_mul(bucket, self.integer(8), "copy_active_offset")
            .or_llvm_err()?;
        self.address(table, offset)
    }
    fn active_room(&self) -> Result<()> {
        let count = self.load(self.active_len, 0)?;
        let needed = self
            .builder
            .build_int_add(count, self.integer(1), "copy_active_needed")
            .or_llvm_err()?;
        self.guard_reason(
            self.builder
                .build_int_compare(
                    IntPredicate::ULE,
                    needed,
                    self.max_work,
                    "copy_active_bound",
                )
                .or_llvm_err()?,
            "copy_active_allowed",
            CopyFailure::WorkLimit,
        )?;
        let capacity = self.load(self.active_capacity, 0)?;
        let twice = self
            .builder
            .build_int_mul(needed, self.integer(2), "copy_active_load")
            .or_llvm_err()?;
        let grow = self.block("copy_active_grow");
        let ready = self.block("copy_active_ready");
        self.builder
            .build_conditional_branch(
                self.builder
                    .build_int_compare(IntPredicate::UGT, twice, capacity, "copy_active_full")
                    .or_llvm_err()?,
                grow,
                ready,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(grow);
        let doubled = self
            .builder
            .build_int_mul(capacity, self.integer(2), "copy_active_double")
            .or_llvm_err()?;
        let enlarged = self
            .builder
            .build_select(
                self.equal(capacity, self.integer(0))?,
                self.integer(8),
                doubled,
                "copy_active_capacity",
            )
            .or_llvm_err()?
            .into_int_value();
        let bytes = self
            .builder
            .build_int_mul(enlarged, self.integer(8), "copy_active_bytes")
            .or_llvm_err()?;
        let table = self.malloc(bytes)?;
        self.builder
            .build_call(
                RuntimeLowering::new(self.context).get_or_declare_memset(self.module)?,
                &[
                    table.into(),
                    self.context.i32_type().const_zero().into(),
                    bytes.into(),
                ],
                "",
            )
            .or_llvm_err()?;
        // Rehash only live DFS frames. The table is private, and no fallible
        // operation occurs between allocation and publication of the new table.
        let old = self.pointer(self.load(self.active, 0)?)?;
        let depth = self.load(self.depth, 0)?;
        let scan = self.block("copy_active_rehash");
        let row = self.block("copy_active_rehash_row");
        let insert = self.block("copy_active_rehash_insert");
        let next = self.block("copy_active_rehash_next");
        let publish = self.block("copy_active_publish");
        let entry = self
            .builder
            .get_insert_block()
            .or_internal("copy active predecessor")?;
        self.jump(scan)?;
        self.builder.position_at_end(scan);
        let phi = self
            .builder
            .build_phi(self.context.i64_type(), "copy_rehash_index")
            .or_llvm_err()?;
        phi.add_incoming(&[(&self.integer(0), entry)]);
        let index = phi.as_basic_value().into_int_value();
        self.builder
            .build_conditional_branch(self.equal(index, depth)?, publish, row)
            .or_llvm_err()?;
        self.builder.position_at_end(row);
        let frame = self.frame(index)?;
        self.builder
            .build_conditional_branch(
                self.equal(self.load(frame, 88)?, self.integer(0))?,
                next,
                insert,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(insert);
        let slot = self.active_bucket(table, enlarged, self.load(frame, 0)?)?;
        self.store(frame, 80, self.load(slot, 0)?)?;
        self.store(
            slot,
            0,
            self.builder
                .build_int_add(index, self.integer(1), "copy_rehash_link")
                .or_llvm_err()?,
        )?;
        self.jump(next)?;
        self.builder.position_at_end(next);
        let following = self
            .builder
            .build_int_add(index, self.integer(1), "copy_rehash_next")
            .or_llvm_err()?;
        self.jump(scan)?;
        phi.add_incoming(&[(&following, next)]);
        self.builder.position_at_end(publish);
        self.store(self.active, 0, self.word(table)?)?;
        self.store(self.active_capacity, 0, enlarged)?;
        let release = self.block("copy_active_release_old");
        self.builder
            .build_conditional_branch(self.equal(capacity, self.integer(0))?, ready, release)
            .or_llvm_err()?;
        self.builder.position_at_end(release);
        let old_bytes = self
            .builder
            .build_int_mul(capacity, self.integer(8), "copy_active_old_bytes")
            .or_llvm_err()?;
        self.release(old, old_bytes)?;
        self.jump(ready)?;
        self.builder.position_at_end(ready);
        Ok(())
    }
    fn acyclic(&self, frame: PointerValue<'ctx>, source: IntValue<'ctx>) -> Result<()> {
        // Bucket chains are links in the active DFS stack. Removal is LIFO,
        // so a finished node is always its bucket head: no tombstones or stale
        // graph-wide identities remain to reject a repeated independent copy.
        self.active_room()?;
        let table = self.pointer(self.load(self.active, 0)?)?;
        let slot = self.active_bucket(table, self.load(self.active_capacity, 0)?, source)?;
        let head = self.load(slot, 0)?;
        let depth = self.load(self.depth, 0)?;
        let scan = self.block("copy_active_lookup");
        let row = self.block("copy_active_probe");
        let insert = self.block("copy_active_insert");
        let entry = self
            .builder
            .get_insert_block()
            .or_internal("copy active entry")?;
        self.jump(scan)?;
        self.builder.position_at_end(scan);
        let link = self
            .builder
            .build_phi(self.context.i64_type(), "copy_active_link")
            .or_llvm_err()?;
        let examined = self
            .builder
            .build_phi(self.context.i64_type(), "copy_active_examined")
            .or_llvm_err()?;
        link.add_incoming(&[(&head, entry)]);
        examined.add_incoming(&[(&self.integer(0), entry)]);
        let selected = link.as_basic_value().into_int_value();
        let steps = examined.as_basic_value().into_int_value();
        self.builder
            .build_conditional_branch(self.equal(selected, self.integer(0))?, insert, row)
            .or_llvm_err()?;
        self.builder.position_at_end(row);
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::ULE, selected, depth, "copy_active_link_bound")
                .or_llvm_err()?,
            "copy_active_link_valid",
        )?;
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::ULT, steps, depth, "copy_active_probe_bound")
                .or_llvm_err()?,
            "copy_active_probe_valid",
        )?;
        let index = self
            .builder
            .build_int_sub(selected, self.integer(1), "copy_active_index")
            .or_llvm_err()?;
        let ancestor = self.frame(index)?;
        self.guard_reason(
            self.builder
                .build_int_compare(
                    IntPredicate::NE,
                    self.load(ancestor, 0)?,
                    source,
                    "copy_acyclic",
                )
                .or_llvm_err()?,
            "copy_ancestor_distinct",
            CopyFailure::OwnedCycle,
        )?;
        let following = self.load(ancestor, 80)?;
        let next_steps = self
            .builder
            .build_int_add(steps, self.integer(1), "copy_active_next_probe")
            .or_llvm_err()?;
        let predecessor = self
            .builder
            .get_insert_block()
            .or_internal("copy active probe predecessor")?;
        self.jump(scan)?;
        link.add_incoming(&[(&following, predecessor)]);
        examined.add_incoming(&[(&next_steps, predecessor)]);
        self.builder.position_at_end(insert);
        self.store(frame, 80, head)?;
        self.store(frame, 88, self.integer(1))?;
        self.store(slot, 0, depth)?;
        self.store(
            self.active_len,
            0,
            self.builder
                .build_int_add(
                    self.load(self.active_len, 0)?,
                    self.integer(1),
                    "copy_active_count",
                )
                .or_llvm_err()?,
        )
    }
    fn active_remove(&self, frame: PointerValue<'ctx>) -> Result<()> {
        let remove = self.block("copy_active_remove");
        let done = self.block("copy_active_removed");
        self.builder
            .build_conditional_branch(
                self.equal(self.load(frame, 88)?, self.integer(0))?,
                done,
                remove,
            )
            .or_llvm_err()?;
        self.builder.position_at_end(remove);
        let table = self.pointer(self.load(self.active, 0)?)?;
        let slot = self.active_bucket(
            table,
            self.load(self.active_capacity, 0)?,
            self.load(frame, 0)?,
        )?;
        self.guard(
            self.equal(self.load(slot, 0)?, self.load(self.depth, 0)?)?,
            "copy_active_lifo",
        )?;
        self.store(slot, 0, self.load(frame, 80)?)?;
        self.store(frame, 88, self.integer(0))?;
        let count = self.load(self.active_len, 0)?;
        self.guard(
            self.builder
                .build_int_compare(
                    IntPredicate::UGT,
                    count,
                    self.integer(0),
                    "copy_active_nonempty",
                )
                .or_llvm_err()?,
            "copy_active_count_valid",
        )?;
        self.store(
            self.active_len,
            0,
            self.builder
                .build_int_sub(count, self.integer(1), "copy_active_count")
                .or_llvm_err()?,
        )?;
        self.jump(done)?;
        self.builder.position_at_end(done);
        Ok(())
    }
    fn duplicate(
        &self,
        source: PointerValue<'ctx>,
        size: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>> {
        let owner = self
            .builder
            .build_load(self.context.i32_type(), source, "copy_declared_owner")
            .or_llvm_err()?
            .into_int_value();
        let target = self.new_object(owner, size)?;
        let header = self.integer(RuntimeLowering::OBJECT_HEADER_SIZE);
        self.memcpy(
            self.address(target, header)?,
            self.address(source, header)?,
            self.builder
                .build_int_sub(size, header, "copy_owned_payload")
                .or_llvm_err()?,
        )?;
        Ok(target)
    }
    fn shape(&self, source: PointerValue<'ctx>, bytes: u64) -> Result<()> {
        let size = self
            .builder
            .build_load(
                self.context.i32_type(),
                self.address(source, self.integer(12))?,
                "copy_object_size",
            )
            .or_llvm_err()?
            .into_int_value();
        self.guard(
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    size,
                    self.context.i32_type().const_int(bytes, false),
                    "copy_exact_extent",
                )
                .or_llvm_err()?,
            "copy_shape_valid",
        )
    }
    fn entered(
        &self,
        frame: PointerValue<'ctx>,
        result: PointerValue<'ctx>,
        input: PointerValue<'ctx>,
        output: PointerValue<'ctx>,
        count: IntValue<'ctx>,
    ) -> Result<()> {
        self.store(frame, 32, self.word(result)?)?;
        self.store(frame, 40, self.word(input)?)?;
        self.store(frame, 48, self.word(output)?)?;
        self.store(frame, 56, count)?;
        self.store(frame, 24, self.integer(0))?;
        self.jump(self.top)
    }
    fn record(
        &self,
        frame: PointerValue<'ctx>,
        owner: TypeId,
        fields: &[NodeId],
        offset: u64,
    ) -> Result<()> {
        let source = self.owned(frame, owner)?;
        let slots = fields.len();
        let bytes = RuntimeLowering::OBJECT_HEADER_SIZE + offset + slots as u64 * 8;
        self.shape(source, bytes)?;
        let output = self.duplicate(source, self.integer(bytes))?;
        self.entered(
            frame,
            output,
            self.address(source, self.integer(24 + offset))?,
            self.address(output, self.integer(24 + offset))?,
            self.integer(fields.len() as u64),
        )
    }
    fn field_child(&self, frame: PointerValue<'ctx>, fields: &[NodeId]) -> Result<()> {
        let index = self.load(frame, 24)?;
        let cases: List<_> = fields
            .iter()
            .enumerate()
            .map(|(i, _)| (self.integer(i as u64), self.block("copy_field")))
            .collect();
        self.builder
            .build_switch(index, self.fail, cases.as_slice())
            .or_llvm_err()?;
        for ((_, block), node) in cases.iter().zip(fields) {
            self.builder.position_at_end(*block);
            let offset = self
                .builder
                .build_int_mul(index, self.integer(8), "copy_field_offset")
                .or_llvm_err()?;
            let input = self.address(self.pointer(self.load(frame, 40)?)?, offset)?;
            let output = self.address(self.pointer(self.load(frame, 48)?)?, offset)?;
            self.child(frame, input, output, *node)?;
        }
        Ok(())
    }
    fn list(
        &self,
        frame: PointerValue<'ctx>,
        element: NodeId,
        length: Option<u64>,
        plan: &CopyPlan,
    ) -> Result<()> {
        let source = self.pointer(self.load(frame, 0)?)?;
        self.guard(
            self.builder
                .build_is_not_null(source, "copy_list_present")
                .or_llvm_err()?,
            "copy_list_live",
        )?;
        self.shape(source, 48)?;
        let len = self.load(source, verum_common::layout::LIST_LEN_OFFSET)?;
        let cap = self.load(source, verum_common::layout::LIST_CAP_OFFSET)?;
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::ULE, len, cap, "copy_list_length")
                .or_llvm_err()?,
            "copy_list_initialized",
        )?;
        if let Some(length) = length {
            self.guard(self.equal(len, self.integer(length))?, "copy_array_length")?;
        }
        let checked = self
            .module
            .get_function("verum_list_storage_address_checked")
            .or_missing_fn("verum_list_storage_address_checked")?;
        let result = self
            .builder
            .build_call(
                checked,
                &[source.into(), self.integer(0).into(), len.into()],
                "copy_list_shape",
            )
            .or_llvm_err()?
            .basic_value_or("checked List storage")?
            .into_struct_value();
        let valid = self
            .builder
            .build_extract_value(result, 0, "copy_list_valid")
            .or_llvm_err()?
            .into_int_value();
        self.guard(valid, "copy_list_storage_valid")?;
        self.acyclic(frame, self.load(frame, 0)?)?;
        let input = self
            .builder
            .build_extract_value(result, 1, "copy_list_input")
            .or_llvm_err()?
            .into_pointer_value();
        let width = self
            .builder
            .build_extract_value(result, 2, "copy_list_width")
            .or_llvm_err()?
            .into_int_value();
        let scalar = matches!(plan.nodes[element.0], CopyNode::Identity);
        if !scalar {
            self.guard(
                self.equal(width, self.integer(8))?,
                "copy_compound_encoding",
            )?;
        }
        let encoding = self
            .builder
            .build_load(self.context.i32_type(), source, "copy_list_encoding")
            .or_llvm_err()?
            .into_int_value();
        let output = self.new_object(encoding, self.integer(48))?;
        self.store(output, verum_common::layout::LIST_LEN_OFFSET, len)?;
        self.store(output, verum_common::layout::LIST_CAP_OFFSET, cap)?;
        let empty = self.block("copy_list_empty");
        let nonempty = self.block("copy_list_backing");
        let ready = self.block("copy_list_ready");
        self.builder
            .build_conditional_branch(self.equal(cap, self.integer(0))?, empty, nonempty)
            .or_llvm_err()?;
        self.builder.position_at_end(empty);
        self.jump(ready)?;
        self.builder.position_at_end(nonempty);
        let target = self.raw(
            self.builder
                .build_int_mul(cap, width, "copy_list_backing_extent")
                .or_llvm_err()?,
        )?;
        let filled = self.block("copy_list_filled");
        let copy = self.block("copy_list_initialized_prefix");
        self.builder
            .build_conditional_branch(self.equal(len, self.integer(0))?, filled, copy)
            .or_llvm_err()?;
        self.builder.position_at_end(copy);
        self.memcpy(
            target,
            input,
            self.builder
                .build_int_mul(len, width, "copy_list_initialized_bytes")
                .or_llvm_err()?,
        )?;
        self.jump(filled)?;
        self.builder.position_at_end(filled);
        self.store(
            output,
            verum_common::layout::LIST_PTR_OFFSET,
            self.word(target)?,
        )?;
        self.jump(ready)?;
        self.builder.position_at_end(ready);
        let address = self
            .builder
            .build_phi(target.get_type(), "copy_list_output")
            .or_llvm_err()?;
        address.add_incoming(&[(&target.get_type().const_null(), empty), (&target, filled)]);
        self.entered(
            frame,
            output,
            input,
            address.as_basic_value().into_pointer_value(),
            if scalar { self.integer(0) } else { len },
        )
    }
    fn container(&self, frame: PointerValue<'ctx>, owner: TypeId, map: bool) -> Result<()> {
        let source = self.owned(frame, owner)?;
        self.shape(source, 56)?;
        let cap = self.load(source, if map { 32 } else { 48 })?;
        let len = self.load(source, if map { 24 } else { 40 })?;
        let stride = if map { 32 } else { 8 };
        self.bounded(
            cap,
            u32::MAX as u64 / stride as u64,
            "copy_container_capacity",
        )?;
        self.guard(
            self.builder
                .build_int_compare(IntPredicate::ULE, len, cap, "copy_container_length")
                .or_llvm_err()?,
            "copy_container_length_valid",
        )?;
        let pointer_offset = if map { 40 } else { 24 };
        let input = self.pointer(self.load(source, pointer_offset)?)?;
        let extent = self
            .builder
            .build_int_mul(cap, self.integer(stride as u64), "copy_container_extent")
            .or_llvm_err()?;
        let output = self.duplicate(source, self.integer(56))?;
        let empty = self.block("copy_empty_container");
        let nonempty = self.block("copy_container_backing");
        let ready = self.block("copy_container_ready");
        self.builder
            .build_conditional_branch(self.equal(cap, self.integer(0))?, empty, nonempty)
            .or_llvm_err()?;
        self.builder.position_at_end(empty);
        self.guard(
            self.equal(self.word(input)?, self.integer(0))?,
            "copy_empty_backing",
        )?;
        let empty_predecessor = self.builder.get_insert_block().or_internal("empty copy")?;
        self.jump(ready)?;
        self.builder.position_at_end(nonempty);
        self.guard(
            self.builder
                .build_is_not_null(input, "copy_backing_present")
                .or_llvm_err()?,
            "copy_backing_live",
        )?;
        let runtime = RuntimeLowering::new(self.context);
        let header = runtime.emit_cbgr_get_header(&self.builder, input)?;
        let actual = self
            .builder
            .build_load(
                self.context.i32_type(),
                self.address(
                    header,
                    self.integer(verum_common::layout::ALLOCATION_HEADER_SIZE_OFFSET),
                )?,
                "copy_backing_size",
            )
            .or_llvm_err()?
            .into_int_value();
        let actual = self
            .builder
            .build_int_z_extend(actual, self.context.i64_type(), "copy_backing_extent")
            .or_llvm_err()?;
        self.guard(self.equal(actual, extent)?, "copy_owned_backing_valid")?;
        if map {
            self.map_occupancy(input, cap, len)?;
        }
        let target = self.raw(extent)?;
        self.memcpy(target, input, extent)?;
        let filled_predecessor = self.builder.get_insert_block().or_internal("filled copy")?;
        self.jump(ready)?;
        self.builder.position_at_end(ready);
        let joined = self
            .builder
            .build_phi(input.get_type(), "copy_container_target")
            .or_llvm_err()?;
        joined.add_incoming(&[(&input, empty_predecessor), (&target, filled_predecessor)]);
        let target = joined.as_basic_value().into_pointer_value();
        self.store(output, pointer_offset, self.word(target)?)?;
        self.store(frame, 64, cap)?;
        if !map {
            let head = self.load(source, 32)?;
            let empty = self.equal(cap, self.integer(0))?;
            let valid_head = self
                .builder
                .build_int_compare(IntPredicate::ULT, head, cap, "copy_ring_head")
                .or_llvm_err()?;
            let zero_head = self.equal(head, self.integer(0))?;
            self.guard(
                self.builder
                    .build_select(empty, zero_head, valid_head, "copy_ring_valid")
                    .or_llvm_err()?
                    .into_int_value(),
                "copy_ring_head_valid",
            )?;
            self.store(frame, 72, head)?;
        }
        let count = if map {
            self.builder
                .build_int_mul(cap, self.integer(2), "copy_map_slots")
                .or_llvm_err()?
        } else {
            len
        };
        self.entered(frame, output, input, target, count)
    }
    fn map_occupancy(
        &self,
        input: PointerValue<'ctx>,
        capacity: IntValue<'ctx>,
        length: IntValue<'ctx>,
    ) -> Result<()> {
        let scan = self.block("copy_map_scan");
        let probe = self.block("copy_map_probe");
        let done = self.block("copy_map_checked");
        let entry = self
            .builder
            .get_insert_block()
            .or_internal("map scan entry")?;
        self.jump(scan)?;
        self.builder.position_at_end(scan);
        let index = self
            .builder
            .build_phi(self.context.i64_type(), "copy_map_bucket")
            .or_llvm_err()?;
        let occupied = self
            .builder
            .build_phi(self.context.i64_type(), "copy_map_occupied")
            .or_llvm_err()?;
        index.add_incoming(&[(&self.integer(0), entry)]);
        occupied.add_incoming(&[(&self.integer(0), entry)]);
        let selected = index.as_basic_value().into_int_value();
        let count = occupied.as_basic_value().into_int_value();
        self.builder
            .build_conditional_branch(self.equal(selected, capacity)?, done, probe)
            .or_llvm_err()?;
        self.builder.position_at_end(probe);
        self.charge(self.work, self.integer(1), "copy_map_scan_budget")?;
        let offset = self
            .builder
            .build_int_mul(selected, self.integer(32), "copy_map_bucket_offset")
            .or_llvm_err()?;
        let hash = self.load(self.address(input, offset)?, 16)?;
        self.guard(
            self.builder
                .build_int_compare(
                    IntPredicate::SGE,
                    hash,
                    self.integer(u64::MAX),
                    "copy_map_hash_valid",
                )
                .or_llvm_err()?,
            "copy_map_hash_checked",
        )?;
        let used = self
            .builder
            .build_int_compare(IntPredicate::SGT, hash, self.integer(0), "copy_map_used")
            .or_llvm_err()?;
        let next_count = self
            .builder
            .build_int_add(
                count,
                self.builder
                    .build_int_z_extend(used, self.context.i64_type(), "copy_map_used_count")
                    .or_llvm_err()?,
                "copy_map_next_count",
            )
            .or_llvm_err()?;
        let next_index = self
            .builder
            .build_int_add(selected, self.integer(1), "copy_map_next_bucket")
            .or_llvm_err()?;
        let predecessor = self
            .builder
            .get_insert_block()
            .or_internal("map scan predecessor")?;
        self.jump(scan)?;
        index.add_incoming(&[(&next_index, predecessor)]);
        occupied.add_incoming(&[(&next_count, predecessor)]);
        self.builder.position_at_end(done);
        self.guard(
            self.equal(count, length)?,
            "copy_map_occupancy_matches_length",
        )
    }
    fn children(&self, frame: PointerValue<'ctx>, node: &CopyNode) -> Result<()> {
        let index = self.load(frame, 24)?;
        let input = self.pointer(self.load(frame, 40)?)?;
        let output = self.pointer(self.load(frame, 48)?)?;
        match node {
            CopyNode::Record { fields, .. } => self.field_child(frame, fields),
            CopyNode::Sum { variants, .. } => {
                let source = self.pointer(self.load(frame, 0)?)?;
                let tag = self
                    .builder
                    .build_load(
                        self.context.i32_type(),
                        self.address(source, self.integer(24))?,
                        "copy_tag",
                    )
                    .or_llvm_err()?
                    .into_int_value();
                let cases: List<_> = variants
                    .iter()
                    .map(|(tag, _)| {
                        (
                            self.context.i32_type().const_int(u64::from(*tag), false),
                            self.block("copy_variant_fields"),
                        )
                    })
                    .collect();
                self.builder
                    .build_switch(tag, self.fail, cases.as_slice())
                    .or_llvm_err()?;
                for ((_, block), (_, fields)) in cases.iter().zip(variants) {
                    self.builder.position_at_end(*block);
                    self.field_child(frame, fields)?;
                }
                Ok(())
            }
            CopyNode::List { element, .. } | CopyNode::Array { element, .. } => {
                let offset = self
                    .builder
                    .build_int_mul(index, self.integer(8), "copy_element_offset")
                    .or_llvm_err()?;
                self.child(
                    frame,
                    self.address(input, offset)?,
                    self.address(output, offset)?,
                    *element,
                )
            }
            CopyNode::Deque { element, .. } => {
                let cap = self.load(frame, 64)?;
                let selected = self
                    .builder
                    .build_int_unsigned_rem(
                        self.builder
                            .build_int_add(index, self.load(frame, 72)?, "copy_ring_unwrapped")
                            .or_llvm_err()?,
                        cap,
                        "copy_ring_index",
                    )
                    .or_llvm_err()?;
                let offset = self
                    .builder
                    .build_int_mul(selected, self.integer(8), "copy_ring_offset")
                    .or_llvm_err()?;
                self.child(
                    frame,
                    self.address(input, offset)?,
                    self.address(output, offset)?,
                    *element,
                )
            }
            CopyNode::Map { key, value, .. } => {
                let bucket = self
                    .builder
                    .build_int_unsigned_div(index, self.integer(2), "copy_bucket")
                    .or_llvm_err()?;
                let offset = self
                    .builder
                    .build_int_mul(bucket, self.integer(32), "copy_bucket_offset")
                    .or_llvm_err()?;
                let hash = self.load(self.address(input, offset)?, 16)?;
                let occupied = self
                    .builder
                    .build_int_compare(IntPredicate::SGT, hash, self.integer(0), "copy_occupied")
                    .or_llvm_err()?;
                let next = self.block("copy_skip_empty");
                let child = self.block("copy_occupied_entry");
                self.builder
                    .build_conditional_branch(occupied, child, next)
                    .or_llvm_err()?;
                self.builder.position_at_end(next);
                self.charge(self.work, self.integer(2), "copy_map_vacant_budget")?;
                self.store(
                    frame,
                    24,
                    self.builder
                        .build_int_mul(
                            self.builder
                                .build_int_add(bucket, self.integer(1), "copy_next_bucket")
                                .or_llvm_err()?,
                            self.integer(2),
                            "copy_next_key",
                        )
                        .or_llvm_err()?,
                )?;
                self.jump(self.top)?;
                self.builder.position_at_end(child);
                let part = self
                    .builder
                    .build_and(index, self.integer(1), "copy_entry_part")
                    .or_llvm_err()?;
                let key_block = self.block("copy_key");
                let value_block = self.block("copy_value");
                self.builder
                    .build_conditional_branch(
                        self.equal(part, self.integer(0))?,
                        key_block,
                        value_block,
                    )
                    .or_llvm_err()?;
                self.builder.position_at_end(key_block);
                self.child(
                    frame,
                    self.address(input, offset)?,
                    self.address(output, offset)?,
                    *key,
                )?;
                self.builder.position_at_end(value_block);
                let offset = self
                    .builder
                    .build_int_add(offset, self.integer(8), "copy_value_offset")
                    .or_llvm_err()?;
                self.child(
                    frame,
                    self.address(input, offset)?,
                    self.address(output, offset)?,
                    *value,
                )
            }
            _ => self.jump(self.fail),
        }
    }
    fn retained(
        &self,
        frame: PointerValue<'ctx>,
        owner: TypeId,
        contract: &verum_vbc::retain_forward::RetainForward,
    ) -> Result<()> {
        let source = self.pointer(self.load(frame, 0)?)?;
        self.guard(
            self.builder
                .build_is_not_null(source, "copy_handle_present")
                .or_llvm_err()?,
            "copy_handle_live",
        )?;
        let actual = self
            .builder
            .build_load(self.context.i32_type(), source, "copy_handle_owner")
            .or_llvm_err()?
            .into_int_value();
        self.guard(
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    actual,
                    self.context.i32_type().const_int(u64::from(owner.0), false),
                    "copy_handle_exact",
                )
                .or_llvm_err()?,
            "copy_handle_declared",
        )?;
        let layout = verum_vbc::retain_copy::layout(self.declarations, contract.owner())
            .map_err(LlvmLoweringError::Internal)?;
        let bytes = RuntimeLowering::OBJECT_HEADER_SIZE + layout.fields as u64 * 8;
        self.shape(source, bytes)?;
        let mut current = source;
        let mut raw = false;
        let mut counter = None;
        for (index, field) in contract.counter_path().iter().enumerate() {
            self.charge(self.work, self.integer(1), "copy_counter_path_budget")?;
            self.guard(
                self.builder
                    .build_is_not_null(current, "copy_counter_owner_present")
                    .or_llvm_err()?,
                "copy_counter_owner_live",
            )?;
            let id = match &field.record {
                TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => *id,
                _ => {
                    return Err(LlvmLoweringError::internal(
                        "retained field lacks its record owner",
                    ));
                }
            };
            let descriptor = self
                .declarations
                .get_type(id)
                .or_internal("retained field descriptor")?;
            let size = descriptor.fields.len().max(1) as u64 * 8;
            let base = if raw {
                let header = RuntimeLowering::new(self.context)
                    .emit_cbgr_get_header(&self.builder, current)?;
                let extent = self
                    .builder
                    .build_load(
                        self.context.i32_type(),
                        self.address(
                            header,
                            self.integer(verum_common::layout::ALLOCATION_HEADER_SIZE_OFFSET),
                        )?,
                        "copy_counter_extent",
                    )
                    .or_llvm_err()?
                    .into_int_value();
                self.guard(
                    self.builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            extent,
                            self.context.i32_type().const_int(size, false),
                            "copy_counter_extent_exact",
                        )
                        .or_llvm_err()?,
                    "copy_counter_extent_valid",
                )?;
                current
            } else {
                let actual = self
                    .builder
                    .build_load(
                        self.context.i32_type(),
                        current,
                        "copy_counter_record_owner",
                    )
                    .or_llvm_err()?
                    .into_int_value();
                self.guard(
                    self.builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            actual,
                            self.context.i32_type().const_int(u64::from(id.0), false),
                            "copy_counter_record_exact",
                        )
                        .or_llvm_err()?,
                    "copy_counter_record_valid",
                )?;
                self.shape(current, size + RuntimeLowering::OBJECT_HEADER_SIZE)?;
                self.address(current, self.integer(RuntimeLowering::OBJECT_HEADER_SIZE))?
            };
            let slot = self.address(base, self.integer(u64::from(field.index) * 8))?;
            if index + 1 == contract.counter_path().len() {
                counter = Some(slot);
                break;
            }
            raw = match &field.field {
                TypeRef::Reference {
                    tier: verum_vbc::types::CbgrTier::Tier2,
                    ..
                } => true,
                TypeRef::Reference { .. } => {
                    return Err(LlvmLoweringError::internal(
                        "native retained path needs its declared safe reference carrier",
                    ));
                }
                _ => false,
            };
            current = self.pointer(self.load(slot, 0)?)?;
        }
        let counter = counter.or_internal("retained counter field path")?;
        let low = self
            .builder
            .build_and(
                self.word(counter)?,
                self.integer(3),
                "copy_counter_alignment",
            )
            .or_llvm_err()?;
        self.guard(self.equal(low, self.integer(0))?, "copy_counter_aligned")?;
        self.charge(
            self.bytes,
            self.integer(layout.fields as u64 * 8),
            "copy_allocation_budget",
        )?;
        self.journal_room()?;
        let output = super::retain_copy::prepare_wrapper(
            self.context,
            &self.builder,
            self.module,
            source,
            layout,
        )?;
        self.guard_reason(
            self.builder
                .build_is_not_null(output, "copy_retained_wrapper_prepared")
                .or_llvm_err()?,
            "copy_retained_wrapper_ready",
            CopyFailure::AllocationFailure,
        )?;
        // Journal capacity preceded allocation, so this publication cannot fail.
        self.record_allocation(2, output, self.integer(bytes), self.word(counter)?)?;
        self.store(frame, 32, self.word(output)?)?;
        self.jump(self.finish)
    }
    fn enter(
        &self,
        frame: PointerValue<'ctx>,
        node: &CopyNode,
        _id: usize,
        plan: &CopyPlan,
    ) -> Result<()> {
        match node {
            CopyNode::Identity => {
                self.store(frame, 32, self.load(frame, 0)?)?;
                self.jump(self.finish)
            }
            CopyNode::Alias(next) => {
                self.store(frame, 16, self.integer(next.0 as u64))?;
                self.jump(self.top)
            }
            CopyNode::Record { owner, fields } => self.record(frame, *owner, fields, 0),
            CopyNode::Sum { owner, variants } => {
                let source = self.pointer(self.load(frame, 0)?)?;
                self.guard(
                    self.builder
                        .build_int_compare(
                            IntPredicate::NE,
                            self.word(source)?,
                            self.integer(0),
                            "copy_sum_present",
                        )
                        .or_llvm_err()?,
                    "copy_sum_tag_read",
                )?;
                let tag = self
                    .builder
                    .build_load(
                        self.context.i32_type(),
                        self.address(source, self.integer(24))?,
                        "copy_tag",
                    )
                    .or_llvm_err()?
                    .into_int_value();
                let cases: List<_> = variants
                    .iter()
                    .map(|(tag, _)| {
                        (
                            self.context.i32_type().const_int(u64::from(*tag), false),
                            self.block("copy_variant"),
                        )
                    })
                    .collect();
                self.builder
                    .build_switch(tag, self.fail, cases.as_slice())
                    .or_llvm_err()?;
                for ((_, block), (_, fields)) in cases.iter().zip(variants) {
                    self.builder.position_at_end(*block);
                    self.record(frame, *owner, fields, 8)?;
                }
                Ok(())
            }
            CopyNode::List { element, .. } => self.list(frame, *element, None, plan),
            CopyNode::Array { element, length } => self.list(frame, *element, Some(*length), plan),
            CopyNode::Map { owner, .. } => self.container(frame, *owner, true),
            CopyNode::Deque { owner, .. } => self.container(frame, *owner, false),
            CopyNode::Shared {
                owner, contract, ..
            } => self.retained(frame, *owner, contract),
        }
    }
}

pub(super) fn lower<'ctx>(
    ctx: &mut FunctionContext<'_, 'ctx>,
    dst: Reg,
    src: Reg,
    ty: &TypeRef,
) -> Result<()> {
    let module = ctx
        .vbc_module()
        .or_internal("CloneTyped requires declaration metadata")?;
    let plan = match verum_vbc::copy_plan::build(module, ty) {
        Ok(plan) => plan,
        Err(reason) if !ty.is_generic() => return Err(LlvmLoweringError::Internal(reason)),
        Err(reason) => {
            emit_runtime_abort(ctx, &reason, "copy_unresolved")?;
            ctx.set_register(dst.0, ctx.types().i64_type().const_zero().into());
            return Ok(());
        }
    };
    let source = ctx.get_register(src.0)?;
    if matches!(plan.nodes[0], CopyNode::Identity) {
        ctx.set_register(dst.0, source);
        return Ok(());
    }
    let counters = if plan
        .nodes
        .iter()
        .any(|node| matches!(node, CopyNode::Shared { .. }))
    {
        Some(
            ctx.retained_counters
                .as_ref()
                .or_internal("typed copy needs module-owned counters")?
                .get()?,
        )
    } else {
        None
    };
    let context = ctx.llvm_context();
    let llvm_module = ctx.get_module();
    let i64_type = context.i64_type();
    let ptr_type = context.ptr_type(Default::default());
    let function = llvm_module.add_function(
        "verum_typed_value_copy",
        context.i32_type().fn_type(
            &[
                i64_type.into(),
                ptr_type.into(),
                i64_type.into(),
                i64_type.into(),
            ],
            false,
        ),
        Some(Linkage::Internal),
    );
    let builder = context.create_builder();
    let entry = context.append_basic_block(function, "entry");
    let top = context.append_basic_block(function, "copy_top");
    let finish = context.append_basic_block(function, "copy_finish");
    let fail = context.append_basic_block(function, "copy_refused");
    builder.position_at_end(entry);
    let stack = builder.build_alloca(i64_type, "copy_stack").or_llvm_err()?;
    let capacity = builder
        .build_alloca(i64_type, "copy_capacity")
        .or_llvm_err()?;
    let depth = builder.build_alloca(i64_type, "copy_depth").or_llvm_err()?;
    let active = builder
        .build_alloca(i64_type, "copy_active")
        .or_llvm_err()?;
    let active_capacity = builder
        .build_alloca(i64_type, "copy_active_capacity")
        .or_llvm_err()?;
    let active_len = builder
        .build_alloca(i64_type, "copy_active_len")
        .or_llvm_err()?;
    let work = builder.build_alloca(i64_type, "copy_work").or_llvm_err()?;
    let bytes = builder.build_alloca(i64_type, "copy_bytes").or_llvm_err()?;
    let failure = builder
        .build_alloca(context.i32_type(), "copy_failure")
        .or_llvm_err()?;
    builder
        .build_store(
            failure,
            context
                .i32_type()
                .const_int(CopyFailure::InvalidStorage as u64, false),
        )
        .or_llvm_err()?;
    let result = builder
        .build_alloca(i64_type, "copy_result")
        .or_llvm_err()?;
    let journal = builder
        .build_alloca(i64_type, "copy_journal")
        .or_llvm_err()?;
    let journal_capacity = builder
        .build_alloca(i64_type, "copy_journal_capacity")
        .or_llvm_err()?;
    let journal_len = builder
        .build_alloca(i64_type, "copy_journal_len")
        .or_llvm_err()?;
    let max_work = function
        .get_nth_param(2)
        .or_internal("copy work policy")?
        .into_int_value();
    let e = Emitter {
        context,
        module: llvm_module,
        builder,
        function,
        stack,
        capacity,
        depth,
        active,
        active_capacity,
        active_len,
        work,
        bytes,
        top,
        finish,
        fail,
        failure,
        declarations: module,
        journal,
        journal_capacity,
        journal_len,
        max_work,
        counters,
    };
    for cell in [
        stack,
        capacity,
        active,
        active_capacity,
        active_len,
        journal,
        journal_capacity,
        journal_len,
        result,
    ] {
        e.store(cell, 0, e.integer(0))?;
    }
    e.store(depth, 0, e.integer(1))?;
    e.store(work, 0, max_work)?;
    e.store(
        bytes,
        0,
        function
            .get_nth_param(3)
            .or_internal("copy allocation policy")?
            .into_int_value(),
    )?;
    e.bounded(
        max_work,
        u64::MAX / (FRAME_BYTES.max(JOURNAL_BYTES) * 4),
        "copy_workspace_representable",
    )?;
    e.workspace(stack, capacity, e.integer(1), FRAME_BYTES, max_work)?;
    let allocation = e.pointer(e.load(stack, 0)?)?;
    e.initialize(
        allocation,
        function
            .get_first_param()
            .or_internal("copy source")?
            .into_int_value(),
        result,
        NodeId(0),
    )?;
    e.jump(top)?;
    e.builder.position_at_end(top);
    let index = e
        .builder
        .build_int_sub(e.load(depth, 0)?, e.integer(1), "copy_top_index")
        .or_llvm_err()?;
    let frame = e.frame(index)?;
    let node = e.load(frame, 16)?;
    let phase = e.load(frame, 24)?;
    let enter = e.block("copy_enter");
    let children = e.block("copy_children");
    e.builder
        .build_conditional_branch(e.equal(phase, e.integer(u64::MAX))?, enter, children)
        .or_llvm_err()?;
    e.builder.position_at_end(enter);
    e.charge(work, e.integer(1), "copy_work_available")?;
    let cases: List<_> = plan
        .nodes
        .iter()
        .enumerate()
        .map(|(i, _)| (e.integer(i as u64), e.block("copy_node")))
        .collect();
    e.builder
        .build_switch(node, fail, cases.as_slice())
        .or_llvm_err()?;
    for ((_, block), (index, node)) in cases.iter().zip(plan.nodes.iter().enumerate()) {
        e.builder.position_at_end(*block);
        e.enter(frame, node, index, &plan)?;
    }
    e.builder.position_at_end(children);
    let child = e.block("copy_select_child");
    e.builder
        .build_conditional_branch(e.equal(phase, e.load(frame, 56)?)?, finish, child)
        .or_llvm_err()?;
    e.builder.position_at_end(child);
    let cases: List<_> = plan
        .nodes
        .iter()
        .enumerate()
        .map(|(i, _)| (e.integer(i as u64), e.block("copy_child_node")))
        .collect();
    e.builder
        .build_switch(node, fail, cases.as_slice())
        .or_llvm_err()?;
    for ((_, block), node) in cases.iter().zip(&plan.nodes) {
        e.builder.position_at_end(*block);
        e.children(frame, node)?;
    }
    e.builder.position_at_end(finish);
    e.active_remove(frame)?;
    e.store(e.pointer(e.load(frame, 8)?)?, 0, e.load(frame, 32)?)?;
    e.store(depth, 0, index)?;
    let done = e.block("copy_done");
    e.builder
        .build_conditional_branch(e.equal(index, e.integer(0))?, done, top)
        .or_llvm_err()?;
    e.builder.position_at_end(done);
    e.commit()?;
    // No allocation, user callback, or fallible publication follows retain.
    let output = function
        .get_nth_param(1)
        .or_internal("copy output")?
        .into_pointer_value();
    e.store(output, 0, e.load(result, 0)?)?;
    e.workspace_release(active, active_capacity, 8)?;
    e.workspace_release(stack, capacity, FRAME_BYTES)?;
    e.workspace_release(journal, journal_capacity, JOURNAL_BYTES)?;
    e.builder
        .build_return(Some(&context.i32_type().const_zero()))
        .or_llvm_err()?;
    e.builder.position_at_end(fail);
    let rolled_back = e.rollback()?;
    let failure = e
        .builder
        .build_load(context.i32_type(), e.failure, "copy_failure_reason")
        .or_llvm_err()?
        .into_int_value();
    let failure = e
        .builder
        .build_select(
            rolled_back,
            failure,
            context
                .i32_type()
                .const_int(CopyFailure::RollbackFailure as u64, false),
            "copy_final_failure",
        )
        .or_llvm_err()?
        .into_int_value();
    e.workspace_release(active, active_capacity, 8)?;
    e.workspace_release(stack, capacity, FRAME_BYTES)?;
    e.workspace_release(journal, journal_capacity, JOURNAL_BYTES)?;
    e.builder.build_return(Some(&failure)).or_llvm_err()?;
    let pointer = as_ptr(ctx, source, "typed_copy_source")?;
    let source = ctx
        .builder()
        .build_ptr_to_int(pointer, i64_type, "typed_copy_source_word")
        .or_llvm_err()?;
    let output = ctx
        .builder()
        .build_alloca(i64_type, "typed_copy_output")
        .or_llvm_err()?;
    let limits = ctx.typed_copy_limits;
    let success = ctx
        .builder()
        .build_call(
            function,
            &[
                source.into(),
                output.into(),
                i64_type.const_int(limits.max_work as u64, false).into(),
                i64_type
                    .const_int(limits.max_allocation_bytes as u64, false)
                    .into(),
            ],
            "typed_copy",
        )
        .or_llvm_err()?
        .basic_value_or("typed copy status")?
        .into_int_value();
    let accepted = ctx
        .builder()
        .build_int_compare(
            IntPredicate::EQ,
            success,
            context.i32_type().const_zero(),
            "typed_copy_succeeded",
        )
        .or_llvm_err()?;
    let done = context.append_basic_block(ctx.function(), "typed_copy_published");
    let refused = context.append_basic_block(ctx.function(), "typed_copy_refused");
    ctx.builder()
        .build_conditional_branch(accepted, done, refused)
        .or_llvm_err()?;
    ctx.builder().position_at_end(refused);
    let unknown = context.append_basic_block(ctx.function(), "typed_copy_unknown_failure");
    let cases: List<_> = CopyFailure::ALL
        .iter()
        .map(|reason| {
            (
                context.i32_type().const_int(*reason as u64, false),
                context.append_basic_block(ctx.function(), "typed_copy_failure"),
            )
        })
        .collect();
    ctx.builder()
        .build_switch(success, unknown, &cases)
        .or_llvm_err()?;
    for ((_, block), reason) in cases.iter().zip(CopyFailure::ALL) {
        ctx.builder().position_at_end(*block);
        emit_runtime_abort(ctx, reason.message(), "typed_copy_failure")?;
        ctx.builder().build_unreachable().or_llvm_err()?;
    }
    ctx.builder().position_at_end(unknown);
    emit_runtime_abort(
        ctx,
        "typed copy returned an invalid failure code",
        "typed_copy_failure",
    )?;
    ctx.builder().build_unreachable().or_llvm_err()?;
    ctx.builder().position_at_end(done);
    let value = ctx
        .builder()
        .build_load(i64_type, output, "typed_copy_result_word")
        .or_llvm_err()?
        .into_int_value();
    let value = ctx
        .builder()
        .build_int_to_ptr(value, ptr_type, "typed_copy_result")
        .or_llvm_err()?;
    ctx.set_register(dst.0, value.into());
    Ok(())
}
