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
//! Comprehensive tests for semantic types API
//!
//! Tests all methods added to Text, List, Map, Set, OrderedMap, OrderedSet

use verum_common::semantic_types::{List, Map, OrderedMap, OrderedSet, Set, Text};

// ============================================================================
// TEXT TYPE TESTS
// ============================================================================

#[test]
fn test_text_new() {
    let text = Text::new();
    assert!(text.is_empty());
    assert_eq!(text.len(), 0);
}

#[test]
fn test_text_with_capacity() {
    let text = Text::with_capacity(100);
    assert!(text.capacity() >= 100);
    assert!(text.is_empty());
}

#[test]
fn test_text_from_utf8() {
    let bytes = vec![72, 101, 108, 108, 111]; // "Hello"
    let text = Text::from_utf8(bytes).unwrap();
    assert_eq!(text.as_str(), "Hello");
}

#[test]
fn test_text_from_utf8_lossy() {
    let bytes = vec![72, 101, 108, 108, 111, 0xFF]; // "Hello" + invalid byte
    let text = Text::from_utf8_lossy(&bytes);
    assert!(text.starts_with("Hello"));
}

#[test]
fn test_text_pop() {
    let mut text = Text::from("hello");
    assert_eq!(text.pop(), Some('o'));
    assert_eq!(text.as_str(), "hell");
    assert_eq!(text.pop(), Some('l'));
    assert_eq!(text.as_str(), "hel");

    let mut empty = Text::new();
    assert_eq!(empty.pop(), None);
}

#[test]
fn test_text_push() {
    let mut text = Text::from("hello");
    text.push('!');
    assert_eq!(text.as_str(), "hello!");
    text.push('?');
    assert_eq!(text.as_str(), "hello!?");
}

#[test]
fn test_text_push_str() {
    let mut text = Text::from("hello");
    text.push_str(" world");
    assert_eq!(text.as_str(), "hello world");
}

#[test]
fn test_text_remove_prefix() {
    let mut text = Text::from("hello");
    text.remove_prefix(2);
    assert_eq!(text.as_str(), "llo");

    let mut text2 = Text::from("test");
    text2.remove_prefix(10); // More than length
    assert!(text2.is_empty());
}

#[test]
fn test_text_remove_suffix() {
    let mut text = Text::from("hello");
    text.remove_suffix(2);
    assert_eq!(text.as_str(), "hel");

    let mut text2 = Text::from("test");
    text2.remove_suffix(10); // More than length
    assert!(text2.is_empty());
}

#[test]
fn test_text_truncate() {
    let mut text = Text::from("hello world");
    text.truncate(5);
    assert_eq!(text.as_str(), "hello");

    // Should handle unicode properly
    let mut unicode = Text::from("你好世界");
    unicode.truncate(2);
    assert_eq!(unicode.as_str(), "你好");
}

#[test]
fn test_text_clear() {
    let mut text = Text::from("hello");
    text.clear();
    assert!(text.is_empty());
}

#[test]
fn test_text_insert() {
    let mut text = Text::from("helo");
    text.insert(3, 'l');
    assert_eq!(text.as_str(), "hello");
}

#[test]
fn test_text_insert_str() {
    let mut text = Text::from("heo");
    text.insert_str(2, "ll");
    assert_eq!(text.as_str(), "hello");
}

#[test]
fn test_text_remove() {
    let mut text = Text::from("hello");
    let ch = text.remove(4);
    assert_eq!(ch, 'o');
    assert_eq!(text.as_str(), "hell");
}

#[test]
fn test_text_retain() {
    let mut text = Text::from("hello123");
    text.retain(|c| c.is_alphabetic());
    assert_eq!(text.as_str(), "hello");
}

#[test]
fn test_text_into_bytes() {
    let text = Text::from("hello");
    let bytes = text.into_bytes();
    assert_eq!(bytes, vec![104, 101, 108, 108, 111]);
}

#[test]
fn test_text_as_bytes() {
    let text = Text::from("hello");
    assert_eq!(text.as_bytes(), &[104, 101, 108, 108, 111]);
}

