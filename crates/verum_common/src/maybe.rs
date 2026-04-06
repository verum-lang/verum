//! Maybe type - Optional values
//!
//! Verum semantic type: Maybe<T> is the optional value type (equivalent to Option<T>).

use std::fmt;

/// An optional value that can be either `Some(T)` or `None`.
///
/// This is the semantic equivalent of Option in other languages.
///
/// # Examples
///
/// ```
/// use verum_common::Maybe;
///
/// let some_value: Maybe<i32> = Maybe::Some(42);
/// let no_value: Maybe<i32> = Maybe::None;
///
/// assert_eq!(some_value.unwrap(), 42);
/// assert!(no_value.is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Maybe<T> {
    /// No value
    None,
    /// Some value of type T
    Some(T),
}

impl<T> Maybe<T> {
    /// Returns `true` if the maybe is a `Some` value.
    #[inline]
    pub fn is_some(&self) -> bool {
        matches!(self, Maybe::Some(_))
    }

    /// Returns `true` if the maybe is a `None` value.
    #[inline]
    pub fn is_none(&self) -> bool {
        matches!(self, Maybe::None)
    }

    /// Unwraps the maybe, returning the contained value.
    ///
    /// # Panics
    ///
    /// Panics if the value is `None`.
    #[inline]
    pub fn unwrap(self) -> T {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => panic!("called `Maybe::unwrap()` on a `None` value"),
        }
    }

    /// Returns the contained value or a provided default.
    #[inline]
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => default,
        }
    }

    /// Returns the contained value or computes it from a closure.
    #[inline]
    pub fn unwrap_or_else<F>(self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => f(),
        }
    }

    /// Maps a `Maybe<T>` to `Maybe<U>` by applying a function to the contained value.
    #[inline]
    pub fn map<U, F>(self, f: F) -> Maybe<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Maybe::Some(val) => Maybe::Some(f(val)),
            Maybe::None => Maybe::None,
        }
    }

    /// Returns `None` if the maybe is `None`, otherwise calls `f` with the wrapped value.
    #[inline]
    pub fn and_then<U, F>(self, f: F) -> Maybe<U>
    where
        F: FnOnce(T) -> Maybe<U>,
    {
        match self {
            Maybe::Some(val) => f(val),
            Maybe::None => Maybe::None,
        }
    }

    /// Returns the maybe if it contains a value satisfying the predicate, otherwise returns `None`.
    #[inline]
    pub fn filter<P>(self, predicate: P) -> Maybe<T>
    where
        P: FnOnce(&T) -> bool,
    {
        match self {
            Maybe::Some(ref val) if predicate(val) => self,
            _ => Maybe::None,
        }
    }

    /// Transforms the `Maybe<T>` into a `Result<T, E>`, mapping `Some(v)` to `Ok(v)` and `None` to `Err(err)`.
    #[inline]
    pub fn ok_or<E>(self, err: E) -> crate::result::Result<T, E> {
        match self {
            Maybe::Some(val) => crate::result::Result::Ok(val),
            Maybe::None => crate::result::Result::Err(err),
        }
    }

    /// Transforms the `Maybe<T>` into a `Result<T, E>`, mapping `Some(v)` to `Ok(v)` and `None` to `Err(err())`.
    #[inline]
    pub fn ok_or_else<E, F>(self, err: F) -> crate::result::Result<T, E>
    where
        F: FnOnce() -> E,
    {
        match self {
            Maybe::Some(val) => crate::result::Result::Ok(val),
            Maybe::None => crate::result::Result::Err(err()),
        }
    }

    /// Returns the maybe if it contains a value, otherwise returns `other`.
    #[inline]
    pub fn or(self, other: Maybe<T>) -> Maybe<T> {
        match self {
            Maybe::Some(_) => self,
            Maybe::None => other,
        }
    }

    /// Returns the maybe if it contains a value, otherwise calls `f` and returns the result.
    #[inline]
    pub fn or_else<F>(self, f: F) -> Maybe<T>
    where
        F: FnOnce() -> Maybe<T>,
    {
        match self {
            Maybe::Some(_) => self,
            Maybe::None => f(),
        }
    }

    /// Returns `Some` if exactly one of `self`, `other` is `Some`, otherwise returns `None`.
    #[inline]
    pub fn xor(self, other: Maybe<T>) -> Maybe<T> {
        match (self, other) {
            (Maybe::Some(val), Maybe::None) | (Maybe::None, Maybe::Some(val)) => Maybe::Some(val),
            _ => Maybe::None,
        }
    }

    /// Converts from `&Maybe<T>` to `Maybe<&T>`.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x: Maybe<u32> = Maybe::Some(2);
    /// assert_eq!(x.as_ref(), Maybe::Some(&2));
    ///
    /// let x: Maybe<u32> = Maybe::None;
    /// assert_eq!(x.as_ref(), Maybe::None);
    /// ```
    #[inline]
    pub fn as_ref(&self) -> Maybe<&T> {
        match self {
            Maybe::Some(x) => Maybe::Some(x),
            Maybe::None => Maybe::None,
        }
    }

    /// Converts from `&mut Maybe<T>` to `Maybe<&mut T>`.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let mut x: Maybe<u32> = Maybe::Some(2);
    /// match x.as_mut() {
    ///     Maybe::Some(v) => *v = 42,
    ///     Maybe::None => {},
    /// }
    /// assert_eq!(x, Maybe::Some(42));
    /// ```
    #[inline]
    pub fn as_mut(&mut self) -> Maybe<&mut T> {
        match self {
            Maybe::Some(x) => Maybe::Some(x),
            Maybe::None => Maybe::None,
        }
    }

    /// Takes the value out of the maybe, leaving a `None` in its place.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let mut x: Maybe<u32> = Maybe::Some(2);
    /// let y = x.take();
    /// assert_eq!(x, Maybe::None);
    /// assert_eq!(y, Maybe::Some(2));
    /// ```
    #[inline]
    pub fn take(&mut self) -> Maybe<T> {
        std::mem::replace(self, Maybe::None)
    }

    /// Replaces the actual value in the maybe by the value given in parameter,
    /// returning the old value if present,
    /// leaving a `Some` in its place without deinitializing either one.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let mut x: Maybe<u32> = Maybe::Some(2);
    /// let old = x.replace(5);
    /// assert_eq!(x, Maybe::Some(5));
    /// assert_eq!(old, Maybe::Some(2));
    /// ```
    #[inline]
    pub fn replace(&mut self, value: T) -> Maybe<T> {
        std::mem::replace(self, Maybe::Some(value))
    }

    /// Returns an iterator over the possibly contained value.
    #[inline]
    pub fn iter(&self) -> MaybeIter<&T> {
        MaybeIter {
            inner: self.as_ref(),
        }
    }

    /// Returns a mutable iterator over the possibly contained value.
    #[inline]
    pub fn iter_mut(&mut self) -> MaybeIter<&mut T> {
        MaybeIter {
            inner: self.as_mut(),
        }
    }

    /// Returns the contained value or a default.
    ///
    /// Applies the function `f` to the contained value (if any),
    /// or returns the provided default (if not).
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x = Maybe::Some("foo");
    /// assert_eq!(x.map_or(42, |v| v.len()), 3);
    ///
    /// let x: Maybe<&str> = Maybe::None;
    /// assert_eq!(x.map_or(42, |v| v.len()), 42);
    /// ```
    #[inline]
    pub fn map_or<U, F>(self, default: U, f: F) -> U
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Maybe::Some(t) => f(t),
            Maybe::None => default,
        }
    }

    /// Maps a `Maybe<T>` to `U` by applying a function to a contained value,
    /// or computes a default (if not).
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let k = 21;
    /// let x = Maybe::Some("foo");
    /// assert_eq!(x.map_or_else(|| 2 * k, |v| v.len()), 3);
    ///
    /// let x: Maybe<&str> = Maybe::None;
    /// assert_eq!(x.map_or_else(|| 2 * k, |v| v.len()), 42);
    /// ```
    #[inline]
    pub fn map_or_else<U, D, F>(self, default: D, f: F) -> U
    where
        D: FnOnce() -> U,
        F: FnOnce(T) -> U,
    {
        match self {
            Maybe::Some(t) => f(t),
            Maybe::None => default(),
        }
    }

    /// Flattens a `Maybe<Maybe<T>>` into a `Maybe<T>`.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x: Maybe<Maybe<i32>> = Maybe::Some(Maybe::Some(6));
    /// assert_eq!(Maybe::Some(6), x.flatten());
    ///
    /// let x: Maybe<Maybe<i32>> = Maybe::Some(Maybe::None);
    /// assert_eq!(Maybe::None, x.flatten());
    ///
    /// let x: Maybe<Maybe<i32>> = Maybe::None;
    /// assert_eq!(Maybe::None, x.flatten());
    /// ```
    #[inline]
    pub fn flatten(self) -> Maybe<T>
    where
        T: Into<Maybe<T>>,
    {
        match self {
            Maybe::Some(inner) => inner.into(),
            Maybe::None => Maybe::None,
        }
    }

    /// Converts from `Maybe<T>` to `Option<T>`.
    ///
    /// This is useful for interop with standard library code that expects `Option`.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x: Maybe<u32> = Maybe::Some(2);
    /// assert_eq!(x.to_option(), Some(2));
    ///
    /// let x: Maybe<u32> = Maybe::None;
    /// assert_eq!(x.to_option(), None);
    /// ```
    #[inline]
    pub fn to_option(self) -> Option<T> {
        match self {
            Maybe::Some(val) => Some(val),
            Maybe::None => None,
        }
    }

    /// Converts from `Option<T>` to `Maybe<T>`.
    ///
    /// This is useful for converting from standard library Option types.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x: Option<u32> = Some(2);
    /// assert_eq!(Maybe::from_option(x), Maybe::Some(2));
    ///
    /// let x: Option<u32> = None;
    /// assert_eq!(Maybe::from_option(x), Maybe::None);
    /// ```
    #[inline]
    pub fn from_option(opt: Option<T>) -> Maybe<T> {
        match opt {
            Some(val) => Maybe::Some(val),
            None => Maybe::None,
        }
    }

    /// Transforms the `Maybe<T>` into a `std::result::Result<T, E>`, mapping `Some(v)` to `Ok(v)` and `None` to `Err(err)`.
    ///
    /// This is the std Result version, useful when working with async code or stdlib.
    #[inline]
    pub fn ok_or_std<E>(self, err: E) -> std::result::Result<T, E> {
        match self {
            Maybe::Some(val) => Ok(val),
            Maybe::None => Err(err),
        }
    }

    /// Transforms the `Maybe<T>` into a `std::result::Result<T, E>`, mapping `Some(v)` to `Ok(v)` and `None` to `Err(err())`.
    ///
    /// This is the std Result version, useful when working with async code or stdlib.
    #[inline]
    pub fn ok_or_else_std<E, F>(self, err: F) -> std::result::Result<T, E>
    where
        F: FnOnce() -> E,
    {
        match self {
            Maybe::Some(val) => Ok(val),
            Maybe::None => Err(err()),
        }
    }

    // ADDITIONAL MISSING METHODS (15 methods from audit)

    /// Converts from `&Maybe<T>` to `Maybe<&T::Target>`.
    ///
    /// Coerces the Maybe through Deref and returns it.
    #[inline]
    pub fn as_deref(&self) -> Maybe<&T::Target>
    where
        T: std::ops::Deref,
    {
        match self {
            Maybe::Some(t) => Maybe::Some(t.deref()),
            Maybe::None => Maybe::None,
        }
    }

    /// Converts from `&mut Maybe<T>` to `Maybe<&mut T::Target>`.
    ///
    /// Coerces the Maybe through DerefMut and returns it.
    #[inline]
    pub fn as_deref_mut(&mut self) -> Maybe<&mut T::Target>
    where
        T: std::ops::DerefMut,
    {
        match self {
            Maybe::Some(t) => Maybe::Some(t.deref_mut()),
            Maybe::None => Maybe::None,
        }
    }

    /// Converts from `Maybe<T>` to `Maybe<Pin<&T>>`.
    #[inline]
    pub fn as_pin_ref(self: std::pin::Pin<&Self>) -> Maybe<std::pin::Pin<&T>> {
        unsafe {
            match *std::pin::Pin::get_ref(self) {
                Maybe::Some(ref x) => Maybe::Some(std::pin::Pin::new_unchecked(x)),
                Maybe::None => Maybe::None,
            }
        }
    }

    /// Converts from `Maybe<T>` to `Maybe<Pin<&mut T>>`.
    #[inline]
    pub fn as_pin_mut(self: std::pin::Pin<&mut Self>) -> Maybe<std::pin::Pin<&mut T>> {
        unsafe {
            match *std::pin::Pin::get_unchecked_mut(self) {
                Maybe::Some(ref mut x) => Maybe::Some(std::pin::Pin::new_unchecked(x)),
                Maybe::None => Maybe::None,
            }
        }
    }

    /// Returns the contained value with a custom panic message.
    ///
    /// # Panics
    ///
    /// Panics if the value is `None` with a custom message.
    #[inline]
    pub fn expect(self, msg: &str) -> T {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => panic!("{}", msg),
        }
    }

    /// Inserts value into the maybe, then returns a mutable reference to it.
    ///
    /// If the maybe already contains a value, the old value is dropped.
    #[inline]
    pub fn insert(&mut self, value: T) -> &mut T {
        *self = Maybe::Some(value);
        match self {
            Maybe::Some(ref mut v) => v,
            Maybe::None => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    /// Inserts value into the maybe if it is None, then returns a mutable reference.
    #[inline]
    pub fn get_or_insert(&mut self, value: T) -> &mut T {
        if self.is_none() {
            *self = Maybe::Some(value);
        }
        match self {
            Maybe::Some(ref mut v) => v,
            Maybe::None => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    /// Inserts default value if None, then returns a mutable reference.
    #[inline]
    pub fn get_or_insert_default(&mut self) -> &mut T
    where
        T: Default,
    {
        self.get_or_insert_with(Default::default)
    }

    /// Inserts value computed from function if None, then returns mutable reference.
    #[inline]
    pub fn get_or_insert_with<F>(&mut self, f: F) -> &mut T
    where
        F: FnOnce() -> T,
    {
        if self.is_none() {
            *self = Maybe::Some(f());
        }
        match self {
            Maybe::Some(ref mut v) => v,
            Maybe::None => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    /// Returns true if the maybe is Some and the value satisfies the predicate.
    #[inline]
    pub fn is_some_and<F>(self, f: F) -> bool
    where
        F: FnOnce(T) -> bool,
    {
        match self {
            Maybe::Some(x) => f(x),
            Maybe::None => false,
        }
    }

    /// Transposes a Maybe of a Result into a Result of a Maybe.
    #[inline]
    pub fn transpose<E>(self) -> crate::result::Result<Maybe<T>, E>
    where
        T: Into<crate::result::Result<T, E>>,
    {
        match self {
            Maybe::Some(x) => match x.into() {
                crate::result::Result::Ok(v) => crate::result::Result::Ok(Maybe::Some(v)),
                crate::result::Result::Err(e) => crate::result::Result::Err(e),
            },
            Maybe::None => crate::result::Result::Ok(Maybe::None),
        }
    }

    /// Returns the contained value without checking (unsafe).
    ///
    /// # Safety
    ///
    /// Calling this method on None is undefined behavior.
    #[inline]
    pub unsafe fn unwrap_unchecked(self) -> T {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => std::hint::unreachable_unchecked(),
        }
    }

    /// Returns the contained value or a default.
    #[inline]
    pub fn unwrap_or_default(self) -> T
    where
        T: Default,
    {
        match self {
            Maybe::Some(val) => val,
            Maybe::None => Default::default(),
        }
    }

    /// Zips self with another Maybe.
    #[inline]
    pub fn zip<U>(self, other: Maybe<U>) -> Maybe<(T, U)> {
        match (self, other) {
            (Maybe::Some(a), Maybe::Some(b)) => Maybe::Some((a, b)),
            _ => Maybe::None,
        }
    }

    /// Zips self and another Maybe with function.
    #[inline]
    pub fn zip_with<U, R, F>(self, other: Maybe<U>, f: F) -> Maybe<R>
    where
        F: FnOnce(T, U) -> R,
    {
        match (self, other) {
            (Maybe::Some(a), Maybe::Some(b)) => Maybe::Some(f(a, b)),
            _ => Maybe::None,
        }
    }
}

impl<T: Copy> Maybe<&T> {
    /// Maps a `Maybe<&T>` to a `Maybe<T>` by copying the contents of the maybe.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x = 12;
    /// let opt_x = Maybe::Some(&x);
    /// assert_eq!(opt_x, Maybe::Some(&12));
    /// let copied = opt_x.copied();
    /// assert_eq!(copied, Maybe::Some(12));
    /// ```
    #[inline]
    pub fn copied(self) -> Maybe<T> {
        self.map(|&t| t)
    }
}

impl<T: Clone> Maybe<&T> {
    /// Maps a `Maybe<&T>` to a `Maybe<T>` by cloning the contents of the maybe.
    ///
    /// # Examples
    /// ```
    /// use verum_common::Maybe;
    ///
    /// let x = 12;
    /// let opt_x = Maybe::Some(&x);
    /// assert_eq!(opt_x, Maybe::Some(&12));
    /// let cloned = opt_x.cloned();
    /// assert_eq!(cloned, Maybe::Some(12));
    /// ```
    #[inline]
    pub fn cloned(self) -> Maybe<T> {
        self.map(|t| t.clone())
    }
}

impl<T: Copy> Maybe<&mut T> {
    /// Maps a `Maybe<&mut T>` to a `Maybe<T>` by copying the contents of the maybe.
    #[inline]
    pub fn copied(self) -> Maybe<T> {
        self.map(|&mut t| t)
    }
}

impl<T: Clone> Maybe<&mut T> {
    /// Maps a `Maybe<&mut T>` to a `Maybe<T>` by cloning the contents of the maybe.
    #[inline]
    pub fn cloned(self) -> Maybe<T> {
        self.map(|t| t.clone())
    }
}

/// An iterator over a reference to the [`Maybe::Some`] variant of a [`Maybe`].
///
/// This `struct` is created by the [`Maybe::iter`] method.
pub struct MaybeIter<T> {
    inner: Maybe<T>,
}

impl<T> Iterator for MaybeIter<T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.inner.take().into()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.inner {
            Maybe::Some(_) => (1, Some(1)),
            Maybe::None => (0, Some(0)),
        }
    }
}

impl<T> std::iter::FusedIterator for MaybeIter<T> {}

impl<T> Default for Maybe<T> {
    #[inline]
    fn default() -> Self {
        Maybe::None
    }
}

/// Convert from Rust's Option to Verum's Maybe
impl<T> From<Option<T>> for Maybe<T> {
    #[inline]
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(val) => Maybe::Some(val),
            None => Maybe::None,
        }
    }
}

