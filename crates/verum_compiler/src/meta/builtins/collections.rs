//! Collection Operation Intrinsics (Tier 0 - Always Available)
//!
//! Pure collection functions that operate only on input values without
//! accessing any external state. These are always available in meta expressions.
//!
//! ## List Operations
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `list_len(list)` | `(List<T>) -> Int` | Get list length |
//! | `list_push(list, elem)` | `(List<T>, T) -> List<T>` | Push element (returns new list) |
//! | `list_get(list, index)` | `(List<T>, Int) -> T` | Get element at index |
//! | `list_map(list, fn)` | `(List<T>, fn(T) -> U) -> List<U>` | Map function over list |
//! | `list_filter(list, fn)` | `(List<T>, fn(T) -> Bool) -> List<T>` | Filter with predicate |
//! | `list_fold(list, init, fn)` | `(List<T>, U, fn(U, T) -> U) -> U` | Fold list |
//! | `list_concat(a, b)` | `(List<T>, List<T>) -> List<T>` | Concatenate lists |
//! | `list_reverse(list)` | `(List<T>) -> List<T>` | Reverse list |
//!
//! ## Map Operations
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `map_new()` | `() -> Map<Text, T>` | Create empty map |
//! | `map_len(map)` | `(Map<K, V>) -> Int` | Get map length |
//! | `map_get(map, key)` | `(Map<K, V>, K) -> Maybe<V>` | Get value by key |
//! | `map_insert(map, key, value)` | `(Map<K, V>, K, V) -> Map<K, V>` | Insert key-value |
//! | `map_remove(map, key)` | `(Map<K, V>, K) -> Map<K, V>` | Remove key |
//! | `map_contains(map, key)` | `(Map<K, V>, K) -> Bool` | Check if key exists |
//! | `map_keys(map)` | `(Map<K, V>) -> List<K>` | Get all keys |
//! | `map_values(map)` | `(Map<K, V>) -> List<V>` | Get all values |
//! | `map_entries(map)` | `(Map<K, V>) -> List<(K, V)>` | Get all entries |
//!
//! ## Set Operations
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `set_new()` | `() -> Set<Text>` | Create empty set |
//! | `set_len(set)` | `(Set<T>) -> Int` | Get set length |
//! | `set_insert(set, value)` | `(Set<T>, T) -> Set<T>` | Insert value |
//! | `set_remove(set, value)` | `(Set<T>, T) -> Set<T>` | Remove value |
//! | `set_contains(set, value)` | `(Set<T>, T) -> Bool` | Check if value exists |
//! | `set_to_list(set)` | `(Set<T>) -> List<T>` | Convert to list |
//! | `set_union(a, b)` | `(Set<T>, Set<T>) -> Set<T>` | Set union |
//! | `set_intersection(a, b)` | `(Set<T>, Set<T>) -> Set<T>` | Set intersection |
//! | `set_difference(a, b)` | `(Set<T>, Set<T>) -> Set<T>` | Set difference |
//!
//! ## Text Operations
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `text_concat(parts...)` | `(...Text) -> Text` | Concatenate text values |
//! | `text_len(text)` | `(Text) -> Int` | Get text length |
//! | `text_split(text, sep)` | `(Text, Text) -> List<Text>` | Split by separator |
//! | `text_join(list, sep)` | `(List<Text>, Text) -> Text` | Join with separator |
//! | `text_to_upper(text)` | `(Text) -> Text` | Convert to uppercase |
//! | `text_to_lower(text)` | `(Text) -> Text` | Convert to lowercase |
//! | `text_trim(text)` | `(Text) -> Text` | Trim whitespace |
//! | `text_replace(text, from, to)` | `(Text, Text, Text) -> Text` | Replace substring |
//! | `text_starts_with(text, prefix)` | `(Text, Text) -> Bool` | Check prefix |
//! | `text_ends_with(text, suffix)` | `(Text, Text) -> Bool` | Check suffix |
//! | `text_contains(text, substr)` | `(Text, Text) -> Bool` | Check substring |
//!
//! ## Context Requirements
//!
//! **Tier 0**: No context required - these are pure computation functions.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::well_known_types::variant_tags;
use verum_common::{List, OrderedMap, OrderedSet, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register collection builtins with context requirements
///
/// All collection functions are Tier 0 (always available) since they
/// perform pure computation without accessing external state.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // List Operations (Tier 0)
    // ========================================================================

    map.insert(
        Text::from("list_len"),
        BuiltinInfo::tier0(
            meta_list_len,
            "Get the length of a list",
            "(List<T>) -> Int",
        ),
    );
    map.insert(
        Text::from("list_push"),
        BuiltinInfo::tier0(
            meta_list_push,
            "Push element to list (returns new list)",
            "(List<T>, T) -> List<T>",
        ),
    );
    map.insert(
        Text::from("list_get"),
        BuiltinInfo::tier0(
            meta_list_get,
            "Get element at index (returns None if out of bounds)",
            "(List<T>, Int) -> Maybe<T>",
        ),
    );
    map.insert(
        Text::from("list_map"),
        BuiltinInfo::tier0(
            meta_list_map,
            "Map function over list elements",
            "(List<T>, fn(T) -> U) -> List<U>",
        ),
    );
    map.insert(
        Text::from("list_filter"),
        BuiltinInfo::tier0(
            meta_list_filter,
            "Filter list with predicate",
            "(List<T>, fn(T) -> Bool) -> List<T>",
        ),
    );
    map.insert(
        Text::from("list_fold"),
        BuiltinInfo::tier0(
            meta_list_fold,
            "Fold list with accumulator function",
            "(List<T>, U, fn(U, T) -> U) -> U",
        ),
    );
    map.insert(
        Text::from("list_concat"),
        BuiltinInfo::tier0(
            meta_list_concat,
            "Concatenate two lists",
            "(List<T>, List<T>) -> List<T>",
        ),
    );
    map.insert(
        Text::from("list_reverse"),
        BuiltinInfo::tier0(
            meta_list_reverse,
            "Reverse list order",
            "(List<T>) -> List<T>",
        ),
    );
    map.insert(
        Text::from("list_first"),
        BuiltinInfo::tier0(
            meta_list_first,
            "Get first element of list (returns None if empty)",
            "(List<T>) -> Maybe<T>",
        ),
    );
    map.insert(
        Text::from("list_last"),
        BuiltinInfo::tier0(
            meta_list_last,
            "Get last element of list (returns None if empty)",
            "(List<T>) -> Maybe<T>",
        ),
    );

    // ========================================================================
    // Maybe Operations (Tier 0)
    // ========================================================================

    map.insert(
        Text::from("maybe_unwrap"),
        BuiltinInfo::tier0(
            meta_maybe_unwrap,
            "Unwrap a Maybe value, panicking if None",
            "(Maybe<T>) -> T",
        ),
    );
    map.insert(
        Text::from("maybe_unwrap_or"),
        BuiltinInfo::tier0(
            meta_maybe_unwrap_or,
            "Unwrap a Maybe value, returning default if None",
            "(Maybe<T>, T) -> T",
        ),
    );
    map.insert(
        Text::from("maybe_is_some"),
        BuiltinInfo::tier0(
            meta_maybe_is_some,
            "Check if a Maybe value is Some",
            "(Maybe<T>) -> Bool",
        ),
    );
    map.insert(
        Text::from("maybe_is_none"),
        BuiltinInfo::tier0(
            meta_maybe_is_none,
            "Check if a Maybe value is None",
            "(Maybe<T>) -> Bool",
        ),
    );

    // Maybe constructors
    map.insert(
        Text::from(variant_tags::SOME),
        BuiltinInfo::tier0(
            meta_some,
            "Construct a Some value",
            "(T) -> Maybe<T>",
        ),
    );
    map.insert(
        Text::from(variant_tags::NONE),
        BuiltinInfo::tier0(
            meta_none,
            "Return a None value",
            "() -> Maybe<T>",
        ),
    );

    // ========================================================================
    // Map Operations (Tier 0)
    // ========================================================================

    map.insert(
        Text::from("map_new"),
        BuiltinInfo::tier0(meta_map_new, "Create empty map", "() -> Map<Text, T>"),
    );
    map.insert(
        Text::from("map_len"),
        BuiltinInfo::tier0(meta_map_len, "Get map length", "(Map<K, V>) -> Int"),
    );
    map.insert(
        Text::from("map_get"),
        BuiltinInfo::tier0(
            meta_map_get,
            "Get value by key (returns Maybe)",
            "(Map<K, V>, K) -> Maybe<V>",
        ),
    );
    map.insert(
        Text::from("map_insert"),
        BuiltinInfo::tier0(
            meta_map_insert,
            "Insert key-value pair (returns new map)",
            "(Map<K, V>, K, V) -> Map<K, V>",
        ),
    );
    map.insert(
        Text::from("map_remove"),
        BuiltinInfo::tier0(
            meta_map_remove,
            "Remove key from map (returns new map)",
            "(Map<K, V>, K) -> Map<K, V>",
        ),
    );
    map.insert(
        Text::from("map_contains"),
        BuiltinInfo::tier0(
            meta_map_contains,
            "Check if key exists in map",
            "(Map<K, V>, K) -> Bool",
        ),
    );
    map.insert(
        Text::from("map_keys"),
        BuiltinInfo::tier0(
            meta_map_keys,
            "Get all keys as list",
            "(Map<K, V>) -> List<K>",
        ),
    );
    map.insert(
        Text::from("map_values"),
        BuiltinInfo::tier0(
            meta_map_values,
            "Get all values as list",
            "(Map<K, V>) -> List<V>",
        ),
    );
    map.insert(
        Text::from("map_entries"),
        BuiltinInfo::tier0(
            meta_map_entries,
            "Get all entries as list of tuples",
            "(Map<K, V>) -> List<(K, V)>",
        ),
    );

    // ========================================================================
    // Set Operations (Tier 0)
    // ========================================================================

    map.insert(
        Text::from("set_new"),
        BuiltinInfo::tier0(meta_set_new, "Create empty set", "() -> Set<Text>"),
    );
    map.insert(
        Text::from("set_len"),
        BuiltinInfo::tier0(meta_set_len, "Get set size", "(Set<T>) -> Int"),
    );
    map.insert(
        Text::from("set_insert"),
        BuiltinInfo::tier0(
            meta_set_insert,
            "Insert value into set (returns new set)",
            "(Set<T>, T) -> Set<T>",
        ),
    );
    map.insert(
        Text::from("set_remove"),
        BuiltinInfo::tier0(
            meta_set_remove,
            "Remove value from set (returns new set)",
            "(Set<T>, T) -> Set<T>",
        ),
    );
    map.insert(
        Text::from("set_contains"),
        BuiltinInfo::tier0(
            meta_set_contains,
            "Check if value exists in set",
            "(Set<T>, T) -> Bool",
        ),
    );
    map.insert(
        Text::from("set_to_list"),
        BuiltinInfo::tier0(
            meta_set_to_list,
            "Convert set to list",
            "(Set<T>) -> List<T>",
        ),
    );
    map.insert(
        Text::from("set_union"),
        BuiltinInfo::tier0(
            meta_set_union,
            "Compute set union",
            "(Set<T>, Set<T>) -> Set<T>",
        ),
    );
    map.insert(
        Text::from("set_intersection"),
        BuiltinInfo::tier0(
            meta_set_intersection,
            "Compute set intersection",
            "(Set<T>, Set<T>) -> Set<T>",
        ),
    );
    map.insert(
        Text::from("set_difference"),
        BuiltinInfo::tier0(
            meta_set_difference,
            "Compute set difference (a - b)",
            "(Set<T>, Set<T>) -> Set<T>",
        ),
    );

    // ========================================================================
    // Text Operations (Tier 0)
    // ========================================================================

    map.insert(
        Text::from("text_concat"),
        BuiltinInfo::tier0(
            meta_text_concat,
            "Concatenate text values",
            "(...Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_len"),
        BuiltinInfo::tier0(
            meta_text_len,
            "Get text length in characters",
            "(Text) -> Int",
        ),
    );
    map.insert(
        Text::from("text_split"),
        BuiltinInfo::tier0(
            meta_text_split,
            "Split text by separator",
            "(Text, Text) -> List<Text>",
        ),
    );
    map.insert(
        Text::from("text_join"),
        BuiltinInfo::tier0(
            meta_text_join,
            "Join list elements with separator",
            "(List<Text>, Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_to_upper"),
        BuiltinInfo::tier0(
            meta_text_to_upper,
            "Convert text to uppercase",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_to_lower"),
        BuiltinInfo::tier0(
            meta_text_to_lower,
            "Convert text to lowercase",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_trim"),
        BuiltinInfo::tier0(
            meta_text_trim,
            "Trim whitespace from both ends",
            "(Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_replace"),
        BuiltinInfo::tier0(
            meta_text_replace,
            "Replace all occurrences of substring",
            "(Text, Text, Text) -> Text",
        ),
    );
    map.insert(
        Text::from("text_starts_with"),
        BuiltinInfo::tier0(
            meta_text_starts_with,
            "Check if text starts with prefix",
            "(Text, Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("text_ends_with"),
        BuiltinInfo::tier0(
            meta_text_ends_with,
            "Check if text ends with suffix",
            "(Text, Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("text_contains"),
        BuiltinInfo::tier0(
            meta_text_contains,
            "Check if text contains substring",
            "(Text, Text) -> Bool",
        ),
    );

    // Text equality comparison
    map.insert(
        Text::from("text_eq"),
        BuiltinInfo::tier0(
            meta_text_eq,
            "Check if two text values are equal",
            "(Text, Text) -> Bool",
        ),
    );

    // Additional text manipulation functions
    map.insert(
        Text::from("text_substring"),
        BuiltinInfo::tier0(
            meta_text_substring,
            "Extract substring from start to end index",
            "(Text, Int, Int) -> Text",
        ),
    );
    map.insert(
        Text::from("text_index_of"),
        BuiltinInfo::tier0(
            meta_text_index_of,
            "Find index of substring, returns -1 if not found",
            "(Text, Text) -> Int",
        ),
    );
    map.insert(
        Text::from("text_char_at"),
        BuiltinInfo::tier0(
            meta_text_char_at,
            "Get character at index",
            "(Text, Int) -> Char",
        ),
    );
    map.insert(
        Text::from("text_repeat"),
        BuiltinInfo::tier0(
            meta_text_repeat,
            "Repeat text n times",
            "(Text, Int) -> Text",
        ),
    );
    map.insert(
        Text::from("text_is_empty"),
        BuiltinInfo::tier0(
            meta_text_is_empty,
            "Check if text is empty",
            "(Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("text_lines"),
        BuiltinInfo::tier0(
            meta_text_lines,
            "Split text into lines",
            "(Text) -> List<Text>",
        ),
    );

    // ========================================================================
    // Convenience Aliases
    // ========================================================================

    // Alias: char_at -> text_char_at
    map.insert(
        Text::from("char_at"),
        BuiltinInfo::tier0(
            meta_text_char_at,
            "Get character at index (alias for text_char_at)",
            "(Text, Int) -> Char",
        ),
    );
    // Alias: text_upper -> text_to_upper
    map.insert(
        Text::from("text_upper"),
        BuiltinInfo::tier0(
            meta_text_to_upper,
            "Convert text to uppercase (alias for text_to_upper)",
            "(Text) -> Text",
        ),
    );
    // Alias: text_lower -> text_to_lower
    map.insert(
        Text::from("text_lower"),
        BuiltinInfo::tier0(
            meta_text_to_lower,
            "Convert text to lowercase (alias for text_to_lower)",
            "(Text) -> Text",
        ),
    );
}

// ============================================================================
// List Operations
// ============================================================================

/// Get list length
fn meta_list_len(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => Ok(ConstValue::Int(arr.len() as i128)),
        ConstValue::Tuple(tup) => Ok(ConstValue::Int(tup.len() as i128)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array or Tuple"),
            found: args[0].type_name(),
        }),
    }
}

/// Push element to list (returns new list)
fn meta_list_push(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            let mut new_arr = arr.clone();
            new_arr.push(args[1].clone());
            Ok(ConstValue::Array(new_arr))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array"),
            found: args[0].type_name(),
        }),
    }
}

