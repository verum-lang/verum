# core/base Test Suite

Test coverage for Verum's base module - fundamental types and protocols.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `primitives_test.vr` | `base/primitives` | Primitive type methods and intrinsics | 239 |
| `iterator_test.vr` | `base/iterator` | Iterator protocol, adapters, collectors | 205 |
| `result_test.vr` | `base/result` | Result<T, E>, Error, Exit, Cause, Validated types | 169 |
| `data_test.vr` | `base/data` | Data type (JSON-like dynamic data), DataBuilder | 155 |
| `sized_integers_test.vr` | `base/primitives` | Int8-Int128, UInt8-UInt128 types | 112 |
| `display_test.vr` | `base/protocols` | Display/Debug for primitives, Maybe, Result, List, Map, Set, Deque, BinaryHeap, BTreeMap, BTreeSet | 102 |
| `iterator_adapters_comprehensive_test.vr` | `base/iterator` | Scan, Cycle, Chunks, Windows, Interleave, FlatMap, Flatten, Intersperse, MapWhile, TakeWhile, SkipWhile, StepBy, Dedup, FilterMap | 77 |
| `memory_test.vr` | `base/memory` | Heap, Shared, ManuallyDrop, MaybeUninit | 73 |
| `iterator_size_hint_exact_test.vr` | `base/iterator` | size_hint for all adapters, ExactSizeIterator, DEI for Enumerate/Take/Skip | 67 |
| `protocols_debug_test.vr` | `base/protocols` | Debug/Display formatting, min/max, Eq, Ord | 66 |
| `conversion_test.vr` | `base/protocols` | From/Into/TryFrom/TryInto conversions | 83 |
| `protocols_production_test.vr` | `base/protocols` | ErrorSource, FromStr, ToString, Borrow, compound assignments | 63 |
| `ordering_test.vr` | `base/ordering` | Ordering type and comparisons | 63 |
| `panic_test.vr` | `base/panic` | PanicInfo, Location, assert functions | 62 |
| `eq_ord_test.vr` | `base/protocols` | Eq and Ord protocols for primitives | 62 |
| `env_test.vr` | `base/env` | Environment variables and args | 62 |
| `maybe_test.vr` | `base/maybe` | Maybe<T> type - predicates, transformations, combinators | 61 |
| `exact_size_iterator_test.vr` | `base/iterator` | ExactSizeIterator for Range, RangeInclusive (signed types) | 60 |
| `return_test.vr` | `base/ops` | Return value semantics | 59 |
| `protocols_test.vr` | `base/protocols` | Core protocols - Clone, Hash, Debug, Display, Default, operators | 59 |
| `ops_test.vr` | `base/ops` | ControlFlow (Clone, Eq, Debug), Try, FromResidual | 65 |
| `iterator_extended_test.vr` | `base/iterator` | Extended iterator methods | 59 |
| `iterator_double_ended_adapters_test.vr` | `base/iterator` | DoubleEndedIterator for adapters, size_hint, FusedIterator | 59 |
| `iterator_collections_test.vr` | `base/iterator` | Iterator with collection types | 59 |
| `iterator_transducer_test.vr` | `base/iterator` | Transducer, StatefulTransducer, Reducer patterns | 49 |
| `result_advanced_test.vr` | `base/result` | Validated.and/map_errors/validate_all/sequence, Exit predicates, RetryBackoff | 34 |
| `iterator_comparison_unzip_test.vr` | `base/iterator` | lt/le/gt/ge comparison, unzip, rposition, try_rfold | 34 |
| `data_mutators_test.vr` | `base/data` | Data.as_array_mut, as_object_mut, get_mut, at_mut | 24 |
| `maybe_flatten_default_test.vr` | `base/maybe` | Maybe.flatten(), unwrap_or_default() with multiple types | 23 |
| `maybe_collect_test.vr` | `base/maybe` | collect_maybe, flatten_maybe, flatten_maybe_iter | 23 |
| `iterator_size_hint_test.vr` | `base/iterator` | size_hint for basic adapters | 23 |
| `simple_map_test.vr` | `base/protocols` | Simple map usage with protocols | 22 |
| `iterator_minmax_test.vr` | `base/iterator` | min_by, max_by, min_by_key, max_by_key | 21 |
| `env_protocols_test.vr` | `base/env` | VarError Debug, Display, Clone, Eq | 20 |
| `env_extended_test.vr` | `base/env` | Extended environment operations | 20 |
| `iterator_sum_by_test.vr` | `base/iterator` | sum_by, product_by, fold variants | 19 |
| `iterator_pairwise_test.vr` | `base/iterator` | Pairwise iterator adapter | 19 |
| `result_collect_test.vr` | `base/result` | collect_results, partition_results | 17 |
| `data_protocols_test.vr` | `base/data` | DataError Debug, Display for all 5 variants | 17 |
| `maybe_intoiter_test.vr` | `base/maybe` | Maybe<T> IntoIterator, MaybeIter, iterator adapters | 15 |
| `result_intoiter_test.vr` | `base/result` | Result<T,E> IntoIterator, ResultIter, iterator adapters | 14 |
| `result_default_test.vr` | `base/result` | Result.unwrap_or_default(), Default for Result<T,E> | 19 |
| `sized_integer_protocols_test.vr` | `base/primitives` | Arithmetic protocols (Add/Sub/Mul/Div/Rem), Zero/One, bitwise ops, compound assignments (incl. shifts), iterator sum/product for all sized types | 149 |
| `sized_integer_generic_test.vr` | `base/primitives` | Sized integers in generic contexts: Set, Map, sort, clone, Default | 20 |
| `format_sized_test.vr` | `base/primitives`, `text/format` | Display/Debug formatting via format_display/format_debug for all 13 sized numeric types + Float32, consistency checks | 38 |
| `array_proto_test.vr` | `base/primitives` | Array protocols: Eq, Clone, indexing, mutation, nesting, sized element types, arrays in collections | 20 |
| `numeric_conversions_test.vr` | `base/primitives` | Widening From conversions: Int8→Int16→Int32→Int64, UInt8→UInt16→UInt32→UInt64, Float32→Float, chained conversions, boundary values | 24 |
| `numeric_fromstr_test.vr` | `base/primitives`, `text/text` | FromStr for Int8/Int16/Int32/UInt8/UInt16/UInt32/UInt64: valid, boundaries, overflow, invalid input | 55 |
| `numeric_tryfrom_cross_test.vr` | `base/primitives` | Cross-type TryFrom narrowing: Int32→Int16/Int8, Int16→Int8, UInt64→UInt32, UInt32→UInt16/UInt8, UInt16→UInt8, signed↔unsigned | 44 |
| `iterator_constructors_test.vr` | `base/iterator` | repeat_with, once_with constructors, stateful closures, combinators, type variants, comparison with eager variants | 55 |
| `iterator_laws_test.vr` | `base/iterator` | Iterator algebraic laws: functor, identity, associativity, distributivity | 22 |
| `iterator_zip_longest_test.vr` | `base/iterator` | ZipLongestIter adapter: equal/unequal lengths, size_hint, count, fused, vs zip | 35 |
| `maybe_additional_test.vr` | `base/maybe` | zip_with, take_if, get_or_insert_default | 27 |
| `sized_integer_methods_test.vr` | `base/primitives` | Consistency gap fills: Int16/Int32/Int64/Int128 wrapping_mul/saturating_mul/count_zeros/rotate, UInt16 bit ops, ISize/USize full method suite | 102 |
| `numeric_tryfrom_test.vr` | `base/primitives` | TryFrom narrowing: Int→Int8/Int16/Int32/Int64/Int128/ISize, Int→UInt8/UInt16/UInt32/UInt64/UInt128/USize, boundary testing | 52 |
| `sized_integer_parity_test.vr` | `base/primitives` | pow, is_positive/is_negative, checked_div, in_range, to_float for all 12 sized integer types, edge cases | 56 |
| `float32_parity_test.vr` | `base/primitives` | Float32 parity: classification, signum, fract, clamp, copysign, exp_m1, ln_1p, log_base, powi, trig, to_degrees/to_radians, to_int, special values | 22 |
| `overflowing_arithmetic_test.vr` | `base/primitives` | overflowing_add/sub/mul for all 12 sized types, reverse_bits, to_binary/to_hex/to_octal base conversions | 71 |
| `float64_parity_test.vr` | `base/primitives` | Float64 parity: is_normal, exp_m1, ln_1p, log_base, sin_cos, special values, constants, copysign | 10 |
| `int8_uint8_bitops_test.vr` | `base/primitives` | Int8/UInt8 bit operations: leading/trailing zeros, count ones/zeros, rotate, swap_bytes, byte conversions | 25 |
| `integer_advanced_methods_test.vr` | `base/primitives` | checked_rem, wrapping_neg, checked_neg, abs_diff, ilog2, ilog10, is_power_of_two for all integer types | 82 |
| `missing_coverage_test.vr` | `base/primitives` | Bool.not(), Int/sized integer bitwise NOT, Char.escape_debug() | 20 |
| `next_power_of_two_test.vr` | `base/primitives` | next_power_of_two for UInt8/UInt16/UInt32/UInt64/USize, idempotency, edge cases | 13 |
| `checked_shift_pow_test.vr` | `base/primitives` | checked_shl, checked_shr, checked_pow for all 13 integer types, cross-type consistency, edge cases | 51 |
| `wrapping_euclid_test.vr` | `base/primitives` | wrapping_shl/shr, wrapping_div/rem, div_euclid/rem_euclid, saturating_pow, overflowing_shl/shr for all types | 52 |
| `midpoint_radix_test.vr` | `base/primitives` | midpoint for all types, Int.from_str_radix (bases 2-36), to_ne_bytes/from_ne_bytes | 30 |
| `float_midpoint_lerp_test.vr` | `base/primitives` | Float/Float32/Float64 midpoint, lerp, total_cmp, cross-type consistency | 45 |
| `float_extended_test.vr` | `base/primitives` | Float recip/div_euclid/rem_euclid/is_subnormal, Float64 parity (signum/fract/clamp/powi/to_degrees/to_radians), cross-type consistency | 52 |
| `numeric_fromstr_extended_test.vr` | `base/primitives`, `text/text` | FromStr for Int64/Int128/UInt128/ISize/USize/Float32/Float64: valid, invalid, range, cross-type | 35 |
| `int128_uint128_methods_test.vr` | `base/primitives` | Int128/UInt128 abs_diff, ilog10, next_power_of_two (UInt128), cross-type consistency | 24 |
| `from_str_radix_sized_test.vr` | `base/primitives` | from_str_radix for all 12 sized integer types: decimal, hex, binary, octal, base36, overflow, invalid input | 36 |
| `float_bytes_test.vr` | `base/primitives` | Float/Float32/Float64 to_le_bytes, to_be_bytes, from_le_bytes, from_be_bytes, to_ne_bytes, from_ne_bytes: roundtrips, endianness, special values | 51 |
| `primitive_protocol_compliance_test.vr` | `base/primitives` | Eq/Ord/Hash/Clone/Copy for all 12 sized integer types, Float32 PartialOrd, Byte Display | 66 |
| `float64_protocols_test.vr` | `base/primitives` | Float64 full protocol suite: Add/Sub/Mul/Div/Rem, Eq, PartialOrd, Clone/Copy, Default, Zero/One, special values, cross-type consistency | 40 |

