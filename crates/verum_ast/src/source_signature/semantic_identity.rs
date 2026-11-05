//! Structural identity of a validated, body-free declaration. Source positions
//! are diagnostic data; every other serialized AST fact remains significant.
use super::validate_source_signature;
use crate::decl::FunctionDecl;
use serde::{Serialize, ser};
use verum_common::{List, Text};

// The existing signature validator bounds depth, nodes and text before either
// walk. Bound the encoded representation too; never allocate a JSON/AST copy.
const MAX_IDENTITY_BYTES: usize = 16 * 1024 * 1024;

/// Compare complete retained signatures independently of their source locations.
/// Both inputs must pass the ordinary body-free signature admission. This does
/// not prove equivalence of differently written expressions or ABI descriptors.
pub fn semantically_equal_signatures(
    left: &FunctionDecl,
    right: &FunctionDecl,
) -> Result<bool, Text> {
    validate_source_signature(left, left.is_method())?;
    validate_source_signature(right, right.is_method())?;
    let mut encoded = Output::new(None);
    left.serialize(&mut encoded).map_err(|error| error.0)?;
    let mut compared = Output::new(Some(&encoded.bytes));
    right.serialize(&mut compared).map_err(|error| error.0)?;
    Ok(compared.equal && compared.position == encoded.bytes.len())
}

#[derive(Debug)]
struct Error(Text);
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for Error {}
impl ser::Error for Error {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string().into())
    }
}
type ResultOf<T> = Result<T, Error>;

struct Output<'a> {
    bytes: List<u8>,
    expected: Option<&'a [u8]>,
    position: usize,
    equal: bool,
}
impl<'a> Output<'a> {
    fn new(expected: Option<&'a [u8]>) -> Self {
        Self {
            bytes: List::new(),
            expected,
            position: 0,
            equal: true,
        }
    }
    fn write(&mut self, bytes: &[u8]) -> ResultOf<()> {
        let end = self
            .position
            .checked_add(bytes.len())
            .filter(|end| *end <= MAX_IDENTITY_BYTES)
            .ok_or_else(|| Error("source signature identity exceeds its byte budget".into()))?;
        if let Some(expected) = self.expected {
            self.equal &= expected.get(self.position..end) == Some(bytes);
        } else {
            self.bytes.extend_from_slice(bytes);
        }
        self.position = end;
        Ok(())
    }
    fn tag(&mut self, tag: u8) -> ResultOf<()> {
        self.write(&[tag])
    }
    fn text(&mut self, value: &str) -> ResultOf<()> {
        self.write(&(value.len() as u64).to_le_bytes())?;
        self.write(value.as_bytes())
    }
    fn named(&mut self, tag: u8, name: &str) -> ResultOf<()> {
        self.tag(tag)?;
        self.text(name)
    }
    fn variant(&mut self, tag: u8, name: &str, index: u32, variant: &str) -> ResultOf<()> {
        self.named(tag, name)?;
        self.write(&index.to_le_bytes())?;
        self.text(variant)
    }
}
struct Compound<'a, 'b> {
    output: &'a mut Output<'b>,
    location: bool,
}