/// Convert from Verum's Maybe to Rust's Option
impl<T> From<Maybe<T>> for Option<T> {
    #[inline]
    fn from(maybe: Maybe<T>) -> Self {
        match maybe {
            Maybe::Some(val) => Some(val),
            Maybe::None => None,
        }
    }
}

/// Implementation of Try trait for Maybe to enable the ? operator
impl<T> std::ops::Try for Maybe<T> {
    type Output = T;
    type Residual = Maybe<std::convert::Infallible>;

    fn from_output(output: Self::Output) -> Self {
        Maybe::Some(output)
    }

    fn branch(self) -> std::ops::ControlFlow<Self::Residual, Self::Output> {
        match self {
            Maybe::Some(v) => std::ops::ControlFlow::Continue(v),
            Maybe::None => std::ops::ControlFlow::Break(Maybe::None),
        }
    }
}

impl<T> std::ops::FromResidual for Maybe<T> {
    fn from_residual(residual: Maybe<std::convert::Infallible>) -> Self {
        match residual {
            Maybe::None => Maybe::None,
            Maybe::Some(_) => unreachable!(),
        }
    }
}

impl<T, E> std::ops::FromResidual<crate::result::Result<std::convert::Infallible, E>> for Maybe<T> {
    fn from_residual(_: crate::result::Result<std::convert::Infallible, E>) -> Self {
        Maybe::None
    }
}
