//! Test filtering module for VCS test runner.
//!
//! Provides advanced filtering capabilities for test selection:
//!
//! - **Name patterns**: Glob patterns for test names
//! - **Path patterns**: Glob patterns for test file paths
//! - **Tag filters**: Include/exclude by tags
//! - **Level filters**: Filter by test level (L0-L4)
//! - **Tier filters**: Filter by execution tier
//! - **Type filters**: Filter by test type
//! - **Expression filters**: Complex boolean expressions
//!
//! # Filter Expressions
//!
//! The filter module supports complex boolean expressions:
//!
//! ```text
//! level:L0 AND tag:cbgr
//! (level:L1 OR level:L2) AND NOT tag:slow
//! path:**/memory/** AND type:run
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use vtest::filter::{TestFilter, FilterConfig};
//!
//! let filter = TestFilter::new()
//!     .with_levels(&["L0", "L1"])
//!     .with_tags(&["cbgr", "memory"])
//!     .with_exclude_tags(&["slow", "gpu"])
//!     .with_name_pattern("*ownership*");
//!
//! let matches = filter.matches(&test_directives);
//! ```

use crate::directive::{Level, TestDirectives, TestType, Tier};
use glob::Pattern;
use thiserror::Error;
use verum_common::{List, Set, Text};

/// Error type for filter operations.
#[derive(Debug, Error)]
pub enum FilterError {
    #[error("Invalid pattern: {0}")]
    InvalidPattern(Text),

    #[error("Invalid filter expression: {0}")]
    InvalidExpression(Text),

    #[error("Unknown filter key: {0}")]
    UnknownKey(Text),
}

/// A compiled test filter.
#[derive(Debug, Clone)]
pub struct TestFilter {
    /// Name pattern (glob)
    name_patterns: List<Pattern>,
    /// Path pattern (glob)
    path_patterns: List<Pattern>,
    /// Levels to include (empty = all)
    levels: Set<Level>,
    /// Tiers to include (empty = all from test)
    tiers: Set<Tier>,
    /// Tags to include (empty = all)
    include_tags: Set<Text>,
    /// Tags to exclude
    exclude_tags: Set<Text>,
    /// Test types to include (empty = all)
    test_types: Set<TestType>,
    /// Invert the filter result
    invert: bool,
}

impl Default for TestFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl TestFilter {
    /// Create a new empty filter (matches everything).
    pub fn new() -> Self {
        Self {
            name_patterns: List::new(),
            path_patterns: List::new(),
            levels: Set::new(),
            tiers: Set::new(),
            include_tags: Set::new(),
            exclude_tags: Set::new(),
            test_types: Set::new(),
            invert: false,
        }
    }

    /// Add name patterns (glob).
    pub fn with_name_pattern(mut self, pattern: &str) -> Result<Self, FilterError> {
        let pat = Pattern::new(pattern)
            .map_err(|e| FilterError::InvalidPattern(format!("{}: {}", pattern, e).into()))?;
        self.name_patterns.push(pat);
        Ok(self)
    }

    /// Add name patterns from a list.
    pub fn with_name_patterns(mut self, patterns: &[&str]) -> Result<Self, FilterError> {
        for pattern in patterns {
            let pat = Pattern::new(pattern)
                .map_err(|e| FilterError::InvalidPattern(format!("{}: {}", pattern, e).into()))?;
            self.name_patterns.push(pat);
        }
        Ok(self)
    }

    /// Add path patterns (glob).
    pub fn with_path_pattern(mut self, pattern: &str) -> Result<Self, FilterError> {
        let pat = Pattern::new(pattern)
            .map_err(|e| FilterError::InvalidPattern(format!("{}: {}", pattern, e).into()))?;
        self.path_patterns.push(pat);
        Ok(self)
    }

    /// Add path patterns from a list.
    pub fn with_path_patterns(mut self, patterns: &[&str]) -> Result<Self, FilterError> {
        for pattern in patterns {
            let pat = Pattern::new(pattern)
                .map_err(|e| FilterError::InvalidPattern(format!("{}: {}", pattern, e).into()))?;
            self.path_patterns.push(pat);
        }
        Ok(self)
    }

    /// Add levels to include.
    pub fn with_levels(mut self, levels: &[Level]) -> Self {
        self.levels.extend(levels.iter().cloned());
        self
    }

    /// Add levels from string representations.
    pub fn with_levels_str(mut self, levels: &[&str]) -> Self {
        for level_str in levels {
            if let Ok(level) = Level::from_str(level_str) {
                self.levels.insert(level);
            }
        }
        self
    }

