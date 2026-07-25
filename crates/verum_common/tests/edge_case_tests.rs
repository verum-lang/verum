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
//! Edge case and error path tests for verum_common semantic types
//!
//! Tests critical edge cases, boundary conditions, error paths, and concurrency

use std::sync::Arc;
use std::thread;
use verum_common::semantic_types::{List, Map, OrderedMap, OrderedSet, Set, Text};

// ============================================================================
// TEXT EDGE CASES
// ============================================================================

#[test]
fn test_text_empty_operations() {
    let mut text = Text::new();

    // Pop from empty
    assert_eq!(text.pop(), None);

    // Remove prefix/suffix from empty
    text.remove_prefix(5);
    assert_eq!(text.as_str(), "");
    text.remove_suffix(5);
    assert_eq!(text.as_str(), "");

    // Split empty string
    let parts = text.split(",");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].as_str(), "");

    // Clear empty string (should not panic)
    text.clear();
    assert!(text.is_empty());
}

#[test]
fn test_text_unicode_boundaries() {
    // Multi-byte characters
    let text = Text::from("你好世界🌍");
    assert_eq!(text.chars().count(), 5);

    // Truncate at character boundary
    let mut text1 = Text::from("你好世界");
    text1.truncate(1);
    assert_eq!(text1.as_str(), "你");

    // Truncate beyond length
    let mut text2 = Text::from("abc");
    text2.truncate(1000);
    assert_eq!(text2.as_str(), "abc");

    // Remove prefix/suffix with unicode
    let mut text3 = Text::from("你好世界");
    text3.remove_prefix(1);
    assert_eq!(text3.as_str(), "好世界");
    text3.remove_suffix(1);
    assert_eq!(text3.as_str(), "好世");
}

#[test]
fn test_text_boundary_conditions() {
    // Zero-length truncate
    let mut text = Text::from("hello");
    text.truncate(0);
    assert_eq!(text.as_str(), "");

    // Zero-length pad
    let text2 = Text::from("test");
    let padded = text2.pad_left(0, '0');
    assert_eq!(padded.as_str(), "test");

    // Pad to same length
    let text3 = Text::from("test");
    let padded = text3.pad_right(4, '0');
    assert_eq!(padded.as_str(), "test");

    // Zero repeat
    let text4 = Text::from("ha");
    let repeated = text4.repeat(0);
    assert_eq!(repeated.as_str(), "");

    // Single repeat
    let text5 = Text::from("ha");
    let repeated = text5.repeat(1);
    assert_eq!(repeated.as_str(), "ha");
}

#[test]
fn test_text_large_strings() {
    // Large string allocation
    let large = "a".repeat(1_000_000);
    let text = Text::from(large.as_str());
    assert_eq!(text.len(), 1_000_000);

    // Large string operations
    let mut text2 = Text::with_capacity(1_000_000);
    for _ in 0..1_000_000 {
        text2.push('a');
    }
    assert_eq!(text2.len(), 1_000_000);
}

#[test]
fn test_text_special_characters() {
    // Null character
    let text = Text::from("hello\0world");
    assert_eq!(text.len(), 11);
    assert!(text.contains("\0"));

    // All whitespace
    let text2 = Text::from("   \t\n\r");
    let parts = text2.split_whitespace();
    assert_eq!(parts.len(), 0);

    // Only newlines
    let text3 = Text::from("\n\n\n");
    let lines = text3.lines();
    assert_eq!(lines.len(), 3);
}

#[test]
fn test_text_invalid_indices() {
    let _text = Text::from("hello");

    // Insert at invalid index should panic (expected behavior)
    // Note: This test documents the panic behavior
    let result = std::panic::catch_unwind(|| {
        let mut t = Text::from("hello");
        t.insert(100, 'x');
    });
    assert!(result.is_err());
}