## Test Count: 3,989 tests total (74 files)

## Key Types Tested

### Maybe<T>
- **Predicates**: is_some, is_none, is_some_and, is_none_or
- **Conversion**: ok_or, ok_or_else
- **Unwrapping**: unwrap, expect, unwrap_or, unwrap_or_else, unwrap_or_default
- **Transformations**: map, map_or, map_or_else, and_then, or_else, flatten
- **Combinators**: and, or, xor, zip, zip_with
- **References**: as_ref, as_mut, get_or_insert_default
- **Inspection**: inspect
- **Containment**: contains
- **Protocols**: Eq, Ord, Clone, Hash, Default
- **Iteration**: iter, into_iter
- **Collection helpers**: collect_maybe, flatten_maybe, flatten_maybe_iter

### Result<T, E>
- **Predicates**: is_ok, is_err, is_ok_and, is_err_and
- **Conversion to Maybe**: ok(), err()
- **Unwrapping**: unwrap_or, unwrap_or_else, unwrap_or_default
- **Transformations**: map, map_err, map_or, map_or_else, and_then, or_else, flatten
- **Combinators**: and, or
- **References**: as_ref, as_mut
- **Inspection**: inspect, inspect_err, tap, tap_err, tap_both
- **Recovery**: recover, recover_with
- **Containment**: contains, contains_err
- **Utilities**: flip, fold, context, with_context
- **Protocols**: Eq, Ord, Clone
- **Advanced**: Exit (is_success/is_failure/is_defect), Validated (value_or_default), Cause, RetryBackoff
- **Collection helpers**: collect_results, partition_results

