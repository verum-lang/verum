//! Complete semantic types implementation with full API
//!
//! This module provides newtype wrappers around Rust standard types with
//! comprehensive APIs to support all stdlib needs.
//!
//! Verum's semantic honesty principle: types describe meaning (List, Text, Map),
//! not implementation (Vec, String, HashMap). These wrappers provide rich APIs
//! while maintaining the semantic naming convention.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::ops::{Deref, DerefMut, Index, IndexMut, RangeBounds};
use std::string::FromUtf8Error;

// ============================================================================
// TEXT TYPE - Complete String API
// ============================================================================

/// Semantic text type - wraps String with full API
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Text {
    inner: String,
}

impl Text {
    // CONSTRUCTION

    /// Create a new empty Text
    pub fn new() -> Self {
        Self {
            inner: String::new(),
        }
    }

    /// Create with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: String::with_capacity(capacity),
        }
    }

    /// Create from UTF-8 bytes
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, FromUtf8Error> {
        String::from_utf8(bytes).map(|s| Self { inner: s })
    }

    /// Create from UTF-8 byte slice (convenience)
    pub fn from_utf8_slice(bytes: &[u8]) -> Result<Self, FromUtf8Error> {
        Self::from_utf8(bytes.to_vec())
    }

    /// Create from lossy UTF-8
    pub fn from_utf8_lossy(bytes: &[u8]) -> Self {
        Self {
            inner: String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    // MUTATION

    /// Remove and return the last character
    pub fn pop(&mut self) -> Option<char> {
        self.inner.pop()
    }

    /// Push a character to the end
    pub fn push(&mut self, ch: char) {
        self.inner.push(ch);
    }

    /// Push a string slice to the end
    pub fn push_str(&mut self, s: &str) {
        self.inner.push_str(s);
    }

    /// Remove first n characters
    pub fn remove_prefix(&mut self, n: usize) {
        if n >= self.chars().count() {
            self.inner.clear();
        } else {
            let prefix_end = self
                .char_indices()
                .nth(n)
                .map(|(i, _)| i)
                .unwrap_or(self.len());
            self.inner.drain(..prefix_end);
        }
    }

    /// Remove last n characters
    pub fn remove_suffix(&mut self, n: usize) {
        if n >= self.chars().count() {
            self.inner.clear();
        } else {
            let char_count = self.chars().count();
            let keep_until = char_count - n;
            let byte_index = self
                .char_indices()
                .nth(keep_until)
                .map(|(i, _)| i)
                .unwrap_or(self.len());
            self.inner.truncate(byte_index);
        }
    }

    /// Truncate to length (character count, not bytes)
    pub fn truncate(&mut self, max_chars: usize) {
        if self.chars().count() <= max_chars {
            return;
        }

        let byte_index = self
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(self.len());
        self.inner.truncate(byte_index);
    }

    /// Clear all content
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Insert character at byte position
    pub fn insert(&mut self, idx: usize, ch: char) {
        self.inner.insert(idx, ch);
    }

    /// Insert string at byte position
    pub fn insert_str(&mut self, idx: usize, string: &str) {
        self.inner.insert_str(idx, string);
    }

    /// Remove character at byte position
    pub fn remove(&mut self, idx: usize) -> char {
        self.inner.remove(idx)
    }

    /// Retain characters matching predicate
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(char) -> bool,
    {
        self.inner.retain(f);
    }

    // CONVERSION

    /// Convert into bytes
    pub fn into_bytes(self) -> Vec<u8> {
        self.inner.into_bytes()
    }

    /// Get bytes
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Convert into String
    pub fn into_string(self) -> String {
        self.inner
    }

    /// Get as str
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Get mutable str
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The resulting mutable reference is not used to write invalid UTF-8
    /// - No other references to the string content exist during mutation
    pub unsafe fn as_mut_str(&mut self) -> &mut str {
        self.inner.as_mut_str()
    }

    // QUERY METHODS

    /// Get length in bytes
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get capacity
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserve additional capacity
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Reserve exact capacity
    pub fn reserve_exact(&mut self, additional: usize) {
        self.inner.reserve_exact(additional);
    }

    /// Shrink capacity to fit
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Check if starts with pattern
    pub fn starts_with(&self, pat: &str) -> bool {
        self.inner.starts_with(pat)
    }

    /// Check if ends with pattern
    pub fn ends_with(&self, pat: &str) -> bool {
        self.inner.ends_with(pat)
    }

    /// Find substring position
    pub fn find(&self, pat: &str) -> Option<usize> {
        self.inner.find(pat)
    }

    /// Find substring position from end
    pub fn rfind(&self, pat: &str) -> Option<usize> {
        self.inner.rfind(pat)
    }

    /// Check if contains substring
    pub fn contains(&self, pat: &str) -> bool {
        self.inner.contains(pat)
    }

    /// Get substring by byte range
    pub fn substring(&self, start: usize, end: usize) -> Text {
        Text {
            inner: self.inner[start..end].to_string(),
        }
    }

    // ITERATION

    /// Iterator over characters
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.inner.chars()
    }

    /// Iterator over character indices
    pub fn char_indices(&self) -> impl Iterator<Item = (usize, char)> + '_ {
        self.inner.char_indices()
    }

    /// Iterator over bytes
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.inner.bytes()
    }

    // SPLITTING

    /// Split by pattern
    pub fn split(&self, pat: &str) -> List<Text> {
        self.inner.split(pat).map(Text::from).collect()
    }

    /// Split by pattern (with limit)
    pub fn splitn(&self, n: usize, pat: &str) -> List<Text> {
        self.inner.splitn(n, pat).map(Text::from).collect()
    }

    /// Split by whitespace
    pub fn split_whitespace(&self) -> List<Text> {
        self.inner.split_whitespace().map(Text::from).collect()
    }

    /// Lines iterator
    pub fn lines(&self) -> List<Text> {
        self.inner.lines().map(Text::from).collect()
    }

    /// Split inclusive (keeps delimiter)
    pub fn split_inclusive(&self, pat: &str) -> List<Text> {
        self.inner.split_inclusive(pat).map(Text::from).collect()
    }

    // TRANSFORMATION

    /// Replace all occurrences
    pub fn replace(&self, from: &str, to: &str) -> Text {
        Text {
            inner: self.inner.replace(from, to),
        }
    }

    /// Replace n occurrences
    pub fn replacen(&self, from: &str, to: &str, count: usize) -> Text {
        Text {
            inner: self.inner.replacen(from, to, count),
        }
    }

    /// To lowercase
    pub fn to_lowercase(&self) -> Text {
        Text {
            inner: self.inner.to_lowercase(),
        }
    }

    /// To uppercase
    pub fn to_uppercase(&self) -> Text {
        Text {
            inner: self.inner.to_uppercase(),
        }
    }

    /// Trim whitespace
    pub fn trim(&self) -> Text {
        Text {
            inner: self.inner.trim().to_string(),
        }
    }

    /// Trim start
    pub fn trim_start(&self) -> Text {
        Text {
            inner: self.inner.trim_start().to_string(),
        }
    }

    /// Trim end
    pub fn trim_end(&self) -> Text {
        Text {
            inner: self.inner.trim_end().to_string(),
        }
    }

    /// Trim matches
    pub fn trim_matches<P>(&self, pat: P) -> Text
    where
        P: Fn(char) -> bool,
    {
        Text {
            inner: self.inner.trim_matches(pat).to_string(),
        }
    }

    /// Repeat n times
    pub fn repeat(&self, n: usize) -> Text {
        Text {
            inner: self.inner.repeat(n),
        }
    }

    /// Pad left with character
    pub fn pad_left(&self, width: usize, fill: char) -> Text {
        let char_count = self.chars().count();
        if char_count >= width {
            self.clone()
        } else {
            let padding = fill.to_string().repeat(width - char_count);
            Text {
                inner: format!("{}{}", padding, self.inner),
            }
        }
    }

    /// Pad right with character
    pub fn pad_right(&self, width: usize, fill: char) -> Text {
        let char_count = self.chars().count();
        if char_count >= width {
            self.clone()
        } else {
            let padding = fill.to_string().repeat(width - char_count);
            Text {
                inner: format!("{}{}", self.inner, padding),
            }
        }
    }

    // ADDITIONAL MISSING METHODS (31 methods from audit)

    /// Create from UTF-16 encoded data
    pub fn from_utf16(v: &[u16]) -> Result<Text, std::string::FromUtf16Error> {
        String::from_utf16(v).map(|s| Text { inner: s })
    }

    /// Create from UTF-16 with lossy conversion
    pub fn from_utf16_lossy(v: &[u16]) -> Text {
        Text {
            inner: String::from_utf16_lossy(v),
        }
    }

    /// Create from UTF-8 without validation (unsafe)
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The bytes are valid UTF-8
    pub unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Text {
        Text {
            // SAFETY: Caller guarantees bytes are valid UTF-8
            inner: unsafe { String::from_utf8_unchecked(bytes) },
        }
    }

    /// Convert into boxed str
    pub fn into_boxed_str(self) -> Box<str> {
        self.inner.into_boxed_str()
    }

    /// Check if byte index is char boundary
    pub fn is_char_boundary(&self, index: usize) -> bool {
        self.inner.is_char_boundary(index)
    }

    /// Leak text, returning static str
    pub fn leak<'a>(self) -> &'a mut str {
        self.inner.leak()
    }

    /// Make ASCII lowercase in place
    pub fn make_ascii_lowercase(&mut self) {
        // SAFETY: as_mut_str is safe, make_ascii_lowercase doesn't violate UTF-8
        self.inner.make_ascii_lowercase();
    }

    /// Make ASCII uppercase in place
    pub fn make_ascii_uppercase(&mut self) {
        // SAFETY: as_mut_str is safe, make_ascii_uppercase doesn't violate UTF-8
        self.inner.make_ascii_uppercase();
    }

    /// Iterator over match indices
    pub fn match_indices<'a, P>(&'a self, pat: P) -> impl Iterator<Item = (usize, &'a str)> + 'a
    where
        P: std::str::pattern::Pattern + 'a,
    {
        self.inner.match_indices(pat)
    }

    /// Iterator over matches
    pub fn matches<'a, P>(&'a self, pat: P) -> impl Iterator<Item = &'a str> + 'a
    where
        P: std::str::pattern::Pattern + 'a,
    {
        self.inner.matches(pat)
    }

    /// Parse string into type
    pub fn parse<F: std::str::FromStr>(&self) -> Result<F, F::Err> {
        self.inner.parse()
    }

    /// Replace range with string
    pub fn replace_range<R>(&mut self, range: R, replace_with: &str)
    where
        R: std::ops::RangeBounds<usize>,
    {
        self.inner.replace_range(range, replace_with);
    }

    /// Iterator over reverse match indices
    pub fn rmatch_indices<'a, P>(&'a self, pat: P) -> impl Iterator<Item = (usize, &'a str)> + 'a
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.rmatch_indices(pat)
    }

    /// Iterator over reverse matches
    pub fn rmatches<'a, P>(&'a self, pat: P) -> impl Iterator<Item = &'a str> + 'a
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.rmatches(pat)
    }

    /// Split from right
    pub fn rsplit<'a, P>(&'a self, pat: P) -> List<Text>
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.rsplit(pat).map(Text::from).collect()
    }

    /// Split on terminator from right
    pub fn rsplit_terminator<'a, P>(&'a self, pat: P) -> List<Text>
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.rsplit_terminator(pat).map(Text::from).collect()
    }

    /// Split from right with limit
    pub fn rsplitn<'a, P>(&'a self, n: usize, pat: P) -> List<Text>
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.rsplitn(n, pat).map(Text::from).collect()
    }

    /// Split on ASCII whitespace
    pub fn split_ascii_whitespace(&self) -> List<Text> {
        self.inner
            .split_ascii_whitespace()
            .map(Text::from)
            .collect()
    }

    /// Split on terminator
    pub fn split_terminator<'a, P>(&'a self, pat: P) -> List<Text>
    where
        P: std::str::pattern::Pattern + 'a,
    {
        self.inner.split_terminator(pat).map(Text::from).collect()
    }

    /// Remove prefix if present
    pub fn strip_prefix<'a, P>(&'a self, prefix: P) -> Option<Text>
    where
        P: std::str::pattern::Pattern + 'a,
    {
        self.inner.strip_prefix(prefix).map(Text::from)
    }

    /// Remove suffix if present
    pub fn strip_suffix<'a, P>(&'a self, suffix: P) -> Option<Text>
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        self.inner.strip_suffix(suffix).map(Text::from)
    }

    /// To ASCII lowercase
    pub fn to_ascii_lowercase(&self) -> Text {
        Text {
            inner: self.inner.to_ascii_lowercase(),
        }
    }

    /// To ASCII uppercase
    pub fn to_ascii_uppercase(&self) -> Text {
        Text {
            inner: self.inner.to_ascii_uppercase(),
        }
    }

    /// Trim end matches
    pub fn trim_end_matches<'a, P>(&'a self, pat: P) -> Text
    where
        P: std::str::pattern::Pattern + 'a,
        for<'b> <P as std::str::pattern::Pattern>::Searcher<'b>:
            std::str::pattern::ReverseSearcher<'b>,
    {
        Text {
            inner: self.inner.trim_end_matches(pat).to_string(),
        }
    }

    /// Trim start matches
    pub fn trim_start_matches<'a, P>(&'a self, pat: P) -> Text
    where
        P: std::str::pattern::Pattern + 'a,
    {
        Text {
            inner: self.inner.trim_start_matches(pat).to_string(),
        }
    }

    /// Shrink capacity to specified minimum
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.inner.shrink_to(min_capacity);
    }

    /// Split off at byte index
    pub fn split_off(&mut self, at: usize) -> Text {
        Text {
            inner: self.inner.split_off(at),
        }
    }

    /// Drain characters in range
    pub fn drain<R>(&mut self, range: R) -> impl Iterator<Item = char> + '_
    where
        R: std::ops::RangeBounds<usize>,
    {
        self.inner.drain(range)
    }
}

