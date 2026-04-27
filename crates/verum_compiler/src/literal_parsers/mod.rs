//! Built-in compile-time parsers for tagged literals
//!
//! Compile-time literal protocol: meta functions registered via
//! @tagged_literal/@interpolation_handler for compile-time parsing.
//!
//! This module provides compile-time parsing and validation for:
//! - DateTime literals (d#"2024-01-15T10:30:00Z")
//! - Duration literals (duration#"1h30m")
//! - Regex literals (rx#"[a-z]+")
//! - Interval literals (interval#"[0, 100)")
//! - Matrix literals (mat#"[[1, 2], [3, 4]]")
//! - URI literals (url#"https://example.com")
//! - Email literals (email#"user@example.com")
//! - UUID literals (uuid#"550e8400-e29b-41d4-a716-446655440000")
//! - JSON literals (json#"{ ... }")
//! - XML literals (xml#"<root>...</root>")
//! - YAML literals (yaml#"key: value")

pub mod datetime;
pub mod duration;
pub mod email;
pub mod interval;
pub mod json;
pub mod matrix;
pub mod regex;
pub mod sql;
pub mod uri;
pub mod uuid;
pub mod xml;
pub mod yaml;

// Re-export parser functions
pub use datetime::parse_datetime;
pub use duration::parse_duration;
pub use email::parse_email;
pub use interval::parse_interval;
pub use json::parse_json;
pub use matrix::parse_matrix;
pub use regex::parse_regex;
pub use sql::{parse_sql, SqlDialect};
pub use uri::parse_uri;
pub use uuid::parse_uuid;
pub use xml::parse_xml;
pub use yaml::parse_yaml;