#[test]
fn test_text_starts_with() {
    let text = Text::from("hello world");
    assert!(text.starts_with("hello"));
    assert!(!text.starts_with("world"));
}

#[test]
fn test_text_ends_with() {
    let text = Text::from("hello world");
    assert!(text.ends_with("world"));
    assert!(!text.ends_with("hello"));
}

#[test]
fn test_text_find() {
    let text = Text::from("hello world");
    assert_eq!(text.find("world"), Some(6));
    assert_eq!(text.find("xyz"), None);
}

#[test]
fn test_text_rfind() {
    let text = Text::from("hello hello");
    assert_eq!(text.rfind("hello"), Some(6));
}

#[test]
fn test_text_contains() {
    let text = Text::from("hello world");
    assert!(text.contains("world"));
    assert!(!text.contains("xyz"));
}

#[test]
fn test_text_split() {
    let text = Text::from("a,b,c");
    let parts = text.split(",");
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].as_str(), "a");
    assert_eq!(parts[1].as_str(), "b");
    assert_eq!(parts[2].as_str(), "c");
}

#[test]
fn test_text_splitn() {
    let text = Text::from("a,b,c,d");
    let parts = text.splitn(2, ",");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].as_str(), "a");
    assert_eq!(parts[1].as_str(), "b,c,d");
}

#[test]
fn test_text_split_whitespace() {
    let text = Text::from("hello  world\t\ntest");
    let parts = text.split_whitespace();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].as_str(), "hello");
    assert_eq!(parts[1].as_str(), "world");
    assert_eq!(parts[2].as_str(), "test");
}

#[test]
fn test_text_lines() {
    let text = Text::from("line1\nline2\nline3");
    let lines = text.lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].as_str(), "line1");
    assert_eq!(lines[1].as_str(), "line2");
    assert_eq!(lines[2].as_str(), "line3");
}

#[test]
fn test_text_replace() {
    let text = Text::from("hello world");
    let replaced = text.replace("world", "rust");
    assert_eq!(replaced.as_str(), "hello rust");
}

#[test]
fn test_text_replacen() {
    let text = Text::from("hello hello hello");
    let replaced = text.replacen("hello", "hi", 2);
    assert_eq!(replaced.as_str(), "hi hi hello");
}

#[test]
fn test_text_to_lowercase() {
    let text = Text::from("HELLO World");
    let lower = text.to_lowercase();
    assert_eq!(lower.as_str(), "hello world");
}

#[test]
fn test_text_to_uppercase() {
    let text = Text::from("hello World");
    let upper = text.to_uppercase();
    assert_eq!(upper.as_str(), "HELLO WORLD");
}

#[test]
fn test_text_trim() {
    let text = Text::from("  hello  ");
    let trimmed = text.trim();
    assert_eq!(trimmed.as_str(), "hello");
}

#[test]
fn test_text_trim_start() {
    let text = Text::from("  hello  ");
    let trimmed = text.trim_start();
    assert_eq!(trimmed.as_str(), "hello  ");
}

#[test]
fn test_text_trim_end() {
    let text = Text::from("  hello  ");
    let trimmed = text.trim_end();
    assert_eq!(trimmed.as_str(), "  hello");
}

#[test]
fn test_text_repeat() {
    let text = Text::from("ha");
    let repeated = text.repeat(3);
    assert_eq!(repeated.as_str(), "hahaha");
}

#[test]
fn test_text_pad_left() {
    let text = Text::from("42");
    let padded = text.pad_left(5, '0');
    assert_eq!(padded.as_str(), "00042");
}

#[test]
fn test_text_pad_right() {
    let text = Text::from("42");
    let padded = text.pad_right(5, '0');
    assert_eq!(padded.as_str(), "42000");
}

#[test]
fn test_text_conversions() {
    let string = String::from("hello");
    let text: Text = string.clone().into();
    assert_eq!(text.as_str(), "hello");

    let back: String = text.into();
    assert_eq!(back, string);
}

