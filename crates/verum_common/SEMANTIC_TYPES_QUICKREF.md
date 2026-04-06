# Semantic Types Quick Reference

Quick reference for the most commonly used methods in `verum_core::semantic_types`.

## Import

```rust
use verum_core::semantic_types::{Text, List, Map, Set, OrderedMap, OrderedSet};
```

## Text - String Operations

```rust
// Create
let text = Text::new();
let text = Text::from("hello");
let text = Text::from(String::from("hello"));

// Modify
text.push('!');              // Add character
text.push_str(" world");     // Add string
text.pop();                  // Remove last char
text.clear();                // Empty

// Query
text.len()                   // Length in bytes
text.is_empty()              // Check if empty
text.contains("sub")         // Contains substring?
text.starts_with("pre")      // Starts with?
text.ends_with("suf")        // Ends with?
text.find("sub")             // Find position (Option<usize>)

// Transform
text.to_lowercase()          // Lowercase copy
text.to_uppercase()          // Uppercase copy
text.trim()                  // Trim whitespace copy
text.replace("old", "new")   // Replace all occurrences
text.repeat(3)               // Repeat n times

// Split
text.split(",")              // Split to List<Text>
text.split_whitespace()      // Split on whitespace
text.lines()                 // Split on newlines

// Convert
text.as_str()                // &str
text.into_string()           // String
text.as_bytes()              // &[u8]
text.into_bytes()            // Vec<u8>
```

## List - Dynamic Arrays

```rust
// Create
let list = List::new();
let list = List::from(vec![1, 2, 3]);
let list: List<i32> = (1..=10).collect();

// Modify
list.push(4);                // Add to end
list.pop();                  // Remove from end (Option<T>)
list.insert(0, 1);           // Insert at index
list.remove(0);              // Remove at index
list.clear();                // Empty
list.reverse();              // Reverse in place
list.sort();                 // Sort in place

// Query
list.len()                   // Length
list.is_empty()              // Check if empty
list.first()                 // First element (Option<&T>)
list.last()                  // Last element (Option<&T>)
list.get(i)                  // Element at index (Option<&T>)
list[i]                      // Element at index (panics if out of bounds)
list.contains(&x)            // Contains element?

// Transform
list.map(|x| x * 2)          // Map to new list
list.filter(|&x| x > 0)      // Filter to new list
list.take(5)                 // First n elements
list.skip(2)                 // Skip first n elements

// Functional
list.fold(0, |acc, x| acc + x)     // Fold/reduce
list.filter_map(|x| Some(x))       // Filter and map
list.flat_map(|x| vec![x, x])      // Flat map

// Iterate
for item in &list { }        // Borrow
for item in &mut list { }    // Mutable borrow
for item in list { }         // Consume

// Special
list.join(", ")              // Join to Text (for Display types)
list.windows(3)              // Sliding windows
list.chunks(2)               // Non-overlapping chunks
```

## Map - Hash Maps

```rust
// Create
let map = Map::new();
let map = Map::with_capacity(100);

// Modify
map.insert(key, value);      // Insert/update
map.remove(&key);            // Remove (returns Option<V>)
map.clear();                 // Empty

// Query
map.get(&key)                // Get value (Option<&V>)
map.get_mut(&key)            // Get mutable value (Option<&mut V>)
map.contains_key(&key)       // Has key?
map.len()                    // Number of entries
map.is_empty()               // Check if empty

// Entry API
map.entry(key).or_insert(default);              // Insert if missing
map.entry(key).and_modify(|v| *v += 1);         // Modify if exists
map.get_or_insert_with(key, || default);        // Get or create

// Iterate
for (k, v) in &map { }       // Borrow
for (k, v) in &mut map { }   // Mutable borrow
for (k, v) in map { }        // Consume
for k in map.keys() { }      // Just keys
for v in map.values() { }    // Just values

// Advanced
map.retain(|k, v| condition);     // Keep only matching
map.drain();                      // Remove all, return iterator
```

## Set - Hash Sets

```rust
// Create
let set = Set::new();
let set: Set<i32> = [1, 2, 3].into_iter().collect();

// Modify
set.insert(value);           // Add (returns bool)
set.remove(&value);          // Remove (returns bool)
set.clear();                 // Empty

// Query
set.contains(&value)         // Has element?
set.len()                    // Number of elements
set.is_empty()               // Check if empty

// Set Operations
set1.union(&set2)            // All elements from both
set1.intersection(&set2)     // Common elements
set1.difference(&set2)       // Elements in set1 but not set2
set1.symmetric_difference(&set2)  // Elements in one but not both

set1.is_subset(&set2)        // All set1 elements in set2?
set1.is_superset(&set2)      // All set2 elements in set1?
set1.is_disjoint(&set2)      // No common elements?

// Iterate
for item in &set { }         // Borrow
for item in set { }          // Consume
```

