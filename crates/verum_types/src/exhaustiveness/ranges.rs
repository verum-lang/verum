//! Range Pattern Arithmetic
//!
//! This module provides utilities for analyzing range patterns and determining
//! coverage for numeric types.

use verum_common::List;

/// Represents an interval of values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub start: i128,
    pub end: i128,
}

impl Interval {
    /// Create a new interval
    pub fn new(start: i128, end: i128) -> Self {
        Self { start, end }
    }

    /// Create a singleton interval (single value)
    pub fn singleton(value: i128) -> Self {
        Self {
            start: value,
            end: value,
        }
    }

    /// Check if the interval is empty
    pub fn is_empty(&self) -> bool {
        self.start > self.end
    }

    /// Check if the interval contains a value
    pub fn contains(&self, value: i128) -> bool {
        value >= self.start && value <= self.end
    }

    /// Check if this interval overlaps with another
    pub fn overlaps(&self, other: &Interval) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// Check if this interval is adjacent to another (can be merged)
    pub fn adjacent(&self, other: &Interval) -> bool {
        self.end + 1 == other.start || other.end + 1 == self.start
    }

    /// Merge two overlapping or adjacent intervals
    pub fn merge(&self, other: &Interval) -> Option<Interval> {
        if self.overlaps(other) || self.adjacent(other) {
            Some(Interval {
                start: self.start.min(other.start),
                end: self.end.max(other.end),
            })
        } else {
            None
        }
    }

    /// Subtract another interval from this one
    /// Returns 0, 1, or 2 intervals depending on overlap
    pub fn subtract(&self, other: &Interval) -> List<Interval> {
        if !self.overlaps(other) {
            // No overlap - return self unchanged
            return List::from_iter([*self]);
        }

        let mut result = List::new();

        // Part before the subtraction
        if self.start < other.start {
            result.push(Interval::new(self.start, other.start - 1));
        }

        // Part after the subtraction
        if self.end > other.end {
            result.push(Interval::new(other.end + 1, self.end));
        }

        result
    }

    /// Intersect two intervals
    pub fn intersect(&self, other: &Interval) -> Option<Interval> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end {
            Some(Interval::new(start, end))
        } else {
            None
        }
    }

    /// Size of the interval (number of values)
    pub fn size(&self) -> u128 {
        if self.is_empty() {
            0
        } else {
            (self.end - self.start + 1) as u128
        }
    }
}

/// A set of disjoint intervals
#[derive(Debug, Clone)]
pub struct IntervalSet {
    intervals: List<Interval>,
}

impl IntervalSet {
    /// Create an empty interval set
    pub fn empty() -> Self {
        Self {
            intervals: List::new(),
        }
    }

    /// Create an interval set with a single interval
    pub fn singleton(interval: Interval) -> Self {
        Self {
            intervals: List::from_iter([interval]),
        }
    }

    /// Create a universe set (all values)
    pub fn universe() -> Self {
        Self::singleton(Interval::new(i128::MIN, i128::MAX))
    }

    /// Add an interval to the set (merging overlapping intervals)
    pub fn add(&mut self, interval: Interval) {
        if interval.is_empty() {
            return;
        }

        // Find intervals that overlap or are adjacent
        let mut to_merge = List::from_iter([interval]);
        let mut remaining = List::new();

        for existing in self.intervals.iter() {
            let mut merged = false;
            for i in 0..to_merge.len() {
                if let Some(merged_interval) = to_merge[i].merge(existing) {
                    to_merge[i] = merged_interval;
                    merged = true;
                    break;
                }
            }
            if !merged {
                remaining.push(*existing);
            }
        }

        // Merge the to_merge list
        let mut result: List<Interval> = List::new();
        for interval in to_merge.iter() {
            let mut found_merge = false;
            for i in 0..result.len() {
                if let Some(merged) = result[i].merge(interval) {
                    result[i] = merged;
                    found_merge = true;
                    break;
                }
            }
            if !found_merge {
                result.push(*interval);
            }
        }

        // Add remaining intervals
        for interval in remaining.iter() {
            result.push(*interval);
        }

        // Sort by start
        result.sort_by_key(|i| i.start);

        self.intervals = result;
    }