    /// Add tiers to include.
    pub fn with_tiers(mut self, tiers: &[Tier]) -> Self {
        self.tiers.extend(tiers.iter().cloned());
        self
    }

    /// Add tiers from string representations.
    pub fn with_tiers_str(mut self, tiers: &[&str]) -> Self {
        for tier_str in tiers {
            if let Ok(tier) = Tier::from_str(tier_str) {
                self.tiers.insert(tier);
            }
        }
        self
    }

    /// Add tags to include.
    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.include_tags.extend(tags.iter().map(|s| s.to_string().into()));
        self
    }

    /// Add tags to exclude.
    pub fn with_exclude_tags(mut self, tags: &[&str]) -> Self {
        self.exclude_tags.extend(tags.iter().map(|s| s.to_string().into()));
        self
    }

    /// Add test types to include.
    pub fn with_test_types(mut self, types: &[TestType]) -> Self {
        self.test_types.extend(types.iter().cloned());
        self
    }

    /// Add test types from string representations.
    pub fn with_test_types_str(mut self, types: &[&str]) -> Self {
        for type_str in types {
            if let Ok(test_type) = TestType::from_str(type_str) {
                self.test_types.insert(test_type);
            }
        }
        self
    }

    /// Invert the filter result.
    pub fn inverted(mut self) -> Self {
        self.invert = !self.invert;
        self
    }

    /// Check if a test matches this filter.
    pub fn matches(&self, directives: &TestDirectives) -> bool {
        let result = self.matches_internal(directives);
        if self.invert { !result } else { result }
    }

    /// Internal matching logic.
    fn matches_internal(&self, directives: &TestDirectives) -> bool {
        // Check name patterns
        if !self.name_patterns.is_empty() {
            let name = directives.display_name();
            let matches_name = self.name_patterns.iter().any(|p| p.matches(&name));
            if !matches_name {
                return false;
            }
        }

        // Check path patterns
        if !self.path_patterns.is_empty() {
            let path = &directives.source_path;
            let matches_path = self.path_patterns.iter().any(|p| p.matches(path));
            if !matches_path {
                return false;
            }
        }

        // Check levels
        if !self.levels.is_empty() && !self.levels.contains(&directives.level) {
            return false;
        }

        // Check tiers (at least one tier must match)
        if !self.tiers.is_empty() {
            let has_matching_tier = directives.tiers.iter().any(|t| self.tiers.contains(t));
            if !has_matching_tier {
                return false;
            }
        }

        // Check include tags (at least one must match)
        if !self.include_tags.is_empty() {
            let has_matching_tag = directives
                .tags
                .iter()
                .any(|t| self.include_tags.contains(t));
            if !has_matching_tag {
                return false;
            }
        }

        // Check exclude tags (none must match)
        if !self.exclude_tags.is_empty() {
            let has_excluded_tag = directives
                .tags
                .iter()
                .any(|t| self.exclude_tags.contains(t));
            if has_excluded_tag {
                return false;
            }
        }

        // Check test types
        if !self.test_types.is_empty() && !self.test_types.contains(&directives.test_type) {
            return false;
        }

        true
    }

    /// Filter a list of tests.
    pub fn filter_tests(&self, tests: List<TestDirectives>) -> List<TestDirectives> {
        tests.into_iter().filter(|t| self.matches(t)).collect()
    }

    /// Count matching tests.
    pub fn count_matches(&self, tests: &[TestDirectives]) -> usize {
        tests.iter().filter(|t| self.matches(t)).count()
    }

    /// Check if this filter is empty (matches everything).
    pub fn is_empty(&self) -> bool {
        self.name_patterns.is_empty()
            && self.path_patterns.is_empty()
            && self.levels.is_empty()
            && self.tiers.is_empty()
            && self.include_tags.is_empty()
            && self.exclude_tags.is_empty()
            && self.test_types.is_empty()
    }

    /// Parse a filter from a string expression.
    ///
    /// Supported syntax:
    /// - `level:L0` - filter by level
    /// - `tier:0` - filter by tier
    /// - `tag:cbgr` - filter by tag
    /// - `type:run` - filter by test type
    /// - `name:*pattern*` - filter by name pattern
    /// - `path:**/memory/**` - filter by path pattern
    /// - `NOT expr` - negate expression
    /// - `expr AND expr` - conjunction
    /// - `expr OR expr` - disjunction
    pub fn parse(expr: &str) -> Result<Self, FilterError> {
        let expr = expr.trim();

        if expr.is_empty() {
            return Ok(Self::new());
        }

        // Handle NOT prefix
        if let Some(rest) = expr.strip_prefix("NOT ") {
            return Ok(Self::parse(rest)?.inverted());
        }

        // Handle AND/OR (simple left-to-right parsing) - check these BEFORE key:value
        // to properly handle cases like "level:L0 AND tag:cbgr"
        if let Some(pos) = expr.find(" AND ") {
            let left = Self::parse(&expr[..pos])?;
            let right = Self::parse(&expr[pos + 5..])?;
            return Ok(left.and(right));
        }

        if let Some(pos) = expr.find(" OR ") {
            let left = Self::parse(&expr[..pos])?;
            let right = Self::parse(&expr[pos + 4..])?;
            return Ok(left.or(right));
        }

        // Handle simple key:value expressions (only after checking for AND/OR)
        if let Some((key, value)) = expr.split_once(':') {
            return Self::parse_key_value(key.trim(), value.trim());
        }

        // If no operators, try as a pattern
        Self::new().with_name_pattern(expr)
    }

    /// Parse a key:value expression.
    fn parse_key_value(key: &str, value: &str) -> Result<Self, FilterError> {
        match key.to_lowercase().as_str() {
            "level" | "l" => Ok(Self::new().with_levels_str(&[value])),
            "tier" | "t" => Ok(Self::new().with_tiers_str(&[value])),
            "tag" => Ok(Self::new().with_tags(&[value])),
            "exclude-tag" | "notag" => Ok(Self::new().with_exclude_tags(&[value])),
            "type" => Ok(Self::new().with_test_types_str(&[value])),
            "name" | "n" => Self::new().with_name_pattern(value),
            "path" | "p" => Self::new().with_path_pattern(value),
            _ => Err(FilterError::UnknownKey(key.to_string().into())),
        }
    }

    /// Combine two filters with AND logic.
    pub fn and(mut self, other: Self) -> Self {
        // Merge name patterns (all must match one pattern)
        self.name_patterns.extend(other.name_patterns);

        // Merge path patterns
        self.path_patterns.extend(other.path_patterns);

        // Intersect levels (both must be satisfied)
        if !other.levels.is_empty() {
            if self.levels.is_empty() {
                self.levels = other.levels;
            } else {
                self.levels = self.levels.intersection(&other.levels).cloned().collect();
            }
        }

        // Intersect tiers
        if !other.tiers.is_empty() {
            if self.tiers.is_empty() {
                self.tiers = other.tiers;
            } else {
                self.tiers = self.tiers.intersection(&other.tiers).cloned().collect();
            }
        }

        // Union include tags (any can match)
        self.include_tags.extend(other.include_tags);

        // Union exclude tags (any excludes)
        self.exclude_tags.extend(other.exclude_tags);

        // Intersect test types
        if !other.test_types.is_empty() {
            if self.test_types.is_empty() {
                self.test_types = other.test_types;
            } else {
                self.test_types = self
                    .test_types
                    .intersection(&other.test_types)
                    .cloned()
                    .collect();
            }
        }

        self
    }

    /// Combine two filters with OR logic.
    pub fn or(self, other: Self) -> Self {
        // For OR, we create a composite filter
        CompositeFilter::new_or(vec![self, other]).into_filter()
    }
}

