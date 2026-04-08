//! Test discovery module for VCS test runner.
//!
//! Provides flexible test discovery with support for:
//! - Recursive `.vr` file discovery
//! - Glob pattern matching
//! - Tag-based filtering
//! - Level-based filtering
//! - Parallel discovery for large test suites
//!
//! # Example
//!
//! ```rust,ignore
//! use vtest::discovery::{TestDiscovery, DiscoveryConfig};
//!
//! let config = DiscoveryConfig::default()
//!     .with_paths(vec!["specs/"])
//!     .with_pattern("**/*.vr")
//!     .with_levels(vec![Level::L0, Level::L1])
//!     .with_tags(vec!["cbgr", "memory"]);
//!
//! let discovery = TestDiscovery::new(config);
//! let tests = discovery.discover()?;
//!
//! for test in &tests {
//!     println!("{}: {:?}", test.display_name(), test.level);
//! }
//! ```

use crate::directive::{DirectiveError, Level, TestDirectives, TestType, Tier};
use glob::Pattern;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use thiserror::Error;
use verum_common::{List, Map, Set, Text};
use walkdir::WalkDir;

/// Error type for discovery operations.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Glob pattern error: {0}")]
    GlobError(#[from] glob::PatternError),

    #[error("Directive error: {0}")]
    DirectiveError(#[from] DirectiveError),

    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("Invalid pattern: {0}")]
    InvalidPattern(Text),
}

/// Configuration for test discovery.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Base paths to search for tests
    pub paths: List<PathBuf>,
    /// Glob pattern for test files (default: "**/*.vr")
    pub pattern: Text,
    /// Patterns to exclude (e.g., "**/skip/**")
    pub exclude_patterns: List<Text>,
    /// Filter by tags (empty = include all)
    pub include_tags: Set<Text>,
    /// Exclude tests with these tags
    pub exclude_tags: Set<Text>,
    /// Filter by levels (empty = include all)
    pub levels: Set<Level>,
    /// Filter by tiers (empty = include all)
    pub tiers: Set<Tier>,
    /// Filter by test types (empty = include all)
    pub test_types: Set<TestType>,
    /// Filter by name pattern (glob-style)
    pub name_pattern: Option<Text>,
    /// Enable parallel discovery (faster for large test suites)
    pub parallel: bool,
    /// Include hidden directories (starting with .)
    pub include_hidden: bool,
    /// Maximum depth to search (None = unlimited)
    pub max_depth: Option<usize>,
    /// Follow symbolic links
    pub follow_symlinks: bool,
    /// Verbose mode for debugging
    pub verbose: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        let exclude: List<Text> = vec![
            "**/skip/**".to_string().into(),
            "**/wip/**".to_string().into(),
            "**/experimental/**".to_string().into(),
        ].into();
        Self {
            paths: vec![PathBuf::from("specs")].into(),
            pattern: "**/*.vr".to_string().into(),
            exclude_patterns: exclude,
            include_tags: Set::new(),
            exclude_tags: Set::new(),
            levels: Set::new(),
            tiers: Set::new(),
            test_types: Set::new(),
            name_pattern: None,
            parallel: true,
            include_hidden: false,
            max_depth: None,
            follow_symlinks: false,
            verbose: false,
        }
    }
}

impl DiscoveryConfig {
    /// Create a new discovery config with the given paths.
    pub fn with_paths(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.paths = paths.into_iter().map(|p| p.into()).collect();
        self
    }

    /// Set the glob pattern for test files.
    pub fn with_pattern(mut self, pattern: impl Into<Text>) -> Self {
        self.pattern = pattern.into();
        self
    }

    /// Add exclude patterns.
    pub fn with_exclude(mut self, patterns: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.exclude_patterns = patterns.into_iter().map(|p| p.into()).collect();
        self
    }

