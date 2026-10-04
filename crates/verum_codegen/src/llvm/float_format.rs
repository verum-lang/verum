//! Integer-only f64 shortest-decimal emission, with Verum Display/Debug policy.
//!
//! The interval algorithm and split powers are derived from Ryū, Copyright
//! 2018 Ulf Adams, via ryu 1.0.23. See float_format_tables.rs and its license.
//! Decimal output ties use greater magnitude, matching interpreter Display;
//! binary interval endpoint acceptance retains Ryū's ties-to-even rule.
use super::error::{BuildExt, CallSiteExt, OptionExt, Result};
use verum_common::List;
use verum_llvm::{AddressSpace, IntPredicate};
use verum_llvm::{
    basic_block::BasicBlock,
    builder::Builder,
    context::Context,
    module::{Linkage, Module},
    types::FunctionType,
    values::{FunctionValue, IntValue, PointerValue},
};
#[path = "float_format_tables.rs"]
mod tables;

/// Includes the trailing NUL; negative minimum subnormal needs 327 bytes.
pub(super) const BUFFER_CAPACITY: u32 = 328;

// Small, local builder helpers keep the interval equations visible. Runtime
// quantities are i64 (including signed exponents); predicates remain i1.
struct Emit<'a, 'ctx> {
    context: &'ctx Context,
    module: &'a Module<'ctx>,
    builder: Builder<'ctx>,
    function: FunctionValue<'ctx>,
}
impl<'a, 'ctx> Emit<'a, 'ctx> {
    fn new(
        context: &'ctx Context,
        module: &'a Module<'ctx>,
        function: FunctionValue<'ctx>,
    ) -> Self {
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(function, "entry"));
        Self {
            context,
            module,
            builder,
            function,
        }
    }
    fn c(&self, n: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(n, false)
    }
    fn block(&self, n: &str) -> BasicBlock<'ctx> {
        self.context.append_basic_block(self.function, n)
    }
    fn at(&self, b: BasicBlock<'ctx>) {
        self.builder.position_at_end(b)
    }
    fn jump(&self, b: BasicBlock<'ctx>) -> Result<()> {
        self.builder.build_unconditional_branch(b).or_llvm_err()?;
        Ok(())
    }
    fn branch(&self, c: IntValue<'ctx>, a: BasicBlock<'ctx>, b: BasicBlock<'ctx>) -> Result<()> {
        self.builder
            .build_conditional_branch(c, a, b)
            .or_llvm_err()?;
        Ok(())
    }
    fn add(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_int_add(a, b, "add").or_llvm_err()
    }
    fn sub(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_int_sub(a, b, "sub").or_llvm_err()
    }
    fn mul(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_int_mul(a, b, "mul").or_llvm_err()
    }
    fn div(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_int_unsigned_div(a, b, "div")
            .or_llvm_err()
    }
    fn rem(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_int_unsigned_rem(a, b, "rem")
            .or_llvm_err()
    }
    fn shr(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_right_shift(a, b, false, "shr")
            .or_llvm_err()
    }
    fn and(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_and(a, b, "and").or_llvm_err()
    }
    fn or(&self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_or(a, b, "or").or_llvm_err()
    }
    fn not(&self, a: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_not(a, "not").or_llvm_err()
    }
    fn cmp(&self, p: IntPredicate, a: IntValue<'ctx>, b: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder.build_int_compare(p, a, b, "cmp").or_llvm_err()
    }
    fn select(
        &self,
        c: IntValue<'ctx>,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_select(c, a, b, "select")
            .or_llvm_err()?
            .into_int_value())
    }
    fn widen(&self, a: IntValue<'ctx>) -> Result<IntValue<'ctx>> {
        self.builder
            .build_int_z_extend(a, self.context.i64_type(), "wide")
            .or_llvm_err()
    }
    fn slot(&self, v: IntValue<'ctx>, name: &str) -> Result<PointerValue<'ctx>> {
        let p = self
            .builder
            .build_alloca(v.get_type(), name)
            .or_llvm_err()?;
        self.store(p, v)?;
        Ok(p)
    }
    fn store(&self, p: PointerValue<'ctx>, v: IntValue<'ctx>) -> Result<()> {
        self.builder.build_store(p, v).or_llvm_err()?;
        Ok(())
    }
    fn load(&self, p: PointerValue<'ctx>) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_load(self.context.i64_type(), p, "load")
            .or_llvm_err()?
            .into_int_value())
    }
    fn flag(&self, p: PointerValue<'ctx>) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_load(self.context.bool_type(), p, "flag")
            .or_llvm_err()?
            .into_int_value())
    }
    fn call(
        &self,
        f: FunctionValue<'ctx>,
        args: &[verum_llvm::values::BasicMetadataValueEnum<'ctx>],
    ) -> Result<IntValue<'ctx>> {
        Ok(self
            .builder
            .build_call(f, args, "call")
            .or_llvm_err()?
            .basic_value_or("numeric helper returned void")?
            .into_int_value())
    }
    fn ret(&self, v: IntValue<'ctx>) -> Result<()> {
        self.builder.build_return(Some(&v)).or_llvm_err()?;
        Ok(())
    }
    fn byte(
        &self,
        buf: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        value: IntValue<'ctx>,
    ) -> Result<()> {
        // SAFETY: the caller provides BUFFER_CAPACITY; every index is bounded
        // by the f64 fixed-notation extent proved in the formatting contract.
        let p = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[index], "byte")
                .or_llvm_err()?
        };
        let byte = self
            .builder
            .build_int_truncate(value, self.context.i8_type(), "ascii")
            .or_llvm_err()?;
        // Prevent LoopIdiomRecognize from introducing an external memset for
        // the variable-length zero run. These are at most 327 private bytes.
        self.builder
            .build_store(p, byte)
            .or_llvm_err()?
            .set_volatile(true)
            .or_llvm_err()?;
        Ok(())
    }
    fn literal(&self, buf: PointerValue<'ctx>, offset: IntValue<'ctx>, text: &[u8]) -> Result<()> {
        for (i, c) in text.iter().enumerate() {
            self.byte(
                buf,
                self.add(offset, self.c(i as u64))?,
                self.c(u64::from(*c)),
            )?;
        }
        Ok(())
    }
    fn table(
        &self,
        name: &str,
        values: &[(u64, u64)],
        index: IntValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>)> {
        let i64t = self.context.i64_type();
        let pair = i64t.array_type(2);
        let array = pair.array_type(values.len() as u32);
        let global = if let Some(g) = self.module.get_global(name) {
            g
        } else {
            let constants: List<_> = values
                .iter()
                .map(|(lo, hi)| i64t.const_array(&[self.c(*lo), self.c(*hi)]))
                .collect();
            let g = self.module.add_global(array, None, name);
            g.set_initializer(&pair.const_array(&constants));
            g.set_constant(true);
            g.set_linkage(Linkage::Internal);
            g.set_unnamed_addr(true);
            g
        };
        // SAFETY: Ryū's encoded exponent ranges bound inverse indices to 0..290
        // and power indices to 1..325; the full tables contain 342/326 rows.
        let at = |part| unsafe {
            self.builder
                .build_gep(
                    array,
                    global.as_pointer_value(),
                    &[self.c(0), index, self.c(part)],
                    "power",
                )
                .or_llvm_err()
        };
        Ok((self.load(at(0)?)?, self.load(at(1)?)?))
    }
}
fn declare<'ctx>(module: &Module<'ctx>, name: &str, ty: FunctionType<'ctx>) -> FunctionValue<'ctx> {
    let f = super::error::get_or_declare_function(module, name, ty);
    f.set_linkage(Linkage::Internal);
    f
}