/// Get element at index, returns Maybe<T> (Some(value) or None)
fn meta_list_get(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let index = match &args[1] {
        ConstValue::Int(i) => {
            if *i < 0 {
                // Negative index always returns None
                return Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])));
            }
            *i as usize
        }
        ConstValue::UInt(u) => *u as usize,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Array(arr) => {
            if index < arr.len() {
                // Return Some(value)
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    arr[index].clone(),
                ])))
            } else {
                // Return None
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        ConstValue::Tuple(tup) => {
            if index < tup.len() {
                // Return Some(value)
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    tup[index].clone(),
                ])))
            } else {
                // Return None
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array or Tuple"),
            found: args[0].type_name(),
        }),
    }
}

/// Map function over list (simplified - uses identity for now)
fn meta_list_map(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            // For now, just return the array as-is
            // Full implementation would evaluate the function on each element
            Ok(ConstValue::Array(arr.clone()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array"),
            found: args[0].type_name(),
        }),
    }
}

/// Filter list with predicate (simplified)
fn meta_list_filter(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            // For now, return the array as-is
            // Full implementation would evaluate the predicate
            Ok(ConstValue::Array(arr.clone()))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array"),
            found: args[0].type_name(),
        }),
    }
}

/// Fold list (simplified)
fn meta_list_fold(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 3 {
        return Err(MetaError::ArityMismatch {
            expected: 3,
            got: args.len(),
        });
    }

    // Return initial value for now
    // Full implementation would evaluate the fold function
    Ok(args[1].clone())
}

