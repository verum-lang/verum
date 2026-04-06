//! Result Extension Traits for Structured Contexts
//!
//! Provides ergonomic extension methods for adding structured context to Result types.
//! These traits enable zero-cost error context on the success path while providing
//! rich diagnostics on the error path.
//!
//! # Key Features
//!
//! - **Zero-cost on success** - Closures only execute on error
//! - **Type-safe** - Context values are type-checked
//! - **Composable** - Chain multiple structured contexts
//! - **Ergonomic** - Natural integration with `?` operator
//!
//! # Examples
//!
//! ```rust,ignore
//! use verum_error::prelude::*;
//!
//! fn fetch_user(id: u64) -> Result<User, ContextError<VerumError>> {
//!     database_query()
//!         .with_structured("user_id", id)
//!         .with_structured("operation", "fetch_user")
//!         .with_structured_fn(|| ("timestamp", current_timestamp()))?;
//!     Ok(user)
//! }
//! ```

use crate::context::ContextError;
use crate::structured_context::{ContextValue, ToContextValue};
use verum_common::{Map, Text};

/// Extension trait for adding structured context to Result types
///
/// Provides methods for adding single key-value pairs or maps of structured
/// data to errors. The structured data is preserved through error propagation
/// and can be formatted as JSON, YAML, or Logfmt.
pub trait ResultStructuredContext<T, E> {
    /// Add a single structured context key-value pair
    ///
    /// # Performance
    /// The key and value are always evaluated, even on success.
    /// Use `with_structured_fn()` for expensive computations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::prelude::*;
    ///
    /// fn example(user_id: u64) -> Result<(), ContextError<VerumError>> {
    ///     Err(VerumError::new("Error", ErrorKind::Other))
    ///         .with_structured("user_id", user_id)
    /// }
    /// ```
    fn with_structured<K, V>(self, key: K, value: V) -> Result<T, ContextError<E>>
    where
        K: Into<Text>,
        V: ToContextValue;

    /// Add multiple structured context key-value pairs from a map
    ///
    /// # Performance
    /// The map is always evaluated, even on success.
    /// Use `with_structured_map_fn()` for expensive map construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_common::{Map, Text};
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::prelude::*;
    ///
    /// fn example() -> Result<(), ContextError<VerumError>> {
    ///     let mut map: Map<Text, ContextValue> = Map::new();
    ///     map.insert("key1".to_string().into(), 42.to_context_value());
    ///     map.insert("key2".to_string().into(), "value".to_context_value());
    ///
    ///     Err(VerumError::new("Error", ErrorKind::Other))
    ///         .with_structured_map(map)
    /// }
    /// ```
    fn with_structured_map(self, map: Map<Text, ContextValue>) -> Result<T, ContextError<E>>;
}

/// Extension trait for lazy structured context evaluation
///
/// Provides zero-cost methods where closures are only evaluated on error.
/// This is the preferred method for expensive context value computations.
pub trait ResultStructuredContextFn<T, E> {
    /// Add a structured context using a closure (zero-cost on success)
    ///
    /// # Performance
    /// **Zero-cost on success path** - The closure is only called if there's an error.
    /// This is the preferred method for expensive context value creation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::prelude::*;
    ///
    /// fn expensive_computation() -> i64 {
    ///     // Simulated expensive operation
    ///     42
    /// }
    ///
    /// fn example() -> Result<(), ContextError<VerumError>> {
    ///     Err(VerumError::new("Error", ErrorKind::Other))
    ///         .with_structured_fn(|| ("computed_value", expensive_computation()))
    /// }
    /// ```
    fn with_structured_fn<F, K, V>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> (K, V),
        K: Into<Text>,
        V: ToContextValue;

    /// Add multiple structured contexts using a closure (zero-cost on success)
    ///
    /// # Performance
    /// **Zero-cost on success path** - The closure is only called if there's an error.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_common::{Map, Text};
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::prelude::*;
    ///
    /// fn build_context() -> Map<Text, ContextValue> {
    ///     let mut map: Map<Text, ContextValue> = Map::new();
    ///     map.insert("key1".to_string().into(), 42.to_context_value());
    ///     map
    /// }
    ///
    /// fn example() -> Result<(), ContextError<VerumError>> {
    ///     Err(VerumError::new("Error", ErrorKind::Other))
    ///         .with_structured_map_fn(|| build_context())
    /// }
    /// ```
    fn with_structured_map_fn<F>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> Map<Text, ContextValue>;
}

// Implementation for Result<T, E>

impl<T, E> ResultStructuredContext<T, E> for Result<T, E>
where
    E: Clone,
{
    fn with_structured<K, V>(self, key: K, value: V) -> Result<T, ContextError<E>>
    where
        K: Into<Text>,
        V: ToContextValue,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => Err(ContextError::new(e, "").add_structured(key, value)),
        }
    }

    fn with_structured_map(self, map: Map<Text, ContextValue>) -> Result<T, ContextError<E>> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => Err(ContextError::new(e, "").add_structured_map(map)),
        }
    }
}

impl<T, E> ResultStructuredContextFn<T, E> for Result<T, E>
where
    E: Clone,
{
    fn with_structured_fn<F, K, V>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> (K, V),
        K: Into<Text>,
        V: ToContextValue,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                let (key, value) = f();
                Err(ContextError::new(e, "").add_structured(key, value))
            }
        }
    }

    fn with_structured_map_fn<F>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> Map<Text, ContextValue>,
    {
        match self {
            Ok(v) => Ok(v),
            Err(e) => Err(ContextError::new(e, "").add_structured_map(f())),
        }
    }
}

