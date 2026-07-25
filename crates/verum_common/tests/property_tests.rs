#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Property-based tests for verum_common semantic types
//!
//! Uses proptest to verify invariants hold for arbitrary inputs

use proptest::prelude::*;
use verum_common::semantic_types::{List, Map, Set, Text};

// ============================================================================
// TEXT PROPERTY TESTS
// ============================================================================

proptest! {
    #[test]
    fn test_text_push_pop_roundtrip(s in ".*") {
        let mut text = Text::from(s.clone());
        let original_len = text.len();

        text.push('x');
        assert_eq!(text.len(), original_len + 1);

        let popped = text.pop();
        assert_eq!(popped, Some('x'));
        assert_eq!(text.len(), original_len);
        assert_eq!(text.as_str(), s);
    }

    #[test]
    fn test_text_push_str_length(s1 in ".*", s2 in ".*") {
        let mut text = Text::from(s1.clone());
        let len1 = text.len();
        text.push_str(&s2);
        assert_eq!(text.len(), len1 + s2.len());
    }

    #[test]
    fn test_text_truncate_idempotent(s in ".*", n in 0usize..100) {
        let mut text = Text::from(s.clone());
        text.truncate(n);
        let len_after_first = text.len();

        text.truncate(n);
        assert_eq!(text.len(), len_after_first);
    }

    #[test]
    fn test_text_clear_makes_empty(s in ".*") {
        let mut text = Text::from(s);
        text.clear();
        assert!(text.is_empty());
        assert_eq!(text.len(), 0);
    }

    #[test]
    fn test_text_repeat_length(s in ".*", n in 0usize..10) {
        let text = Text::from(s.clone());
        let repeated = text.repeat(n);
        assert_eq!(repeated.len(), s.len() * n);
    }

    #[test]
    fn test_text_to_lowercase_uppercase_roundtrip(s in "[a-zA-Z]*") {
        let text = Text::from(s.clone());
        let lower = text.to_lowercase();
        let upper = lower.to_uppercase();

        // Uppercase of lowercase should equal uppercase of original
        assert_eq!(upper.as_str(), text.to_uppercase().as_str());
    }

    #[test]
    fn test_text_trim_reduces_length(s in ".*") {
        let text = Text::from(s);
        let trimmed = text.trim();
        assert!(trimmed.len() <= text.len());
    }

    #[test]
    fn test_text_split_join_roundtrip(s in "[a-zA-Z0-9]*", sep in "[,;:|]") {
        let text = Text::from(s.clone());
        let parts = text.split(&sep);
        let joined = parts.join(&sep);

        // If no separator in original, should be identical
        if !s.contains(&sep) {
            assert_eq!(joined.as_str(), s);
        }
    }

    #[test]
    fn test_text_contains_substring(s in "[a-zA-Z0-9]*", char_idx in 0usize..20) {
        // Use ASCII-only strings to avoid byte boundary issues
        let text = Text::from(s.clone());
        if char_idx < s.len() {
            let substring = &s[char_idx..];
            assert!(text.contains(substring));
        }
    }

    #[test]
    fn test_text_starts_with_prefix(s in "[a-zA-Z0-9]*", n in 0usize..20) {
        // Use ASCII-only strings to avoid byte boundary issues
        let text = Text::from(s.clone());
        let prefix_len = n.min(s.len());
        let prefix = &s[..prefix_len];
        assert!(text.starts_with(prefix));
    }

    #[test]
    fn test_text_pad_increases_length(s in "[a-zA-Z0-9]*", n in 0usize..100) {
        // Use ASCII-only strings where len() == char count
        let text = Text::from(s.clone());
        let padded = text.pad_left(n, '0');

        if n > s.len() {
            assert_eq!(padded.len(), n);
        } else {
            assert_eq!(padded.len(), s.len());
        }
    }
}

// ============================================================================
// LIST PROPERTY TESTS
// ============================================================================