/// Concatenate two lists
fn meta_list_concat(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Array(arr1), ConstValue::Array(arr2)) => {
            let mut result = arr1.clone();
            result.extend(arr2.iter().cloned());
            Ok(ConstValue::Array(result))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array"),
            found: args[0].type_name(),
        }),
    }
}

/// Reverse list
fn meta_list_reverse(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            let reversed: List<ConstValue> = arr.iter().rev().cloned().collect();
            Ok(ConstValue::Array(reversed))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array"),
            found: args[0].type_name(),
        }),
    }
}

/// Get first element of list, returns Maybe<T> (Some(value) or None)
fn meta_list_first(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            if let Some(first) = arr.first() {
                // Return Some(value)
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    first.clone(),
                ])))
            } else {
                // Return None
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        ConstValue::Tuple(tup) => {
            if let Some(first) = tup.first() {
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    first.clone(),
                ])))
            } else {
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array or Tuple"),
            found: args[0].type_name(),
        }),
    }
}

/// Get last element of list, returns Maybe<T> (Some(value) or None)
fn meta_list_last(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Array(arr) => {
            if let Some(last) = arr.last() {
                // Return Some(value)
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    last.clone(),
                ])))
            } else {
                // Return None
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        ConstValue::Tuple(tup) => {
            if let Some(last) = tup.last() {
                Ok(ConstValue::Tuple(List::from(vec![
                    ConstValue::Text(Text::from(variant_tags::SOME)),
                    last.clone(),
                ])))
            } else {
                Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                    Text::from(variant_tags::NONE),
                )])))
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Array or Tuple"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Maybe Operations
// ============================================================================

