//! ToText trait for converting values to Text
//!
//! This provides a convenient way to convert various types to Text.

use crate::Text;

/// Trait for converting values to Text.
///
/// This trait provides a consistent way to convert types to Text strings.
/// It is similar to `ToString` but returns `Text` instead of `String`.
///
/// # Examples
///
/// ```
/// use verum_common::{Text, ToText};
///
/// let s = "hello";
/// let text: Text = s.to_text();
/// assert_eq!(text.as_str(), "hello");
/// ```
pub trait ToText {
    /// Converts this value to a Text.
    fn to_text(&self) -> Text;
}

// Implementation for &str
impl ToText for str {
    fn to_text(&self) -> Text {
        Text::from(self)
    }
}

// Implementation for String
impl ToText for String {
    fn to_text(&self) -> Text {
        Text::from(self.clone())
    }
}

// Implementation for Text (identity)
impl ToText for Text {
    fn to_text(&self) -> Text {
        self.clone()
    }
}

// Implementations for common primitive types
impl ToText for bool {
    fn to_text(&self) -> Text {
        Text::from(if *self { "true" } else { "false" })
    }
}

impl ToText for char {
    fn to_text(&self) -> Text {
        let mut s = String::with_capacity(4);
        s.push(*self);
        Text::from(s)
    }
}

macro_rules! impl_to_text_for_numeric {
    ($($t:ty),*) => {
        $(
            impl ToText for $t {
                fn to_text(&self) -> Text {
                    Text::from(format!("{}", self))
                }
            }
        )*
    };
}

impl_to_text_for_numeric!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

// Blanket implementation for references
impl<T: ToText + ?Sized> ToText for &T {
    fn to_text(&self) -> Text {
        (*self).to_text()
    }
}

impl<T: ToText + ?Sized> ToText for &mut T {
    fn to_text(&self) -> Text {
        (**self).to_text()
    }
}

impl<T: ToText> ToText for Box<T> {
    fn to_text(&self) -> Text {
        (**self).to_text()
    }
}
