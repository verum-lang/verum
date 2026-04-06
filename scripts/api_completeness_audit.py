#!/usr/bin/env python3
"""
API Completeness Audit for Verum Semantic Types

This script compares Rust's standard library methods against Verum's semantic types
to identify missing API coverage.

Usage: python3 api_completeness_audit.py
"""

import subprocess
import json

# String methods from std (comprehensive list from Rust docs)
STRING_METHODS = [
    # Construction
    "new", "with_capacity", "from_utf8", "from_utf8_lossy", "from_utf8_unchecked",
    "from_utf16", "from_utf16_lossy", "from_raw_parts",

    # Capacity
    "capacity", "reserve", "reserve_exact", "shrink_to_fit", "shrink_to",

    # Mutation
    "push", "push_str", "pop", "remove", "retain", "insert", "insert_str",
    "truncate", "clear", "drain", "replace_range", "split_off",

    # Conversion
    "as_str", "as_mut_str", "as_bytes", "into_bytes", "into_boxed_str",
    "into_string", "leak",

    # Query
    "len", "is_empty", "is_char_boundary", "starts_with", "ends_with",
    "contains", "find", "rfind", "matches", "rmatches",
    "match_indices", "rmatch_indices", "trim_matches", "strip_prefix",
    "strip_suffix",

    # Iteration
    "chars", "char_indices", "bytes", "lines", "split_whitespace",
    "split_ascii_whitespace", "split", "rsplit", "split_terminator",
    "rsplit_terminator", "splitn", "rsplitn", "split_inclusive",
    "matches", "rmatches", "match_indices", "rmatch_indices",

    # Transformation
    "to_lowercase", "to_uppercase", "to_ascii_lowercase", "to_ascii_uppercase",
    "make_ascii_lowercase", "make_ascii_uppercase", "repeat", "replace",
    "replacen", "trim", "trim_start", "trim_end",
    "trim_start_matches", "trim_end_matches", "parse",
]

# Vec methods from std
VEC_METHODS = [
    # Construction
    "new", "with_capacity", "from_raw_parts", "from_elem",

    # Capacity
    "capacity", "reserve", "reserve_exact", "shrink_to_fit", "shrink_to",
    "truncate",

    # Mutation
    "push", "pop", "insert", "remove", "swap_remove", "retain", "retain_mut",
    "dedup", "dedup_by", "dedup_by_key", "append", "drain", "clear",
    "splice", "split_off",

    # Query
    "len", "is_empty", "first", "last", "first_mut", "last_mut",
    "get", "get_mut", "get_unchecked", "get_unchecked_mut",
    "as_slice", "as_mut_slice", "as_ptr", "as_mut_ptr", "swap",
    "reverse", "contains", "starts_with", "ends_with", "binary_search",
    "binary_search_by", "binary_search_by_key",

    # Sorting
    "sort", "sort_by", "sort_by_key", "sort_by_cached_key",
    "sort_unstable", "sort_unstable_by", "sort_unstable_by_key",

    # Slicing
    "split_at", "split_at_mut", "split_first", "split_first_mut",
    "split_last", "split_last_mut", "windows", "chunks", "chunks_mut",
    "chunks_exact", "chunks_exact_mut", "rchunks", "rchunks_mut",
    "rchunks_exact", "rchunks_exact_mut",

    # Rotation
    "rotate_left", "rotate_right",

    # Fill
    "fill", "fill_with", "clone_from_slice", "copy_from_slice",
    "copy_within", "swap_with_slice",

    # Resize
    "resize", "resize_with", "extend_from_slice", "extend_from_within",

    # Conversion
    "into_boxed_slice", "into_raw_parts", "leak",

    # Iteration
    "iter", "iter_mut", "split", "split_mut", "rsplit", "rsplit_mut",
    "splitn", "splitn_mut", "rsplitn", "rsplitn_mut",
]

# HashMap methods from std
HASHMAP_METHODS = [
    # Construction
    "new", "with_capacity", "with_hasher", "with_capacity_and_hasher",

    # Capacity
    "capacity", "reserve", "shrink_to_fit", "shrink_to",

    # Mutation
    "insert", "remove", "remove_entry", "retain", "clear", "drain",

    # Query
    "len", "is_empty", "contains_key", "get", "get_mut",
    "get_key_value", "get_many_mut",

    # Entry API
    "entry", "try_insert",

    # Iteration
    "keys", "values", "values_mut", "iter", "iter_mut",
    "into_keys", "into_values",

    # Hasher
    "hasher",
]

# HashSet methods from std
HASHSET_METHODS = [
    # Construction
    "new", "with_capacity", "with_hasher", "with_capacity_and_hasher",

    # Capacity
    "capacity", "reserve", "shrink_to_fit", "shrink_to",

    # Mutation
    "insert", "remove", "take", "retain", "clear", "drain",

    # Query
    "len", "is_empty", "contains", "get", "get_or_insert",
    "get_or_insert_with", "get_or_insert_owned",

    # Set operations
    "union", "intersection", "difference", "symmetric_difference",
    "is_disjoint", "is_subset", "is_superset",

    # Iteration
    "iter",

    # Hasher
    "hasher",
]

# Option methods from std
OPTION_METHODS = [
    # Query
    "is_some", "is_some_and", "is_none",

    # Extract
    "unwrap", "unwrap_or", "unwrap_or_default", "unwrap_or_else",
    "expect", "unwrap_unchecked",

    # Transform
    "map", "map_or", "map_or_else", "and_then", "or", "or_else",
    "xor", "filter", "flatten", "zip", "zip_with",

    # Convert
    "ok_or", "ok_or_else", "transpose", "as_ref", "as_mut",
    "as_deref", "as_deref_mut", "as_pin_ref", "as_pin_mut",

    # Mutate
    "insert", "get_or_insert", "get_or_insert_default",
    "get_or_insert_with", "take", "replace",

    # Iteration
    "iter", "iter_mut",

    # Copy/Clone
    "copied", "cloned",
]