/// Helper function to check if a value represents a Some variant
fn is_some(value: &ConstValue) -> bool {
    match value {
        ConstValue::Tuple(tup) => {
            if let Some(first) = tup.first() {
                if let ConstValue::Text(tag) = first {
                    return tag.as_str() == variant_tags::SOME;
                }
            }
            false
        }
        _ => false,
    }
}

/// Helper function to check if a value represents a None variant
fn is_none(value: &ConstValue) -> bool {
    match value {
        ConstValue::Tuple(tup) => {
            if let Some(first) = tup.first() {
                if let ConstValue::Text(tag) = first {
                    return tag.as_str() == variant_tags::NONE;
                }
            }
            false
        }
        _ => false,
    }
}

/// Unwrap a Maybe value, panicking if None
fn meta_maybe_unwrap(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Tuple(tup) => {
            if let Some(ConstValue::Text(tag)) = tup.first() {
                if tag.as_str() == variant_tags::SOME && tup.len() == 2 {
                    return Ok(tup[1].clone());
                } else if tag.as_str() == variant_tags::NONE {
                    return Err(MetaError::BuiltinEvalError {
                        function: Text::from("maybe_unwrap"),
                        message: Text::from("called `maybe_unwrap` on a `None` value"),
                    });
                }
            }
            Err(MetaError::TypeMismatch {
                expected: Text::from("Maybe<T>"),
                found: Text::from("Tuple"),
            })
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Maybe<T>"),
            found: args[0].type_name(),
        }),
    }
}

/// Unwrap a Maybe value, returning default if None
fn meta_maybe_unwrap_or(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Tuple(tup) => {
            if let Some(ConstValue::Text(tag)) = tup.first() {
                if tag.as_str() == variant_tags::SOME && tup.len() == 2 {
                    return Ok(tup[1].clone());
                } else if tag.as_str() == variant_tags::NONE {
                    return Ok(args[1].clone());
                }
            }
            Err(MetaError::TypeMismatch {
                expected: Text::from("Maybe<T>"),
                found: Text::from("Tuple"),
            })
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Maybe<T>"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if a Maybe value is Some
fn meta_maybe_is_some(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    Ok(ConstValue::Bool(is_some(&args[0])))
}

/// Check if a Maybe value is None
fn meta_maybe_is_none(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    Ok(ConstValue::Bool(is_none(&args[0])))
}

/// Construct a Some value
fn meta_some(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    // Return Some(value) as a tagged tuple
    Ok(ConstValue::Tuple(List::from(vec![
        ConstValue::Text(Text::from(variant_tags::SOME)),
        args[0].clone(),
    ])))
}

/// Return a None value
fn meta_none(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    // Return None as a tagged tuple
    Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
        Text::from(variant_tags::NONE),
    )])))
}

// ============================================================================
// Map Operations
// ============================================================================

/// Create empty map
fn meta_map_new(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }
    Ok(ConstValue::Map(OrderedMap::new()))
}