// Conversions
impl From<String> for Text {
    fn from(s: String) -> Self {
        Self { inner: s }
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self {
            inner: s.to_string(),
        }
    }
}

impl From<Text> for String {
    fn from(t: Text) -> Self {
        t.inner
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl std::ops::Add for Text {
    type Output = Text;

    fn add(self, other: Text) -> Text {
        Text {
            inner: self.inner + &other.inner,
        }
    }
}

impl std::ops::AddAssign for Text {
    fn add_assign(&mut self, other: Text) {
        self.inner.push_str(&other.inner);
    }
}

impl std::iter::FromIterator<char> for Text {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
        Text {
            inner: iter.into_iter().collect(),
        }
    }
}

impl<'a> std::iter::FromIterator<&'a char> for Text {
    fn from_iter<I: IntoIterator<Item = &'a char>>(iter: I) -> Self {
        Text {
            inner: iter.into_iter().copied().collect(),
        }
    }
}

impl<'a> std::iter::FromIterator<&'a str> for Text {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        Text {
            inner: iter.into_iter().collect(),
        }
    }
}

// Additional trait implementations for compatibility with stdlib
impl std::ops::Add<&str> for Text {
    type Output = Text;

    fn add(mut self, other: &str) -> Text {
        self.inner.push_str(other);
        self
    }
}

