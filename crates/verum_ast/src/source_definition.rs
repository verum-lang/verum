//! Deterministic structural encoding of an exact callable input. This is an
//! identity witness, never proof of type safety or semantic equivalence.
//!
//! Derived steps enqueue every AST field by reference. One explicit work list
//! visits those fields without recursion, AST copies or process-local IDs.
use crate::{FunctionDecl, Item};
use verum_common::{List, Text};

/// Change when the structural encoding or its location policy changes.
pub const VERSION: u16 = 1;
/// Explicit whole-definition resource limits; no nesting-depth restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub nodes: usize,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: 1_048_576,
            bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NodeLimit { limit: usize },
    ByteLimit { limit: usize },
    NotAnImplementation,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeLimit { limit } => write!(
                f,
                "source definition exceeds its {limit}-node resource limit"
            ),
            Self::ByteLimit { limit } => write!(
                f,
                "source definition exceeds its {limit}-byte resource limit"
            ),
            Self::NotAnImplementation => {
                f.write_str("implementation identity requires an implementation declaration")
            }
        }
    }
}
impl std::error::Error for Error {}
pub(crate) type Result<T> = std::result::Result<T, Error>;

/// Exact lexical import inputs selected by the declaring source producer.
/// Named entries and targets must be in canonical lexical order.
#[derive(Debug, Clone, PartialEq, Eq, verum_ast_derive::DefinitionNode)]
pub struct LexicalScope {
    pub owner: Text,
    pub known: bool,
    pub named: List<(Text, Option<List<Text>>)>,
    pub globs: List<Text>,
}

#[derive(verum_ast_derive::DefinitionNode)]
struct Input<'a> {
    owner: &'a str,
    lexical_scopes: &'a [LexicalScope],
    lexical_parent_definition: Option<&'a [u8]>,
    implementation_definition: Option<&'a [u8]>,
    function: &'a FunctionDecl,
}

/// Feed an unambiguous, versioned encoding to the caller's digest or recorder.
/// Locations are diagnostic only; target-selection sidecars and every other
/// AST field remain significant. Missing metadata cannot fabricate this input.
pub fn encode(
    function: &FunctionDecl,
    owner: &str,
    lexical_parent_definition: Option<&[u8]>,
    implementation_definition: Option<&[u8]>,
    lexical_scopes: &[LexicalScope],
    limits: Limits,
    sink: &mut impl FnMut(&[u8]),
) -> std::result::Result<(), Error> {
    let input = Input {
        owner,
        lexical_scopes,
        lexical_parent_definition,
        implementation_definition,
        function,
    };
    encode_input(&input, b"callable", limits, sink)
}

#[derive(verum_ast_derive::DefinitionNode)]
struct ImplementationInput<'a> {
    owner: &'a str,
    lexical_scopes: &'a [LexicalScope],
    item: &'a Item,
}

/// Declaration selection key, never authority for executable-body reuse.
/// The complete implementation encoding below remains the definition witness.
#[derive(verum_ast_derive::DefinitionNode)]
struct ImplementationScope<'a> {
    owner: &'a str,
    attributes: &'a [crate::Attribute],
    is_unsafe: bool,
    generics: &'a [crate::ty::GenericParam],
    kind: &'a crate::decl::ImplKind,
    generic_where_clause: &'a Option<crate::ty::WhereClause>,
    meta_where_clause: &'a Option<crate::ty::WhereClause>,
    specialize_attr: &'a Option<crate::attr::SpecializeAttr>,
}

/// The source selector distinguishes independent protocol implementations that
/// own same-spelled methods. It cannot establish that their bodies are equal.
pub fn encode_implementation_scope(
    item: &Item,
    owner: &str,
    limits: Limits,
    sink: &mut impl FnMut(&[u8]),
) -> std::result::Result<(), Error> {
    let crate::Item {
        kind,
        attributes,
        span: _,
    } = item;
    let crate::ItemKind::Impl(declaration) = kind else {
        return Err(Error::NotAnImplementation);
    };
    let crate::decl::ImplDecl {
        is_unsafe,
        generics,
        kind,
        generic_where_clause,
        meta_where_clause,
        specialize_attr,
        items: _,
        span: _,
    } = declaration;
    let input = ImplementationScope {
        owner,
        attributes,
        is_unsafe: *is_unsafe,
        generics,
        kind,
        generic_where_clause,
        meta_where_clause,
        specialize_attr,
    };
    encode_input(&input, b"implementation-scope", limits, sink)
}

/// Identify the complete implementation, including sibling declarations,
/// source attributes, specialization and both kinds of constraint. The producer
/// retains this digest while visiting its methods; no partial header is copied.
pub fn encode_implementation(
    item: &Item,
    owner: &str,
    lexical_scopes: &[LexicalScope],
    limits: Limits,
    sink: &mut impl FnMut(&[u8]),
) -> std::result::Result<(), Error> {
    let input = ImplementationInput {
        owner,
        lexical_scopes,
        item,
    };
    encode_input(&input, b"implementation", limits, sink)
}