fn mul_shift<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
    let t = context.i64_type();
    let f = declare(
        module,
        "verum_decimal_mul_shift",
        t.fn_type(&[t.into(), t.into(), t.into(), t.into()], false),
    );
    if f.count_basic_blocks() != 0 {
        return Ok(f);
    }
    let e = Emit::new(context, module, f);
    let wide = context.custom_width_int_type(128);
    let p = |n| {
        f.get_nth_param(n)
            .or_internal("decimal multiply parameter")
            .map(|v| v.into_int_value())
    };
    let m = e
        .builder
        .build_int_z_extend(p(0)?, wide, "m")
        .or_llvm_err()?;
    let lo = e
        .builder
        .build_int_z_extend(p(1)?, wide, "lo")
        .or_llvm_err()?;
    let hi = e
        .builder
        .build_int_z_extend(p(2)?, wide, "hi")
        .or_llvm_err()?;
    let low = e.mul(m, lo)?;
    let high = e.mul(m, hi)?;
    let sum = e.add(e.shr(low, wide.const_int(64, false))?, high)?;
    let shift = e
        .builder
        .build_int_z_extend(e.sub(p(3)?, e.c(64))?, wide, "shift")
        .or_llvm_err()?;
    e.ret(
        e.builder
            .build_int_truncate(e.shr(sum, shift)?, t, "significand")
            .or_llvm_err()?,
    )?;
    Ok(f)
}
fn multiple_of_5<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<FunctionValue<'ctx>> {
    let t = context.i64_type();
    let f = declare(
        module,
        "verum_decimal_multiple_of_5",
        context.bool_type().fn_type(&[t.into(), t.into()], false),
    );
    if f.count_basic_blocks() != 0 {
        return Ok(f);
    }
    let e = Emit::new(context, module, f);
    let value = e.slot(
        f.get_nth_param(0)
            .or_internal("power5 value")?
            .into_int_value(),
        "value",
    )?;
    let power = e.slot(
        f.get_nth_param(1)
            .or_internal("power5 exponent")?
            .into_int_value(),
        "power",
    )?;
    let test = e.block("test");
    let check = e.block("check");
    let step = e.block("step");
    let yes = e.block("yes");
    let no = e.block("no");
    e.jump(test)?;
    e.at(test);
    e.branch(e.cmp(IntPredicate::EQ, e.load(power)?, e.c(0))?, yes, check)?;
    e.at(check);
    e.branch(
        e.cmp(IntPredicate::EQ, e.rem(e.load(value)?, e.c(5))?, e.c(0))?,
        step,
        no,
    )?;
    e.at(step);
    e.store(value, e.div(e.load(value)?, e.c(5))?)?;
    e.store(power, e.sub(e.load(power)?, e.c(1))?)?;
    e.jump(test)?;
    e.at(yes);
    e.ret(context.bool_type().const_int(1, false))?;
    e.at(no);
    e.ret(context.bool_type().const_zero())?;
    Ok(f)
}