### Iterator Protocol
- **Core methods**: next(), size_hint()
- **Transformations**: map, filter, filter_map, flatten, flat_map, scan
- **Folding**: fold, reduce, sum, product, sum_by, product_by
- **Collection**: collect, count
- **Searching**: find, position, any, all, min_by, max_by, min_by_key, max_by_key
- **Taking/Skipping**: take, skip, take_while, skip_while, step_by
- **Chaining**: chain, zip, enumerate, intersperse, intersperse_with, interleave
- **Windowing**: chunks, windows
- **Dedup**: dedup
- **Utilities**: nth, last, peek, fuse, cycle, map_while
- **Double-ended**: next_back, rfold, try_rfold, rposition
- **Comparison**: lt, le, gt, ge, unzip
- **Size**: size_hint (all adapters), ExactSizeIterator, FusedIterator
- **Transducers**: Transducer, StatefulTransducer, Reducer

### Data Type (data_test.vr + data_mutators_test.vr)
- **Variants**: Null, Bool, Int, Float, Text, Array, Object
- **Constructors**: null(), from_bool(), from_int(), from_float(), from_text(), from_array(), from_object(), empty_object(), empty_array()
- **Type predicates**: is_null, is_bool, is_int, is_float, is_number, is_text, is_array, is_object
- **Safe accessors**: as_bool, as_int, as_float, as_text, as_array, as_object
- **Mutable accessors**: as_array_mut, as_object_mut, get_mut, at_mut
- **Object operations**: get, contains_key, set, remove, values, keys
- **Array operations**: at, push, pop
- **Utilities**: len, is_empty, type_name
- **Conversion**: to_json, to_json_pretty, to_number, to_string, parse_json, merge, deep_merge
- **Protocols**: Clone, PartialEq, Default, Debug
- **DataBuilder**: fluent builder pattern for objects and arrays
- **DataError**: TypeMismatch, KeyNotFound, IndexOutOfBounds, ParseError, InvalidCast

