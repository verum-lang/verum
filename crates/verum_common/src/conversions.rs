//! Type Conversions - Central conversion utilities for Rust std ↔ Verum types
//!
//! This module provides THE central place for all conversions between Rust std types
//! and Verum semantic types. This eliminates duplication across the codebase.
//!
//! # Architecture
//!
//! - Generic conversion functions for common operations
//! - Trait-based conversions for ergonomic usage
//! - From/Into implementations for seamless interop
//!
//! # Examples
//!
//! ```
//! use verum_common::conversions::*;
//! use verum_common::Maybe;
//!
//! // Option to Maybe
//! let opt: Option<i32> = Some(42);
//! let maybe: Maybe<i32> = option_to_maybe(opt);
//! assert_eq!(maybe, Maybe::Some(42));
//!
//! // Maybe to Option
//! let maybe: Maybe<i32> = Maybe::Some(42);
//! let opt: Option<i32> = maybe_to_option(maybe);
//! assert_eq!(opt, Some(42));
//!
//! // Using traits
//! let opt: Option<i32> = Some(42);
//! let maybe: Maybe<i32> = opt.to_maybe();
//! assert_eq!(maybe, Maybe::Some(42));
//! ```
//!
//! Central conversion utilities between Rust std types and Verum semantic types,
//! ensuring seamless interop while maintaining the semantic naming convention.

use crate::{List, Map, Maybe, OrderedMap, OrderedSet, Set, Text};
#[allow(clippy::disallowed_types)]
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

// ==================== Maybe/Option Conversions ====================

/// Converts std::option::Option to Maybe
///
/// This is a central helper function used throughout verum crates for seamless
/// conversion between Rust's Option and Verum's semantic Maybe type.
///
/// # Examples
///
/// ```
/// use verum_common::conversions::option_to_maybe;
/// use verum_common::Maybe;
///
/// let opt: Option<i32> = Some(42);
/// let maybe = option_to_maybe(opt);
/// assert_eq!(maybe, Maybe::Some(42));
///
/// let opt: Option<i32> = None;
/// let maybe = option_to_maybe(opt);
/// assert_eq!(maybe, Maybe::None);
/// ```
#[inline]
#[allow(clippy::manual_map)]
pub fn option_to_maybe<T>(opt: Option<T>) -> Maybe<T> {
    match opt {
        Some(val) => Maybe::Some(val),
        None => Maybe::None,
    }
}

/// Converts Maybe to std::option::Option
///
/// Used when calling APIs that expect Option<T> (like Z3, external crates, etc).
///
/// # Examples
///
/// ```
/// use verum_common::conversions::maybe_to_option;
/// use verum_common::Maybe;
///
/// let maybe: Maybe<i32> = Maybe::Some(42);
/// let opt = maybe_to_option(maybe);
/// assert_eq!(opt, Some(42));
///
/// let maybe: Maybe<i32> = Maybe::None;
/// let opt = maybe_to_option(maybe);
/// assert_eq!(opt, None);
/// ```
#[inline]
#[allow(clippy::manual_map)]
pub fn maybe_to_option<T>(maybe: Maybe<T>) -> Option<T> {
    match maybe {
        Maybe::Some(val) => Some(val),
        Maybe::None => None,
    }
}

// ==================== Vec/List Conversions ====================

/// Converts Vec to List
///
/// # Examples
///
/// ```
/// use verum_common::conversions::vec_to_list;
/// use verum_common::List;
///
/// let vec = vec![1, 2, 3];
/// let list = vec_to_list(vec);
/// assert_eq!(list, List::from(vec![1, 2, 3]));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn vec_to_list<T>(vec: Vec<T>) -> List<T> {
    List::from(vec)
}

/// Converts List to Vec
///
/// # Examples
///
/// ```
/// use verum_common::conversions::list_to_vec;
/// use verum_common::List;
///
/// let list: List<i32> = List::from(vec![1, 2, 3]);
/// let vec = list_to_vec(list);
/// assert_eq!(vec, vec![1, 2, 3]);
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn list_to_vec<T>(list: List<T>) -> Vec<T> {
    Vec::from(list)
}

// ==================== HashMap/Map Conversions ====================

/// Converts HashMap to Map
///
/// # Examples
///
/// ```
/// use verum_common::conversions::hashmap_to_map;
/// #[allow(clippy::disallowed_types)]
/// use std::collections::HashMap;
///
/// let mut hm = HashMap::new();
/// hm.insert("key", 42);
/// let map = hashmap_to_map(hm);
/// assert_eq!(map.get(&"key"), Some(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn hashmap_to_map<K: Eq + std::hash::Hash, V>(hm: HashMap<K, V>) -> Map<K, V> {
    Map::from(hm)
}