    /// Filter by tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.include_tags = tags.into_iter().map(|t| t.into()).collect();
        self
    }

    /// Exclude tests with certain tags.
    pub fn excluding_tags(mut self, tags: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.exclude_tags = tags.into_iter().map(|t| t.into()).collect();
        self
    }

    /// Filter by levels.
    pub fn with_levels(mut self, levels: impl IntoIterator<Item = Level>) -> Self {
        self.levels = levels.into_iter().collect();
        self
    }

    /// Filter by tiers.
    pub fn with_tiers(mut self, tiers: impl IntoIterator<Item = Tier>) -> Self {
        self.tiers = tiers.into_iter().collect();
        self
    }

    /// Filter by test types.
    pub fn with_test_types(mut self, types: impl IntoIterator<Item = TestType>) -> Self {
        self.test_types = types.into_iter().collect();
        self
    }

    /// Filter by name pattern.
    pub fn with_name_pattern(mut self, pattern: impl Into<Text>) -> Self {
        self.name_pattern = Some(pattern.into());
        self
    }

    /// Enable or disable parallel discovery.
    pub fn parallel(mut self, enabled: bool) -> Self {
        self.parallel = enabled;
        self
    }

    /// Set maximum search depth.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Enable verbose mode.
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }
}

/// Result of a discovery operation.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Discovered tests
    pub tests: List<TestDirectives>,
    /// Statistics about discovery
    pub stats: DiscoveryStats,
    /// Number of errors encountered during discovery
    pub error_count: usize,
}

impl DiscoveryResult {
    /// Get tests filtered by a predicate.
    pub fn filter<F>(&self, predicate: F) -> List<&TestDirectives>
    where
        F: Fn(&TestDirectives) -> bool,
    {
        self.tests.iter().filter(|t| predicate(t)).collect()
    }

    /// Get tests grouped by level.
    pub fn by_level(&self) -> Map<Level, List<&TestDirectives>> {
        let mut result: Map<Level, List<&TestDirectives>> = Map::new();
        for test in &self.tests {
            result.entry(test.level).or_default().push(test);
        }
        result
    }

    /// Get tests grouped by test type.
    pub fn by_type(&self) -> Map<TestType, List<&TestDirectives>> {
        let mut result: Map<TestType, List<&TestDirectives>> = Map::new();
        for test in &self.tests {
            result.entry(test.test_type).or_default().push(test);
        }
        result
    }

    /// Get tests grouped by tag.
    pub fn by_tag(&self) -> Map<Text, List<&TestDirectives>> {
        let mut result: Map<Text, List<&TestDirectives>> = Map::new();
        for test in &self.tests {
            for tag in &test.tags {
                result.entry(tag.clone()).or_default().push(test);
            }
        }
        result
    }

    /// Get all unique tags from discovered tests.
    pub fn all_tags(&self) -> Set<Text> {
        self.tests
            .iter()
            .flat_map(|t| t.tags.iter().cloned())
            .collect()
    }
}

/// Statistics about a discovery operation.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryStats {
    /// Total files scanned
    pub files_scanned: usize,
    /// Files matching pattern
    pub files_matched: usize,
    /// Tests discovered
    pub tests_found: usize,
    /// Tests filtered out
    pub tests_filtered: usize,
    /// Parse errors encountered
    pub parse_errors: usize,
    /// Time taken for discovery
    pub duration_ms: u64,
    /// Breakdown by level
    pub by_level: Map<Level, usize>,
    /// Breakdown by test type
    pub by_type: Map<TestType, usize>,
}

impl DiscoveryStats {
    /// Get total tests (found - filtered).
    pub fn total(&self) -> usize {
        self.tests_found.saturating_sub(self.tests_filtered)
    }
}

/// Test discovery engine.
#[derive(Clone)]
pub struct TestDiscovery {
    config: DiscoveryConfig,
    exclude_patterns: List<Pattern>,
    name_pattern: Option<Pattern>,
}

impl TestDiscovery {
    /// Create a new test discovery engine with the given configuration.
    pub fn new(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        // Compile exclude patterns
        let exclude_patterns: Result<List<Pattern>, _> = config
            .exclude_patterns
            .iter()
            .map(|p| Pattern::new(p))
            .collect();

        let name_pattern = match &config.name_pattern {
            Some(p) => Some(Pattern::new(p)?),
            None => None,
        };

        Ok(Self {
            config,
            exclude_patterns: exclude_patterns?,
            name_pattern,
        })
    }