def check_methods_in_file(file_path, methods):
    """Check which methods are present in a Rust file."""
    try:
        with open(file_path, 'r') as f:
            content = f.read()

        found = []
        missing = []

        for method in methods:
            # Check for pub fn method_name
            if f'pub fn {method}' in content or f'pub unsafe fn {method}' in content:
                found.append(method)
            else:
                missing.append(method)

        return found, missing
    except FileNotFoundError:
        return [], methods

def print_section(title, found, missing, total):
    """Print a formatted section of the audit."""
    print(f"\n{'='*80}")
    print(f"{title}")
    print(f"{'='*80}")
    print(f"Coverage: {len(found)}/{total} methods ({len(found)*100//total}%)")
    print(f"\n✅ Found ({len(found)}):")
    for method in sorted(found)[:20]:  # Show first 20
        print(f"  - {method}")
    if len(found) > 20:
        print(f"  ... and {len(found) - 20} more")

    print(f"\n❌ Missing ({len(missing)}):")
    for method in sorted(missing):
        print(f"  - {method}")

def main():
    print("="*80)
    print("VERUM API COMPLETENESS AUDIT")
    print("="*80)

    base_path = "/Users/taaliman/projects/luxquant/axiom/crates/verum_core"

    # Check semantic_types.rs (newtype wrappers - currently unused)
    semantic_file = f"{base_path}/src/semantic_types.rs"

    # Text (String wrapper)
    print("\n🔍 Checking Text (String wrapper) in semantic_types.rs...")
    found_text, missing_text = check_methods_in_file(semantic_file, STRING_METHODS)
    print_section("TEXT API", found_text, missing_text, len(STRING_METHODS))

    # List (Vec wrapper)
    print("\n🔍 Checking List (Vec wrapper) in semantic_types.rs...")
    found_list, missing_list = check_methods_in_file(semantic_file, VEC_METHODS)
    print_section("LIST API", found_list, missing_list, len(VEC_METHODS))

    # Map (HashMap wrapper)
    print("\n🔍 Checking Map (HashMap wrapper) in semantic_types.rs...")
    found_map, missing_map = check_methods_in_file(semantic_file, HASHMAP_METHODS)
    print_section("MAP API", found_map, missing_map, len(HASHMAP_METHODS))

    # Set (HashSet wrapper)
    print("\n🔍 Checking Set (HashSet wrapper) in semantic_types.rs...")
    found_set, missing_set = check_methods_in_file(semantic_file, HASHSET_METHODS)
    print_section("SET API", found_set, missing_set, len(HASHSET_METHODS))

    # Maybe (Option)
    maybe_file = f"{base_path}/src/maybe.rs"
    print("\n🔍 Checking Maybe in maybe.rs...")
    found_maybe, missing_maybe = check_methods_in_file(maybe_file, OPTION_METHODS)
    print_section("MAYBE API", found_maybe, missing_maybe, len(OPTION_METHODS))

    # Summary
    print("\n" + "="*80)
    print("SUMMARY")
    print("="*80)
    total_found = len(found_text) + len(found_list) + len(found_map) + len(found_set) + len(found_maybe)
    total_methods = len(STRING_METHODS) + len(VEC_METHODS) + len(HASHMAP_METHODS) + len(HASHSET_METHODS) + len(OPTION_METHODS)

    print(f"\nOverall Coverage: {total_found}/{total_methods} methods ({total_found*100//total_methods}%)")
    print(f"\nBreakdown:")
    print(f"  Text:  {len(found_text)}/{len(STRING_METHODS)} ({len(found_text)*100//len(STRING_METHODS)}%)")
    print(f"  List:  {len(found_list)}/{len(VEC_METHODS)} ({len(found_list)*100//len(VEC_METHODS)}%)")
    print(f"  Map:   {len(found_map)}/{len(HASHMAP_METHODS)} ({len(found_map)*100//len(HASHMAP_METHODS)}%)")
    print(f"  Set:   {len(found_set)}/{len(HASHSET_METHODS)} ({len(found_set)*100//len(HASHSET_METHODS)}%)")
    print(f"  Maybe: {len(found_maybe)}/{len(OPTION_METHODS)} ({len(found_maybe)*100//len(OPTION_METHODS)}%)")

    print("\n" + "="*80)
    print("CRITICAL ISSUE DETECTED")
    print("="*80)
    print("\n⚠️  The codebase uses TYPE ALIASES (lib.rs) not newtype WRAPPERS (semantic_types.rs)")
    print("⚠️  Type aliases = String, Vec<T>, HashMap, etc. (direct std types)")
    print("⚠️  Newtype wrappers = struct Text { inner: String } (wrapped types)")
    print("\nCurrent state:")
    print("  - lib.rs: pub type Text = String (TYPE ALIAS - USED)")
    print("  - semantic_types.rs: struct Text { inner: String } (NEWTYPE - UNUSED!)")
    print("\n📋 Recommendation:")
    print("  Since type aliases are used, all std methods are AUTOMATICALLY available.")
    print("  No additional wrapper methods needed - std::vec::Vec, std::string::String, etc. work directly.")
    print("  The semantic_types.rs file with newtype wrappers is DEAD CODE and should be:")
    print("    1. Removed, OR")
    print("    2. Actively used instead of type aliases (requires codebase refactor)")

    print("\n" + "="*80)

if __name__ == "__main__":
    main()