#[test]
fn test_text_add() {
    let text1 = Text::from("hello");
    let text2 = Text::from(" world");
    let combined = text1 + text2;
    assert_eq!(combined.as_str(), "hello world");
}

#[test]
fn test_text_add_assign() {
    let mut text = Text::from("hello");
    text += Text::from(" world");
    assert_eq!(text.as_str(), "hello world");
}

// ============================================================================
// LIST TYPE TESTS
// ============================================================================

#[test]
fn test_list_new() {
    let list: List<i32> = List::new();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
}

#[test]
fn test_list_with_capacity() {
    let list: List<i32> = List::with_capacity(100);
    assert!(list.capacity() >= 100);
    assert!(list.is_empty());
}

#[test]
fn test_list_push_pop() {
    let mut list = List::new();
    list.push(1);
    list.push(2);
    list.push(3);
    assert_eq!(list.len(), 3);

    assert_eq!(list.pop(), Some(3));
    assert_eq!(list.pop(), Some(2));
    assert_eq!(list.pop(), Some(1));
    assert_eq!(list.pop(), None);
}

#[test]
fn test_list_insert_remove() {
    let mut list = List::from(vec![1, 2, 4]);
    list.insert(2, 3);
    assert_eq!(list[0], 1);
    assert_eq!(list[1], 2);
    assert_eq!(list[2], 3);
    assert_eq!(list[3], 4);

    let removed = list.remove(1);
    assert_eq!(removed, 2);
    assert_eq!(list.len(), 3);
}

#[test]
fn test_list_swap_remove() {
    let mut list = List::from(vec![1, 2, 3, 4]);
    let removed = list.swap_remove(1);
    assert_eq!(removed, 2);
    assert_eq!(list[1], 4); // Last element swapped here
}

#[test]
fn test_list_clear() {
    let mut list = List::from(vec![1, 2, 3]);
    list.clear();
    assert!(list.is_empty());
}

#[test]
fn test_list_truncate() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);
    list.truncate(3);
    assert_eq!(list.len(), 3);
    assert_eq!(list[2], 3);
}

#[test]
fn test_list_retain() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);
    list.retain(|&x| x % 2 == 0);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], 2);
    assert_eq!(list[1], 4);
}

#[test]
fn test_list_dedup() {
    let mut list = List::from(vec![1, 1, 2, 2, 3, 3]);
    list.dedup();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0], 1);
    assert_eq!(list[1], 2);
    assert_eq!(list[2], 3);
}

#[test]
fn test_list_dedup_by_key() {
    // dedup_by_key removes *consecutive* elements with same key
    // [1, 3, 2, 4] with key |x| % 2 -> keys [1, 1, 0, 0] -> keeps [1, 2]
    let mut list = List::from(vec![1, 3, 2, 4]);
    list.dedup_by_key(|x| *x % 2);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], 1); // First odd
    assert_eq!(list[1], 2); // First even
}

#[test]
fn test_list_append() {
    let mut list1 = List::from(vec![1, 2, 3]);
    let mut list2 = List::from(vec![4, 5, 6]);
    list1.append(&mut list2);
    assert_eq!(list1.len(), 6);
    assert!(list2.is_empty());
}

#[test]
fn test_list_first_last() {
    let list = List::from(vec![1, 2, 3]);
    assert_eq!(list.first(), Some(&1));
    assert_eq!(list.last(), Some(&3));

    let empty: List<i32> = List::new();
    assert_eq!(empty.first(), None);
    assert_eq!(empty.last(), None);
}

#[test]
fn test_list_first_mut_last_mut() {
    let mut list = List::from(vec![1, 2, 3]);
    *list.first_mut().unwrap() = 10;
    *list.last_mut().unwrap() = 30;
    assert_eq!(list[0], 10);
    assert_eq!(list[2], 30);
}

#[test]
fn test_list_get() {
    let list = List::from(vec![1, 2, 3]);
    assert_eq!(list.get(1), Some(&2));
    assert_eq!(list.get(10), None);
}

#[test]
fn test_list_contains() {
    let list = List::from(vec![1, 2, 3]);
    assert!(list.contains(&2));
    assert!(!list.contains(&10));
}