    /// Discover all tests matching the configuration.
    pub fn discover(&self) -> Result<DiscoveryResult, DiscoveryError> {
        let start = std::time::Instant::now();
        let mut stats = DiscoveryStats::default();
        let mut error_count = 0usize;

        // Collect all matching files
        let files = self.collect_files(&mut stats)?;

        // Parse test directives (parallel or sequential)
        let parsed: List<Result<TestDirectives, DirectiveError>> = if self.config.parallel {
            let vec: Vec<_> = files
                .par_iter()
                .map(|path| TestDirectives::from_file(path))
                .collect();
            vec.into()
        } else {
            files
                .iter()
                .map(|path| TestDirectives::from_file(path))
                .collect()
        };

        // Process results and apply filters
        let mut tests = List::new();
        for result in parsed {
            match result {
                Ok(directives) => {
                    stats.tests_found += 1;

                    // Apply filters
                    if self.should_include(&directives) {
                        // Update stats
                        *stats.by_level.entry(directives.level).or_default() += 1;
                        *stats.by_type.entry(directives.test_type).or_default() += 1;

                        tests.push(directives);
                    } else {
                        stats.tests_filtered += 1;
                    }
                }
                Err(DirectiveError::MissingTestDirective) => {
                    // Skip files without @test directive (not an error)
                    continue;
                }
                Err(e) => {
                    stats.parse_errors += 1;
                    error_count += 1;
                    if self.config.verbose {
                        eprintln!("Discovery error: {}", e);
                    }
                }
            }
        }

        stats.duration_ms = start.elapsed().as_millis() as u64;

        Ok(DiscoveryResult {
            tests,
            stats,
            error_count,
        })
    }