/// A composite filter that combines multiple filters.
#[derive(Debug, Clone)]
pub struct CompositeFilter {
    filters: List<TestFilter>,
    mode: CompositeMode,
}

/// Mode for combining filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositeMode {
    /// All filters must match (AND)
    All,
    /// Any filter must match (OR)
    Any,
}

impl CompositeFilter {
    /// Create a new AND composite filter.
    pub fn new_and(filters: Vec<TestFilter>) -> Self {
        Self {
            filters: filters.into(),
            mode: CompositeMode::All,
        }
    }

    /// Create a new OR composite filter.
    pub fn new_or(filters: Vec<TestFilter>) -> Self {
        Self {
            filters: filters.into(),
            mode: CompositeMode::Any,
        }
    }

    /// Check if a test matches this composite filter.
    pub fn matches(&self, directives: &TestDirectives) -> bool {
        match self.mode {
            CompositeMode::All => self.filters.iter().all(|f| f.matches(directives)),
            CompositeMode::Any => self.filters.iter().any(|f| f.matches(directives)),
        }
    }

    /// Convert to a simple TestFilter (for OR mode, creates a new filter).
    pub fn into_filter(self) -> TestFilter {
        if self.filters.is_empty() {
            return TestFilter::new();
        }

        if self.filters.len() == 1 {
            return self.filters.into_iter().next().unwrap();
        }

        // For OR mode with multiple filters, we need to merge them
        match self.mode {
            CompositeMode::All => {
                let mut result = TestFilter::new();
                for filter in self.filters {
                    result = result.and(filter);
                }
                result
            }
            CompositeMode::Any => {
                // For OR, we merge all levels, tags, etc.
                let mut result = TestFilter::new();
                for filter in self.filters {
                    result.levels.extend(filter.levels);
                    result.tiers.extend(filter.tiers);
                    result.include_tags.extend(filter.include_tags);
                    result.name_patterns.extend(filter.name_patterns);
                    result.path_patterns.extend(filter.path_patterns);
                    result.test_types.extend(filter.test_types);
                    // exclude_tags are NOT merged in OR mode
                }
                result
            }
        }
    }
}