    /// Subtract an interval from the set
    pub fn subtract(&mut self, interval: Interval) {
        let mut new_intervals = List::new();

        for existing in self.intervals.iter() {
            for remaining in existing.subtract(&interval).iter() {
                if !remaining.is_empty() {
                    new_intervals.push(*remaining);
                }
            }
        }

        self.intervals = new_intervals;
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// Check if the set covers a specific value
    pub fn contains(&self, value: i128) -> bool {
        self.intervals.iter().any(|i| i.contains(value))
    }

    /// Get uncovered values (complement relative to a given range)
    pub fn complement(&self, bounds: Interval) -> IntervalSet {
        let mut result = IntervalSet::singleton(bounds);
        for interval in self.intervals.iter() {
            result.subtract(*interval);
        }
        result
    }

    /// Iterator over intervals
    pub fn iter(&self) -> impl Iterator<Item = &Interval> {
        self.intervals.iter()
    }

    /// Get the first uncovered value in a range
    pub fn first_uncovered(&self, bounds: Interval) -> Option<i128> {
        let complement = self.complement(bounds);
        complement
            .intervals
            .first()
            .map(|i| i.start)
    }
}

/// Check if a set of range patterns covers all values in a type
pub fn ranges_cover_type(
    ranges: &[(Option<i128>, Option<i128>, bool)],
    type_bounds: Interval,
) -> (bool, Option<i128>) {
    let mut covered = IntervalSet::empty();

    for (start, end, inclusive) in ranges {
        let s = start.unwrap_or(type_bounds.start);
        let e = if *inclusive {
            end.unwrap_or(type_bounds.end)
        } else {
            end.map(|e| e - 1).unwrap_or(type_bounds.end)
        };
        if s <= e {
            covered.add(Interval::new(s, e));
        }
    }

    let uncovered = covered.first_uncovered(type_bounds);
    (uncovered.is_none(), uncovered)
}

/// Information about overlapping range patterns
#[derive(Debug, Clone)]
pub struct RangeOverlap {
    /// Index of the first pattern
    pub first_pattern_index: usize,
    /// Index of the second pattern
    pub second_pattern_index: usize,
    /// The overlapping interval
    pub overlap: Interval,
    /// Whether the second pattern is completely covered by the first
    pub is_redundant: bool,
}

impl RangeOverlap {
    /// Create a new overlap report
    pub fn new(first: usize, second: usize, overlap: Interval, is_redundant: bool) -> Self {
        Self {
            first_pattern_index: first,
            second_pattern_index: second,
            overlap,
            is_redundant,
        }
    }
}

/// Result of range overlap analysis
#[derive(Debug, Clone)]
pub struct RangeOverlapAnalysis {
    /// All overlaps detected
    pub overlaps: List<RangeOverlap>,
    /// Patterns that are completely redundant (subsets of earlier patterns)
    pub redundant_patterns: List<usize>,
    /// Uncovered ranges (gaps)
    pub uncovered_ranges: List<Interval>,
}

impl RangeOverlapAnalysis {
    /// Check if there are any issues
    pub fn has_issues(&self) -> bool {
        !self.overlaps.is_empty() || !self.redundant_patterns.is_empty()
    }
}

/// Analyze a set of range patterns for overlaps and redundancy
pub fn analyze_range_overlaps(
    ranges: &[(usize, Interval)], // (pattern_index, interval)
) -> RangeOverlapAnalysis {
    let mut overlaps = List::new();
    let mut redundant_patterns = List::new();

    // Check each pair of ranges for overlap
    for i in 0..ranges.len() {
        let (idx_i, interval_i) = &ranges[i];

        for j in (i + 1)..ranges.len() {
            let (idx_j, interval_j) = &ranges[j];

            if let Some(overlap) = interval_i.intersect(interval_j) {
                // Check if the later pattern is completely covered by the earlier one
                let is_redundant = interval_j.start >= interval_i.start
                    && interval_j.end <= interval_i.end;

                overlaps.push(RangeOverlap::new(*idx_i, *idx_j, overlap, is_redundant));

                if is_redundant && !redundant_patterns.contains(idx_j) {
                    redundant_patterns.push(*idx_j);
                }
            }
        }
    }

    RangeOverlapAnalysis {
        overlaps,
        redundant_patterns,
        uncovered_ranges: List::new(), // Filled in by caller if needed
    }
}

/// Describe a range overlap for error messages
pub fn describe_range_overlap(overlap: &RangeOverlap) -> String {
    let interval = &overlap.overlap;

    if interval.start == interval.end {
        format!(
            "patterns {} and {} both match value {}",
            overlap.first_pattern_index + 1,
            overlap.second_pattern_index + 1,
            interval.start
        )
    } else if overlap.is_redundant {
        format!(
            "pattern {} is completely covered by pattern {} (range {}..={})",
            overlap.second_pattern_index + 1,
            overlap.first_pattern_index + 1,
            interval.start,
            interval.end
        )
    } else {
        format!(
            "patterns {} and {} overlap on range {}..={}",
            overlap.first_pattern_index + 1,
            overlap.second_pattern_index + 1,
            interval.start,
            interval.end
        )
    }
}

/// Generate detailed error message for range overlaps
pub fn format_range_overlap_error(analysis: &RangeOverlapAnalysis) -> String {
    let mut messages = Vec::new();

    // Report redundant patterns first
    for &idx in analysis.redundant_patterns.iter() {
        messages.push(format!("pattern {} is unreachable (completely covered by earlier pattern)", idx + 1));
    }

    // Report non-redundant overlaps
    for overlap in analysis.overlaps.iter() {
        if !overlap.is_redundant {
            messages.push(describe_range_overlap(overlap));
        }
    }

    if messages.is_empty() {
        String::from("no range pattern issues detected")
    } else {
        messages.join("\n")
    }
}

/// Calculate uncovered ranges given a set of patterns and type bounds
pub fn find_uncovered_ranges(
    ranges: &[(usize, Interval)],
    type_bounds: Interval,
) -> List<Interval> {
    let mut covered = IntervalSet::empty();

    for (_, interval) in ranges {
        covered.add(*interval);
    }

    let complement = covered.complement(type_bounds);
    complement.intervals.iter().cloned().collect()
}

/// Suggest patterns to cover uncovered ranges
pub fn suggest_uncovered_ranges(uncovered: &[Interval]) -> String {
    if uncovered.is_empty() {
        return String::new();
    }

    let suggestions: Vec<String> = uncovered.iter().map(|interval| {
        if interval.start == interval.end {
            format!("    {} => todo!(),", interval.start)
        } else {
            format!("    {}..={} => todo!(),", interval.start, interval.end)
        }
    }).collect();

    format!("consider adding arms for:\n{}", suggestions.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_contains() {
        let i = Interval::new(1, 10);
        assert!(i.contains(1));
        assert!(i.contains(5));
        assert!(i.contains(10));
        assert!(!i.contains(0));
        assert!(!i.contains(11));
    }

    #[test]
    fn test_interval_merge() {
        let a = Interval::new(1, 5);
        let b = Interval::new(4, 10);
        let merged = a.merge(&b).unwrap();
        assert_eq!(merged.start, 1);
        assert_eq!(merged.end, 10);
    }

    #[test]
    fn test_interval_merge_adjacent() {
        let a = Interval::new(1, 5);
        let b = Interval::new(6, 10);
        let merged = a.merge(&b).unwrap();
        assert_eq!(merged.start, 1);
        assert_eq!(merged.end, 10);
    }

    #[test]
    fn test_interval_subtract() {
        let a = Interval::new(1, 10);
        let b = Interval::new(4, 7);
        let result = a.subtract(&b);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], Interval::new(1, 3));
        assert_eq!(result[1], Interval::new(8, 10));
    }