#[test]
fn test_list_binary_search() {
    let list = List::from(vec![1, 3, 5, 7, 9]);
    assert_eq!(list.binary_search(&5), Ok(2));
    assert!(list.binary_search(&4).is_err());
}

#[test]
fn test_list_split_at() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let (left, right) = list.split_at(2);
    assert_eq!(left, &[1, 2]);
    assert_eq!(right, &[3, 4, 5]);
}

#[test]
fn test_list_split_off() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);
    let right = list.split_off(2);
    assert_eq!(list.len(), 2);
    assert_eq!(right.len(), 3);
    assert_eq!(list[0], 1);
    assert_eq!(right[0], 3);
}

#[test]
fn test_list_reverse() {
    let mut list = List::from(vec![1, 2, 3, 4]);
    list.reverse();
    assert_eq!(list[0], 4);
    assert_eq!(list[1], 3);
    assert_eq!(list[2], 2);
    assert_eq!(list[3], 1);
}

#[test]
fn test_list_rotate_left() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);
    list.rotate_left(2);
    assert_eq!(list[0], 3);
    assert_eq!(list[1], 4);
    assert_eq!(list[2], 5);
    assert_eq!(list[3], 1);
    assert_eq!(list[4], 2);
}

#[test]
fn test_list_rotate_right() {
    let mut list = List::from(vec![1, 2, 3, 4, 5]);
    list.rotate_right(2);
    assert_eq!(list[0], 4);
    assert_eq!(list[1], 5);
    assert_eq!(list[2], 1);
    assert_eq!(list[3], 2);
    assert_eq!(list[4], 3);
}

#[test]
fn test_list_sort() {
    let mut list = List::from(vec![3, 1, 4, 1, 5, 9, 2, 6]);
    list.sort();
    assert_eq!(list[0], 1);
    assert_eq!(list[1], 1);
    assert_eq!(list[7], 9);
}

#[test]
fn test_list_sort_by_key() {
    let mut list = List::from(vec![(2, "b"), (1, "a"), (3, "c")]);
    list.sort_by_key(|&(n, _)| n);
    assert_eq!(list[0].0, 1);
    assert_eq!(list[1].0, 2);
    assert_eq!(list[2].0, 3);
}

#[test]
fn test_list_fill() {
    let mut list = List::from(vec![1, 2, 3, 4]);
    list.fill(0);
    assert_eq!(list[0], 0);
    assert_eq!(list[1], 0);
    assert_eq!(list[2], 0);
    assert_eq!(list[3], 0);
}

#[test]
fn test_list_fill_with() {
    let mut list = List::from(vec![0, 0, 0]);
    let mut counter = 1;
    list.fill_with(|| {
        let val = counter;
        counter += 1;
        val
    });
    assert_eq!(list[0], 1);
    assert_eq!(list[1], 2);
    assert_eq!(list[2], 3);
}

#[test]
fn test_list_resize() {
    let mut list = List::from(vec![1, 2]);
    list.resize(5, 0);
    assert_eq!(list.len(), 5);
    assert_eq!(list[2], 0);
    assert_eq!(list[4], 0);
}

#[test]
fn test_list_resize_with() {
    let mut list = List::from(vec![1]);
    let mut counter = 2;
    list.resize_with(3, || {
        let val = counter;
        counter += 1;
        val
    });
    assert_eq!(list.len(), 3);
    assert_eq!(list[1], 2);
    assert_eq!(list[2], 3);
}

#[test]
fn test_list_windows() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let windows: Vec<_> = list.windows(3).collect();
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0], &[1, 2, 3]);
    assert_eq!(windows[1], &[2, 3, 4]);
    assert_eq!(windows[2], &[3, 4, 5]);
}

#[test]
fn test_list_chunks() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let chunks: Vec<_> = list.chunks(2).collect();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0], &[1, 2]);
    assert_eq!(chunks[1], &[3, 4]);
    assert_eq!(chunks[2], &[5]);
}