impl std::ops::Add<&Text> for Text {
    type Output = Text;

    fn add(mut self, other: &Text) -> Text {
        self.inner.push_str(&other.inner);
        self
    }
}

impl std::ops::Add<&str> for &Text {
    type Output = Text;

    fn add(self, other: &str) -> Text {
        let mut result = self.clone();
        result.inner.push_str(other);
        result
    }
}

impl std::ops::AddAssign<&str> for Text {
    fn add_assign(&mut self, other: &str) {
        self.inner.push_str(other);
    }
}

impl std::ops::AddAssign<&Text> for Text {
    fn add_assign(&mut self, other: &Text) {
        self.inner.push_str(&other.inner);
    }
}

impl<'a> From<std::borrow::Cow<'a, str>> for Text {
    fn from(s: std::borrow::Cow<'a, str>) -> Self {
        Text {
            inner: s.into_owned(),
        }
    }
}

impl fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.inner.push_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        self.inner.push(c);
        Ok(())
    }
}

impl AsRef<[u8]> for Text {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl AsRef<std::path::Path> for Text {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.inner)
    }
}

impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        &self.inner
    }
}

impl AsRef<std::ffi::OsStr> for Text {
    fn as_ref(&self) -> &std::ffi::OsStr {
        std::ffi::OsStr::new(&self.inner)
    }
}

// PartialEq implementations for str comparison
impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.inner == other
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.inner == *other
    }
}

impl PartialEq<Text> for str {
    fn eq(&self, other: &Text) -> bool {
        self == other.inner
    }
}

impl PartialEq<Text> for &str {
    fn eq(&self, other: &Text) -> bool {
        *self == other.inner
    }
}

// Index implementations for Text range slicing
impl std::ops::Index<std::ops::Range<usize>> for Text {
    type Output = str;

    fn index(&self, index: std::ops::Range<usize>) -> &Self::Output {
        &self.inner[index]
    }
}

impl std::ops::Index<std::ops::RangeFull> for Text {
    type Output = str;

    fn index(&self, _index: std::ops::RangeFull) -> &Self::Output {
        &self.inner
    }
}

impl std::ops::Index<std::ops::RangeFrom<usize>> for Text {
    type Output = str;

    fn index(&self, index: std::ops::RangeFrom<usize>) -> &Self::Output {
        &self.inner[index]
    }
}

impl std::ops::Index<std::ops::RangeTo<usize>> for Text {
    type Output = str;

    fn index(&self, index: std::ops::RangeTo<usize>) -> &Self::Output {
        &self.inner[index]
    }
}

impl std::ops::Index<std::ops::RangeInclusive<usize>> for Text {
    type Output = str;

    fn index(&self, index: std::ops::RangeInclusive<usize>) -> &Self::Output {
        &self.inner[index]
    }
}

impl std::ops::Index<std::ops::RangeToInclusive<usize>> for Text {
    type Output = str;

    fn index(&self, index: std::ops::RangeToInclusive<usize>) -> &Self::Output {
        &self.inner[index]
    }
}

// ============================================================================
// LIST TYPE - Complete Vec API
// ============================================================================

/// Semantic list type - wraps Vec<T> with full API
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct List<T> {
    inner: Vec<T>,
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self { inner: Vec::new() }
    }
}

impl<T> List<T> {
    // CONSTRUCTION

    /// Create a new empty List
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// Create with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// Create from raw parts
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `ptr` must be allocated via the global allocator with proper layout for T
    /// - `length` must be <= `capacity`
    /// - `capacity` must match the original allocation capacity
    /// - The first `length` elements must be properly initialized
    pub unsafe fn from_raw_parts(ptr: *mut T, length: usize, capacity: usize) -> Self {
        // SAFETY: Caller must ensure ptr, length, and capacity satisfy Vec::from_raw_parts invariants:
        // - ptr must be allocated via the global allocator with proper layout for T
        // - length must be <= capacity
        // - capacity must match the original allocation capacity
        // - The first length elements must be properly initialized
        Self {
            inner: unsafe { Vec::from_raw_parts(ptr, length, capacity) },
        }
    }

    // MUTATION

    /// Push an element
    pub fn push(&mut self, value: T) {
        self.inner.push(value);
    }