/// Get map length
fn meta_map_len(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Map(map) => Ok(ConstValue::Int(map.len() as i128)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Get value by key (returns Maybe: Some(value) or None)
fn meta_map_get(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let key = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (map key)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Map(map) => {
            match map.get(&key) {
                Some(value) => {
                    // Return Some(value) as a tuple with tag
                    Ok(ConstValue::Tuple(List::from(vec![
                        ConstValue::Text(Text::from(variant_tags::SOME)),
                        value.clone(),
                    ])))
                }
                None => {
                    // Return None as a tuple with tag
                    Ok(ConstValue::Tuple(List::from(vec![ConstValue::Text(
                        Text::from(variant_tags::NONE),
                    )])))
                }
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Insert key-value pair (returns new map)
fn meta_map_insert(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 3 {
        return Err(MetaError::ArityMismatch {
            expected: 3,
            got: args.len(),
        });
    }

    let key = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (map key)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Map(map) => {
            let mut new_map = map.clone();
            new_map.insert(key, args[2].clone());
            Ok(ConstValue::Map(new_map))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Remove key from map (returns new map)
fn meta_map_remove(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let key = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (map key)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Map(map) => {
            let mut new_map = map.clone();
            new_map.remove(&key);
            Ok(ConstValue::Map(new_map))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if key exists in map
fn meta_map_contains(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let key = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (map key)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Map(map) => Ok(ConstValue::Bool(map.contains_key(&key))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Get all keys as list
fn meta_map_keys(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Map(map) => {
            let keys: List<ConstValue> = map.keys().map(|k| ConstValue::Text(k.clone())).collect();
            Ok(ConstValue::Array(keys))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Get all values as list
fn meta_map_values(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Map(map) => {
            let values: List<ConstValue> = map.values().cloned().collect();
            Ok(ConstValue::Array(values))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

/// Get all entries as list of tuples
fn meta_map_entries(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Map(map) => {
            let entries: List<ConstValue> = map
                .iter()
                .map(|(k, v)| {
                    ConstValue::Tuple(List::from(vec![ConstValue::Text(k.clone()), v.clone()]))
                })
                .collect();
            Ok(ConstValue::Array(entries))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Map"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Set Operations
// ============================================================================

/// Create empty set
fn meta_set_new(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }
    Ok(ConstValue::Set(OrderedSet::new()))
}

/// Get set size
fn meta_set_len(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Set(set) => Ok(ConstValue::Int(set.len() as i128)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Set"),
            found: args[0].type_name(),
        }),
    }
}

/// Insert value into set (returns new set)
fn meta_set_insert(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let value = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (set element)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Set(set) => {
            let mut new_set = set.clone();
            new_set.insert(value);
            Ok(ConstValue::Set(new_set))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Set"),
            found: args[0].type_name(),
        }),
    }
}

/// Remove value from set (returns new set)
fn meta_set_remove(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let value = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (set element)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Set(set) => {
            let mut new_set = set.clone();
            new_set.remove(&value);
            Ok(ConstValue::Set(new_set))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Set"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if value exists in set
fn meta_set_contains(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let value = match &args[1] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text (set element)"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Set(set) => Ok(ConstValue::Bool(set.contains(&value))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Set"),
            found: args[0].type_name(),
        }),
    }
}

/// Convert set to list
fn meta_set_to_list(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Set(set) => {
            let list: List<ConstValue> = set.iter().map(|s| ConstValue::Text(s.clone())).collect();
            Ok(ConstValue::Array(list))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Set"),
            found: args[0].type_name(),
        }),
    }
}

/// Compute set union
fn meta_set_union(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Set(set1), ConstValue::Set(set2)) => {
            let union: OrderedSet<Text> = set1.union(set2).cloned().collect();
            Ok(ConstValue::Set(union))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Set, Set)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Compute set intersection
fn meta_set_intersection(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Set(set1), ConstValue::Set(set2)) => {
            let intersection: OrderedSet<Text> = set1.intersection(set2).cloned().collect();
            Ok(ConstValue::Set(intersection))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Set, Set)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Compute set difference (a - b)
fn meta_set_difference(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Set(set1), ConstValue::Set(set2)) => {
            let difference: OrderedSet<Text> = set1.difference(set2).cloned().collect();
            Ok(ConstValue::Set(difference))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Set, Set)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

// ============================================================================
// Text Operations
// ============================================================================

/// Concatenate text values
fn meta_text_concat(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let mut result = String::new();
    for arg in &args {
        match arg {
            ConstValue::Text(t) => result.push_str(t.as_str()),
            ConstValue::Char(c) => result.push(*c),
            _ => {
                return Err(MetaError::TypeMismatch {
                    expected: Text::from("Text"),
                    found: arg.type_name(),
                });
            }
        }
    }
    Ok(ConstValue::Text(Text::from(result)))
}

/// Get text length
fn meta_text_len(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(t) => Ok(ConstValue::Int(t.len() as i128)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Split text by separator
fn meta_text_split(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(text), ConstValue::Text(sep)) => {
            let parts: Vec<ConstValue> = text
                .as_str()
                .split(sep.as_str())
                .map(|s| ConstValue::Text(Text::from(s)))
                .collect();
            Ok(ConstValue::Array(List::from(parts)))
        }
        (ConstValue::Text(text), ConstValue::Char(sep)) => {
            let sep_str = sep.to_string();
            let parts: Vec<ConstValue> = text
                .as_str()
                .split(&sep_str)
                .map(|s| ConstValue::Text(Text::from(s)))
                .collect();
            Ok(ConstValue::Array(List::from(parts)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text) or (Text, Char)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Join list with separator
fn meta_text_join(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Array(arr), ConstValue::Text(sep)) => {
            let parts: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    ConstValue::Text(t) => t.to_string(),
                    _ => format!("{:?}", v),
                })
                .collect();
            Ok(ConstValue::Text(Text::from(parts.join(sep.as_str()))))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Array, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Convert to uppercase
fn meta_text_to_upper(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(t) => Ok(ConstValue::Text(Text::from(t.to_uppercase()))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Convert to lowercase
fn meta_text_to_lower(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(t) => Ok(ConstValue::Text(Text::from(t.to_lowercase()))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Trim whitespace
fn meta_text_trim(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(t) => Ok(ConstValue::Text(Text::from(t.trim()))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Replace substring
fn meta_text_replace(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 3 {
        return Err(MetaError::ArityMismatch {
            expected: 3,
            got: args.len(),
        });
    }

    match (&args[0], &args[1], &args[2]) {
        (ConstValue::Text(text), ConstValue::Text(from), ConstValue::Text(to)) => Ok(
            ConstValue::Text(Text::from(text.replace(from.as_str(), to.as_str()))),
        ),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text, Text)"),
            found: Text::from(format!(
                "({}, {}, {})",
                args[0].type_name(),
                args[1].type_name(),
                args[2].type_name()
            )),
        }),
    }
}

/// Check if text starts with prefix
fn meta_text_starts_with(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(text), ConstValue::Text(prefix)) => {
            Ok(ConstValue::Bool(text.starts_with(prefix.as_str())))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Check if text ends with suffix
fn meta_text_ends_with(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(text), ConstValue::Text(suffix)) => {
            Ok(ConstValue::Bool(text.ends_with(suffix.as_str())))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Check if text contains substring
fn meta_text_contains(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(text), ConstValue::Text(substr)) => {
            Ok(ConstValue::Bool(text.contains(substr.as_str())))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Check if two text values are equal
fn meta_text_eq(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(a), ConstValue::Text(b)) => Ok(ConstValue::Bool(a == b)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Extract substring from start to end index
fn meta_text_substring(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 3 {
        return Err(MetaError::ArityMismatch {
            expected: 3,
            got: args.len(),
        });
    }

    let start = match &args[1] {
        ConstValue::Int(i) => *i as usize,
        ConstValue::UInt(u) => *u as usize,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    let end = match &args[2] {
        ConstValue::Int(i) => *i as usize,
        ConstValue::UInt(u) => *u as usize,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[2].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Text(text) => {
            let s = text.as_str();
            if start > s.len() || end > s.len() || start > end {
                return Err(MetaError::IndexOutOfBounds {
                    index: end as i128,
                    length: s.len(),
                });
            }
            // Use char indices for proper UTF-8 handling
            let chars: Vec<char> = s.chars().collect();
            if start > chars.len() || end > chars.len() {
                return Err(MetaError::IndexOutOfBounds {
                    index: end as i128,
                    length: chars.len(),
                });
            }
            let substring: String = chars[start..end].iter().collect();
            Ok(ConstValue::Text(Text::from(substring)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Find index of substring, returns -1 if not found
fn meta_text_index_of(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Text(text), ConstValue::Text(substr)) => {
            match text.find(substr.as_str()) {
                Some(idx) => {
                    // Convert byte index to char index for proper UTF-8 handling
                    let char_idx = text[..idx].chars().count();
                    Ok(ConstValue::Int(char_idx as i128))
                }
                None => Ok(ConstValue::Int(-1)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("(Text, Text)"),
            found: Text::from(format!(
                "({}, {})",
                args[0].type_name(),
                args[1].type_name()
            )),
        }),
    }
}

/// Get character at index
fn meta_text_char_at(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let index = match &args[1] {
        ConstValue::Int(i) => *i as usize,
        ConstValue::UInt(u) => *u as usize,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Text(text) => {
            let chars: Vec<char> = text.chars().collect();
            if index >= chars.len() {
                return Err(MetaError::IndexOutOfBounds {
                    index: index as i128,
                    length: chars.len(),
                });
            }
            Ok(ConstValue::Char(chars[index]))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Repeat text n times
fn meta_text_repeat(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let count = match &args[1] {
        ConstValue::Int(i) => {
            if *i < 0 {
                return Err(MetaError::BuiltinEvalError {
                    function: Text::from("text_repeat"),
                    message: Text::from("Repeat count cannot be negative"),
                });
            }
            *i as usize
        }
        ConstValue::UInt(u) => *u as usize,
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Int"),
                found: args[1].type_name(),
            });
        }
    };

    match &args[0] {
        ConstValue::Text(text) => Ok(ConstValue::Text(Text::from(text.repeat(count)))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if text is empty
fn meta_text_is_empty(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(text) => Ok(ConstValue::Bool(text.is_empty())),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Split text into lines
fn meta_text_lines(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    match &args[0] {
        ConstValue::Text(text) => {
            let lines: Vec<ConstValue> = text
                .lines()
                .into_iter()
                .map(|s| ConstValue::Text(s))
                .collect();
            Ok(ConstValue::Array(List::from(lines)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_len() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Array(List::from(vec![
            ConstValue::Int(1),
            ConstValue::Int(2),
            ConstValue::Int(3),
        ]))]);
        let result = meta_list_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(3));
    }

    #[test]
    fn test_list_push() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![ConstValue::Int(1)])),
            ConstValue::Int(2),
        ]);
        let result = meta_list_push(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => assert_eq!(arr.len(), 2),
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_list_get() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![
                ConstValue::Int(10),
                ConstValue::Int(20),
                ConstValue::Int(30),
            ])),
            ConstValue::Int(1),
        ]);
        let result = meta_list_get(&mut ctx, args).unwrap();
        // Should return Some(20)
        match result {
            ConstValue::Tuple(tup) => {
                assert_eq!(tup[0], ConstValue::Text(Text::from(variant_tags::SOME)));
                assert_eq!(tup[1], ConstValue::Int(20));
            }
            _ => panic!("Expected Tuple (Some)"),
        }
    }

    #[test]
    fn test_list_get_out_of_bounds() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![
                ConstValue::Int(10),
                ConstValue::Int(20),
            ])),
            ConstValue::Int(5),
        ]);
        let result = meta_list_get(&mut ctx, args).unwrap();
        // Should return None
        match result {
            ConstValue::Tuple(tup) => {
                assert_eq!(tup.len(), 1);
                assert_eq!(tup[0], ConstValue::Text(Text::from(variant_tags::NONE)));
            }
            _ => panic!("Expected Tuple (None)"),
        }
    }

    #[test]
    fn test_list_get_negative_index() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![ConstValue::Int(10)])),
            ConstValue::Int(-1),
        ]);
        let result = meta_list_get(&mut ctx, args).unwrap();
        // Should return None
        match result {
            ConstValue::Tuple(tup) => {
                assert_eq!(tup.len(), 1);
                assert_eq!(tup[0], ConstValue::Text(Text::from(variant_tags::NONE)));
            }
            _ => panic!("Expected Tuple (None)"),
        }
    }

    #[test]
    fn test_list_concat() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![ConstValue::Int(1)])),
            ConstValue::Array(List::from(vec![ConstValue::Int(2)])),
        ]);
        let result = meta_list_concat(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => assert_eq!(arr.len(), 2),
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_list_reverse() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Array(List::from(vec![
            ConstValue::Int(1),
            ConstValue::Int(2),
            ConstValue::Int(3),
        ]))]);
        let result = meta_list_reverse(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr[0], ConstValue::Int(3));
                assert_eq!(arr[2], ConstValue::Int(1));
            }
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_text_concat() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("Hello")),
            ConstValue::Text(Text::from(" ")),
            ConstValue::Text(Text::from("World")),
        ]);
        let result = meta_text_concat(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("Hello World")));
    }

    #[test]
    fn test_text_len() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("Hello"))]);
        let result = meta_text_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(5));
    }

    #[test]
    fn test_text_split() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("a,b,c")),
            ConstValue::Text(Text::from(",")),
        ]);
        let result = meta_text_split(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], ConstValue::Text(Text::from("a")));
            }
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_text_join() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Array(List::from(vec![
                ConstValue::Text(Text::from("a")),
                ConstValue::Text(Text::from("b")),
                ConstValue::Text(Text::from("c")),
            ])),
            ConstValue::Text(Text::from(",")),
        ]);
        let result = meta_text_join(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("a,b,c")));
    }

    #[test]
    fn test_text_to_upper() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("hello"))]);
        let result = meta_text_to_upper(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("HELLO")));
    }

    #[test]
    fn test_text_trim() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("  hello  "))]);
        let result = meta_text_trim(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("hello")));
    }

    #[test]
    fn test_text_replace() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("hello world")),
            ConstValue::Text(Text::from("world")),
            ConstValue::Text(Text::from("Verum")),
        ]);
        let result = meta_text_replace(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("hello Verum")));
    }

    #[test]
    fn test_text_starts_with() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("hello world")),
            ConstValue::Text(Text::from("hello")),
        ]);
        let result = meta_text_starts_with(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_text_contains() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("hello world")),
            ConstValue::Text(Text::from("wor")),
        ]);
        let result = meta_text_contains(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_text_eq_equal() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("hello")),
            ConstValue::Text(Text::from("hello")),
        ]);
        let result = meta_text_eq(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_text_eq_not_equal() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("hello")),
            ConstValue::Text(Text::from("world")),
        ]);
        let result = meta_text_eq(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_text_eq_empty() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("")),
            ConstValue::Text(Text::from("")),
        ]);
        let result = meta_text_eq(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    // ========================================================================
    // Map Operation Tests
    // ========================================================================

    #[test]
    fn test_map_new() {
        let mut ctx = MetaContext::new();
        let result = meta_map_new(&mut ctx, List::new()).unwrap();
        match result {
            ConstValue::Map(map) => assert!(map.is_empty()),
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_map_new_arity_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(1)]);
        let result = meta_map_new(&mut ctx, args);
        assert!(matches!(
            result,
            Err(MetaError::ArityMismatch {
                expected: 0,
                got: 1
            })
        ));
    }

    #[test]
    fn test_map_len_empty() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Map(OrderedMap::new())]);
        let result = meta_map_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(0));
    }

    #[test]
    fn test_map_len_with_entries() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("a"), ConstValue::Int(1));
        map.insert(Text::from("b"), ConstValue::Int(2));
        let args = List::from(vec![ConstValue::Map(map)]);
        let result = meta_map_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(2));
    }

    #[test]
    fn test_map_len_type_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_map_len(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }

    #[test]
    fn test_map_insert() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Map(OrderedMap::new()),
            ConstValue::Text(Text::from("key")),
            ConstValue::Int(42),
        ]);
        let result = meta_map_insert(&mut ctx, args).unwrap();
        match result {
            ConstValue::Map(map) => {
                assert_eq!(map.len(), 1);
                assert_eq!(map.get(&Text::from("key")), Some(&ConstValue::Int(42)));
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_map_insert_overwrite() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("key"), ConstValue::Int(1));
        let args = List::from(vec![
            ConstValue::Map(map),
            ConstValue::Text(Text::from("key")),
            ConstValue::Int(2),
        ]);
        let result = meta_map_insert(&mut ctx, args).unwrap();
        match result {
            ConstValue::Map(map) => {
                assert_eq!(map.len(), 1);
                assert_eq!(map.get(&Text::from("key")), Some(&ConstValue::Int(2)));
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_map_get_existing() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("key"), ConstValue::Int(42));
        let args = List::from(vec![
            ConstValue::Map(map),
            ConstValue::Text(Text::from("key")),
        ]);
        let result = meta_map_get(&mut ctx, args).unwrap();
        match result {
            ConstValue::Tuple(tup) => {
                assert_eq!(tup[0], ConstValue::Text(Text::from(variant_tags::SOME)));
                assert_eq!(tup[1], ConstValue::Int(42));
            }
            _ => panic!("Expected Tuple (Some)"),
        }
    }

    #[test]
    fn test_map_get_missing() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Map(OrderedMap::new()),
            ConstValue::Text(Text::from("missing")),
        ]);
        let result = meta_map_get(&mut ctx, args).unwrap();
        match result {
            ConstValue::Tuple(tup) => {
                assert_eq!(tup.len(), 1);
                assert_eq!(tup[0], ConstValue::Text(Text::from(variant_tags::NONE)));
            }
            _ => panic!("Expected Tuple (None)"),
        }
    }

    #[test]
    fn test_map_remove_existing() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("a"), ConstValue::Int(1));
        map.insert(Text::from("b"), ConstValue::Int(2));
        let args = List::from(vec![
            ConstValue::Map(map),
            ConstValue::Text(Text::from("a")),
        ]);
        let result = meta_map_remove(&mut ctx, args).unwrap();
        match result {
            ConstValue::Map(map) => {
                assert_eq!(map.len(), 1);
                assert!(!map.contains_key(&Text::from("a")));
                assert!(map.contains_key(&Text::from("b")));
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_map_remove_missing() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("a"), ConstValue::Int(1));
        let args = List::from(vec![
            ConstValue::Map(map),
            ConstValue::Text(Text::from("missing")),
        ]);
        let result = meta_map_remove(&mut ctx, args).unwrap();
        match result {
            ConstValue::Map(map) => {
                assert_eq!(map.len(), 1);
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_map_contains_true() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("key"), ConstValue::Int(42));
        let args = List::from(vec![
            ConstValue::Map(map),
            ConstValue::Text(Text::from("key")),
        ]);
        let result = meta_map_contains(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_map_contains_false() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Map(OrderedMap::new()),
            ConstValue::Text(Text::from("missing")),
        ]);
        let result = meta_map_contains(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_map_keys() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("a"), ConstValue::Int(1));
        map.insert(Text::from("b"), ConstValue::Int(2));
        map.insert(Text::from("c"), ConstValue::Int(3));
        let args = List::from(vec![ConstValue::Map(map)]);
        let result = meta_map_keys(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                // OrderedMap keeps keys in sorted order
                assert_eq!(arr[0], ConstValue::Text(Text::from("a")));
                assert_eq!(arr[1], ConstValue::Text(Text::from("b")));
                assert_eq!(arr[2], ConstValue::Text(Text::from("c")));
            }
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_map_values() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("a"), ConstValue::Int(1));
        map.insert(Text::from("b"), ConstValue::Int(2));
        let args = List::from(vec![ConstValue::Map(map)]);
        let result = meta_map_values(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], ConstValue::Int(1));
                assert_eq!(arr[1], ConstValue::Int(2));
            }
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_map_entries() {
        let mut ctx = MetaContext::new();
        let mut map = OrderedMap::new();
        map.insert(Text::from("x"), ConstValue::Int(10));
        map.insert(Text::from("y"), ConstValue::Int(20));
        let args = List::from(vec![ConstValue::Map(map)]);
        let result = meta_map_entries(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr.len(), 2);
                match &arr[0] {
                    ConstValue::Tuple(tup) => {
                        assert_eq!(tup[0], ConstValue::Text(Text::from("x")));
                        assert_eq!(tup[1], ConstValue::Int(10));
                    }
                    _ => panic!("Expected tuple entry"),
                }
            }
            _ => panic!("Expected Array"),
        }
    }

    // ========================================================================
    // Set Operation Tests
    // ========================================================================

    #[test]
    fn test_set_new() {
        let mut ctx = MetaContext::new();
        let result = meta_set_new(&mut ctx, List::new()).unwrap();
        match result {
            ConstValue::Set(set) => assert!(set.is_empty()),
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_new_arity_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(1)]);
        let result = meta_set_new(&mut ctx, args);
        assert!(matches!(
            result,
            Err(MetaError::ArityMismatch {
                expected: 0,
                got: 1
            })
        ));
    }

    #[test]
    fn test_set_len_empty() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Set(OrderedSet::new())]);
        let result = meta_set_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(0));
    }

    #[test]
    fn test_set_len_with_elements() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("a"));
        set.insert(Text::from("b"));
        set.insert(Text::from("c"));
        let args = List::from(vec![ConstValue::Set(set)]);
        let result = meta_set_len(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(3));
    }

    #[test]
    fn test_set_insert() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Set(OrderedSet::new()),
            ConstValue::Text(Text::from("element")),
        ]);
        let result = meta_set_insert(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 1);
                assert!(set.contains(&Text::from("element")));
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_insert_duplicate() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("element"));
        let args = List::from(vec![
            ConstValue::Set(set),
            ConstValue::Text(Text::from("element")),
        ]);
        let result = meta_set_insert(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                // Should still have 1 element (no duplicates)
                assert_eq!(set.len(), 1);
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_remove_existing() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("a"));
        set.insert(Text::from("b"));
        let args = List::from(vec![
            ConstValue::Set(set),
            ConstValue::Text(Text::from("a")),
        ]);
        let result = meta_set_remove(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 1);
                assert!(!set.contains(&Text::from("a")));
                assert!(set.contains(&Text::from("b")));
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_remove_missing() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("a"));
        let args = List::from(vec![
            ConstValue::Set(set),
            ConstValue::Text(Text::from("missing")),
        ]);
        let result = meta_set_remove(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 1);
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_contains_true() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("element"));
        let args = List::from(vec![
            ConstValue::Set(set),
            ConstValue::Text(Text::from("element")),
        ]);
        let result = meta_set_contains(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_set_contains_false() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Set(OrderedSet::new()),
            ConstValue::Text(Text::from("missing")),
        ]);
        let result = meta_set_contains(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_set_to_list() {
        let mut ctx = MetaContext::new();
        let mut set = OrderedSet::new();
        set.insert(Text::from("b"));
        set.insert(Text::from("a"));
        set.insert(Text::from("c"));
        let args = List::from(vec![ConstValue::Set(set)]);
        let result = meta_set_to_list(&mut ctx, args).unwrap();
        match result {
            ConstValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                // OrderedSet keeps elements in sorted order
                assert_eq!(arr[0], ConstValue::Text(Text::from("a")));
                assert_eq!(arr[1], ConstValue::Text(Text::from("b")));
                assert_eq!(arr[2], ConstValue::Text(Text::from("c")));
            }
            _ => panic!("Expected Array"),
        }
    }

    #[test]
    fn test_set_union() {
        let mut ctx = MetaContext::new();
        let mut set1 = OrderedSet::new();
        set1.insert(Text::from("a"));
        set1.insert(Text::from("b"));
        let mut set2 = OrderedSet::new();
        set2.insert(Text::from("b"));
        set2.insert(Text::from("c"));
        let args = List::from(vec![ConstValue::Set(set1), ConstValue::Set(set2)]);
        let result = meta_set_union(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 3);
                assert!(set.contains(&Text::from("a")));
                assert!(set.contains(&Text::from("b")));
                assert!(set.contains(&Text::from("c")));
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_intersection() {
        let mut ctx = MetaContext::new();
        let mut set1 = OrderedSet::new();
        set1.insert(Text::from("a"));
        set1.insert(Text::from("b"));
        set1.insert(Text::from("c"));
        let mut set2 = OrderedSet::new();
        set2.insert(Text::from("b"));
        set2.insert(Text::from("c"));
        set2.insert(Text::from("d"));
        let args = List::from(vec![ConstValue::Set(set1), ConstValue::Set(set2)]);
        let result = meta_set_intersection(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 2);
                assert!(set.contains(&Text::from("b")));
                assert!(set.contains(&Text::from("c")));
                assert!(!set.contains(&Text::from("a")));
                assert!(!set.contains(&Text::from("d")));
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_difference() {
        let mut ctx = MetaContext::new();
        let mut set1 = OrderedSet::new();
        set1.insert(Text::from("a"));
        set1.insert(Text::from("b"));
        set1.insert(Text::from("c"));
        let mut set2 = OrderedSet::new();
        set2.insert(Text::from("b"));
        let args = List::from(vec![ConstValue::Set(set1), ConstValue::Set(set2)]);
        let result = meta_set_difference(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert_eq!(set.len(), 2);
                assert!(set.contains(&Text::from("a")));
                assert!(set.contains(&Text::from("c")));
                assert!(!set.contains(&Text::from("b")));
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_union_empty() {
        let mut ctx = MetaContext::new();
        let set1 = OrderedSet::new();
        let set2 = OrderedSet::new();
        let args = List::from(vec![ConstValue::Set(set1), ConstValue::Set(set2)]);
        let result = meta_set_union(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert!(set.is_empty());
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_intersection_disjoint() {
        let mut ctx = MetaContext::new();
        let mut set1 = OrderedSet::new();
        set1.insert(Text::from("a"));
        let mut set2 = OrderedSet::new();
        set2.insert(Text::from("b"));
        let args = List::from(vec![ConstValue::Set(set1), ConstValue::Set(set2)]);
        let result = meta_set_intersection(&mut ctx, args).unwrap();
        match result {
            ConstValue::Set(set) => {
                assert!(set.is_empty());
            }
            _ => panic!("Expected Set"),
        }
    }

    #[test]
    fn test_set_type_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_set_len(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }

    #[test]
    fn test_set_insert_type_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Set(OrderedSet::new()),
            ConstValue::Int(42), // Should be Text
        ]);
        let result = meta_set_insert(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }

    #[test]
    fn test_map_get_key_type_error() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Map(OrderedMap::new()),
            ConstValue::Int(42), // Should be Text
        ]);
        let result = meta_map_get(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }
}