#[test]
fn test_text_concurrent_read() {
    let text = Arc::new(Text::from("hello world"));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let text = Arc::clone(&text);
            thread::spawn(move || {
                for _ in 0..1000 {
                    assert_eq!(text.as_str(), "hello world");
                    assert_eq!(text.len(), 11);
                    assert!(text.contains("world"));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// LIST EDGE CASES
// ============================================================================

#[test]
fn test_list_empty_operations() {
    let mut list: List<i32> = List::new();

    // Pop from empty
    assert_eq!(list.pop(), None);

    // First/last on empty
    assert_eq!(list.first(), None);
    assert_eq!(list.last(), None);
    assert_eq!(list.first_mut(), None);
    assert_eq!(list.last_mut(), None);

    // Clear empty list
    list.clear();
    assert!(list.is_empty());

    // Truncate empty list
    list.truncate(5);
    assert!(list.is_empty());

    // Reverse empty list
    list.reverse();
    assert!(list.is_empty());

    // Sort empty list
    list.sort();
    assert!(list.is_empty());
}

#[test]
fn test_list_single_element() {
    let mut list = List::from(vec![42]);

    // Pop single element
    assert_eq!(list.pop(), Some(42));
    assert!(list.is_empty());

    // Reverse single element
    let mut list2 = List::from(vec![42]);
    list2.reverse();
    assert_eq!(list2[0], 42);

    // Sort single element
    let mut list3 = List::from(vec![42]);
    list3.sort();
    assert_eq!(list3[0], 42);
}

#[test]
fn test_list_boundary_indices() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);

    // Get at boundaries
    assert_eq!(list.get(0), Some(&1));
    assert_eq!(list.get(4), Some(&5));
    assert_eq!(list.get(5), None);
    assert_eq!(list.get(100), None);

    // Remove at boundaries
    let last = list.remove(4);
    assert_eq!(last, 5);
    assert_eq!(list.len(), 4);

    let first = list.remove(0);
    assert_eq!(first, 1);
    assert_eq!(list.len(), 3);
}

#[test]
fn test_list_zero_capacity() {
    let list: List<i32> = List::with_capacity(0);
    assert!(list.is_empty());
    assert_eq!(list.capacity(), 0);
}

#[test]
fn test_list_large_capacity() {
    let list: List<i32> = List::with_capacity(1_000_000);
    assert!(list.capacity() >= 1_000_000);
    assert!(list.is_empty());
}

#[test]
fn test_list_resize_edge_cases() {
    // Resize to 0
    let mut list = List::from(vec![1, 2, 3]);
    list.resize(0, 0);
    assert!(list.is_empty());

    // Resize to same size
    let mut list2 = List::from(vec![1, 2, 3]);
    list2.resize(3, 0);
    assert_eq!(list2.len(), 3);
    assert_eq!(list2[0], 1);
    assert_eq!(list2[1], 2);
    assert_eq!(list2[2], 3);

    // Resize smaller
    let mut list3 = List::from(vec![1, 2, 3, 4, 5]);
    list3.resize(2, 0);
    assert_eq!(list3.len(), 2);
    assert_eq!(list3[0], 1);
    assert_eq!(list3[1], 2);
}

#[test]
fn test_list_dedup_edge_cases() {
    // Empty list
    let mut list: List<i32> = List::new();
    list.dedup();
    assert!(list.is_empty());

    // Single element
    let mut list2 = List::from(vec![1]);
    list2.dedup();
    assert_eq!(list2.len(), 1);

    // No duplicates
    let mut list3 = List::from(vec![1, 2, 3]);
    list3.dedup();
    assert_eq!(list3.len(), 3);

    // All duplicates
    let mut list4 = List::from(vec![1, 1, 1, 1]);
    list4.dedup();
    assert_eq!(list4.len(), 1);
}

#[test]
fn test_list_split_edge_cases() {
    // Split at 0
    let list = List::from(vec![1, 2, 3]);
    let (left, right) = list.split_at(0);
    assert_eq!(left.len(), 0);
    assert_eq!(right.len(), 3);

    // Split at end
    let list2 = List::from(vec![1, 2, 3]);
    let (left, right) = list2.split_at(3);
    assert_eq!(left.len(), 3);
    assert_eq!(right.len(), 0);

    // Split single element at 0
    let list3 = List::from(vec![42]);
    let (left, right) = list3.split_at(0);
    assert_eq!(left.len(), 0);
    assert_eq!(right.len(), 1);
}

#[test]
fn test_list_rotate_edge_cases() {
    // Rotate by 0
    let mut list = List::from(vec![1, 2, 3]);
    list.rotate_left(0);
    assert_eq!(list[0], 1);
    assert_eq!(list[1], 2);
    assert_eq!(list[2], 3);

    // Rotate by length
    let mut list2 = List::from(vec![1, 2, 3]);
    list2.rotate_left(3);
    assert_eq!(list2[0], 1);
    assert_eq!(list2[1], 2);
    assert_eq!(list2[2], 3);

    // Rotate empty list
    let mut list3: List<i32> = List::new();
    list3.rotate_left(5);
    assert!(list3.is_empty());
}

#[test]
fn test_list_concurrent_read() {
    let list = Arc::new(List::from(vec![1, 2, 3, 4, 5]));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let list = Arc::clone(&list);
            thread::spawn(move || {
                for _ in 0..1000 {
                    assert_eq!(list.len(), 5);
                    assert_eq!(list[0], 1);
                    assert_eq!(list[4], 5);
                    assert!(list.contains(&3));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// MAP EDGE CASES
// ============================================================================

#[test]
fn test_map_empty_operations() {
    let mut map: Map<i32, Text> = Map::new();

    // Get from empty
    assert_eq!(map.get(&1), None);

    // Remove from empty
    assert_eq!(map.remove(&1), None);

    // Contains on empty
    assert!(!map.contains_key(&1));

    // Clear empty
    map.clear();
    assert!(map.is_empty());

    // Retain on empty
    map.retain(|_, _| true);
    assert!(map.is_empty());
}

#[test]
fn test_map_zero_capacity() {
    let map: Map<i32, i32> = Map::with_capacity(0);
    assert!(map.is_empty());
    assert_eq!(map.capacity(), 0);
}

#[test]
fn test_map_large_capacity() {
    let map: Map<i32, i32> = Map::with_capacity(1_000_000);
    assert!(map.capacity() >= 1_000_000);
    assert!(map.is_empty());
}

#[test]
fn test_map_overwrite_behavior() {
    let mut map = Map::new();

    // Insert new
    assert_eq!(map.insert(1, Text::from("first")), None);

    // Overwrite existing
    let old = map.insert(1, Text::from("second"));
    assert_eq!(old.unwrap().as_str(), "first");
    assert_eq!(map.get(&1).unwrap().as_str(), "second");
}

#[test]
fn test_map_retain_all_removed() {
    let mut map = Map::new();
    map.insert(1, 10);
    map.insert(2, 20);
    map.insert(3, 30);

    // Remove all
    map.retain(|_, _| false);
    assert!(map.is_empty());
}

#[test]
fn test_map_retain_none_removed() {
    let mut map = Map::new();
    map.insert(1, 10);
    map.insert(2, 20);
    map.insert(3, 30);

    // Keep all
    let original_len = map.len();
    map.retain(|_, _| true);
    assert_eq!(map.len(), original_len);
}

#[test]
fn test_map_concurrent_read() {
    let mut initial_map = Map::new();
    for i in 0..100 {
        initial_map.insert(i, i * 10);
    }
    let map = Arc::new(initial_map);

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let map = Arc::clone(&map);
            thread::spawn(move || {
                for _ in 0..1000 {
                    assert_eq!(map.len(), 100);
                    assert_eq!(map.get(&5), Some(&50));
                    assert!(map.contains_key(&10));
                    assert!(!map.contains_key(&200));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// SET EDGE CASES
// ============================================================================

#[test]
fn test_set_empty_operations() {
    let mut set: Set<i32> = Set::new();

    // Insert into empty
    assert!(set.insert(1));

    // Remove from one-element
    assert!(set.remove(&1));
    assert!(set.is_empty());

    // Operations on empty
    assert!(!set.contains(&1));
    assert!(set.is_empty());

    set.clear();
    assert!(set.is_empty());
}

#[test]
fn test_set_duplicate_inserts() {
    let mut set = Set::new();

    // First insert succeeds
    assert!(set.insert(1));

    // Second insert fails (already exists)
    assert!(!set.insert(1));

    // Still only one element
    assert_eq!(set.len(), 1);
}

#[test]
fn test_set_operations_edge_cases() {
    // Empty set operations
    let empty: Set<i32> = Set::new();
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    // Union with empty
    let union: Vec<_> = empty.union(&set1).copied().collect();
    assert_eq!(union.len(), 2);

    // Intersection with empty
    let intersection: Vec<_> = set1.intersection(&empty).copied().collect();
    assert_eq!(intersection.len(), 0);

    // Difference with empty
    let diff: Vec<_> = set1.difference(&empty).copied().collect();
    assert_eq!(diff.len(), 2);
}

#[test]
fn test_set_subset_superset_edge_cases() {
    let empty: Set<i32> = Set::new();
    let mut set = Set::new();
    set.insert(1);

    // Empty is subset of everything
    assert!(empty.is_subset(&set));
    assert!(empty.is_subset(&empty));

    // Everything is superset of empty
    assert!(set.is_superset(&empty));
    assert!(empty.is_superset(&empty));

    // Set with itself
    assert!(set.is_subset(&set));
    assert!(set.is_superset(&set));
}

#[test]
fn test_set_disjoint_edge_cases() {
    let empty: Set<i32> = Set::new();
    let mut set = Set::new();
    set.insert(1);

    // Empty is disjoint with everything
    assert!(empty.is_disjoint(&set));
    assert!(empty.is_disjoint(&empty));
}

// ============================================================================
// ORDERED COLLECTIONS EDGE CASES
// ============================================================================

#[test]
fn test_ordered_map_empty_operations() {
    let mut map: OrderedMap<i32, Text> = OrderedMap::new();

    assert_eq!(map.first_key_value(), None);
    assert_eq!(map.last_key_value(), None);
    assert_eq!(map.pop_first(), None);
    assert_eq!(map.pop_last(), None);
}

#[test]
fn test_ordered_map_single_element() {
    let mut map = OrderedMap::new();
    map.insert(1, Text::from("one"));

    // First and last are the same
    assert_eq!(map.first_key_value().unwrap().0, &1);
    assert_eq!(map.last_key_value().unwrap().0, &1);

    // Pop first
    let (k, v) = map.pop_first().unwrap();
    assert_eq!(k, 1);
    assert_eq!(v.as_str(), "one");
    assert!(map.is_empty());
}

#[test]
fn test_ordered_set_empty_operations() {
    let mut set: OrderedSet<i32> = OrderedSet::new();

    assert_eq!(set.first(), None);
    assert_eq!(set.last(), None);
    assert_eq!(set.pop_first(), None);
    assert_eq!(set.pop_last(), None);
}

#[test]
fn test_ordered_set_single_element() {
    let mut set = OrderedSet::new();
    set.insert(42);

    // First and last are the same
    assert_eq!(set.first(), Some(&42));
    assert_eq!(set.last(), Some(&42));

    // Pop first
    assert_eq!(set.pop_first(), Some(42));
    assert!(set.is_empty());
}

#[test]
fn test_ordered_collections_ordering() {
    // Verify ordering is maintained with random inserts
    let mut map = OrderedMap::new();
    let values = vec![5, 1, 9, 3, 7, 2, 8, 4, 6];

    for (i, &v) in values.iter().enumerate() {
        map.insert(v, i);
    }

    // Keys should be sorted
    let keys: Vec<_> = map.keys().copied().collect();
    let mut expected = values.clone();
    expected.sort();
    assert_eq!(keys, expected);
}

// ============================================================================
// MEMORY STRESS TESTS
// ============================================================================

#[test]
fn test_text_memory_stress() {
    // Allocate and deallocate many strings
    for _ in 0..1000 {
        let text = Text::from("test");
        assert_eq!(text.as_str(), "test");
    }

    // Large string operations
    let mut large = Text::with_capacity(100_000);
    for _ in 0..100_000 {
        large.push('a');
    }
    large.clear();
    assert!(large.is_empty());
}

#[test]
fn test_list_memory_stress() {
    // Allocate and deallocate many lists
    for _ in 0..1000 {
        let list = List::from(vec![1, 2, 3, 4, 5]);
        assert_eq!(list.len(), 5);
    }

    // Large list operations
    let mut large = List::with_capacity(100_000);
    for i in 0..100_000 {
        large.push(i);
    }
    large.clear();
    assert!(large.is_empty());
}

#[test]
fn test_map_memory_stress() {
    // Allocate and deallocate many maps
    for _ in 0..1000 {
        let mut map = Map::new();
        for i in 0..10 {
            map.insert(i, i * 10);
        }
        assert_eq!(map.len(), 10);
    }

    // Large map operations
    let mut large = Map::with_capacity(10_000);
    for i in 0..10_000 {
        large.insert(i, i * 10);
    }
    large.clear();
    assert!(large.is_empty());
}

// ============================================================================
// NESTED COLLECTIONS EDGE CASES
// ============================================================================

#[test]
fn test_deeply_nested_collections() {
    // Map<Text, List<Map<i32, Text>>>
    let mut outer: Map<Text, List<Map<i32, Text>>> = Map::new();

    let mut inner_list = List::new();
    for i in 0..3 {
        let mut inner_map = Map::new();
        inner_map.insert(i, Text::from(format!("value_{}", i)));
        inner_list.push(inner_map);
    }

    outer.insert(Text::from("key"), inner_list);

    // Verify structure
    let retrieved = outer.get(&Text::from("key")).unwrap();
    assert_eq!(retrieved.len(), 3);
    assert_eq!(retrieved[0].get(&0).unwrap().as_str(), "value_0");
}

#[test]
fn test_empty_nested_collections() {
    // Map of empty lists
    let mut map: Map<i32, List<Text>> = Map::new();
    map.insert(1, List::new());
    map.insert(2, List::new());

    assert_eq!(map.len(), 2);
    assert!(map.get(&1).unwrap().is_empty());
    assert!(map.get(&2).unwrap().is_empty());
}