fn encode_input(
    input: &impl Node,
    domain: &[u8],
    limits: Limits,
    sink: &mut impl FnMut(&[u8]),
) -> std::result::Result<(), Error> {
    let mut encoder = Encoder {
        pending: List::new(),
        sink,
        limits,
        visited: 0,
        bytes: 0,
    };
    encoder.write(b"verum.ast.source-definition\0")?;
    encoder.write(&VERSION.to_le_bytes())?;
    encoder.length(domain.len())?;
    encoder.write(domain)?;
    encoder.value(input)?;
    while let Some(work) = encoder.pending.pop() {
        encoder.visited += 1;
        match work {
            Work::Value(value) => value.encode(&mut encoder)?,
            Work::Field(name) => {
                encoder.write(&[1])?;
                encoder.text(name)?;
            }
        }
    }
    Ok(())
}

pub(crate) trait Node {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()>;
}
enum Work<'a> {
    Value(&'a dyn Node),
    Field(&'static str),
}
pub(crate) struct Encoder<'a, 'w> {
    pending: List<Work<'a>>,
    sink: &'w mut dyn FnMut(&[u8]),
    limits: Limits,
    visited: usize,
    bytes: usize,
}
impl<'a> Encoder<'a, '_> {
    fn push(&mut self, work: Work<'a>) -> Result<()> {
        if self.visited.saturating_add(self.pending.len()) >= self.limits.nodes {
            return Err(Error::NodeLimit {
                limit: self.limits.nodes,
            });
        }
        self.pending.push(work);
        Ok(())
    }
    fn value(&mut self, value: &'a dyn Node) -> Result<()> {
        self.push(Work::Value(value))
    }
    pub(crate) fn field(&mut self, name: &'static str, value: &'a dyn Node) -> Result<()> {
        self.value(value)?;
        self.push(Work::Field(name))
    }
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= self.limits.bytes)
            .ok_or(Error::ByteLimit {
                limit: self.limits.bytes,
            })?;
        (self.sink)(bytes);
        Ok(())
    }
    fn length(&mut self, count: usize) -> Result<()> {
        self.write(&(count as u64).to_le_bytes())
    }
    fn text(&mut self, value: &str) -> Result<()> {
        self.length(value.len())?;
        self.write(value.as_bytes())
    }
    pub(crate) fn record(&mut self, name: &str, count: usize) -> Result<()> {
        self.write(&[2])?;
        self.text(name)?;
        self.length(count)
    }
    pub(crate) fn variant(&mut self, name: &str, variant: &str, count: usize) -> Result<()> {
        self.write(&[3])?;
        self.text(name)?;
        self.text(variant)?;
        self.length(count)
    }
    fn sequence<T: Node>(&mut self, values: &'a [T]) -> Result<()> {
        self.write(&[4])?;
        self.length(values.len())?;
        for value in values.iter().rev() {
            self.value(value)?;
        }
        Ok(())
    }
}

macro_rules! number {
    ($($ty:ty => $tag:expr),* $(,)?) => { $(impl Node for $ty {
        fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
            encoder.write(&[$tag])?; encoder.write(&self.to_le_bytes())
        }
    })* };
}
number!(u8=>10, u16=>11, u32=>12, u64=>13, u128=>14,
    i8=>15, i16=>16, i32=>17, i64=>18, i128=>19);
impl Node for usize {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[20])?;
        encoder.write(&(*self as u64).to_le_bytes())
    }
}
impl Node for bool {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[21, u8::from(*self)])
    }
}
impl Node for char {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[22])?;
        encoder.write(&(*self as u32).to_le_bytes())
    }
}
impl Node for f32 {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[23])?;
        encoder.write(&self.to_bits().to_le_bytes())
    }
}
impl Node for f64 {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[24])?;
        encoder.write(&self.to_bits().to_le_bytes())
    }
}
impl Node for Text {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[25])?;
        encoder.text(self.as_str())
    }
}
// Retain the existing AST's string fields without allocating a conversion.
impl Node for std::string::String {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[25])?;
        encoder.text(self.as_str())
    }
}
impl Node for &str {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[25])?;
        encoder.text(self)
    }
}
impl<T: Node> Node for &T {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.value(*self)
    }
}
impl<T: Node> Node for List<T> {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.sequence(self.as_slice())
    }
}
// Existing AST byte literals and statement attributes use this carrier.
impl<T: Node> Node for std::vec::Vec<T> {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.sequence(self.as_slice())
    }
}
impl<T: Node> Node for &[T] {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.sequence(self)
    }
}
impl<A: smallvec::Array> Node for smallvec::SmallVec<A>
where
    A::Item: Node,
{
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.sequence(self.as_slice())
    }
}
impl<T: Node> Node for Option<T> {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[26, u8::from(self.is_some())])?;
        if let Some(value) = self {
            encoder.value(value)?;
        }
        Ok(())
    }
}
impl<T: Node> Node for verum_common::Heap<T> {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.value(self.as_ref())
    }
}
impl<A: Node, B: Node> Node for (A, B) {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.record("tuple", 2)?;
        encoder.field("1", &self.1)?;
        encoder.field("0", &self.0)
    }
}
impl Node for crate::Span {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[27])
    }
}
impl Node for crate::FileId {
    fn encode<'a>(&'a self, encoder: &mut Encoder<'a, '_>) -> Result<()> {
        encoder.write(&[27])
    }
}