/// Converts Map to HashMap
///
/// # Examples
///
/// ```
/// use verum_common::conversions::map_to_hashmap;
/// use verum_common::Map;
///
/// let mut map = Map::new();
/// map.insert("key", 42);
/// let hm = map_to_hashmap(map);
/// assert_eq!(hm.get(&"key"), Some(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn map_to_hashmap<K: Eq + std::hash::Hash, V>(map: Map<K, V>) -> HashMap<K, V> {
    HashMap::from(map)
}

// ==================== HashSet/Set Conversions ====================

/// Converts HashSet to Set
///
/// # Examples
///
/// ```
/// use verum_common::conversions::hashset_to_set;
/// #[allow(clippy::disallowed_types)]
/// use std::collections::HashSet;
///
/// let mut hs = HashSet::new();
/// hs.insert(42);
/// let set = hashset_to_set(hs);
/// assert!(set.contains(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn hashset_to_set<T>(hs: HashSet<T>) -> Set<T> {
    Set::from(hs)
}

/// Converts Set to HashSet
///
/// # Examples
///
/// ```
/// use verum_common::conversions::set_to_hashset;
/// use verum_common::Set;
///
/// let mut set = Set::new();
/// set.insert(42);
/// let hs = set_to_hashset(set);
/// assert!(hs.contains(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn set_to_hashset<T>(set: Set<T>) -> HashSet<T> {
    HashSet::from(set)
}

// ==================== BTreeMap/OrderedMap Conversions ====================

/// Converts BTreeMap to OrderedMap
///
/// # Examples
///
/// ```
/// use verum_common::conversions::btreemap_to_ordered_map;
/// #[allow(clippy::disallowed_types)]
/// use std::collections::BTreeMap;
///
/// let mut btm = BTreeMap::new();
/// btm.insert("key", 42);
/// let map = btreemap_to_ordered_map(btm);
/// assert_eq!(map.get(&"key"), Some(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn btreemap_to_ordered_map<K, V>(btm: BTreeMap<K, V>) -> OrderedMap<K, V> {
    OrderedMap::from(btm)
}

/// Converts OrderedMap to BTreeMap
///
/// # Examples
///
/// ```
/// use verum_common::conversions::ordered_map_to_btreemap;
/// use verum_common::OrderedMap;
///
/// let mut map = OrderedMap::new();
/// map.insert("key", 42);
/// let btm = ordered_map_to_btreemap(map);
/// assert_eq!(btm.get(&"key"), Some(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn ordered_map_to_btreemap<K, V>(map: OrderedMap<K, V>) -> BTreeMap<K, V> {
    BTreeMap::from(map)
}

// ==================== BTreeSet/OrderedSet Conversions ====================

/// Converts BTreeSet to OrderedSet
///
/// # Examples
///
/// ```
/// use verum_common::conversions::btreeset_to_ordered_set;
/// #[allow(clippy::disallowed_types)]
/// use std::collections::BTreeSet;
///
/// let mut bts = BTreeSet::new();
/// bts.insert(42);
/// let set = btreeset_to_ordered_set(bts);
/// assert!(set.contains(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn btreeset_to_ordered_set<T>(bts: BTreeSet<T>) -> OrderedSet<T> {
    OrderedSet::from(bts)
}

/// Converts OrderedSet to BTreeSet
///
/// # Examples
///
/// ```
/// use verum_common::conversions::ordered_set_to_btreeset;
/// use verum_common::OrderedSet;
///
/// let mut set = OrderedSet::new();
/// set.insert(42);
/// let bts = ordered_set_to_btreeset(set);
/// assert!(bts.contains(&42));
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn ordered_set_to_btreeset<T>(set: OrderedSet<T>) -> BTreeSet<T> {
    BTreeSet::from(set)
}

// ==================== String/Text Conversions ====================

/// Converts String to Text
///
/// # Examples
///
/// ```
/// use verum_common::conversions::string_to_text;
///
/// let s = String::from("hello");
/// let text = string_to_text(s);
/// assert_eq!(text, "hello");
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn string_to_text(s: String) -> Text {
    Text::from(s)
}

/// Converts Text to String
///
/// # Examples
///
/// ```
/// use verum_common::conversions::text_to_string;
/// use verum_common::Text;
///
/// let text: Text = Text::from("hello");
/// let s = text_to_string(text);
/// assert_eq!(s, "hello");
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn text_to_string(text: Text) -> String {
    text.into_string()
}

/// Converts &str to Text
///
/// # Examples
///
/// ```
/// use verum_common::conversions::str_to_text;
///
/// let text = str_to_text("hello");
/// assert_eq!(text, "hello");
/// ```
#[inline]
pub fn str_to_text(s: &str) -> Text {
    Text::from(s)
}