/// Nonzero finite mantissa/exponent → coefficient; writes decimal exponent.
fn coefficient<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<FunctionValue<'ctx>> {
    let t = context.i64_type();
    let pt = context.ptr_type(AddressSpace::default());
    let f = declare(
        module,
        "verum_f64_decimal_coefficient",
        t.fn_type(&[t.into(), t.into(), pt.into()], false),
    );
    if f.count_basic_blocks() != 0 {
        return Ok(f);
    }
    let multiply = mul_shift(context, module)?;
    let power5 = multiple_of_5(context, module)?;
    let e = Emit::new(context, module, f);
    let p = |n| {
        f.get_nth_param(n)
            .or_internal("decimal coefficient parameter")
    };
    let mantissa = p(0)?.into_int_value();
    let exponent = p(1)?.into_int_value();
    let output_exp = p(2)?.into_pointer_value();
    let denormal = e.cmp(IntPredicate::EQ, exponent, e.c(0))?;
    let e2 = e.select(
        denormal,
        e.c((-1076_i64) as u64),
        e.sub(exponent, e.c(1077))?,
    )?;
    let m2 = e.select(denormal, mantissa, e.or(mantissa, e.c(1 << 52))?)?;
    let accept = e.cmp(IntPredicate::EQ, e.and(m2, e.c(1))?, e.c(0))?;
    let mm_shift = e.widen(e.or(
        e.cmp(IntPredicate::NE, mantissa, e.c(0))?,
        e.cmp(IntPredicate::ULE, exponent, e.c(1))?,
    )?)?;
    let mv = e.mul(m2, e.c(4))?;
    let lower = e.sub(e.sub(mv, e.c(1))?, mm_shift)?;
    let upper = e.add(mv, e.c(2))?;
    let vr = e.slot(e.c(0), "vr")?;
    let vp = e.slot(e.c(0), "vp")?;
    let vm = e.slot(e.c(0), "vm")?;
    let e10 = e.slot(e.c(0), "e10")?;
    let vm_zeros = e.slot(context.bool_type().const_zero(), "lower_trailing_zeros")?;
    let removed = e.slot(e.c(0), "removed")?;
    let last = e.slot(e.c(0), "last")?;
    let pos = e.block("positive_exponent");
    let neg = e.block("negative_exponent");
    let trim = e.block("trim");
    let set_interval = |lo: IntValue<'ctx>, hi: IntValue<'ctx>, j: IntValue<'ctx>| -> Result<()> {
        for (dst, m) in [(vr, mv), (vp, upper), (vm, lower)] {
            e.store(
                dst,
                e.call(multiply, &[m.into(), lo.into(), hi.into(), j.into()])?,
            )?;
        }
        Ok(())
    };
    let pow5_bits = |q| e.add(e.shr(e.mul(q, e.c(1217359))?, e.c(19))?, e.c(1));
    e.branch(e.cmp(IntPredicate::SGE, e2, e.c(0))?, pos, neg)?;
    e.at(pos);
    let q = e.sub(
        e.shr(e.mul(e2, e.c(78913))?, e.c(18))?,
        e.widen(e.cmp(IntPredicate::UGT, e2, e.c(3))?)?,
    )?;
    e.store(e10, q)?;
    let k = e.add(e.c(124), pow5_bits(q)?)?;
    let j = e.add(e.sub(q, e2)?, k)?;
    let (lo, hi) = e.table("verum_decimal_inv5", &tables::DOUBLE_POW5_INV_SPLIT, q)?;
    set_interval(lo, hi, j)?;
    let small = e.block("positive_small");
    let lower_check = e.block("positive_lower");
    let upper_check = e.block("positive_upper");
    e.branch(e.cmp(IntPredicate::ULE, q, e.c(21))?, small, trim)?;
    e.at(small);
    // With greater-magnitude decimal ties, the center's trailing-zero flag
    // does not affect the general removal path. Its multiple-of-five branch
    // still excludes adjusting either interval endpoint.
    let adjust = e.block("positive_adjust");
    e.branch(
        e.cmp(IntPredicate::EQ, e.rem(mv, e.c(5))?, e.c(0))?,
        trim,
        adjust,
    )?;
    e.at(adjust);
    e.branch(accept, lower_check, upper_check)?;
    e.at(lower_check);
    e.store(vm_zeros, e.call(power5, &[lower.into(), q.into()])?)?;
    e.jump(trim)?;
    e.at(upper_check);
    let delta = e.widen(e.call(power5, &[upper.into(), q.into()])?)?;
    e.store(vp, e.sub(e.load(vp)?, delta)?)?;
    e.jump(trim)?;
    e.at(neg);
    let ne = e.sub(e.c(0), e2)?;
    let q = e.sub(
        e.shr(e.mul(ne, e.c(732923))?, e.c(20))?,
        e.widen(e.cmp(IntPredicate::UGT, ne, e.c(1))?)?,
    )?;
    e.store(e10, e.add(q, e2)?)?;
    let i = e.sub(ne, q)?;
    let k = e.sub(pow5_bits(i)?, e.c(125))?;
    let j = e.sub(q, k)?;
    let (lo, hi) = e.table("verum_decimal_pow5", &tables::DOUBLE_POW5_SPLIT, i)?;
    set_interval(lo, hi, j)?;
    let neg_small = e.block("negative_small");
    let neg_even = e.block("negative_even");
    let neg_odd = e.block("negative_odd");
    e.branch(e.cmp(IntPredicate::ULE, q, e.c(1))?, neg_small, trim)?;
    e.at(neg_small);
    e.branch(accept, neg_even, neg_odd)?;
    e.at(neg_even);
    e.store(vm_zeros, e.cmp(IntPredicate::EQ, mm_shift, e.c(1))?)?;
    e.jump(trim)?;
    e.at(neg_odd);
    e.store(vp, e.sub(e.load(vp)?, e.c(1))?)?;
    e.jump(trim)?;
    // Use Ryū's general digit-removal loop for every value. The common-case
    // two-digit optimization is omitted; it changes work, not the interval.
    let step = e.block("trim_step");
    let extra_test = e.block("lower_zero_test");
    let extra = e.block("lower_zero_step");
    let finish = e.block("finish");
    e.at(trim);
    e.branch(
        e.cmp(
            IntPredicate::UGT,
            e.div(e.load(vp)?, e.c(10))?,
            e.div(e.load(vm)?, e.c(10))?,
        )?,
        step,
        extra_test,
    )?;
    let remove_digit = || -> Result<()> {
        e.store(last, e.rem(e.load(vr)?, e.c(10))?)?;
        for p in [vr, vp, vm] {
            e.store(p, e.div(e.load(p)?, e.c(10))?)?;
        }
        e.store(removed, e.add(e.load(removed)?, e.c(1))?)?;
        Ok(())
    };
    e.at(step);
    e.store(
        vm_zeros,
        e.and(
            e.flag(vm_zeros)?,
            e.cmp(IntPredicate::EQ, e.rem(e.load(vm)?, e.c(10))?, e.c(0))?,
        )?,
    )?;
    remove_digit()?;
    e.jump(trim)?;
    e.at(extra_test);
    e.branch(
        e.and(
            e.flag(vm_zeros)?,
            e.cmp(IntPredicate::EQ, e.rem(e.load(vm)?, e.c(10))?, e.c(0))?,
        )?,
        extra,
        finish,
    )?;
    e.at(extra);
    remove_digit()?;
    e.jump(extra_test)?;
    e.at(finish);
    let excluded = e.and(
        e.cmp(IntPredicate::EQ, e.load(vr)?, e.load(vm)?)?,
        e.or(e.not(accept)?, e.not(e.flag(vm_zeros)?)?)?,
    )?;
    let round = e.or(excluded, e.cmp(IntPredicate::UGE, e.load(last)?, e.c(5))?)?;
    e.store(output_exp, e.add(e.load(e10)?, e.load(removed)?)?)?;
    e.ret(e.add(e.load(vr)?, e.widen(round)?)?)?;
    Ok(f)
}