    /// Pop the last element
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop()
    }

    /// Insert at index
    pub fn insert(&mut self, index: usize, element: T) {
        self.inner.insert(index, element);
    }

    /// Remove element at index
    pub fn remove(&mut self, index: usize) -> T {
        self.inner.remove(index)
    }

    /// Swap remove (faster, doesn't preserve order)
    pub fn swap_remove(&mut self, index: usize) -> T {
        self.inner.swap_remove(index)
    }

    /// Clear all elements
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Truncate to length
    pub fn truncate(&mut self, len: usize) {
        self.inner.truncate(len);
    }

    /// Retain elements matching predicate
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.inner.retain(f);
    }

    /// Deduplicate consecutive elements
    pub fn dedup(&mut self)
    where
        T: PartialEq,
    {
        self.inner.dedup();
    }

    /// Deduplicate by key
    pub fn dedup_by_key<F, K>(&mut self, f: F)
    where
        F: FnMut(&mut T) -> K,
        K: PartialEq,
    {
        self.inner.dedup_by_key(f);
    }

    /// Deduplicate by comparison
    pub fn dedup_by<F>(&mut self, f: F)
    where
        F: FnMut(&mut T, &mut T) -> bool,
    {
        self.inner.dedup_by(f);
    }

    /// Append another list
    pub fn append(&mut self, other: &mut List<T>) {
        self.inner.append(&mut other.inner);
    }

    /// Drain elements in range
    pub fn drain<R>(&mut self, range: R) -> impl Iterator<Item = T> + '_
    where
        R: RangeBounds<usize>,
    {
        self.inner.drain(range)
    }

    /// Splice operation
    pub fn splice<'a, R, I>(&'a mut self, range: R, replace_with: I) -> impl Iterator<Item = T> + 'a
    where
        R: RangeBounds<usize>,
        I: IntoIterator<Item = T> + 'a,
    {
        self.inner.splice(range, replace_with)
    }

    // QUERY

    /// Get length
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get capacity
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserve additional capacity
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Reserve exact capacity
    pub fn reserve_exact(&mut self, additional: usize) {
        self.inner.reserve_exact(additional);
    }

    /// Shrink capacity to fit
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Get first element
    pub fn first(&self) -> Option<&T> {
        self.inner.first()
    }

    /// Get last element
    pub fn last(&self) -> Option<&T> {
        self.inner.last()
    }

    /// Get mutable first element
    pub fn first_mut(&mut self) -> Option<&mut T> {
        self.inner.first_mut()
    }

    /// Get mutable last element
    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.inner.last_mut()
    }

    /// Get element at index
    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner.get(index)
    }

    /// Get mutable element at index
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.inner.get_mut(index)
    }

    /// Check if contains element
    pub fn contains(&self, x: &T) -> bool
    where
        T: PartialEq,
    {
        self.inner.contains(x)
    }

    /// Binary search (requires sorted list)
    pub fn binary_search(&self, x: &T) -> Result<usize, usize>
    where
        T: Ord,
    {
        self.inner.binary_search(x)
    }

    /// Binary search by key
    pub fn binary_search_by_key<B, F>(&self, b: &B, f: F) -> Result<usize, usize>
    where
        F: FnMut(&T) -> B,
        B: Ord,
    {
        self.inner.binary_search_by_key(b, f)
    }

    // TRANSFORMATION

    /// Split at index
    pub fn split_at(&self, mid: usize) -> (&[T], &[T]) {
        self.inner.split_at(mid)
    }

    /// Split at index (mutable)
    pub fn split_at_mut(&mut self, mid: usize) -> (&mut [T], &mut [T]) {
        self.inner.split_at_mut(mid)
    }

    /// Split off at index
    pub fn split_off(&mut self, at: usize) -> List<T> {
        List {
            inner: self.inner.split_off(at),
        }
    }

    /// Reverse in place
    pub fn reverse(&mut self) {
        self.inner.reverse();
    }

    /// Rotate left by n
    /// If n > len, this is a no-op (consistent with empty list behavior)
    pub fn rotate_left(&mut self, n: usize) {
        let len = self.inner.len();
        if len > 0 && n <= len {
            self.inner.rotate_left(n % len);
        }
    }

    /// Rotate right by n
    /// If n > len, this is a no-op (consistent with empty list behavior)
    pub fn rotate_right(&mut self, n: usize) {
        let len = self.inner.len();
        if len > 0 && n <= len {
            self.inner.rotate_right(n % len);
        }
    }

    /// Sort the list
    pub fn sort(&mut self)
    where
        T: Ord,
    {
        self.inner.sort();
    }

    /// Sort by key
    pub fn sort_by_key<K, F>(&mut self, f: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
    {
        self.inner.sort_by_key(f);
    }

    /// Sort by comparison
    pub fn sort_by<F>(&mut self, f: F)
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        self.inner.sort_by(f);
    }

    /// Fill with value
    pub fn fill(&mut self, value: T)
    where
        T: Clone,
    {
        self.inner.fill(value);
    }

    /// Fill with function
    pub fn fill_with<F>(&mut self, f: F)
    where
        F: FnMut() -> T,
    {
        self.inner.fill_with(f);
    }

    /// Resize with value
    pub fn resize(&mut self, new_len: usize, value: T)
    where
        T: Clone,
    {
        self.inner.resize(new_len, value);
    }

    /// Resize with function
    pub fn resize_with<F>(&mut self, new_len: usize, f: F)
    where
        F: FnMut() -> T,
    {
        self.inner.resize_with(new_len, f);
    }

    /// Extend with default values
    pub fn extend_default(&mut self, n: usize)
    where
        T: Default,
    {
        self.inner.resize_with(self.len() + n, Default::default);
    }

    // ADDITIONAL MISSING METHODS (31 methods from audit)

    /// Get raw pointer
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    /// Get mutable raw pointer
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.inner.as_mut_ptr()
    }

    /// Clone from slice
    pub fn clone_from_slice(&mut self, src: &[T])
    where
        T: Clone,
    {
        self.inner.clear();
        self.inner.extend_from_slice(src);
    }

    /// Copy from slice
    pub fn copy_from_slice(&mut self, src: &[T])
    where
        T: Copy,
    {
        self.inner.copy_from_slice(src);
    }

    /// Copy within the slice
    pub fn copy_within<R>(&mut self, src: R, dest: usize)
    where
        R: RangeBounds<usize>,
        T: Copy,
    {
        self.inner.copy_within(src, dest);
    }

    /// Extend from slice
    pub fn extend_from_slice(&mut self, other: &[T])
    where
        T: Clone,
    {
        self.inner.extend_from_slice(other);
    }

    /// Extend from within self
    pub fn extend_from_within<R>(&mut self, src: R)
    where
        R: RangeBounds<usize>,
        T: Clone,
    {
        self.inner.extend_from_within(src);
    }

    /// Create from element repeated n times
    pub fn from_elem(elem: T, n: usize) -> List<T>
    where
        T: Clone,
    {
        List {
            inner: vec![elem; n],
        }
    }

    /// Create a list with a single element
    pub fn from_single(elem: T) -> List<T> {
        List { inner: vec![elem] }
    }

    /// Get unchecked (unsafe)
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `index` is within bounds (index < self.len())
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        // SAFETY: Caller must guarantee index is within bounds
        unsafe { self.inner.get_unchecked(index) }
    }

    /// Get unchecked mutable (unsafe)
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `index` is within bounds (index < self.len())
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        // SAFETY: Caller must guarantee index is within bounds
        unsafe { self.inner.get_unchecked_mut(index) }
    }

    /// Convert into boxed slice
    pub fn into_boxed_slice(self) -> Box<[T]> {
        self.inner.into_boxed_slice()
    }

    /// Into raw parts (unsafe)
    pub fn into_raw_parts(self) -> (*mut T, usize, usize) {
        self.inner.into_raw_parts()
    }

    /// Leak list, returning static slice
    pub fn leak<'a>(self) -> &'a mut [T] {
        self.inner.leak()
    }

    /// Reverse chunks exact
    pub fn rchunks_exact(&self, chunk_size: usize) -> impl Iterator<Item = &[T]> + '_ {
        self.inner.rchunks_exact(chunk_size)
    }

    /// Reverse chunks exact mutable
    pub fn rchunks_exact_mut(&mut self, chunk_size: usize) -> impl Iterator<Item = &mut [T]> + '_ {
        self.inner.rchunks_exact_mut(chunk_size)
    }

    /// Retain with mutable access
    pub fn retain_mut<F>(&mut self, f: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        self.inner.retain_mut(f);
    }

    /// Reverse split
    pub fn rsplit<'a, F>(&'a self, pred: F) -> impl Iterator<Item = &'a [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.rsplit(pred)
    }

    /// Reverse split mutable
    pub fn rsplit_mut<'a, F>(&'a mut self, pred: F) -> impl Iterator<Item = &'a mut [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.rsplit_mut(pred)
    }

    /// Reverse split n times
    pub fn rsplitn<'a, F>(&'a self, n: usize, pred: F) -> impl Iterator<Item = &'a [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.rsplitn(n, pred)
    }

    /// Reverse split n times mutable
    pub fn rsplitn_mut<'a, F>(
        &'a mut self,
        n: usize,
        pred: F,
    ) -> impl Iterator<Item = &'a mut [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.rsplitn_mut(n, pred)
    }

    /// Sort by cached key
    pub fn sort_by_cached_key<K, F>(&mut self, f: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
    {
        self.inner.sort_by_cached_key(f);
    }

    /// Unstable sort
    pub fn sort_unstable(&mut self)
    where
        T: Ord,
    {
        self.inner.sort_unstable();
    }

    /// Unstable sort by comparison
    pub fn sort_unstable_by<F>(&mut self, f: F)
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        self.inner.sort_unstable_by(f);
    }

    /// Unstable sort by key
    pub fn sort_unstable_by_key<K, F>(&mut self, f: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
    {
        self.inner.sort_unstable_by_key(f);
    }

    /// Split first element
    pub fn split_first(&self) -> Option<(&T, &[T])> {
        self.inner.split_first()
    }

    /// Split first element mutable
    pub fn split_first_mut(&mut self) -> Option<(&mut T, &mut [T])> {
        self.inner.split_first_mut()
    }

    /// Split last element
    pub fn split_last(&self) -> Option<(&T, &[T])> {
        self.inner.split_last()
    }

    /// Split last element mutable
    pub fn split_last_mut(&mut self) -> Option<(&mut T, &mut [T])> {
        self.inner.split_last_mut()
    }

    /// Split mutable
    pub fn split_mut<'a, F>(&'a mut self, pred: F) -> impl Iterator<Item = &'a mut [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.split_mut(pred)
    }

    /// Split n mutable
    pub fn splitn_mut<'a, F>(
        &'a mut self,
        n: usize,
        pred: F,
    ) -> impl Iterator<Item = &'a mut [T]> + 'a
    where
        F: FnMut(&T) -> bool + 'a,
    {
        self.inner.splitn_mut(n, pred)
    }

    /// Swap with slice
    pub fn swap_with_slice(&mut self, other: &mut [T]) {
        self.inner.swap_with_slice(other);
    }

    /// Starts with slice
    pub fn starts_with(&self, needle: &[T]) -> bool
    where
        T: PartialEq,
    {
        self.inner.starts_with(needle)
    }

    /// Ends with slice
    pub fn ends_with(&self, needle: &[T]) -> bool
    where
        T: PartialEq,
    {
        self.inner.ends_with(needle)
    }

    /// Shrink to specified capacity
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.inner.shrink_to(min_capacity);
    }

    // SLICING & WINDOWS

    /// Get as slice
    pub fn as_slice(&self) -> &[T] {
        &self.inner
    }

    /// Get as mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.inner
    }

    /// Windows of size n
    pub fn windows(&self, size: usize) -> impl Iterator<Item = &[T]> + '_ {
        self.inner.windows(size)
    }

    /// Chunks of size n
    pub fn chunks(&self, chunk_size: usize) -> impl Iterator<Item = &[T]> + '_ {
        self.inner.chunks(chunk_size)
    }

    /// Chunks (mutable)
    pub fn chunks_mut(&mut self, chunk_size: usize) -> impl Iterator<Item = &mut [T]> + '_ {
        self.inner.chunks_mut(chunk_size)
    }

    /// Exact chunks
    pub fn chunks_exact(&self, chunk_size: usize) -> impl Iterator<Item = &[T]> + '_ {
        self.inner.chunks_exact(chunk_size)
    }

    /// Exact chunks (mutable)
    pub fn chunks_exact_mut(&mut self, chunk_size: usize) -> impl Iterator<Item = &mut [T]> + '_ {
        self.inner.chunks_exact_mut(chunk_size)
    }

    /// Reverse chunks
    pub fn rchunks(&self, chunk_size: usize) -> impl Iterator<Item = &[T]> + '_ {
        self.inner.rchunks(chunk_size)
    }

    /// Reverse chunks (mutable)
    pub fn rchunks_mut(&mut self, chunk_size: usize) -> impl Iterator<Item = &mut [T]> + '_ {
        self.inner.rchunks_mut(chunk_size)
    }

    // ITERATION

    /// Iterator over elements
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.inner.iter()
    }

    /// Mutable iterator
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.inner.iter_mut()
    }

    // FUNCTIONAL OPERATIONS

    /// Map elements to new type
    pub fn map<U, F>(self, f: F) -> List<U>
    where
        F: FnMut(T) -> U,
    {
        List {
            inner: self.inner.into_iter().map(f).collect(),
        }
    }

    /// Filter elements
    pub fn filter<F>(self, f: F) -> List<T>
    where
        F: FnMut(&T) -> bool,
    {
        List {
            inner: self.inner.into_iter().filter(f).collect(),
        }
    }

    /// Filter and map
    pub fn filter_map<U, F>(self, f: F) -> List<U>
    where
        F: FnMut(T) -> Option<U>,
    {
        List {
            inner: self.inner.into_iter().filter_map(f).collect(),
        }
    }

    /// Flat map
    pub fn flat_map<U, I, F>(self, f: F) -> List<U>
    where
        F: FnMut(T) -> I,
        I: IntoIterator<Item = U>,
    {
        List {
            inner: self.inner.into_iter().flat_map(f).collect(),
        }
    }

    /// Fold elements
    pub fn fold<B, F>(self, init: B, f: F) -> B
    where
        F: FnMut(B, T) -> B,
    {
        self.inner.into_iter().fold(init, f)
    }

    /// Take first n elements
    pub fn take(self, n: usize) -> List<T> {
        List {
            inner: self.inner.into_iter().take(n).collect(),
        }
    }

    /// Skip first n elements
    pub fn skip(self, n: usize) -> List<T> {
        List {
            inner: self.inner.into_iter().skip(n).collect(),
        }
    }
}