#[test]
fn test_list_map() {
    let list = List::from(vec![1, 2, 3]);
    let doubled = list.map(|x| x * 2);
    assert_eq!(doubled[0], 2);
    assert_eq!(doubled[1], 4);
    assert_eq!(doubled[2], 6);
}

#[test]
fn test_list_filter() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let evens = list.filter(|&x| x % 2 == 0);
    assert_eq!(evens.len(), 2);
    assert_eq!(evens[0], 2);
    assert_eq!(evens[1], 4);
}

#[test]
fn test_list_filter_map() {
    let list = List::from(vec![1, 2, 3, 4]);
    let result = list.filter_map(|x| if x % 2 == 0 { Some(x * 10) } else { None });
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], 20);
    assert_eq!(result[1], 40);
}

#[test]
fn test_list_flat_map() {
    let list = List::from(vec![1, 2, 3]);
    let result = list.flat_map(|x| vec![x, x * 10]);
    assert_eq!(result.len(), 6);
    assert_eq!(result[0], 1);
    assert_eq!(result[1], 10);
    assert_eq!(result[2], 2);
    assert_eq!(result[3], 20);
}

#[test]
fn test_list_fold() {
    let list = List::from(vec![1, 2, 3, 4]);
    let sum = list.fold(0, |acc, x| acc + x);
    assert_eq!(sum, 10);
}

#[test]
fn test_list_take() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let taken = list.take(3);
    assert_eq!(taken.len(), 3);
    assert_eq!(taken[0], 1);
    assert_eq!(taken[2], 3);
}

#[test]
fn test_list_skip() {
    let list = List::from(vec![1, 2, 3, 4, 5]);
    let skipped = list.skip(2);
    assert_eq!(skipped.len(), 3);
    assert_eq!(skipped[0], 3);
    assert_eq!(skipped[2], 5);
}

#[test]
fn test_list_join() {
    let list = List::from(vec![1, 2, 3]);
    let joined = list.join(", ");
    assert_eq!(joined.as_str(), "1, 2, 3");
}

#[test]
fn test_list_indexing() {
    let mut list = List::from(vec![1, 2, 3]);
    assert_eq!(list[1], 2);
    list[1] = 20;
    assert_eq!(list[1], 20);
}

// ============================================================================
// MAP TYPE TESTS
// ============================================================================

#[test]
fn test_map_new() {
    let map: Map<i32, Text> = Map::new();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
}

#[test]
fn test_map_with_capacity() {
    let map: Map<i32, Text> = Map::with_capacity(100);
    assert!(map.capacity() >= 100);
    assert!(map.is_empty());
}

#[test]
fn test_map_insert_get() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    assert_eq!(map.get(&1).unwrap().as_str(), "one");
    assert_eq!(map.get(&2).unwrap().as_str(), "two");
    assert_eq!(map.get(&3), None);
}

#[test]
fn test_map_insert_replace() {
    let mut map = Map::new();
    assert_eq!(map.insert(1, Text::from("one")), None);
    assert_eq!(map.insert(1, Text::from("ONE")).unwrap().as_str(), "one");
}

#[test]
fn test_map_remove() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));

    let removed = map.remove(&1);
    assert_eq!(removed.unwrap().as_str(), "one");
    assert!(map.is_empty());
}

#[test]
fn test_map_contains_key() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));

    assert!(map.contains_key(&1));
    assert!(!map.contains_key(&2));
}

#[test]
fn test_map_clear() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    map.clear();
    assert!(map.is_empty());
}

#[test]
fn test_map_get_or_insert_with() {
    let mut map = Map::new();
    let value = map.get_or_insert_with(1, || Text::from("one"));
    assert_eq!(value.as_str(), "one");

    let value2 = map.get_or_insert_with(1, || Text::from("ONE"));
    assert_eq!(value2.as_str(), "one"); // Should return existing
}

#[test]
fn test_map_retain() {
    let mut map = Map::new();
    map.insert(1, 10);
    map.insert(2, 20);
    map.insert(3, 30);

    map.retain(|&k, &mut v| k % 2 == 0 && v > 15);
    assert_eq!(map.len(), 1);
    assert!(map.contains_key(&2));
}

