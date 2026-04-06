//! Comprehensive regex tests for Verum standard library
//!
//! This test suite validates all regex functionality including:
//! - Basic matching and capture groups
//! - Named groups
//! - Unicode support
//! - Advanced features (lookahead, lookbehind, backreferences)
//! - RegexSet
//! - RegexBuilder
//! - Security features (ReDoS prevention)
//! - Parallel matching
//! - Edge cases

use verum_std::regex::*;
use verum_std::core::{Result as VerumResult, Text, Maybe};

// ============================================================================
// Basic Matching Tests
// ============================================================================

#[test]
fn test_basic_literal_match() {
    let re = Regex::new("hello").expect("valid pattern");
    assert!(re.is_match("hello"));
    assert!(re.is_match("say hello world"));
    assert!(!re.is_match("goodbye"));
}

#[test]
fn test_digit_matching() {
    let re = Regex::new(r"\d+").expect("valid pattern");
    assert!(re.is_match("123"));
    assert!(re.is_match("abc123def"));
    assert!(!re.is_match("abc"));

    let m = re.find("abc123def").expect("should match");
    assert_eq!(m.as_str(), "123");
    assert_eq!(m.start(), 3);
    assert_eq!(m.end(), 6);
}

#[test]
fn test_word_matching() {
    let re = Regex::new(r"\w+").expect("valid pattern");
    let words: Vec<_> = re.find_all("hello world from rust")
        .map(|m| m.as_str())
        .collect();
    assert_eq!(words, vec!["hello", "world", "from", "rust"]);
}

#[test]
fn test_anchors() {
    let start_re = Regex::new(r"^hello").expect("valid pattern");
    assert!(start_re.is_match("hello world"));
    assert!(!start_re.is_match("say hello"));

    let end_re = Regex::new(r"world$").expect("valid pattern");
    assert!(end_re.is_match("hello world"));
    assert!(!end_re.is_match("world hello"));
}

// ============================================================================
// Character Classes Tests
// ============================================================================

#[test]
fn test_character_classes() {
    let re = Regex::new(r"[aeiou]").expect("valid pattern");
    assert!(re.is_match("hello"));
    assert!(re.is_match("AEIOU".to_lowercase().as_str()));
    assert!(!re.is_match("xyz"));
}

#[test]
fn test_negated_character_class() {
    let re = Regex::new(r"[^aeiou]").expect("valid pattern");
    assert!(re.is_match("xyz"));
    assert!(re.is_match("hello")); // has consonants
}

#[test]
fn test_range_character_class() {
    let re = Regex::new(r"[a-z]+").expect("valid pattern");
    assert!(re.is_match("hello"));
    assert!(!re.is_match("123"));

    let m = re.find("abc123def").expect("should match");
    assert_eq!(m.as_str(), "abc");
}

// ============================================================================
// Quantifier Tests
// ============================================================================

#[test]
fn test_quantifiers() {
    // Zero or more
    let re_star = Regex::new(r"ab*c").expect("valid pattern");
    assert!(re_star.is_match("ac"));
    assert!(re_star.is_match("abc"));
    assert!(re_star.is_match("abbbc"));

    // One or more
    let re_plus = Regex::new(r"ab+c").expect("valid pattern");
    assert!(!re_plus.is_match("ac"));
    assert!(re_plus.is_match("abc"));
    assert!(re_plus.is_match("abbbc"));

    // Zero or one
    let re_question = Regex::new(r"ab?c").expect("valid pattern");
    assert!(re_question.is_match("ac"));
    assert!(re_question.is_match("abc"));
    assert!(!re_question.is_match("abbc"));
}