    #[test]
    fn test_interval_set_coverage() {
        let mut set = IntervalSet::empty();
        set.add(Interval::new(1, 5));
        set.add(Interval::new(6, 10));

        assert!(set.contains(1));
        assert!(set.contains(5));
        assert!(set.contains(6));
        assert!(set.contains(10));
        assert!(!set.contains(0));
        assert!(!set.contains(11));
    }

    #[test]
    fn test_ranges_cover_bool() {
        // Bool is 0..=1
        let bounds = Interval::new(0, 1);

        // Just false (0)
        let (covers, uncovered) = ranges_cover_type(&[(Some(0), Some(0), true)], bounds);
        assert!(!covers);
        assert_eq!(uncovered, Some(1));

        // Both values
        let (covers, _) = ranges_cover_type(
            &[(Some(0), Some(0), true), (Some(1), Some(1), true)],
            bounds,
        );
        assert!(covers);
    }

    #[test]
    fn test_range_overlap_detection() {
        // Pattern 0: 1..=5, Pattern 1: 3..=7
        let ranges = vec![
            (0, Interval::new(1, 5)),
            (1, Interval::new(3, 7)),
        ];
        let analysis = analyze_range_overlaps(&ranges);

        assert_eq!(analysis.overlaps.len(), 1);
        let overlap = &analysis.overlaps[0];
        assert_eq!(overlap.first_pattern_index, 0);
        assert_eq!(overlap.second_pattern_index, 1);
        assert_eq!(overlap.overlap, Interval::new(3, 5));
        assert!(!overlap.is_redundant);
    }

    #[test]
    fn test_range_redundancy_detection() {
        // Pattern 0: 1..=10, Pattern 1: 3..=7 (subset, redundant)
        let ranges = vec![
            (0, Interval::new(1, 10)),
            (1, Interval::new(3, 7)),
        ];
        let analysis = analyze_range_overlaps(&ranges);

        assert_eq!(analysis.overlaps.len(), 1);
        assert!(analysis.overlaps[0].is_redundant);
        assert!(analysis.redundant_patterns.contains(&1));
    }

    #[test]
    fn test_no_overlap() {
        // Pattern 0: 1..=5, Pattern 1: 7..=10
        let ranges = vec![
            (0, Interval::new(1, 5)),
            (1, Interval::new(7, 10)),
        ];
        let analysis = analyze_range_overlaps(&ranges);

        assert!(analysis.overlaps.is_empty());
        assert!(analysis.redundant_patterns.is_empty());
    }

    #[test]
    fn test_find_uncovered() {
        let ranges = vec![
            (0, Interval::new(1, 5)),
            (1, Interval::new(8, 10)),
        ];
        let bounds = Interval::new(1, 10);
        let uncovered = find_uncovered_ranges(&ranges, bounds);

        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0], Interval::new(6, 7));
    }

    #[test]
    fn test_describe_overlap() {
        let overlap = RangeOverlap::new(0, 1, Interval::new(3, 5), false);
        let desc = describe_range_overlap(&overlap);
        assert!(desc.contains("patterns 1 and 2 overlap on range 3..=5"));
    }
}