impl<T: fmt::Display> List<T> {
    /// Join elements into Text
    pub fn join(&self, separator: &str) -> Text {
        let result = self
            .inner
            .iter()
            .map(|item| format!("{}", item))
            .collect::<Vec<_>>()
            .join(separator);
        Text::from(result)
    }
}

// Conversions
impl<T> From<Vec<T>> for List<T> {
    fn from(v: Vec<T>) -> Self {
        Self { inner: v }
    }
}

impl<T: Clone> From<&[T]> for List<T> {
    fn from(slice: &[T]) -> Self {
        Self {
            inner: slice.to_vec(),
        }
    }
}

impl<T> From<List<T>> for Vec<T> {
    fn from(l: List<T>) -> Self {
        l.inner
    }
}

impl<T> FromIterator<T> for List<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            inner: Vec::from_iter(iter),
        }
    }
}

impl<T> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut List<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}

impl<T> Index<usize> for List<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<T> IndexMut<usize> for List<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.inner[index]
    }
}

impl<T> Deref for List<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for List<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> Extend<T> for List<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl std::io::Write for List<u8> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AsRef<[u8]> for List<u8> {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

// ============================================================================
// MAP TYPE - Complete HashMap API
// ============================================================================

/// Semantic map type - wraps HashMap<K, V> with full API
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "serde", serde(bound = "K: serde::Serialize + serde::de::DeserializeOwned + Eq + Hash, V: serde::Serialize + serde::de::DeserializeOwned"))]
pub struct Map<K, V> {
    inner: HashMap<K, V>,
}