## OrderedMap - Sorted Maps

```rust
// Same as Map, plus:
map.first_key_value()        // First entry (Option<(&K, &V)>)
map.last_key_value()         // Last entry
map.pop_first()              // Remove and return first
map.pop_last()               // Remove and return last

// Iteration is in sorted key order
for (k, v) in &map { }       // Sorted by key
```

## OrderedSet - Sorted Sets

```rust
// Same as Set, plus:
set.first()                  // First element (Option<&T>)
set.last()                   // Last element
set.pop_first()              // Remove and return first
set.pop_last()               // Remove and return last

// Iteration is in sorted order
for item in &set { }         // Sorted
```

## Convenience Macros

```rust
use verum_core::{text, list, map, set};

let t = text!("hello");                    // Text
let l = list![1, 2, 3];                   // List
let m = map! { text!("k") => 1 };         // Map
let s = set![1, 2, 3];                    // Set
```

## Common Patterns

### CSV Parsing
```rust
let csv = Text::from("a,b,c\n1,2,3");
let lines = csv.lines();
let headers = lines[0].split(",");
for line in lines.iter().skip(1) {
    let values = line.split(",");
    // Process row...
}
```

### Word Frequency
```rust
let text = Text::from("hello world hello");
let mut freq = Map::new();
for word in text.split_whitespace() {
    *freq.entry(word).or_insert(0) += 1;
}
```

### Deduplication
```rust
let list = List::from(vec![1, 2, 2, 3, 3, 3]);
let unique: Set<_> = list.into_iter().collect();
```

### Sorting
```rust
let mut list = List::from(vec![3, 1, 2]);
list.sort();                              // Ascending
list.sort_by_key(|x| -x);                // Descending (negate)
```

### Chaining Operations
```rust
let result = list
    .filter(|&x| x > 0)
    .map(|x| x * 2)
    .take(10);
```

### Configuration Parsing
```rust
let config = Text::from("key=value\nfoo=bar");
let mut settings = Map::new();
for line in config.lines() {
    let parts = line.split("=");
    if parts.len() == 2 {
        settings.insert(parts[0], parts[1]);
    }
}
```

## Conversion

### To/From Std Types
```rust
// Text ↔ String
let text = Text::from("hello");
let string: String = text.into();

// List ↔ Vec
let list = List::from(vec![1, 2, 3]);
let vec: Vec<_> = list.into();

// Map ↔ HashMap
let map = Map::from(hashmap);
let hashmap: HashMap<_, _> = map.into();

// Set ↔ HashSet
let set = Set::from(hashset);
let hashset: HashSet<_> = set.into();
```

### Deref Coercion
```rust
fn takes_str(s: &str) { }
fn takes_slice(s: &[i32]) { }

let text = Text::from("hello");
let list = List::from(vec![1, 2, 3]);

takes_str(&text);       // Works via Deref
takes_slice(&list);     // Works via Deref
```

## Performance Tips

1. **Capacity**: Pre-allocate with `with_capacity` when size is known
2. **Borrowing**: Use `&` to avoid cloning when possible
3. **Entry API**: Use for Map to avoid double lookups
4. **Deref**: Leverage automatic deref coercion
5. **Iterators**: Chain operations to avoid intermediate allocations

## Common Mistakes

```rust
// ❌ Wrong - creates unnecessary copies
let text = Text::from("hello");
let upper = text.to_uppercase();
println!("{}", text);  // text was moved!

// ✅ Correct - borrow or clone
let text = Text::from("hello");
let upper = (&text).to_uppercase();  // or text.clone()
println!("{}", text);
```

```rust
// ❌ Wrong - modifies while iterating
for item in &list {
    list.push(item * 2);  // Error!
}

// ✅ Correct - collect to new list
let doubled = list.iter().map(|x| x * 2).collect();
```

## See Also

- Full API Reference: `SEMANTIC_TYPES_API.md`
- Usage Examples: `examples/semantic_types_usage.rs`
- Tests: `tests/semantic_types_tests.rs`
- Implementation: `src/semantic_types.rs`