/// `(buf: ptr, ieee_bits: i64, debug: i1) -> length`, without NUL. Every caller
/// supplies BUFFER_CAPACITY bytes; special values and zero bypass the kernel.
pub(super) fn get_or_declare<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<FunctionValue<'ctx>> {
    let t = context.i64_type();
    let pt = context.ptr_type(AddressSpace::default());
    let f = declare(
        module,
        "verum_internal_f64_to_decimal",
        t.fn_type(&[pt.into(), t.into(), context.bool_type().into()], false),
    );
    if f.count_basic_blocks() != 0 {
        return Ok(f);
    }
    let kernel = coefficient(context, module)?;
    let e = Emit::new(context, module, f);
    let buf = f
        .get_nth_param(0)
        .or_internal("float format buffer")?
        .into_pointer_value();
    let bits = f
        .get_nth_param(1)
        .or_internal("float format bits")?
        .into_int_value();
    let debug = f
        .get_nth_param(2)
        .or_internal("float format policy")?
        .into_int_value();
    let mantissa = e.and(bits, e.c((1 << 52) - 1))?;
    let exponent = e.and(e.shr(bits, e.c(52))?, e.c(2047))?;
    let sign = e.shr(bits, e.c(63))?;
    let special = e.block("special");
    let signed = e.block("signed");
    let nan = e.block("nan");
    let infinity = e.block("infinity");
    let finite = e.block("finite");
    let zero = e.block("zero");
    let nonzero = e.block("nonzero");
    // All stack slots precede loops; formatting inside a caller loop does not
    // accumulate per-digit allocations.
    let decimal_exp = e.slot(e.c(0), "decimal_exp")?;
    let count = e.slot(e.c(1), "digit_count")?;
    let temp = e.slot(e.c(0), "temp")?;
    let position = e.slot(e.c(0), "position")?;
    e.branch(
        e.cmp(IntPredicate::EQ, exponent, e.c(2047))?,
        special,
        signed,
    )?;
    e.at(special);
    e.branch(e.cmp(IntPredicate::NE, mantissa, e.c(0))?, nan, signed)?;
    e.at(nan);
    e.literal(buf, e.c(0), b"NaN")?;
    e.ret(e.c(3))?;
    e.at(signed);
    e.literal(buf, e.c(0), b"-")?;
    e.branch(
        e.cmp(IntPredicate::EQ, exponent, e.c(2047))?,
        infinity,
        finite,
    )?;
    e.at(infinity);
    e.literal(buf, sign, b"inf")?;
    e.ret(e.add(sign, e.c(3))?)?;
    e.at(finite);
    e.branch(
        e.cmp(IntPredicate::EQ, e.or(mantissa, exponent)?, e.c(0))?,
        zero,
        nonzero,
    )?;
    e.at(zero);
    e.literal(buf, sign, b"0")?;
    let zero_debug = e.block("zero_debug");
    let zero_display = e.block("zero_display");
    e.branch(debug, zero_debug, zero_display)?;
    e.at(zero_debug);
    e.literal(buf, e.add(sign, e.c(1))?, b".0")?;
    e.ret(e.add(sign, e.c(3))?)?;
    e.at(zero_display);
    e.ret(e.add(sign, e.c(1))?)?;
    e.at(nonzero);
    let coefficient = e.call(
        kernel,
        &[mantissa.into(), exponent.into(), decimal_exp.into()],
    )?;
    e.store(temp, coefficient)?;
    let count_test = e.block("count_test");
    let count_step = e.block("count_step");
    let extent = e.block("extent");
    e.jump(count_test)?;
    e.at(count_test);
    e.branch(
        e.cmp(IntPredicate::UGE, e.load(temp)?, e.c(10))?,
        count_step,
        extent,
    )?;
    e.at(count_step);
    e.store(temp, e.div(e.load(temp)?, e.c(10))?)?;
    e.store(count, e.add(e.load(count)?, e.c(1))?)?;
    e.jump(count_test)?;
    e.at(extent);
    let digits = e.load(count)?;
    let point = e.add(digits, e.load(decimal_exp)?)?;
    let leading = e.cmp(IntPredicate::SLE, point, e.c(0))?;
    let integral = e.cmp(IntPredicate::SGE, point, digits)?;
    let body = e.select(
        leading,
        e.sub(e.add(digits, e.c(2))?, point)?,
        e.select(integral, point, e.add(digits, e.c(1))?)?,
    )?;
    let add_dot = e.and(debug, integral)?;
    let total = e.add(e.add(sign, body)?, e.select(add_dot, e.c(2), e.c(0))?)?;
    e.store(position, sign)?;
    let fill_test = e.block("fill_test");
    let fill = e.block("fill");
    let dot = e.block("dot");
    let put_dot = e.block("put_dot");
    let digit_begin = e.block("digit_begin");
    let digit_loop = e.block("digit_loop");
    let done = e.block("done");
    e.jump(fill_test)?;
    e.at(fill_test);
    e.branch(
        e.cmp(IntPredicate::ULT, e.load(position)?, total)?,
        fill,
        dot,
    )?;
    e.at(fill);
    e.byte(buf, e.load(position)?, e.c(u64::from(b'0')))?;
    e.store(position, e.add(e.load(position)?, e.c(1))?)?;
    e.jump(fill_test)?;
    e.at(dot);
    e.branch(e.or(e.not(integral)?, debug)?, put_dot, digit_begin)?;
    e.at(put_dot);
    let dot_pos = e.select(leading, e.c(1), e.select(integral, body, point)?)?;
    e.byte(buf, e.add(sign, dot_pos)?, e.c(u64::from(b'.')))?;
    e.jump(digit_begin)?;
    e.at(digit_begin);
    e.store(temp, coefficient)?;
    e.store(position, digits)?;
    e.jump(digit_loop)?;
    e.at(digit_loop);
    let index = e.sub(e.load(position)?, e.c(1))?;
    let after_point = e.widen(e.and(e.not(integral)?, e.cmp(IntPredicate::SGE, index, point)?)?)?;
    let digit_pos = e.select(
        leading,
        e.add(e.sub(e.c(2), point)?, index)?,
        e.add(index, after_point)?,
    )?;
    e.byte(
        buf,
        e.add(sign, digit_pos)?,
        e.add(e.rem(e.load(temp)?, e.c(10))?, e.c(u64::from(b'0')))?,
    )?;
    e.store(temp, e.div(e.load(temp)?, e.c(10))?)?;
    e.store(position, index)?;
    e.branch(e.cmp(IntPredicate::EQ, index, e.c(0))?, done, digit_loop)?;
    e.at(done);
    e.ret(total)?;
    Ok(f)
}