/// Filter configuration for the test runner.
#[derive(Debug, Clone, Default)]
pub struct FilterConfig {
    /// Main filter
    pub filter: Option<TestFilter>,
    /// Additional include filter expression
    pub include_expr: Option<Text>,
    /// Additional exclude filter expression
    pub exclude_expr: Option<Text>,
    /// Skip tests marked with @skip
    pub skip_marked: bool,
    /// Only run tests that have failed before
    pub failed_only: bool,
    /// Only run tests that have changed since last run
    pub changed_only: bool,
}

impl FilterConfig {
    /// Create a new filter configuration.
    pub fn new() -> Self {
        Self {
            skip_marked: true,
            ..Default::default()
        }
    }

    /// Set the main filter.
    pub fn with_filter(mut self, filter: TestFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set include expression.
    pub fn with_include(mut self, expr: &str) -> Self {
        self.include_expr = Some(expr.to_string().into());
        self
    }

    /// Set exclude expression.
    pub fn with_exclude(mut self, expr: &str) -> Self {
        self.exclude_expr = Some(expr.to_string().into());
        self
    }

    /// Build the effective filter.
    pub fn build(&self) -> Result<TestFilter, FilterError> {
        let mut filter = self.filter.clone().unwrap_or_default();

        // Apply include expression
        if let Some(ref expr) = self.include_expr {
            let include_filter = TestFilter::parse(expr)?;
            filter = filter.and(include_filter);
        }

        // Apply exclude expression
        // For exclude, we convert tag filters to exclude_tags instead of inverting
        if let Some(ref expr) = self.exclude_expr {
            let exclude_filter = Self::parse_as_exclude(expr)?;
            filter = filter.and(exclude_filter);
        }

        Ok(filter)
    }

    /// Parse an expression as an exclusion filter.
    /// Converts include_tags to exclude_tags, etc.
    fn parse_as_exclude(expr: &str) -> Result<TestFilter, FilterError> {
        let parsed = TestFilter::parse(expr)?;

        // If it has include_tags, convert them to exclude_tags
        if !parsed.include_tags.is_empty() {
            return Ok(TestFilter {
                exclude_tags: parsed.include_tags,
                ..TestFilter::new()
            });
        }

        // Otherwise, just invert the whole filter
        Ok(parsed.inverted())
    }

    /// Check if a test matches the configuration.
    pub fn matches(&self, directives: &TestDirectives) -> Result<bool, FilterError> {
        // Skip tests marked with @skip
        if self.skip_marked && directives.skip.is_some() {
            return Ok(false);
        }

        let filter = self.build()?;
        Ok(filter.matches(directives))
    }
}

/// Quick filter builder for common use cases.
pub struct QuickFilter;

impl QuickFilter {
    /// Filter for L0 (critical) tests only.
    pub fn critical() -> TestFilter {
        TestFilter::new().with_levels(&[Level::L0])
    }

    /// Filter for L0 and L1 (critical + core) tests.
    pub fn core() -> TestFilter {
        TestFilter::new().with_levels(&[Level::L0, Level::L1])
    }

    /// Filter for fast tests (excludes slow, gpu, benchmark).
    pub fn fast() -> TestFilter {
        TestFilter::new().with_exclude_tags(&["slow", "gpu", "benchmark"])
    }

    /// Filter for CBGR-related tests.
    pub fn cbgr() -> TestFilter {
        TestFilter::new().with_tags(&["cbgr", "memory", "ownership", "borrow"])
    }

    /// Filter for type system tests.
    pub fn types() -> TestFilter {
        TestFilter::new().with_tags(&["types", "inference", "generics", "refinement"])
    }