impl<K, V> Map<K, V>
where
    K: Eq + Hash,
{
    /// Create a new empty Map
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Create with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: HashMap::with_capacity(capacity),
        }
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.inner.insert(key, value)
    }

    /// Get value by key
    pub fn get(&self, key: &K) -> Option<&V> {
        self.inner.get(key)
    }

    /// Get mutable value by key
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.inner.get_mut(key)
    }

    /// Remove value by key
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.inner.remove(key)
    }

    /// Check if contains key
    pub fn contains_key(&self, key: &K) -> bool {
        self.inner.contains_key(key)
    }

    /// Get length
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Get capacity
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserve additional capacity
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Shrink capacity to fit
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Get entry API
    pub fn entry(&mut self, key: K) -> std::collections::hash_map::Entry<'_, K, V> {
        self.inner.entry(key)
    }

    /// Get or insert with function
    pub fn get_or_insert_with<F>(&mut self, key: K, f: F) -> &mut V
    where
        F: FnOnce() -> V,
    {
        self.inner.entry(key).or_insert_with(f)
    }

    /// Retain entries matching predicate
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        self.inner.retain(f);
    }

    /// Drain all entries
    pub fn drain(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        self.inner.drain()
    }

    /// Iterator over keys
    pub fn keys(&self) -> impl Iterator<Item = &K> + '_ {
        self.inner.keys()
    }

    /// Iterator over values
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.inner.values()
    }

    /// Mutable iterator over values
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> + '_ {
        self.inner.values_mut()
    }

    /// Iterator over entries
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> + '_ {
        self.inner.iter()
    }

    /// Mutable iterator over entries
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> + '_ {
        self.inner.iter_mut()
    }

    // ADDITIONAL MISSING METHODS (9 methods from audit)

    /// Get key-value pair
    pub fn get_key_value(&self, k: &K) -> Option<(&K, &V)> {
        self.inner.get_key_value(k)
    }

    /// Get multiple mutable references to different keys simultaneously.
    ///
    /// Returns `None` if any keys are duplicates or if any key is not found.
    /// This ensures safe aliasing - all returned references point to different values.
    ///
    /// # Safety Guarantees
    ///
    /// This implementation uses unsafe code internally but maintains full safety by:
    /// 1. Checking that all keys are unique (no aliasing)
    /// 2. Verifying all keys exist before creating any mutable references
    /// 3. Using raw pointers to bypass Rust's borrow checker limitations
    ///
    /// # Example
    ///
    /// ```
    /// use verum_common::semantic_types::Map;
    ///
    /// let mut map = Map::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    /// map.insert("c", 3);
    ///
    /// if let Some([a, b]) = map.get_many_mut(["a", "b"]) {
    ///     *a += 10;
    ///     *b += 20;
    /// }
    /// assert_eq!(map.get(&"a"), Some(&11));
    /// assert_eq!(map.get(&"b"), Some(&22));
    ///
    /// // Returns None if keys are duplicated
    /// assert!(map.get_many_mut(["a", "a"]).is_none());
    ///
    /// // Returns None if any key is missing
    /// assert!(map.get_many_mut(["a", "missing"]).is_none());
    /// ```
    pub fn get_many_mut<Q, const N: usize>(&mut self, keys: [&Q; N]) -> Option<[&mut V; N]>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        // Check for duplicate keys - if any duplicates exist, we cannot safely
        // return multiple mutable references to the same value
        for i in 0..N {
            for j in (i + 1)..N {
                if keys[i] == keys[j] {
                    return None;
                }
            }
        }

        // Verify all keys exist before creating any mutable references
        for key in &keys {
            if !self.inner.contains_key(*key) {
                return None;
            }
        }

        // SAFETY: We have verified that:
        // 1. All keys are unique (no aliasing possible)
        // 2. All keys exist in the map
        // 3. We have exclusive access to the map via &mut self
        //
        // We use raw pointers to get mutable references to different values,
        // which is safe because HashMap guarantees each key maps to a unique value.
        // The borrow checker cannot verify this at compile time, but we've checked
        // uniqueness at runtime.
        unsafe {
            // Create an array of mutable raw pointers
            let mut result: [std::mem::MaybeUninit<&mut V>; N] =
                std::mem::MaybeUninit::uninit().assume_init();

            for (i, key) in keys.iter().enumerate() {
                // Get a raw pointer to the value
                // SAFETY: We verified above that the key exists
                let ptr = self.inner.get_mut(*key).unwrap() as *mut V;

                // Convert the raw pointer back to a mutable reference
                // SAFETY: The pointer is valid, aligned, and points to initialized data.
                // We've verified no aliasing by checking for duplicate keys.
                result[i] = std::mem::MaybeUninit::new(&mut *ptr);
            }

            // Convert MaybeUninit array to initialized array
            // SAFETY: We've initialized all N elements above
            let result_ptr =
                &result as *const [std::mem::MaybeUninit<&mut V>; N] as *const [&mut V; N];
            Some(std::ptr::read(result_ptr))
        }
    }

    /// Get hasher
    pub fn hasher(&self) -> &std::collections::hash_map::RandomState {
        self.inner.hasher()
    }

    /// Into keys iterator
    pub fn into_keys(self) -> impl Iterator<Item = K> {
        self.inner.into_keys()
    }

    /// Into values iterator
    pub fn into_values(self) -> impl Iterator<Item = V> {
        self.inner.into_values()
    }

    /// Remove entry and return key-value pair
    pub fn remove_entry(&mut self, k: &K) -> Option<(K, V)> {
        self.inner.remove_entry(k)
    }

    /// Try insert (only if key not present)
    pub fn try_insert(&mut self, key: K, value: V) -> Result<&mut V, (&mut V, V)> {
        match self.inner.entry(key) {
            std::collections::hash_map::Entry::Vacant(e) => Ok(e.insert(value)),
            std::collections::hash_map::Entry::Occupied(e) => Err((e.into_mut(), value)),
        }
    }

    /// With hasher
    pub fn with_hasher(hash_builder: std::collections::hash_map::RandomState) -> Self {
        Self {
            inner: HashMap::with_hasher(hash_builder),
        }
    }

    /// With capacity and hasher
    pub fn with_capacity_and_hasher(
        capacity: usize,
        hash_builder: std::collections::hash_map::RandomState,
    ) -> Self {
        Self {
            inner: HashMap::with_capacity_and_hasher(capacity, hash_builder),
        }
    }

    /// Shrink to specified capacity
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.inner.shrink_to(min_capacity);
    }
}

