//! Shared atomic ownership transitions for RetainCopy and typed-copy journals.
//!
//! The caller validates the reference and its UInt32 storage before calling
//! these helpers, and keeps the original owner alive through rollback. Neither
//! transition allocates, invokes user code, or restores an earlier count.
use super::error::{BuildExt, OptionExt, Result};
use verum_llvm::{
    AtomicOrdering, IntPredicate,
    context::ContextRef,
    module::{Linkage, Module},
    values::{BasicValue, FunctionValue},
};

/// Exact bodies owned by one live LLVM module. The module borrow prevents a
/// cache from outliving its owner; symbols cannot populate the cache.
///
/// ```compile_fail,E0505
/// use verum_codegen::llvm::retained_counter::RetainedCounters;
/// use verum_llvm::context::Context;
/// let context = Context::create();
/// let module = context.create_module("owner");
/// let counters = RetainedCounters::new(&module);
/// drop(module);
/// counters.get().unwrap();
/// ```
pub struct RetainedCounters<'module, 'ctx> {
    module: &'module Module<'ctx>,
    functions: CounterCache<'module, 'ctx>,
}

enum CounterCache<'module, 'ctx> {
    Owned(std::cell::OnceCell<EmittedCounters<'ctx>>),
    Lowering(&'module std::cell::OnceCell<EmittedCounters<'ctx>>),
}

/// Private to the native emitter. The driver owns this cell alongside its
/// module; only a borrowed view of that same owner reaches FunctionContext.
#[derive(Clone, Copy)]
pub(super) struct EmittedCounters<'ctx> {
    retain: FunctionValue<'ctx>,
    release: FunctionValue<'ctx>,
    owner_release: FunctionValue<'ctx>,
}
impl<'module, 'ctx> RetainedCounters<'module, 'ctx> {
    /// Create a cache whose lifetime is bound to one exact module.
    pub fn new(module: &'module Module<'ctx>) -> Self {
        Self {
            module,
            functions: CounterCache::Owned(Default::default()),
        }
    }

    /// The lowering driver supplies two fields of the same owner. This
    /// constructor is not exposed as a transferable public cache capability.
    pub(super) fn for_lowering(
        module: &'module Module<'ctx>,
        functions: &'module std::cell::OnceCell<EmittedCounters<'ctx>>,
    ) -> Self {
        Self {
            module,
            functions: CounterCache::Lowering(functions),
        }
    }

    /// Obtain the generated retain/release identities. The module is fixed at
    /// construction, never recovered by an address or supplied by a consumer.
    pub fn get(&self) -> Result<(FunctionValue<'ctx>, FunctionValue<'ctx>)> {
        let owned = self.emitted()?;
        Ok((owned.retain, owned.release))
    }

    /// Release one owner, including the last. Returns the previous positive
    /// count, or zero on refusal. Only a returned one authorizes destruction.
    pub(super) fn owner_release(&self) -> Result<FunctionValue<'ctx>> {
        Ok(self.emitted()?.owner_release)
    }

    fn emitted(&self) -> Result<EmittedCounters<'ctx>> {
        let functions = match &self.functions {
            CounterCache::Owned(functions) => functions,
            CounterCache::Lowering(functions) => functions,
        };
        if let Some(owned) = functions.get() {
            return Ok(*owned);
        }
        for name in ["verum_retain_u32_checked", "verum_release_local_retain_u32", "verum_release_owner_u32"] {
            if self.module.get_function(name).is_some() {
                return Err(super::error::LlvmLoweringError::internal(
                    "retained counter helper collides with a foreign declaration",
                ));
            }
        }
        let owned = EmittedCounters {
            retain: transition(self.module.get_context(), self.module, Transition::Retain)?,
            release: transition(self.module.get_context(), self.module, Transition::Rollback)?,
            owner_release: transition(self.module.get_context(), self.module, Transition::ReleaseOwner)?,
        };
        functions.set(owned).map_err(|_| {
            super::error::LlvmLoweringError::internal("retained counter emitter initialized twice")
        })?;
        Ok(owned)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transition { Retain, Rollback, ReleaseOwner }