macro_rules! integer {
    ($method:ident, $ty:ty, $tag:expr) => {
        fn $method(self, value: $ty) -> ResultOf<()> {
            self.tag($tag)?;
            self.write(&value.to_le_bytes())
        }
    };
}
impl<'a, 'b> ser::Serializer for &'a mut Output<'b> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'a, 'b>;
    type SerializeTuple = Compound<'a, 'b>;
    type SerializeTupleStruct = Compound<'a, 'b>;
    type SerializeTupleVariant = Compound<'a, 'b>;
    type SerializeMap = Compound<'a, 'b>;
    type SerializeStruct = Compound<'a, 'b>;
    type SerializeStructVariant = Compound<'a, 'b>;
    fn serialize_bool(self, value: bool) -> ResultOf<()> {
        self.write(&[1, u8::from(value)])
    }
    integer!(serialize_i8, i8, 2);
    integer!(serialize_i16, i16, 3);
    integer!(serialize_i32, i32, 4);
    integer!(serialize_i64, i64, 5);
    integer!(serialize_i128, i128, 6);
    integer!(serialize_u8, u8, 7);
    integer!(serialize_u16, u16, 8);
    integer!(serialize_u32, u32, 9);
    integer!(serialize_u64, u64, 10);
    integer!(serialize_u128, u128, 11);
    fn serialize_f32(self, value: f32) -> ResultOf<()> {
        self.tag(12)?;
        self.write(&value.to_bits().to_le_bytes())
    }
    fn serialize_f64(self, value: f64) -> ResultOf<()> {
        self.tag(13)?;
        self.write(&value.to_bits().to_le_bytes())
    }
    fn serialize_char(self, value: char) -> ResultOf<()> {
        self.tag(14)?;
        self.write(&(value as u32).to_le_bytes())
    }
    fn serialize_str(self, value: &str) -> ResultOf<()> {
        self.named(15, value)
    }
    fn serialize_bytes(self, value: &[u8]) -> ResultOf<()> {
        self.tag(16)?;
        self.write(&(value.len() as u64).to_le_bytes())?;
        self.write(value)
    }
    fn serialize_none(self) -> ResultOf<()> {
        self.tag(17)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> ResultOf<()> {
        self.tag(18)?;
        value.serialize(self)
    }
    fn serialize_unit(self) -> ResultOf<()> {
        self.tag(19)
    }
    fn serialize_unit_struct(self, name: &'static str) -> ResultOf<()> {
        self.named(20, name)
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> ResultOf<()> {
        self.variant(21, name, index, variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> ResultOf<()> {
        // Match the typed serializer node, never a user field/key spelling.
        if name == "FileId" {
            return self.tag(22);
        }
        self.named(23, name)?;
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> ResultOf<()> {
        self.variant(24, name, index, variant)?;
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> ResultOf<Self::SerializeSeq> {
        self.tag(25)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
    fn serialize_tuple(self, _: usize) -> ResultOf<Self::SerializeTuple> {
        self.tag(26)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
    fn serialize_tuple_struct(
        self,
        name: &'static str,
        _: usize,
    ) -> ResultOf<Self::SerializeTupleStruct> {
        self.named(27, name)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        _: usize,
    ) -> ResultOf<Self::SerializeTupleVariant> {
        self.variant(28, name, index, variant)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
    fn serialize_map(self, _: Option<usize>) -> ResultOf<Self::SerializeMap> {
        self.tag(29)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
    fn serialize_struct(self, name: &'static str, _: usize) -> ResultOf<Self::SerializeStruct> {
        let location = name == "Span";
        self.named(30, name)?;
        Ok(Compound {
            output: self,
            location,
        })
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        _: usize,
    ) -> ResultOf<Self::SerializeStructVariant> {
        self.variant(31, name, index, variant)?;
        Ok(Compound {
            output: self,
            location: false,
        })
    }
}
macro_rules! compound {
    ($trait:ident, $method:ident) => {
        impl ser::$trait for Compound<'_, '_> {
            type Ok = ();
            type Error = Error;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> ResultOf<()> {
                value.serialize(&mut *self.output)
            }
            fn end(self) -> ResultOf<()> {
                self.output.tag(32)
            }
        }
    };
}
compound!(SerializeSeq, serialize_element);
compound!(SerializeTuple, serialize_element);
compound!(SerializeTupleStruct, serialize_field);
compound!(SerializeTupleVariant, serialize_field);
impl ser::SerializeMap for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, value: &T) -> ResultOf<()> {
        value.serialize(&mut *self.output)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> ResultOf<()> {
        value.serialize(&mut *self.output)
    }
    fn end(self) -> ResultOf<()> {
        self.output.tag(32)
    }
}
impl ser::SerializeStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> ResultOf<()> {
        if self.location {
            return Ok(());
        }
        self.output.text(name)?;
        value.serialize(&mut *self.output)
    }
    fn end(self) -> ResultOf<()> {
        self.output.tag(32)
    }
}
impl ser::SerializeStructVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> ResultOf<()> {
        self.output.text(name)?;
        value.serialize(&mut *self.output)
    }
    fn end(self) -> ResultOf<()> {
        self.output.tag(32)
    }
}