impl<K, V> Default for Map<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> From<HashMap<K, V>> for Map<K, V> {
    fn from(map: HashMap<K, V>) -> Self {
        Self { inner: map }
    }
}

impl<K, V> From<Map<K, V>> for HashMap<K, V> {
    fn from(map: Map<K, V>) -> Self {
        map.inner
    }
}

impl<K, V> FromIterator<(K, V)> for Map<K, V>
where
    K: Eq + Hash,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            inner: HashMap::from_iter(iter),
        }
    }
}

impl<K, V> IntoIterator for Map<K, V> {
    type Item = (K, V);
    type IntoIter = std::collections::hash_map::IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<K, V> Index<&K> for Map<K, V>
where
    K: Eq + Hash,
{
    type Output = V;

    fn index(&self, key: &K) -> &Self::Output {
        &self.inner[key]
    }
}

impl<K, V> Extend<(K, V)> for Map<K, V>
where
    K: Eq + Hash,
{
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl<K, V> PartialEq for Map<K, V>
where
    K: Eq + Hash,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K, V> Eq for Map<K, V>
where
    K: Eq + Hash,
    V: Eq,
{
}

impl<'a, K, V> IntoIterator for &'a Map<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::hash_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<'a, K, V> IntoIterator for &'a mut Map<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = std::collections::hash_map::IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}

// ============================================================================
// SET TYPE - Complete HashSet API
// ============================================================================

/// Semantic set type - wraps HashSet<T> with full API
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "serde", serde(bound = "T: serde::Serialize + serde::de::DeserializeOwned + Eq + Hash"))]
pub struct Set<T> {
    inner: HashSet<T>,
}

impl<T> Set<T>
where
    T: Eq + Hash,
{
    /// Create a new empty Set
    pub fn new() -> Self {
        Self {
            inner: HashSet::new(),
        }
    }

    /// Create with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: HashSet::with_capacity(capacity),
        }
    }

    /// Insert an element
    pub fn insert(&mut self, value: T) -> bool {
        self.inner.insert(value)
    }

    /// Remove an element
    pub fn remove(&mut self, value: &T) -> bool {
        self.inner.remove(value)
    }

    /// Check if contains element
    pub fn contains(&self, value: &T) -> bool {
        self.inner.contains(value)
    }

    /// Get length
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all elements
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Get capacity
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserve additional capacity
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Shrink capacity to fit
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Retain elements matching predicate
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.inner.retain(f);
    }

    /// Drain all elements
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.inner.drain()
    }

    /// Iterator over elements
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.inner.iter()
    }

    /// Union with another set
    pub fn union<'a>(&'a self, other: &'a Set<T>) -> impl Iterator<Item = &'a T> + 'a {
        self.inner.union(&other.inner)
    }

    /// Intersection with another set
    pub fn intersection<'a>(&'a self, other: &'a Set<T>) -> impl Iterator<Item = &'a T> + 'a {
        self.inner.intersection(&other.inner)
    }

    /// Difference with another set
    pub fn difference<'a>(&'a self, other: &'a Set<T>) -> impl Iterator<Item = &'a T> + 'a {
        self.inner.difference(&other.inner)
    }

    /// Symmetric difference with another set
    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a Set<T>,
    ) -> impl Iterator<Item = &'a T> + 'a {
        self.inner.symmetric_difference(&other.inner)
    }

    /// Check if subset
    pub fn is_subset(&self, other: &Set<T>) -> bool {
        self.inner.is_subset(&other.inner)
    }

    /// Check if superset
    pub fn is_superset(&self, other: &Set<T>) -> bool {
        self.inner.is_superset(&other.inner)
    }

    /// Check if disjoint
    pub fn is_disjoint(&self, other: &Set<T>) -> bool {
        self.inner.is_disjoint(&other.inner)
    }

    // ADDITIONAL MISSING METHODS (4 methods from audit)

    /// Get or insert owned value
    pub fn get_or_insert_owned(&mut self, value: &T) -> &T
    where
        T: Clone,
    {
        if !self.inner.contains(value) {
            self.inner.insert(value.clone());
        }
        self.inner.get(value).unwrap()
    }

    /// Get hasher
    pub fn hasher(&self) -> &std::collections::hash_map::RandomState {
        self.inner.hasher()
    }

    /// With hasher
    pub fn with_hasher(hasher: std::collections::hash_map::RandomState) -> Self {
        Self {
            inner: HashSet::with_hasher(hasher),
        }
    }

    /// With capacity and hasher
    pub fn with_capacity_and_hasher(
        capacity: usize,
        hasher: std::collections::hash_map::RandomState,
    ) -> Self {
        Self {
            inner: HashSet::with_capacity_and_hasher(capacity, hasher),
        }
    }

    /// Take value from set
    pub fn take(&mut self, value: &T) -> Option<T> {
        self.inner.take(value)
    }

    /// Shrink to specified capacity
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.inner.shrink_to(min_capacity);
    }
}

impl<T> Default for Set<T>
where
    T: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> From<HashSet<T>> for Set<T> {
    fn from(set: HashSet<T>) -> Self {
        Self { inner: set }
    }
}

impl<T> From<Set<T>> for HashSet<T> {
    fn from(set: Set<T>) -> Self {
        set.inner
    }
}

impl<T> FromIterator<T> for Set<T>
where
    T: Eq + Hash,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            inner: HashSet::from_iter(iter),
        }
    }
}

impl<T> IntoIterator for Set<T> {
    type Item = T;
    type IntoIter = std::collections::hash_set::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<T> Extend<T> for Set<T>
where
    T: Eq + Hash,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl<T> PartialEq for Set<T>
where
    T: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T> Eq for Set<T> where T: Eq + Hash {}

impl<'a, T> IntoIterator for &'a Set<T> {
    type Item = &'a T;
    type IntoIter = std::collections::hash_set::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

// ============================================================================
// ORDERED MAP TYPE - Complete BTreeMap API
// ============================================================================

/// Semantic ordered map type - wraps BTreeMap<K, V> with full API
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "serde", serde(bound = "K: serde::Serialize + serde::de::DeserializeOwned + Ord, V: serde::Serialize + serde::de::DeserializeOwned"))]
pub struct OrderedMap<K, V> {
    inner: BTreeMap<K, V>,
}