#[test]
fn test_counted_quantifiers() {
    let re_exact = Regex::new(r"\d{3}").expect("valid pattern");
    assert!(!re_exact.is_match("12"));
    assert!(re_exact.is_match("123"));
    assert!(re_exact.is_match("1234")); // matches first 3

    let re_min = Regex::new(r"\d{2,}").expect("valid pattern");
    assert!(!re_min.is_match("1"));
    assert!(re_min.is_match("12"));
    assert!(re_min.is_match("123"));

    let re_range = Regex::new(r"\d{2,4}").expect("valid pattern");
    let m = re_range.find("12345").expect("should match");
    assert_eq!(m.as_str(), "1234"); // Greedy, takes 4
}

#[test]
fn test_lazy_quantifiers() {
    let re_greedy = Regex::new(r"a.*b").expect("valid pattern");
    let m = re_greedy.find("axxxbyyybzzz").expect("should match");
    assert_eq!(m.as_str(), "axxxbyyybzzz"); // Matches to last 'b'

    let re_lazy = Regex::new(r"a.*?b").expect("valid pattern");
    let m = re_lazy.find("axxxbyyybzzz").expect("should match");
    assert_eq!(m.as_str(), "axxxb"); // Matches to first 'b'
}

// ============================================================================
// Capture Group Tests
// ============================================================================

#[test]
fn test_simple_capture_groups() {
    let re = Regex::new(r"(\d+)-(\d+)-(\d+)").expect("valid pattern");
    let caps = re.captures("2025-11-24").expect("should match");

    assert_eq!(caps.get(0).expect("full match").as_str(), "2025-11-24");
    assert_eq!(caps.get(1).expect("group 1").as_str(), "2025");
    assert_eq!(caps.get(2).expect("group 2").as_str(), "11");
    assert_eq!(caps.get(3).expect("group 3").as_str(), "24");
}

#[test]
fn test_named_capture_groups() {
    let re = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")
        .expect("valid pattern");

    let caps = re.captures("2025-11-24").expect("should match");
    assert_eq!(caps.name("year").expect("year").as_str(), "2025");
    assert_eq!(caps.name("month").expect("month").as_str(), "11");
    assert_eq!(caps.name("day").expect("day").as_str(), "24");
}

#[test]
fn test_non_capturing_groups() {
    let re = Regex::new(r"(?:\d+)-(\w+)").expect("valid pattern");
    let caps = re.captures("123-abc").expect("should match");

    assert_eq!(caps.len(), 2); // Full match + 1 capturing group
    assert_eq!(caps.get(0).expect("full match").as_str(), "123-abc");
    assert_eq!(caps.get(1).expect("group 1").as_str(), "abc");
}

#[test]
fn test_nested_capture_groups() {
    let re = Regex::new(r"((\d+)-(\w+))").expect("valid pattern");
    let caps = re.captures("123-abc").expect("should match");

    assert_eq!(caps.get(1).expect("outer group").as_str(), "123-abc");
    assert_eq!(caps.get(2).expect("first nested").as_str(), "123");
    assert_eq!(caps.get(3).expect("second nested").as_str(), "abc");
}

// ============================================================================
// Unicode Tests
// ============================================================================

#[test]
fn test_unicode_matching() {
    let re = Regex::new(r"\w+").expect("valid pattern");
    assert!(re.is_match("hello"));
    assert!(re.is_match("مرحبا")); // Arabic
    assert!(re.is_match("你好")); // Chinese
    assert!(re.is_match("привет")); // Russian
}

#[test]
fn test_unicode_character_classes() {
    // Unicode letter class
    let re = Regex::new(r"\p{L}+").expect("valid pattern");
    assert!(re.is_match("hello"));
    assert!(re.is_match("你好"));
    assert!(re.is_match("مرحبا"));

    // Unicode number class
    let re_num = Regex::new(r"\p{N}+").expect("valid pattern");
    assert!(re_num.is_match("123"));
    assert!(re_num.is_match("٠١٢")); // Arabic-Indic digits
}

#[test]
fn test_emoji_matching() {
    let re = Regex::new(r"\p{Emoji}+").expect("valid pattern");
    assert!(re.is_match("😀"));
    assert!(re.is_match("🎉🎊"));
}