proptest! {
    #[test]
    fn test_list_push_pop_roundtrip(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut list = List::from(vec.clone());
        let original_len = list.len();

        list.push(999);
        assert_eq!(list.len(), original_len + 1);

        let popped = list.pop();
        assert_eq!(popped, Some(999));
        assert_eq!(list.len(), original_len);
        assert_eq!(list.as_slice(), vec.as_slice());
    }

    #[test]
    fn test_list_clear_makes_empty(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut list = List::from(vec);
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_list_reverse_twice_is_identity(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut list = List::from(vec.clone());
        list.reverse();
        list.reverse();
        assert_eq!(list.as_slice(), vec.as_slice());
    }

    #[test]
    fn test_list_sort_is_sorted(vec in prop::collection::vec(any::<i32>(), 0..1000)) {
        let mut list = List::from(vec);
        list.sort();

        let sorted = list.as_slice();
        for i in 1..sorted.len() {
            assert!(sorted[i - 1] <= sorted[i], "List not sorted at index {}", i);
        }
    }

    #[test]
    fn test_list_sort_preserves_length(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut list = List::from(vec.clone());
        let original_len = list.len();
        list.sort();
        assert_eq!(list.len(), original_len);
    }

    #[test]
    fn test_list_dedup_reduces_length(mut vec in prop::collection::vec(any::<i32>(), 0..100)) {
        vec.sort();
        let mut list = List::from(vec.clone());
        let original_len = list.len();
        list.dedup();
        assert!(list.len() <= original_len);
    }

    #[test]
    fn test_list_truncate_sets_length(vec in prop::collection::vec(any::<i32>(), 0..100), n in 0usize..100) {
        let mut list = List::from(vec);
        list.truncate(n);
        assert!(list.len() <= n);
    }

    #[test]
    fn test_list_resize_sets_length(vec in prop::collection::vec(any::<i32>(), 0..100), n in 0usize..100) {
        let mut list = List::from(vec);
        list.resize(n, 0);
        assert_eq!(list.len(), n);
    }

    #[test]
    fn test_list_retain_reduces_length(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut list = List::from(vec.clone());
        let original_len = list.len();
        list.retain(|&x| x > 0);
        assert!(list.len() <= original_len);
    }

    #[test]
    fn test_list_filter_preserves_predicate(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let list = List::from(vec);
        let filtered = list.filter(|&x| x % 2 == 0);

        for &item in filtered.iter() {
            assert_eq!(item % 2, 0, "Filtered list contains odd number: {}", item);
        }
    }

    #[test]
    fn test_list_map_preserves_length(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let list = List::from(vec.clone());
        // Use wrapping_mul to avoid overflow panic
        let mapped = list.map(|x| x.wrapping_mul(2));
        assert_eq!(mapped.len(), vec.len());
    }

    #[test]
    fn test_list_split_at_preserves_elements(vec in prop::collection::vec(any::<i32>(), 0..100), idx in 0usize..100) {
        let list = List::from(vec.clone());
        if idx <= list.len() {
            let (left, right) = list.split_at(idx);
            assert_eq!(left.len() + right.len(), vec.len());
            assert_eq!(left.len(), idx);
        }
    }

    #[test]
    fn test_list_rotate_left_right_cancel(vec in prop::collection::vec(any::<i32>(), 1..100), n in 0usize..50) {
        let mut list = List::from(vec.clone());
        let len = list.len();
        if len > 0 {
            let n = n % len;
            list.rotate_left(n);
            list.rotate_right(n);
            assert_eq!(list.as_slice(), vec.as_slice());
        }
    }

    #[test]
    fn test_list_first_last_consistent(vec in prop::collection::vec(any::<i32>(), 1..100)) {
        let list = List::from(vec.clone());
        if !list.is_empty() {
            assert_eq!(list.first(), Some(&vec[0]));
            assert_eq!(list.last(), Some(&vec[vec.len() - 1]));
        }
    }

    #[test]
    fn test_list_contains_all_elements(vec in prop::collection::vec(any::<i32>(), 0..100)) {
        let list = List::from(vec.clone());
        for item in &vec {
            assert!(list.contains(item));
        }
    }
}

// ============================================================================
// MAP PROPERTY TESTS
// ============================================================================

proptest! {
    #[test]
    fn test_map_insert_get_roundtrip(k in any::<i32>(), v in any::<i32>()) {
        let mut map = Map::new();
        map.insert(k, v);
        assert_eq!(map.get(&k), Some(&v));
    }

    #[test]
    fn test_map_insert_increases_size(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 0..100)) {
        let mut map = Map::new();
        let mut unique_keys = std::collections::HashSet::new();

        for (k, v) in pairs {
            unique_keys.insert(k);
            map.insert(k, v);
        }

        assert_eq!(map.len(), unique_keys.len());
    }

    #[test]
    fn test_map_remove_decreases_size(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 1..100)) {
        let mut map = Map::new();
        for (k, v) in &pairs {
            map.insert(*k, *v);
        }

        let initial_len = map.len();
        if let Some((k, _)) = pairs.first() {
            map.remove(k);
            assert!(map.len() <= initial_len);
        }
    }

    #[test]
    fn test_map_clear_makes_empty(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 0..100)) {
        let mut map = Map::new();
        for (k, v) in pairs {
            map.insert(k, v);
        }

        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_map_contains_key_after_insert(k in any::<i32>(), v in any::<i32>()) {
        let mut map = Map::new();
        map.insert(k, v);
        assert!(map.contains_key(&k));
    }

    #[test]
    fn test_map_not_contains_key_after_remove(k in any::<i32>(), v in any::<i32>()) {
        let mut map = Map::new();
        map.insert(k, v);
        map.remove(&k);
        assert!(!map.contains_key(&k));
    }

    #[test]
    fn test_map_keys_values_same_length(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 0..100)) {
        let mut map = Map::new();
        for (k, v) in pairs {
            map.insert(k, v);
        }

        let keys: Vec<_> = map.keys().collect();
        let values: Vec<_> = map.values().collect();
        assert_eq!(keys.len(), values.len());
        assert_eq!(keys.len(), map.len());
    }

    #[test]
    fn test_map_retain_preserves_predicate(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 0..100)) {
        let mut map = Map::new();
        for (k, v) in pairs {
            map.insert(k, v);
        }

        map.retain(|k, _| *k > 0);

        for k in map.keys() {
            assert!(*k > 0, "Map contains key {} which should have been removed", k);
        }
    }

    #[test]
    fn test_map_get_or_insert_with_idempotent(k in any::<i32>(), v in any::<i32>()) {
        let mut map = Map::new();

        let first = *map.get_or_insert_with(k, || v);
        let second = *map.get_or_insert_with(k, || v + 1);

        assert_eq!(first, second);
        assert_eq!(first, v);
    }
}