#[test]
fn test_map_keys_values() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    let keys: Vec<_> = map.keys().collect();
    assert_eq!(keys.len(), 2);

    let values: Vec<_> = map.values().collect();
    assert_eq!(values.len(), 2);
}

#[test]
fn test_map_iter() {
    let mut map = Map::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    let count = map.iter().count();
    assert_eq!(count, 2);
}

#[test]
fn test_map_from_iterator() {
    let pairs = vec![(1, Text::from("one")), (2, Text::from("two"))];
    let map: Map<_, _> = pairs.into_iter().collect();
    assert_eq!(map.len(), 2);
}

#[test]
fn test_map_get_many_mut_basic() {
    let mut map = Map::new();
    map.insert("a", 1);
    map.insert("b", 2);
    map.insert("c", 3);

    // Get two distinct keys
    if let Some([a, b]) = map.get_many_mut(["a", "b"]) {
        *a += 10;
        *b += 20;
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&"a"), Some(&11));
    assert_eq!(map.get(&"b"), Some(&22));
    assert_eq!(map.get(&"c"), Some(&3));
}

#[test]
fn test_map_get_many_mut_three_keys() {
    let mut map = Map::new();
    map.insert(1, 100);
    map.insert(2, 200);
    map.insert(3, 300);

    // Get three distinct keys
    if let Some([x, y, z]) = map.get_many_mut([&1, &2, &3]) {
        *x *= 2;
        *y *= 3;
        *z *= 4;
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&1), Some(&200));
    assert_eq!(map.get(&2), Some(&600));
    assert_eq!(map.get(&3), Some(&1200));
}

#[test]
fn test_map_get_many_mut_duplicate_keys() {
    let mut map = Map::new();
    map.insert("a", 1);
    map.insert("b", 2);

    // Duplicate keys should return None (aliasing not allowed)
    let result = map.get_many_mut(["a", "a"]);
    assert!(result.is_none());

    // Values should be unchanged
    assert_eq!(map.get(&"a"), Some(&1));
    assert_eq!(map.get(&"b"), Some(&2));
}

#[test]
fn test_map_get_many_mut_missing_key() {
    let mut map = Map::new();
    map.insert("a", 1);
    map.insert("b", 2);

    // One key exists, one doesn't - should return None
    let result = map.get_many_mut(["a", "missing"]);
    assert!(result.is_none());

    // Values should be unchanged
    assert_eq!(map.get(&"a"), Some(&1));
}

#[test]
fn test_map_get_many_mut_all_missing() {
    let mut map: Map<&str, i32> = Map::new();
    map.insert("a", 1);

    // All keys missing - should return None
    let result = map.get_many_mut(["x", "y"]);
    assert!(result.is_none());
}

#[test]
fn test_map_get_many_mut_single_key() {
    let mut map = Map::new();
    map.insert("a", 1);

    // Single key (N=1) should work
    if let Some([a]) = map.get_many_mut(["a"]) {
        *a = 100;
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&"a"), Some(&100));
}

#[test]
fn test_map_get_many_mut_empty_array() {
    let mut map = Map::new();
    map.insert("a", 1);

    // Empty array (N=0) should return Some with empty array
    let result: Option<[&mut i32; 0]> = map.get_many_mut::<&str, 0>([]);
    assert!(result.is_some());
}

#[test]
fn test_map_get_many_mut_borrowed_key() {
    let mut map = Map::new();
    let key1 = String::from("a");
    let key2 = String::from("b");
    map.insert(key1.clone(), 1);
    map.insert(key2.clone(), 2);

    // Test with borrowed String keys
    if let Some([a, b]) = map.get_many_mut([key1.as_str(), key2.as_str()]) {
        *a += 10;
        *b += 20;
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&key1), Some(&11));
    assert_eq!(map.get(&key2), Some(&22));
}