// ============================================================================
// Advanced Features Tests
// ============================================================================

#[test]
fn test_lookahead() {
    // Positive lookahead
    let re = Regex::new(r"\d+(?=px)").expect("valid pattern");
    assert!(re.is_match("100px"));
    let m = re.find("100px").expect("should match");
    assert_eq!(m.as_str(), "100"); // Doesn't include 'px'

    // Negative lookahead
    let re_neg = Regex::new(r"\d+(?!px)").expect("valid pattern");
    assert!(re_neg.is_match("100pt"));
    assert!(!re_neg.find("100px").is_some() || re_neg.find("100px").expect("match").as_str() != "100");
}

#[test]
fn test_lookbehind() {
    // Positive lookbehind
    let re = Regex::new(r"(?<=\$)\d+").expect("valid pattern");
    let m = re.find("$100").expect("should match");
    assert_eq!(m.as_str(), "100"); // Doesn't include '$'

    // Negative lookbehind
    let re_neg = Regex::new(r"(?<!\\)\"").expect("valid pattern");
    assert!(re_neg.is_match("\"hello\""));
    assert!(!re_neg.is_match("\\\""));
}

#[test]
fn test_backreferences() {
    // Match repeated words
    let re = Regex::new(r"(\w+)\s+\1").expect("valid pattern");
    assert!(re.is_match("hello hello"));
    assert!(!re.is_match("hello world"));

    let m = re.find("the the cat").expect("should match");
    assert_eq!(m.as_str(), "the the");
}

#[test]
fn test_alternation() {
    let re = Regex::new(r"cat|dog|bird").expect("valid pattern");
    assert!(re.is_match("I have a cat"));
    assert!(re.is_match("I have a dog"));
    assert!(re.is_match("I have a bird"));
    assert!(!re.is_match("I have a fish"));
}

// ============================================================================
// Replacement Tests
// ============================================================================

#[test]
fn test_simple_replacement() {
    let re = Regex::new(r"\d+").expect("valid pattern");

    let result = re.replace("abc123def", "XXX");
    assert_eq!(result.as_str(), "abcXXXdef");

    let result_all = re.replace_all("abc123def456", "X");
    assert_eq!(result_all.as_str(), "abcXdefX");
}

#[test]
fn test_replacement_with_captures() {
    let re = Regex::new(r"(\d{4})-(\d{2})-(\d{2})").expect("valid pattern");

    let result = re.replace("Date: 2025-11-24", "$2/$3/$1");
    assert_eq!(result.as_str(), "Date: 11/24/2025");
}

#[test]
fn test_replacement_with_closure() {
    let re = Regex::new(r"\d+").expect("valid pattern");

    let result = re.replace_all_with("a1b2c3", |caps| {
        let n: i32 = caps.get(0).expect("match").as_str().parse().expect("number");
        Text::from(format!("{}", n * 2))
    });

    assert_eq!(result.as_str(), "a2b4c6");
}

#[test]
fn test_named_group_replacement() {
    let re = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")
        .expect("valid pattern");

    let caps = re.captures("2025-11-24").expect("should match");
    let result = caps.expand("$month/$day/$year");
    assert_eq!(result.as_str(), "11/24/2025");
}

// ============================================================================
// Split Tests
// ============================================================================

#[test]
fn test_split() {
    let re = Regex::new(r"\s+").expect("valid pattern");

    let parts: Vec<_> = re.split("a  b\tc\nd").collect();
    assert_eq!(parts, vec!["a", "b", "c", "d"]);
}

#[test]
fn test_split_with_limit() {
    let re = Regex::new(r"\s+").expect("valid pattern");

    let parts: Vec<_> = re.splitn("a b c d e", 3).collect();
    assert_eq!(parts, vec!["a", "b", "c d e"]);
}