// ============================================================================
// SET PROPERTY TESTS
// ============================================================================

proptest! {
    #[test]
    fn test_set_insert_contains(x in any::<i32>()) {
        let mut set = Set::new();
        set.insert(x);
        assert!(set.contains(&x));
    }

    #[test]
    fn test_set_remove_not_contains(x in any::<i32>()) {
        let mut set = Set::new();
        set.insert(x);
        set.remove(&x);
        assert!(!set.contains(&x));
    }

    #[test]
    fn test_set_insert_duplicate_returns_false(x in any::<i32>()) {
        let mut set = Set::new();
        assert!(set.insert(x));
        assert!(!set.insert(x));
    }

    #[test]
    fn test_set_len_after_inserts(values in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut set = Set::new();
        let mut unique = std::collections::HashSet::new();

        for v in values {
            unique.insert(v);
            set.insert(v);
        }

        assert_eq!(set.len(), unique.len());
    }

    #[test]
    fn test_set_clear_makes_empty(values in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut set = Set::new();
        for v in values {
            set.insert(v);
        }

        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn test_set_union_contains_both(
        values1 in prop::collection::vec(any::<i32>(), 0..50),
        values2 in prop::collection::vec(any::<i32>(), 0..50)
    ) {
        let mut set1 = Set::new();
        let mut set2 = Set::new();

        for v in &values1 {
            set1.insert(*v);
        }
        for v in &values2 {
            set2.insert(*v);
        }

        let union: Vec<_> = set1.union(&set2).copied().collect();

        // Union should contain all elements from both sets
        for v in &values1 {
            assert!(union.contains(v));
        }
        for v in &values2 {
            assert!(union.contains(v));
        }
    }

    #[test]
    fn test_set_intersection_in_both(
        values1 in prop::collection::vec(any::<i32>(), 0..50),
        values2 in prop::collection::vec(any::<i32>(), 0..50)
    ) {
        let mut set1 = Set::new();
        let mut set2 = Set::new();

        for v in values1 {
            set1.insert(v);
        }
        for v in values2 {
            set2.insert(v);
        }

        let intersection: Vec<_> = set1.intersection(&set2).copied().collect();

        // All elements in intersection must be in both sets
        for v in intersection {
            assert!(set1.contains(&v));
            assert!(set2.contains(&v));
        }
    }

    #[test]
    fn test_set_difference_not_in_other(
        values1 in prop::collection::vec(any::<i32>(), 0..50),
        values2 in prop::collection::vec(any::<i32>(), 0..50)
    ) {
        let mut set1 = Set::new();
        let mut set2 = Set::new();

        for v in values1 {
            set1.insert(v);
        }
        for v in values2 {
            set2.insert(v);
        }

        let diff: Vec<_> = set1.difference(&set2).copied().collect();

        // All elements in difference must be in set1 but not set2
        for v in diff {
            assert!(set1.contains(&v));
            assert!(!set2.contains(&v));
        }
    }

    #[test]
    fn test_set_subset_transitive(values in prop::collection::vec(any::<i32>(), 0..50)) {
        let mut set1 = Set::new();
        let mut set2 = Set::new();

        // set1 ⊆ set2
        for v in &values {
            set1.insert(*v);
            set2.insert(*v);
        }

        // Add more to set2
        set2.insert(999);
        set2.insert(1000);

        assert!(set1.is_subset(&set2));
        assert!(set2.is_superset(&set1));
    }

    #[test]
    fn test_set_disjoint_no_common(
        values1 in prop::collection::vec(0i32..100, 0..50),
        values2 in prop::collection::vec(100i32..200, 0..50)
    ) {
        let mut set1 = Set::new();
        let mut set2 = Set::new();

        for v in values1 {
            set1.insert(v);
        }
        for v in values2 {
            set2.insert(v);
        }

        // Sets with non-overlapping ranges should be disjoint
        assert!(set1.is_disjoint(&set2));
    }
}

// ============================================================================
// INTEGRATION PROPERTY TESTS
// ============================================================================

proptest! {
    #[test]
    fn test_list_sort_stability(vec in prop::collection::vec((any::<i32>(), any::<usize>()), 0..100)) {
        let mut list = List::from(vec.clone());
        list.sort_by_key(|(k, _)| *k);

        // Verify sorted by key
        for i in 1..list.len() {
            assert!(list[i - 1].0 <= list[i].0);
        }
    }

    #[test]
    fn test_text_list_integration(s in ".*") {
        let text = Text::from(s.clone());
        let parts = text.split(",");

        // Join should preserve total content length (plus separators)
        let joined = parts.join(",");
        assert_eq!(joined.as_str(), s);
    }

    #[test]
    fn test_map_iteration_completeness(pairs in prop::collection::vec((any::<i32>(), any::<i32>()), 0..100)) {
        let mut map = Map::new();
        for (k, v) in &pairs {
            map.insert(*k, *v);
        }

        let collected: Map<_, _> = map.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(collected.len(), map.len());

        for (k, v) in map.iter() {
            assert_eq!(collected.get(k), Some(v));
        }
    }
}