#[test]
fn test_map_get_many_mut_complex_values() {
    let mut map = Map::new();
    map.insert("vec1", vec![1, 2, 3]);
    map.insert("vec2", vec![4, 5, 6]);

    // Mutate complex values
    if let Some([v1, v2]) = map.get_many_mut(["vec1", "vec2"]) {
        v1.push(99);
        v2.push(88);
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&"vec1"), Some(&vec![1, 2, 3, 99]));
    assert_eq!(map.get(&"vec2"), Some(&vec![4, 5, 6, 88]));
}

#[test]
fn test_map_get_many_mut_order_independence() {
    let mut map = Map::new();
    map.insert("a", 1);
    map.insert("b", 2);

    // Order shouldn't matter
    if let Some([b, a]) = map.get_many_mut(["b", "a"]) {
        *a = 10;
        *b = 20;
    } else {
        panic!("Expected Some but got None");
    }

    assert_eq!(map.get(&"a"), Some(&10));
    assert_eq!(map.get(&"b"), Some(&20));
}

// ============================================================================
// SET TYPE TESTS
// ============================================================================

#[test]
fn test_set_new() {
    let set: Set<i32> = Set::new();
    assert!(set.is_empty());
    assert_eq!(set.len(), 0);
}

#[test]
fn test_set_insert() {
    let mut set = Set::new();
    assert!(set.insert(1));
    assert!(set.insert(2));
    assert!(!set.insert(1)); // Already exists
    assert_eq!(set.len(), 2);
}

#[test]
fn test_set_contains() {
    let mut set = Set::new();
    set.insert(1);
    set.insert(2);

    assert!(set.contains(&1));
    assert!(set.contains(&2));
    assert!(!set.contains(&3));
}

#[test]
fn test_set_remove() {
    let mut set = Set::new();
    set.insert(1);

    assert!(set.remove(&1));
    assert!(!set.remove(&1)); // Already removed
    assert!(set.is_empty());
}

#[test]
fn test_set_clear() {
    let mut set = Set::new();
    set.insert(1);
    set.insert(2);

    set.clear();
    assert!(set.is_empty());
}

#[test]
fn test_set_union() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    let mut set2 = Set::new();
    set2.insert(2);
    set2.insert(3);

    let union: Vec<_> = set1.union(&set2).copied().collect();
    assert_eq!(union.len(), 3);
}

#[test]
fn test_set_intersection() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    let mut set2 = Set::new();
    set2.insert(2);
    set2.insert(3);

    let intersection: Vec<_> = set1.intersection(&set2).copied().collect();
    assert_eq!(intersection.len(), 1);
    assert_eq!(intersection[0], 2);
}

#[test]
fn test_set_difference() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    let mut set2 = Set::new();
    set2.insert(2);
    set2.insert(3);

    let diff: Vec<_> = set1.difference(&set2).copied().collect();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0], 1);
}

#[test]
fn test_set_is_subset() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    let mut set2 = Set::new();
    set2.insert(1);
    set2.insert(2);
    set2.insert(3);

    assert!(set1.is_subset(&set2));
    assert!(!set2.is_subset(&set1));
}

#[test]
fn test_set_is_superset() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);
    set1.insert(3);

    let mut set2 = Set::new();
    set2.insert(1);
    set2.insert(2);

    assert!(set1.is_superset(&set2));
    assert!(!set2.is_superset(&set1));
}

#[test]
fn test_set_is_disjoint() {
    let mut set1 = Set::new();
    set1.insert(1);
    set1.insert(2);

    let mut set2 = Set::new();
    set2.insert(3);
    set2.insert(4);

    assert!(set1.is_disjoint(&set2));

    set2.insert(2);
    assert!(!set1.is_disjoint(&set2));
}

// ============================================================================
// ORDERED MAP TYPE TESTS
// ============================================================================

#[test]
fn test_ordered_map_new() {
    let map: OrderedMap<i32, Text> = OrderedMap::new();
    assert!(map.is_empty());
}

#[test]
fn test_ordered_map_ordering() {
    let mut map = OrderedMap::new();
    map.insert(3, Text::from("three"));
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    let keys: Vec<_> = map.keys().copied().collect();
    assert_eq!(keys, vec![1, 2, 3]);
}