    /// Filter for verification tests.
    pub fn verification() -> TestFilter {
        TestFilter::new().with_tags(&["verify", "smt", "z3", "proof"])
    }

    /// Filter for benchmark tests.
    pub fn benchmarks() -> TestFilter {
        TestFilter::new().with_test_types(&[TestType::Benchmark])
    }

    /// Filter for interpreter-only tests.
    pub fn interpreter() -> TestFilter {
        TestFilter::new().with_tiers(&[Tier::Tier0])
    }

    /// Filter for compiled (non-interpreter) tests.
    pub fn compiled() -> TestFilter {
        TestFilter::new().with_tiers(&[Tier::Tier1, Tier::Tier2, Tier::Tier3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_directives(name: &str, level: Level, tags: &[&str]) -> TestDirectives {
        let mut directives = TestDirectives::default();
        directives.source_path = format!("tests/{}.vr", name).into();
        directives.level = level;
        directives.tags = tags.iter().map(|s| s.to_string().into()).collect();
        directives
    }

    #[test]
    fn test_filter_by_level() {
        let filter = TestFilter::new().with_levels(&[Level::L0, Level::L1]);

        let l0_test = make_test_directives("l0_test", Level::L0, &[]);
        let l2_test = make_test_directives("l2_test", Level::L2, &[]);

        assert!(filter.matches(&l0_test));
        assert!(!filter.matches(&l2_test));
    }

    #[test]
    fn test_filter_by_tags() {
        let filter = TestFilter::new()
            .with_tags(&["cbgr", "memory"])
            .with_exclude_tags(&["slow"]);

        let cbgr_test = make_test_directives("cbgr_test", Level::L1, &["cbgr"]);
        let slow_test = make_test_directives("slow_test", Level::L1, &["cbgr", "slow"]);
        let other_test = make_test_directives("other_test", Level::L1, &["types"]);

        assert!(filter.matches(&cbgr_test));
        assert!(!filter.matches(&slow_test)); // excluded by slow tag
        assert!(!filter.matches(&other_test)); // no matching include tag
    }

    #[test]
    fn test_filter_by_name_pattern() {
        let filter = TestFilter::new().with_name_pattern("*cbgr*").unwrap();

        let cbgr_test = make_test_directives("test_cbgr_ownership", Level::L1, &[]);
        let other_test = make_test_directives("test_types", Level::L1, &[]);

        assert!(filter.matches(&cbgr_test));
        assert!(!filter.matches(&other_test));
    }

    #[test]
    fn test_filter_parse_simple() {
        let filter = TestFilter::parse("level:L0").unwrap();
        let l0_test = make_test_directives("test", Level::L0, &[]);
        let l1_test = make_test_directives("test", Level::L1, &[]);

        assert!(filter.matches(&l0_test));
        assert!(!filter.matches(&l1_test));
    }

    #[test]
    fn test_filter_parse_and() {
        let filter = TestFilter::parse("level:L0 AND tag:cbgr").unwrap();

        let matching = make_test_directives("test", Level::L0, &["cbgr"]);
        let wrong_level = make_test_directives("test", Level::L1, &["cbgr"]);
        let wrong_tag = make_test_directives("test", Level::L0, &["types"]);

        assert!(filter.matches(&matching));
        assert!(!filter.matches(&wrong_level));
        assert!(!filter.matches(&wrong_tag));
    }

    #[test]
    fn test_filter_inverted() {
        let filter = TestFilter::new().with_levels(&[Level::L0]).inverted();

        let l0_test = make_test_directives("test", Level::L0, &[]);
        let l1_test = make_test_directives("test", Level::L1, &[]);

        assert!(!filter.matches(&l0_test)); // inverted
        assert!(filter.matches(&l1_test));
    }

    #[test]
    fn test_quick_filters() {
        let l0_test = make_test_directives("test", Level::L0, &[]);
        let l2_test = make_test_directives("test", Level::L2, &[]);

        assert!(QuickFilter::critical().matches(&l0_test));
        assert!(!QuickFilter::critical().matches(&l2_test));

        assert!(QuickFilter::core().matches(&l0_test));
        assert!(!QuickFilter::core().matches(&l2_test));
    }

    #[test]
    fn test_filter_config() {
        let config = FilterConfig::new()
            .with_filter(TestFilter::new().with_levels(&[Level::L0]))
            .with_exclude("tag:slow");

        let matching = make_test_directives("test", Level::L0, &[]);
        let excluded = make_test_directives("test", Level::L0, &["slow"]);

        assert!(config.matches(&matching).unwrap());
        assert!(!config.matches(&excluded).unwrap());
    }
}