// ==================== Conversion Traits ====================

/// Trait for types that can be converted to Maybe<T>
pub trait ToMaybe<T> {
    /// Convert to Maybe<T>
    fn to_maybe(self) -> Maybe<T>;
}

/// Trait for types that can be converted to Option<T>
pub trait ToOption<T> {
    /// Convert to Option<T>
    fn to_option(self) -> Option<T>;
}

// Implement ToMaybe for Option
impl<T> ToMaybe<T> for Option<T> {
    #[inline]
    fn to_maybe(self) -> Maybe<T> {
        option_to_maybe(self)
    }
}

// Implement ToOption for Maybe
impl<T> ToOption<T> for Maybe<T> {
    #[inline]
    fn to_option(self) -> Option<T> {
        maybe_to_option(self)
    }
}

// ==================== From/Into Implementations ====================

// Note: We cannot implement From<Option<T>> for Maybe<T> due to orphan rules.
// Instead, users should use:
// - option_to_maybe() function
// - .to_maybe() trait method
// - Maybe::from_option() associated function (if we add it in the future)

// ==================== Result Conversions ====================

/// Result type alias for convenience
pub type VerumResult<T, E> = crate::Result<T, E>;

/// Converts std::result::Result to Verum Result
///
/// Since both are the same type currently, this is a semantic no-op.
///
/// # Examples
///
/// ```
/// use verum_common::conversions::result_to_verum;
///
/// let res: Result<i32, &str> = Ok(42);
/// let verum_res = result_to_verum(res);
/// assert_eq!(verum_res, Ok(42));
/// ```
#[inline]
pub fn result_to_verum<T, E>(res: Result<T, E>) -> VerumResult<T, E> {
    res
}

/// Converts Verum Result to std::result::Result
///
/// Since both are the same type currently, this is a semantic no-op.
///
/// # Examples
///
/// ```
/// use verum_common::conversions::verum_to_result;
/// use verum_common::Result as VerumResult;
///
/// let verum_res: VerumResult<i32, &str> = Ok(42);
/// let res = verum_to_result(verum_res);
/// assert_eq!(res, Ok(42));
/// ```
#[inline]
pub fn verum_to_result<T, E>(res: VerumResult<T, E>) -> Result<T, E> {
    res
}

// ==================== Box/Heap Conversions ====================

/// Converts Box to Heap
///
/// Since Heap is currently a type alias for Box, this is a no-op,
/// but provides semantic clarity and future-proofs code.
///
/// # Examples
///
/// ```
/// use verum_common::conversions::box_to_heap;
///
/// let b = Box::new(42);
/// let heap = box_to_heap(b);
/// assert_eq!(*heap, 42);
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn box_to_heap<T>(b: Box<T>) -> crate::Heap<T> {
    b
}

/// Converts Heap to Box
///
/// Since Heap is currently a type alias for Box, this is a no-op,
/// but provides semantic clarity and future-proofs code.
///
/// # Examples
///
/// ```
/// use verum_common::conversions::heap_to_box;
/// use verum_common::Heap;
///
/// let heap: Heap<i32> = Box::new(42);
/// let b = heap_to_box(heap);
/// assert_eq!(*b, 42);
/// ```
#[inline]
#[allow(clippy::disallowed_types)]
pub fn heap_to_box<T>(heap: crate::Heap<T>) -> Box<T> {
    heap
}

// ==================== Batch Conversions ====================

/// Convert iterator of Options to iterator of Maybes
///
/// # Examples
///
/// ```
/// use verum_common::conversions::options_to_maybes;
/// use verum_common::Maybe;
///
/// let opts = vec![Some(1), None, Some(3)];
/// let maybes: Vec<Maybe<i32>> = options_to_maybes(opts.into_iter()).collect();
/// assert_eq!(maybes, vec![Maybe::Some(1), Maybe::None, Maybe::Some(3)]);
/// ```
pub fn options_to_maybes<T, I>(iter: I) -> impl Iterator<Item = Maybe<T>>
where
    I: Iterator<Item = Option<T>>,
{
    iter.map(option_to_maybe)
}

/// Convert iterator of Maybes to iterator of Options
///
/// # Examples
///
/// ```
/// use verum_common::conversions::maybes_to_options;
/// use verum_common::Maybe;
///
/// let maybes = vec![Maybe::Some(1), Maybe::None, Maybe::Some(3)];
/// let opts: Vec<Option<i32>> = maybes_to_options(maybes.into_iter()).collect();
/// assert_eq!(opts, vec![Some(1), None, Some(3)]);
/// ```
pub fn maybes_to_options<T, I>(iter: I) -> impl Iterator<Item = Option<T>>
where
    I: Iterator<Item = Maybe<T>>,
{
    iter.map(maybe_to_option)
}