fn transition<'ctx>(
    c: ContextRef<'ctx>,
    module: &Module<'ctx>,
    mode: Transition,
) -> Result<FunctionValue<'ctx>> {
    let acquire = mode == Transition::Retain;
    let final_release = mode == Transition::ReleaseOwner;
    let name = match mode {
        Transition::Retain => "verum_retain_u32_checked",
        Transition::Rollback => "verum_release_local_retain_u32",
        Transition::ReleaseOwner => "verum_release_owner_u32",
    };
    let ptr = c.ptr_type(Default::default());
    let i32_type = c.i32_type();
    let result_type = if final_release { i32_type } else { c.bool_type() };
    let signature = result_type.fn_type(&[ptr.into()], false);
    let function = module.add_function(name, signature, Some(Linkage::Internal));
    let b = c.create_builder();
    let entry = c.append_basic_block(function, "entry");
    let probe = c.append_basic_block(function, "probe");
    let cas = c.append_basic_block(function, "cas");
    let refused = c.append_basic_block(function, "refused");
    let accepted = c.append_basic_block(function, "accepted");
    b.position_at_end(entry);
    let counter = function
        .get_nth_param(0)
        .or_internal("retained counter parameter")?
        .into_pointer_value();
    let initial = b.build_load(i32_type, counter, "observed").or_llvm_err()?;
    initial
        .as_instruction_value()
        .or_internal("atomic counter load")?
        .set_atomic_ordering(AtomicOrdering::Monotonic)
        .map_err(|_| {
            super::error::LlvmLoweringError::internal("retained counter atomic ordering")
        })?;
    b.build_unconditional_branch(probe).or_llvm_err()?;
    b.position_at_end(probe);
    let phi = b.build_phi(i32_type, "current").or_llvm_err()?;
    phi.add_incoming(&[(&initial.into_int_value(), entry)]);
    let value = phi.as_basic_value().into_int_value();
    let valid = if acquire {
        let nonzero = b
            .build_int_compare(IntPredicate::NE, value, i32_type.const_zero(), "live")
            .or_llvm_err()?;
        let room = b
            .build_int_compare(IntPredicate::NE, value, i32_type.const_all_ones(), "room")
            .or_llvm_err()?;
        b.build_and(nonzero, room, "valid").or_llvm_err()?
    } else {
        b.build_int_compare(
            IntPredicate::UGT,
            value,
            i32_type.const_int(u64::from(!final_release), false),
            "release_preserves_owners",
        )
        .or_llvm_err()?
    };
    b.build_conditional_branch(valid, cas, refused)
        .or_llvm_err()?;
    b.position_at_end(cas);
    let next = if acquire {
        b.build_int_add(value, i32_type.const_int(1, false), "retained")
            .or_llvm_err()?
    } else {
        b.build_int_sub(value, i32_type.const_int(1, false), "released_local")
            .or_llvm_err()?
    };
    let result = b
        .build_cmpxchg(
            counter,
            value,
            next,
            if final_release { AtomicOrdering::AcquireRelease } else { AtomicOrdering::Monotonic },
            AtomicOrdering::Monotonic,
        )
        .or_llvm_err()?;
    let actual = b
        .build_extract_value(result, 0, "actual")
        .or_llvm_err()?
        .into_int_value();
    let success = b
        .build_extract_value(result, 1, "success")
        .or_llvm_err()?
        .into_int_value();
    phi.add_incoming(&[(&actual, cas)]);
    b.build_conditional_branch(success, accepted, probe)
        .or_llvm_err()?;
    b.position_at_end(refused);
    b.build_return(Some(&result_type.const_zero()))
        .or_llvm_err()?;
    b.position_at_end(accepted);
    b.build_return(Some(&if final_release { value } else { result_type.const_int(1, false) }))
        .or_llvm_err()?;
    Ok(function)
}