#[test]
fn test_split_empty() {
    let re = Regex::new(r"\s+").expect("valid pattern");

    let parts: Vec<_> = re.split("").collect();
    assert_eq!(parts, vec![""]);
}

// ============================================================================
// RegexBuilder Tests
// ============================================================================

#[test]
fn test_case_insensitive() {
    let re = RegexBuilder::new(r"hello")
        .case_insensitive(true)
        .build()
        .expect("valid pattern");

    assert!(re.is_match("HELLO"));
    assert!(re.is_match("hello"));
    assert!(re.is_match("HeLLo"));
}

#[test]
fn test_multi_line() {
    let re = RegexBuilder::new(r"^line")
        .multi_line(true)
        .build()
        .expect("valid pattern");

    assert!(re.is_match("line 1\nline 2"));

    let matches: Vec<_> = re.find_all("line 1\nline 2\ntext")
        .map(|m| m.as_str())
        .collect();
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_dot_matches_newline() {
    let re = RegexBuilder::new(r"a.b")
        .dot_matches_newline(true)
        .build()
        .expect("valid pattern");

    assert!(re.is_match("a\nb"));

    let re_default = Regex::new(r"a.b").expect("valid pattern");
    assert!(!re_default.is_match("a\nb"));
}

#[test]
fn test_ignore_whitespace() {
    let re = RegexBuilder::new(r"
        \d{4}  # year
        -      # separator
        \d{2}  # month
        -      # separator
        \d{2}  # day
    ")
    .ignore_whitespace(true)
    .build()
    .expect("valid pattern");

    assert!(re.is_match("2025-11-24"));
}

// ============================================================================
// RegexSet Tests
// ============================================================================

#[test]
fn test_regex_set_basic() {
    let set = RegexSet::new(&[
        r"\w+",
        r"\d+",
        r"\s+",
    ]).expect("valid patterns");

    assert_eq!(set.len(), 3);
    assert!(!set.is_empty());

    let matches = set.matches("foo123");
    assert!(matches.matched(0)); // \w+ matches
    assert!(matches.matched(1)); // \d+ matches
    assert!(!matches.matched(2)); // \s+ doesn't match
}

#[test]
fn test_regex_set_all_match() {
    let set = RegexSet::new(&[
        r"\w+",
        r"\d+",
        r"[a-z]+",
    ]).expect("valid patterns");

    let matches = set.matches("abc123");
    assert!(matches.matched(0)); // \w+ matches
    assert!(matches.matched(1)); // \d+ matches
    assert!(matches.matched(2)); // [a-z]+ matches
}

#[test]
fn test_regex_set_iteration() {
    let set = RegexSet::new(&[
        r"cat",
        r"dog",
        r"bird",
    ]).expect("valid patterns");

    let matches = set.matches("I have a dog and a bird");
    let matched_indices: Vec<_> = matches.iter().collect();
    assert_eq!(matched_indices, vec![1, 2]); // dog and bird
}

// ============================================================================
// Security Tests
// ============================================================================

#[test]
fn test_redos_prevention_with_config() {
    let config = RegexConfig::strict();
    assert!(config.timeout.is_some());
    assert!(config.size_limit < 10 * (1 << 20));
}

#[test]
fn test_size_limits() {
    let config = RegexConfig::strict();

    // This should work with strict config
    let simple = Regex::with_config(r"\d+", config.clone());
    assert!(simple.is_ok());
}

// ============================================================================
// Pattern Analysis Tests
// ============================================================================

#[test]
fn test_pattern_info() {
    let info = PatternInfo::analyze(r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")
        .expect("valid pattern");

    assert_eq!(info.pattern.as_str(), r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})");
    assert_eq!(info.capture_count, 4); // full match + 3 groups
    assert_eq!(info.named_groups.len(), 3);
    assert!(info.complexity > 0);
}

#[test]
fn test_pattern_complexity() {
    let simple = PatternInfo::analyze(r"\d+").expect("valid pattern");
    let complex = PatternInfo::analyze(r"(?:(?P<a>\d+)|(?P<b>[a-z]+))*(?=\w)").expect("valid pattern");

    assert!(complex.complexity > simple.complexity);
}

// ============================================================================
// Utility Function Tests
// ============================================================================

#[test]
fn test_escape() {
    let escaped = escape("a.b*c?d+e[f]g(h)i{j}k|l^m$n\\o");
    let re = Regex::new(&escaped).expect("valid pattern");

    assert!(re.is_match("a.b*c?d+e[f]g(h)i{j}k|l^m$n\\o"));
    assert!(!re.is_match("aXbXcXdXeXfXgXhXiXjXkXlXmXnXo"));
}

#[test]
fn test_validate() {
    assert!(validate(r"\d+").is_ok());
    assert!(validate(r"[a-z]+").is_ok());
    assert!(validate(r"(?P<name>\w+)").is_ok());

    assert!(validate(r"[unclosed").is_err());
    assert!(validate(r"(?P<name>)").is_ok()); // Empty group is valid
}

#[test]
fn test_convenience_functions() {
    assert!(is_match(r"\d+", "123").expect("valid").as_bool());
    assert!(!is_match(r"\d+", "abc").expect("valid").as_bool());

    let result = replace(r"\d+", "abc123def", "X").expect("valid");
    assert_eq!(result.as_str(), "abcXdef");

    let result = replace_all(r"\d+", "abc123def456", "X").expect("valid");
    assert_eq!(result.as_str(), "abcXdefX");

    let parts = split(r"\s+", "a b c").expect("valid");
    assert_eq!(parts.len(), 3);
}

// ============================================================================
// Cache Tests
// ============================================================================

#[test]
fn test_regex_cache() {
    let cache = RegexCache::new(10);

    let re1 = cache.get_or_compile(r"\d+").expect("valid pattern");
    assert_eq!(cache.len(), 1);

    let re2 = cache.get_or_compile(r"\d+").expect("valid pattern");
    assert_eq!(cache.len(), 1); // Still 1, retrieved from cache

    assert_eq!(re1.as_str(), re2.as_str());

    // Add more patterns
    for i in 0..15 {
        let pattern = format!(r"\d{{{}}}", i);
        cache.get_or_compile(&pattern).expect("valid pattern");
    }

    assert!(cache.len() <= 10); // Should not exceed capacity
}

#[test]
fn test_cache_clear() {
    let cache = RegexCache::new(10);

    cache.get_or_compile(r"\d+").expect("valid pattern");
    assert_eq!(cache.len(), 1);

    cache.clear();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

// ============================================================================
// Parallel Matching Tests
// ============================================================================

#[test]
fn test_parallel_match() {
    let pattern = r"\d+";
    let texts = vec!["abc".to_string(), "123".to_string(), "def".to_string(),
                     "456".to_string(), "ghi".to_string(), "789".to_string()];

    let results = parallel_match(pattern, &texts).expect("valid");

    assert_eq!(results.len(), 6);
    assert!(!results[0]); // "abc"
    assert!(results[1]);  // "123"
    assert!(!results[2]); // "def"
    assert!(results[3]);  // "456"
    assert!(!results[4]); // "ghi"
    assert!(results[5]);  // "789"
}

#[test]
fn test_parallel_find_all() {
    let pattern = r"\d+";
    let texts = vec!["a1b2".to_string(), "3c4d5".to_string(), "no digits".to_string()];

    let results = parallel_find_all(pattern, &texts).expect("valid");

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].len(), 2); // ["1", "2"]
    assert_eq!(results[1].len(), 3); // ["3", "4", "5"]
    assert_eq!(results[2].len(), 0); // []
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_empty_pattern() {
    let re = Regex::new("").expect("valid pattern");
    assert!(re.is_match(""));
    assert!(re.is_match("anything"));
}

#[test]
fn test_empty_text() {
    let re = Regex::new(r"\d+").expect("valid pattern");
    assert!(!re.is_match(""));

    let re_optional = Regex::new(r"\d*").expect("valid pattern");
    assert!(re_optional.is_match("")); // Matches empty
}

#[test]
fn test_very_long_match() {
    let re = Regex::new(r"a+").expect("valid pattern");
    let long_string = "a".repeat(10000);
    assert!(re.is_match(&long_string));

    let m = re.find(&long_string).expect("should match");
    assert_eq!(m.len(), 10000);
}

#[test]
fn test_many_captures() {
    let re = Regex::new(r"(\d)(\d)(\d)(\d)(\d)(\d)(\d)(\d)(\d)(\d)").expect("valid pattern");
    let caps = re.captures("1234567890").expect("should match");

    assert_eq!(caps.len(), 11); // full match + 10 groups
    for i in 1..=10 {
        let digit = caps.get(i).expect("group").as_str();
        assert_eq!(digit, format!("{}", i % 10));
    }
}

#[test]
fn test_overlapping_matches() {
    let re = Regex::new(r"\w\w").expect("valid pattern");
    let matches: Vec<_> = re.find_all("abcd").map(|m| m.as_str()).collect();

    // Non-overlapping matches
    assert_eq!(matches, vec!["ab", "cd"]);
}

// ============================================================================
// Real-world Pattern Tests
// ============================================================================

#[test]
fn test_email_pattern() {
    let re = Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("valid pattern");

    assert!(re.is_match("user@example.com"));
    assert!(re.is_match("first.last@subdomain.example.co.uk"));
    assert!(!re.is_match("invalid@"));
    assert!(!re.is_match("@invalid.com"));
}

#[test]
fn test_url_pattern() {
    let re = Regex::new(r"https?://[^\s]+").expect("valid pattern");

    assert!(re.is_match("http://example.com"));
    assert!(re.is_match("https://example.com/path?query=value"));
    assert!(!re.is_match("ftp://example.com"));
}

#[test]
fn test_phone_pattern() {
    let re = Regex::new(r"\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}")
        .expect("valid pattern");

    assert!(re.is_match("(123) 456-7890"));
    assert!(re.is_match("123-456-7890"));
    assert!(re.is_match("123.456.7890"));
    assert!(re.is_match("1234567890"));
}

#[test]
fn test_ipv4_pattern() {
    let re = Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")
        .expect("valid pattern");

    assert!(re.is_match("192.168.1.1"));
    assert!(re.is_match("10.0.0.1"));
    assert!(!re.is_match("256.1.1.1")); // Would need more validation for valid IPs
}

#[test]
fn test_hex_color_pattern() {
    let re = Regex::new(r"#[0-9a-fA-F]{6}\b").expect("valid pattern");

    assert!(re.is_match("#FF5733"));
    assert!(re.is_match("#000000"));
    assert!(re.is_match("#ffffff"));
    assert!(!re.is_match("#GGG"));
}

// ============================================================================
// Performance Edge Cases
// ============================================================================

#[test]
fn test_catastrophic_backtracking_protection() {
    // This pattern could cause catastrophic backtracking on some inputs
    let config = RegexConfig::strict();
    let re = Regex::with_config(r"(a+)+b", config).expect("valid pattern");

    // This should not hang due to timeout
    let result = re.is_match("aaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    // May match or timeout, but should not hang indefinitely
    let _ = result;
}

// Helper trait to convert Maybe to bool for tests
trait MaybeBool {
    fn as_bool(self) -> bool;
}

impl MaybeBool for VerumResult<bool, regex::Error> {
    fn as_bool(self) -> bool {
        match self {
            VerumResult::Ok(b) => b,
            VerumResult::Err(_) => false,
        }
    }
}