## Known Limitations

- Closure type inference for method calls like `.len()` may fail - use explicit type annotations
- `ResultIter<T>` doesn't implement `IntoIterator`, use `.next()` directly
- Some advanced features require specific protocol bounds that may not be fully inferred
- Inline `fn(x) -> T { ... }` closure syntax not supported - use `|x| expr` or named functions
- `FromStr` associated type must be named `ParseError` (not `Err` - conflicts with Result::Err)
- `dyn Describable` casts not supported in type checker
- `.collect::<List<Int>>()` turbofish syntax not supported - use `let result: List<Int> = iter.collect();`

## Source Files Coverage

| File | Tested | Notes |
|------|--------|-------|
| maybe.vr | Yes | Full coverage in maybe_test.vr + maybe_flatten_default_test.vr |
| result.vr | Yes | Full coverage in result_test.vr + result_default_test.vr + result_advanced_test.vr |
| data.vr | Yes | Full coverage in data_test.vr + data_mutators_test.vr |
| iterator.vr | Yes | Core + adapters across 12 iterator test files |
| ordering.vr | Yes | Full coverage in ordering_test.vr |
| protocols.vr | Yes | Eq/Ord/Clone/Hash/Debug/Display + ErrorSource/FromStr/ToString/Borrow/Assignments |
| ops.vr | Yes | ControlFlow, Try, FromResidual in ops_test.vr |
| primitives.vr | Yes | Full coverage including all intrinsics in primitives_test.vr |
| panic.vr | Yes | PanicInfo, Location, assert functions in panic_test.vr |
| memory.vr | Yes | Heap, Shared, ManuallyDrop in memory_test.vr |
| env.vr | Yes | Environment operations in env_test.vr + env_extended_test.vr |