impl<K, V> OrderedMap<K, V>
where
    K: Ord,
{
    /// Create a new empty OrderedMap
    pub fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.inner.insert(key, value)
    }

    /// Get value by key
    pub fn get(&self, key: &K) -> Option<&V> {
        self.inner.get(key)
    }

    /// Get mutable value by key
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.inner.get_mut(key)
    }

    /// Remove value by key
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.inner.remove(key)
    }

    /// Check if contains key
    pub fn contains_key(&self, key: &K) -> bool {
        self.inner.contains_key(key)
    }

    /// Get length
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Get entry API
    pub fn entry(&mut self, key: K) -> std::collections::btree_map::Entry<'_, K, V> {
        self.inner.entry(key)
    }

    /// Iterator over keys
    pub fn keys(&self) -> impl Iterator<Item = &K> + '_ {
        self.inner.keys()
    }

    /// Iterator over values
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.inner.values()
    }

    /// Mutable iterator over values
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> + '_ {
        self.inner.values_mut()
    }

    /// Iterator over entries
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> + '_ {
        self.inner.iter()
    }

    /// Mutable iterator over entries
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> + '_ {
        self.inner.iter_mut()
    }

    /// Get first key-value pair
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        self.inner.first_key_value()
    }

    /// Get last key-value pair
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        self.inner.last_key_value()
    }

    /// Pop first key-value pair
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        self.inner.pop_first()
    }

    /// Pop last key-value pair
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        self.inner.pop_last()
    }
}

impl<K, V> Default for OrderedMap<K, V>
where
    K: Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> From<BTreeMap<K, V>> for OrderedMap<K, V> {
    fn from(map: BTreeMap<K, V>) -> Self {
        Self { inner: map }
    }
}

impl<K, V> From<OrderedMap<K, V>> for BTreeMap<K, V> {
    fn from(map: OrderedMap<K, V>) -> Self {
        map.inner
    }
}

impl<K, V> FromIterator<(K, V)> for OrderedMap<K, V>
where
    K: Ord,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            inner: BTreeMap::from_iter(iter),
        }
    }
}

impl<K, V> PartialEq for OrderedMap<K, V>
where
    K: Ord,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K, V> Eq for OrderedMap<K, V>
where
    K: Ord,
    V: Eq,
{
}

impl<K, V> IntoIterator for OrderedMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::collections::btree_map::IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, K, V> IntoIterator for &'a OrderedMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<'a, K, V> IntoIterator for &'a mut OrderedMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = std::collections::btree_map::IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}

// ============================================================================
// ORDERED SET TYPE - Complete BTreeSet API
// ============================================================================

/// Semantic ordered set type - wraps BTreeSet<T> with full API
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "serde", serde(bound = "T: serde::Serialize + serde::de::DeserializeOwned + Ord"))]
pub struct OrderedSet<T> {
    inner: BTreeSet<T>,
}

impl<T> OrderedSet<T>
where
    T: Ord,
{
    /// Create a new empty OrderedSet
    pub fn new() -> Self {
        Self {
            inner: BTreeSet::new(),
        }
    }

    /// Insert an element
    pub fn insert(&mut self, value: T) -> bool {
        self.inner.insert(value)
    }

    /// Remove an element
    pub fn remove(&mut self, value: &T) -> bool {
        self.inner.remove(value)
    }

    /// Check if contains element
    pub fn contains(&self, value: &T) -> bool {
        self.inner.contains(value)
    }

    /// Get length
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all elements
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Iterator over elements
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.inner.iter()
    }

    /// Get first element
    pub fn first(&self) -> Option<&T> {
        self.inner.first()
    }

    /// Get last element
    pub fn last(&self) -> Option<&T> {
        self.inner.last()
    }

    /// Pop first element
    pub fn pop_first(&mut self) -> Option<T> {
        self.inner.pop_first()
    }

    /// Pop last element
    pub fn pop_last(&mut self) -> Option<T> {
        self.inner.pop_last()
    }

    /// Set union - returns elements in either set
    pub fn union<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a T> + 'a
    where
        T: 'a,
    {
        self.inner.union(&other.inner)
    }

    /// Set intersection - returns elements in both sets
    pub fn intersection<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a T> + 'a
    where
        T: 'a,
    {
        self.inner.intersection(&other.inner)
    }

    /// Set difference - returns elements in self but not in other
    pub fn difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a T> + 'a
    where
        T: 'a,
    {
        self.inner.difference(&other.inner)
    }

    /// Set symmetric difference - returns elements in either set but not both
    pub fn symmetric_difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = &'a T> + 'a
    where
        T: 'a,
    {
        self.inner.symmetric_difference(&other.inner)
    }

    /// Check if self is a subset of other
    pub fn is_subset(&self, other: &Self) -> bool {
        self.inner.is_subset(&other.inner)
    }

    /// Check if self is a superset of other
    pub fn is_superset(&self, other: &Self) -> bool {
        self.inner.is_superset(&other.inner)
    }

    /// Check if sets are disjoint (no common elements)
    pub fn is_disjoint(&self, other: &Self) -> bool {
        self.inner.is_disjoint(&other.inner)
    }
}

impl<T> Default for OrderedSet<T>
where
    T: Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> From<BTreeSet<T>> for OrderedSet<T> {
    fn from(set: BTreeSet<T>) -> Self {
        Self { inner: set }
    }
}

impl<T> From<OrderedSet<T>> for BTreeSet<T> {
    fn from(set: OrderedSet<T>) -> Self {
        set.inner
    }
}

impl<T> FromIterator<T> for OrderedSet<T>
where
    T: Ord,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            inner: BTreeSet::from_iter(iter),
        }
    }
}

impl<T> PartialEq for OrderedSet<T>
where
    T: Ord,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T> Eq for OrderedSet<T> where T: Ord {}

impl<T> IntoIterator for OrderedSet<T> {
    type Item = T;
    type IntoIter = std::collections::btree_set::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a OrderedSet<T> {
    type Item = &'a T;
    type IntoIter = std::collections::btree_set::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

// ============================================================================
// CONVENIENCE MACROS
// ============================================================================

/// Create a Text from string literals
#[macro_export]
macro_rules! text {
    ($s:expr) => {
        $crate::semantic_types::Text::from($s)
    };
}

/// Create a List from elements
#[macro_export]
macro_rules! list {
    () => {
        $crate::semantic_types::List::new()
    };
    ($($x:expr),+ $(,)?) => {
        $crate::semantic_types::List::from(vec![$($x),+])
    };
}

/// Create a Map from key-value pairs
#[macro_export]
macro_rules! map {
    () => {
        $crate::semantic_types::Map::new()
    };
    ($($k:expr => $v:expr),+ $(,)?) => {{
        let mut m = $crate::semantic_types::Map::new();
        $(m.insert($k, $v);)+
        m
    }};
}

/// Create a Set from elements
#[macro_export]
macro_rules! set {
    () => {
        $crate::semantic_types::Set::new()
    };
    ($($x:expr),+ $(,)?) => {{
        let mut s = $crate::semantic_types::Set::new();
        $(s.insert($x);)+
        s
    }};
}