#[test]
fn test_ordered_map_first_last() {
    let mut map = OrderedMap::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));
    map.insert(3, Text::from("three"));

    assert_eq!(map.first_key_value().unwrap().0, &1);
    assert_eq!(map.last_key_value().unwrap().0, &3);
}

#[test]
fn test_ordered_map_pop_first() {
    let mut map = OrderedMap::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    let (k, v) = map.pop_first().unwrap();
    assert_eq!(k, 1);
    assert_eq!(v.as_str(), "one");
    assert_eq!(map.len(), 1);
}

#[test]
fn test_ordered_map_pop_last() {
    let mut map = OrderedMap::new();
    map.insert(1, Text::from("one"));
    map.insert(2, Text::from("two"));

    let (k, v) = map.pop_last().unwrap();
    assert_eq!(k, 2);
    assert_eq!(v.as_str(), "two");
    assert_eq!(map.len(), 1);
}

// ============================================================================
// ORDERED SET TYPE TESTS
// ============================================================================

#[test]
fn test_ordered_set_new() {
    let set: OrderedSet<i32> = OrderedSet::new();
    assert!(set.is_empty());
}

#[test]
fn test_ordered_set_ordering() {
    let mut set = OrderedSet::new();
    set.insert(3);
    set.insert(1);
    set.insert(2);

    let values: Vec<_> = set.iter().copied().collect();
    assert_eq!(values, vec![1, 2, 3]);
}

#[test]
fn test_ordered_set_first_last() {
    let mut set = OrderedSet::new();
    set.insert(1);
    set.insert(2);
    set.insert(3);

    assert_eq!(set.first(), Some(&1));
    assert_eq!(set.last(), Some(&3));
}

#[test]
fn test_ordered_set_pop_first() {
    let mut set = OrderedSet::new();
    set.insert(1);
    set.insert(2);

    assert_eq!(set.pop_first(), Some(1));
    assert_eq!(set.len(), 1);
}

#[test]
fn test_ordered_set_pop_last() {
    let mut set = OrderedSet::new();
    set.insert(1);
    set.insert(2);

    assert_eq!(set.pop_last(), Some(2));
    assert_eq!(set.len(), 1);
}

// ============================================================================
// INTEGRATION TESTS
// ============================================================================

#[test]
fn test_text_list_integration() {
    let text = Text::from("hello,world,rust");
    let parts = text.split(",");
    assert_eq!(parts.len(), 3);

    let joined = parts.join("|");
    assert_eq!(joined.as_str(), "hello|world|rust");
}

#[test]
fn test_list_map_integration() {
    let list = List::from(vec![1, 2, 3]);
    let mut map = Map::new();

    for (i, &val) in list.iter().enumerate() {
        map.insert(i, val * 10);
    }

    assert_eq!(map.len(), 3);
    assert_eq!(map.get(&0), Some(&10));
    assert_eq!(map.get(&1), Some(&20));
    assert_eq!(map.get(&2), Some(&30));
}

#[test]
fn test_complex_nested_types() {
    let mut map: Map<Text, List<i32>> = Map::new();
    map.insert(Text::from("numbers"), List::from(vec![1, 2, 3]));
    map.insert(Text::from("more"), List::from(vec![4, 5, 6]));

    let numbers = map.get(&Text::from("numbers")).unwrap();
    assert_eq!(numbers.len(), 3);
    assert_eq!(numbers[0], 1);
}

#[test]
fn test_unicode_handling() {
    let text = Text::from("你好世界");
    assert_eq!(text.chars().count(), 4);

    let mut truncated = text.clone();
    truncated.truncate(2);
    assert_eq!(truncated.as_str(), "你好");
}

#[test]
fn test_empty_collections() {
    let text = Text::new();
    let list: List<i32> = List::new();
    let map: Map<i32, i32> = Map::new();
    let set: Set<i32> = Set::new();

    assert!(text.is_empty());
    assert!(list.is_empty());
    assert!(map.is_empty());
    assert!(set.is_empty());
}