/// Extension trait for adding structured context to ContextError results (for chaining)
///
/// This trait is automatically implemented for Result<T, ContextError<E>> and allows
/// chaining structured contexts on already-wrapped errors.
pub trait ContextErrorStructuredExt<T, E> {
    /// Add a structured context to an existing ContextError
    fn add_structured<K, V>(self, key: K, value: V) -> Result<T, ContextError<E>>
    where
        K: Into<Text>,
        V: ToContextValue;

    /// Add multiple structured contexts to an existing ContextError
    fn add_structured_map(self, map: Map<Text, ContextValue>) -> Result<T, ContextError<E>>;

    /// Add a structured context using a closure (zero-cost on success)
    fn add_structured_fn<F, K, V>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> (K, V),
        K: Into<Text>,
        V: ToContextValue;
}

impl<T, E> ContextErrorStructuredExt<T, E> for Result<T, ContextError<E>> {
    fn add_structured<K, V>(self, key: K, value: V) -> Result<T, ContextError<E>>
    where
        K: Into<Text>,
        V: ToContextValue,
    {
        self.map_err(|e| e.add_structured(key, value))
    }

    fn add_structured_map(self, map: Map<Text, ContextValue>) -> Result<T, ContextError<E>> {
        self.map_err(|e| e.add_structured_map(map))
    }

    fn add_structured_fn<F, K, V>(self, f: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> (K, V),
        K: Into<Text>,
        V: ToContextValue,
    {
        self.map_err(|e| {
            let (key, value) = f();
            e.add_structured(key, value)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorKind, VerumError};

    #[test]
    fn test_with_structured_basic() {
        let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result.with_structured("key", 42);

        assert!(ctx_result.is_err());
        let err = ctx_result.unwrap_err();
        assert_eq!(
            err.get_structured_context("key").unwrap().as_int(),
            Some(42)
        );
    }

    #[test]
    fn test_with_structured_string() {
        let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result.with_structured("message", "Hello");

        assert!(ctx_result.is_err());
        let err = ctx_result.unwrap_err();
        assert_eq!(
            err.get_structured_context("message").unwrap().as_text(),
            Some(&Text::from("Hello"))
        );
    }

    #[test]
    fn test_with_structured_map() {
        let mut map = Map::new();
        map.insert(Text::from("key1"), ContextValue::Int(42));
        map.insert(Text::from("key2"), ContextValue::Text(Text::from("value")));

        let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result.with_structured_map(map);

        assert!(ctx_result.is_err());
        let err = ctx_result.unwrap_err();
        assert_eq!(
            err.get_structured_context("key1").unwrap().as_int(),
            Some(42)
        );
        assert_eq!(
            err.get_structured_context("key2").unwrap().as_text(),
            Some(&Text::from("value"))
        );
    }

    #[test]
    fn test_with_structured_fn_zero_cost() {
        let mut call_count = 0;

        // Success path - closure should NOT be called
        let result: Result<i32, VerumError> = Ok(42);
        let ctx_result = result.with_structured_fn(|| {
            call_count += 1;
            ("key", 100)
        });

        assert!(ctx_result.is_ok());
        assert_eq!(call_count, 0, "Closure should not be called on success");

        // Error path - closure SHOULD be called
        let result: Result<i32, VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result.with_structured_fn(|| {
            call_count += 1;
            ("key", 100)
        });

        assert!(ctx_result.is_err());
        assert_eq!(call_count, 1, "Closure should be called on error");
        assert_eq!(
            ctx_result
                .unwrap_err()
                .get_structured_context("key")
                .unwrap()
                .as_int(),
            Some(100)
        );
    }

    #[test]
    fn test_with_structured_map_fn_zero_cost() {
        let mut call_count = 0;

        // Success path
        let result: Result<i32, VerumError> = Ok(42);
        let ctx_result = result.with_structured_map_fn(|| {
            call_count += 1;
            let mut map = Map::new();
            map.insert(Text::from("key"), ContextValue::Int(100));
            map
        });

        assert!(ctx_result.is_ok());
        assert_eq!(call_count, 0);

        // Error path
        let result: Result<i32, VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result.with_structured_map_fn(|| {
            call_count += 1;
            let mut map = Map::new();
            map.insert(Text::from("key"), ContextValue::Int(100));
            map
        });

        assert!(ctx_result.is_err());
        assert_eq!(call_count, 1);
    }

    #[test]
    fn test_chaining_structured_contexts() {
        let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result
            .with_structured("key1", 42)
            .add_structured("key2", "value")
            .add_structured("key3", true);

        assert!(ctx_result.is_err());
        let err = ctx_result.unwrap_err();
        assert_eq!(
            err.get_structured_context("key1").unwrap().as_int(),
            Some(42)
        );
        assert_eq!(
            err.get_structured_context("key2").unwrap().as_text(),
            Some(&Text::from("value"))
        );
        assert_eq!(
            err.get_structured_context("key3").unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_mixed_structured_and_lazy() {
        let result: Result<(), VerumError> = Err(VerumError::new("Error", ErrorKind::Other));
        let ctx_result = result
            .with_structured("static_key", 42)
            .add_structured_fn(|| ("lazy_key", 100));

        assert!(ctx_result.is_err());
        let err = ctx_result.unwrap_err();
        assert_eq!(
            err.get_structured_context("static_key").unwrap().as_int(),
            Some(42)
        );
        assert_eq!(
            err.get_structured_context("lazy_key").unwrap().as_int(),
            Some(100)
        );
    }

    #[test]
    fn test_success_path_performance() {
        // Ensure success path doesn't execute expensive operations
        fn expensive_operation() -> i64 {
            panic!("This should not be called on success path");
        }

        let result: Result<i32, VerumError> = Ok(42);
        let ctx_result = result.with_structured_fn(|| ("key", expensive_operation()));

        assert_eq!(ctx_result.unwrap(), 42);
    }
}