    /// Collect all files matching the pattern.
    fn collect_files(&self, stats: &mut DiscoveryStats) -> Result<List<PathBuf>, DiscoveryError> {
        let pattern = Pattern::new(&self.config.pattern)?;
        let mut files = List::new();

        for base_path in &self.config.paths {
            if !base_path.exists() {
                if self.config.verbose {
                    eprintln!("Warning: Path not found: {}", base_path.display());
                }
                continue;
            }

            let mut walker = WalkDir::new(base_path);

            if let Some(max_depth) = self.config.max_depth {
                walker = walker.max_depth(max_depth);
            }

            if self.config.follow_symlinks {
                walker = walker.follow_links(true);
            }

            for entry in walker.into_iter().filter_map(|e| e.ok()) {
                stats.files_scanned += 1;

                let path = entry.path();

                // Skip directories
                if !path.is_file() {
                    continue;
                }

                // Skip hidden files unless configured
                if !self.config.include_hidden {
                    if let Some(name) = path.file_name() {
                        if name.to_string_lossy().starts_with('.') {
                            continue;
                        }
                    }
                }

                // Match against pattern — normalize to forward slashes so glob
                // patterns work identically on Windows and Unix.
                let relative_path = path.strip_prefix(base_path).unwrap_or(path);
                let path_str = relative_path.to_string_lossy();
                #[cfg(windows)]
                let path_str = std::borrow::Cow::Owned(path_str.replace('\\', "/"));

                if !pattern.matches(&path_str) {
                    continue;
                }

                // Check against exclude patterns
                let excluded = self
                    .exclude_patterns
                    .iter()
                    .any(|p| p.matches(&path_str) || path_str.contains(p.as_str()));

                if excluded {
                    continue;
                }

                stats.files_matched += 1;
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    /// Check if a test should be included based on filters.
    fn should_include(&self, test: &TestDirectives) -> bool {
        // Level filter
        if !self.config.levels.is_empty() && !self.config.levels.contains(&test.level) {
            return false;
        }

        // Tier filter
        if !self.config.tiers.is_empty() {
            let has_matching_tier = test.tiers.iter().any(|t| self.config.tiers.contains(t));
            if !has_matching_tier {
                return false;
            }
        }

        // Test type filter
        if !self.config.test_types.is_empty() && !self.config.test_types.contains(&test.test_type) {
            return false;
        }

        // Tag filters
        if !self.config.include_tags.is_empty() {
            let has_required_tag = test
                .tags
                .iter()
                .any(|t| self.config.include_tags.contains(t));
            if !has_required_tag {
                return false;
            }
        }

        // Exclude tags
        if !self.config.exclude_tags.is_empty() {
            let has_excluded_tag = test
                .tags
                .iter()
                .any(|t| self.config.exclude_tags.contains(t));
            if has_excluded_tag {
                return false;
            }
        }

        // Name pattern filter
        if let Some(ref pattern) = self.name_pattern {
            let name = test.display_name();
            if !pattern.matches(&name) {
                return false;
            }
        }

        true
    }

    /// Discover and return tests as an iterator.
    pub fn iter(&self) -> Result<impl Iterator<Item = TestDirectives>, DiscoveryError> {
        let result = self.discover()?;
        Ok(result.tests.into_iter())
    }
}

/// Builder pattern for creating discovery configurations.
pub struct DiscoveryBuilder {
    config: DiscoveryConfig,
}

impl DiscoveryBuilder {
    /// Create a new discovery builder.
    pub fn new() -> Self {
        Self {
            config: DiscoveryConfig::default(),
        }
    }

    /// Add paths to search.
    pub fn paths<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.config.paths = paths.into_iter().map(|p| p.into()).collect();
        self
    }

    /// Set the file pattern.
    pub fn pattern<S: Into<Text>>(mut self, pattern: S) -> Self {
        self.config.pattern = pattern.into();
        self
    }

    /// Add exclude patterns.
    pub fn exclude<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        self.config.exclude_patterns = patterns.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Filter by levels.
    pub fn levels<I>(mut self, levels: I) -> Self
    where
        I: IntoIterator<Item = Level>,
    {
        self.config.levels = levels.into_iter().collect();
        self
    }

    /// Filter by tags.
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        self.config.include_tags = tags.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Build the discovery engine.
    pub fn build(self) -> Result<TestDiscovery, DiscoveryError> {
        TestDiscovery::new(self.config)
    }

    /// Discover tests immediately.
    pub fn discover(self) -> Result<DiscoveryResult, DiscoveryError> {
        self.build()?.discover()
    }
}

impl Default for DiscoveryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Quick discovery helper functions.
pub mod quick {
    use super::*;

    /// Discover all tests in the default location.
    pub fn discover_all() -> Result<List<TestDirectives>, DiscoveryError> {
        Ok(TestDiscovery::new(DiscoveryConfig::default())?
            .discover()?
            .tests)
    }

    /// Discover tests at a specific level.
    pub fn discover_level(level: Level) -> Result<List<TestDirectives>, DiscoveryError> {
        let config = DiscoveryConfig::default().with_levels(vec![level]);
        Ok(TestDiscovery::new(config)?.discover()?.tests)
    }

    /// Discover tests with specific tags.
    pub fn discover_tagged(tags: &[&str]) -> Result<List<TestDirectives>, DiscoveryError> {
        let config = DiscoveryConfig::default().with_tags(tags.iter().map(|s| s.to_string()));
        Ok(TestDiscovery::new(config)?.discover()?.tests)
    }

    /// Discover tests in a specific path.
    pub fn discover_path<P: AsRef<Path>>(path: P) -> Result<List<TestDirectives>, DiscoveryError> {
        let config = DiscoveryConfig::default().with_paths(vec![path.as_ref().to_path_buf()]);
        Ok(TestDiscovery::new(config)?.discover()?.tests)
    }

    /// Count tests at each level.
    pub fn count_by_level() -> Result<Map<Level, usize>, DiscoveryError> {
        let result = TestDiscovery::new(DiscoveryConfig::default())?.discover()?;
        Ok(result.stats.by_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_config_default() {
        let config = DiscoveryConfig::default();
        assert!(!config.paths.is_empty());
        assert!(config.parallel);
        assert!(!config.include_hidden);
    }

    #[test]
    fn test_discovery_config_builder() {
        let config = DiscoveryConfig::default()
            .with_paths(vec!["test/"])
            .with_pattern("*.vr")
            .with_levels(vec![Level::L0, Level::L1])
            .with_tags(vec!["cbgr"]);

        assert_eq!(config.paths.len(), 1);
        assert_eq!(config.pattern.as_str(), "*.vr");
        assert!(config.levels.contains(&Level::L0));
        assert!(config.include_tags.contains(&"cbgr".to_string().into()));
    }

    #[test]
    fn test_discovery_builder() {
        let builder = DiscoveryBuilder::new()
            .paths(vec!["specs/"])
            .pattern("**/*.vr")
            .levels(vec![Level::L0]);

        // Build should succeed
        assert!(builder.build().is_ok());
    }

    #[test]
    fn test_discovery_stats_default() {
        let stats = DiscoveryStats::default();
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.tests_found, 0);
        assert_eq!(stats.total(), 0);
    }
}
