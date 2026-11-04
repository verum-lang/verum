//! A strict conversion boundary for specialized record field families.
//! The ordinary checker types the value. The kernel checks a Sigma introduction at the
//! expected finite family code, using a local witness of the actual payload type.

use super::TypeChecker;
use crate::{
    Result, Type, TypeError,
    ty::{RecordFieldSlot, RecordReceiver},
    universe_level::NormalLevel,
};
use verum_ast::Span;
use verum_common::{ConstValue, Heap, Text, ToText};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, InductiveRegistry, UniverseLevel, verify_with_inductives,
};

#[derive(PartialEq)]
enum Symbol {
    Primitive(Type),
    Nominal(Text),
    Universe(NormalLevel),
    Projection(RecordFieldSlot, RecordReceiver, NormalLevel),
    Tuple(usize),
    Wrapper(&'static str, bool),
    MetaInteger(i128),
    MetaUnsigned(u128),
    MetaBool(bool),
}

struct Encoding<'a> {
    checker: &'a TypeChecker,
    context: Context,
    symbols: Vec<(Symbol, usize)>,
    remaining: usize,
    span: Span,
}

fn sort() -> CoreTerm {
    CoreTerm::Var("record.code".into())
}

impl Encoding<'_> {
    fn refuse(&self, message: &str) -> TypeError {
        TypeError::UniverseBinding {
            message: format!("cannot establish dependent record family agreement: {message}")
                .into(),
            span: self.span,
        }
    }

    // Intern actual typed identities, never hashes, pretty-printed types, or
    // bare leaf names. All generated names belong to this one local context.
    fn node(&mut self, symbol: Symbol, args: Vec<CoreTerm>) -> Result<CoreTerm> {
        let index = if let Some(index) = self.symbols.iter().position(|(key, _)| key == &symbol) {
            if self.symbols[index].1 != args.len() {
                return Err(self.refuse("inconsistent constructor arity"));
            }
            index
        } else {
            let index = self.symbols.len();
            let mut signature = sort();
            for _ in &args {
                signature = CoreTerm::Pi {
                    binder: "record.arg".into(),
                    domain: Heap::new(sort()),
                    codomain: Heap::new(signature),
                };
            }
            self.context = self
                .context
                .extend(format!("record.symbol{index}").into(), signature);
            self.symbols.push((symbol, args.len()));
            index
        };
        Ok(args.into_iter().fold(
            CoreTerm::Var(format!("record.symbol{index}").into()),
            |head, arg| CoreTerm::App(Heap::new(head), Heap::new(arg)),
        ))
    }

    fn nominal_key(&self, spelling: &Text) -> Result<Text> {
        if spelling.contains(".") {
            return Ok(spelling.clone());
        }
        // A binding carrying its resolved declaration path is evidence for
        // qualification. A declaration in the current source module also has
        // an exact key. A matching leaf elsewhere supplies neither fact.
        if let Some(ty) = self.checker.ctx.lookup_type(spelling.as_str()) {
            let key = match ty {
                Type::Named { path, .. } => Some(self.checker.path_to_string(path)),
                Type::Generic { name, .. } => Some(name.clone()),
                _ => None,
            };
            if let Some(key) = key
                && key.contains(".")
            {
                return Ok(key);
            }
        }
        if self
            .checker
            .current_module_declared_types
            .contains(spelling.as_str())
        {
            return Ok(self.checker.declared_type_key(spelling.as_str()));
        }
        Err(self.refuse(&format!("no declaration owner for {spelling}")))
    }

    fn nominal(&mut self, name: Text, args: &[Type], depth: usize) -> Result<CoreTerm> {
        let key = self.nominal_key(&name)?;
        let arity = self
            .checker
            .recorded_type_arity(&key)
            .or_else(|| {
                self.checker.core_metadata.as_ref().and_then(|metadata| {
                    TypeChecker::metadata_type_with_declaring_key(metadata, &key)
                        .map(|descriptor| descriptor.generic_params.len())
                })
            })
            .ok_or_else(|| self.refuse(&format!("missing declared arity for {key}")))?;
        if args.len() != arity || arity > 32 {
            return Err(self.refuse(&format!("invalid or unsupported argument count for {key}")));
        }
        // Validate every argument, including unused parameters of an alias.
        let encoded: Result<Vec<_>> = args.iter().map(|arg| self.encode(arg, depth + 1)).collect();
        let encoded = encoded?;
        if let Some(target) = self.checker.ctx.type_aliases.get(&key) {
            if arity != 0 && self.checker.recorded_type_arity(&key) != Some(arity) {
                return Err(self.refuse("missing transparent alias parameter slots"));
            }
            let expanded = self
                .checker
                .substitute_with_params(key.as_str(), target, args);
            return self.encode(&expanded, depth + 1);
        }
        self.node(Symbol::Nominal(key), encoded)
    }

    fn encode(&mut self, ty: &Type, depth: usize) -> Result<CoreTerm> {
        if self.remaining == 0 || depth > 32 {
            return Err(self.refuse("type code exceeds its structural limit"));
        }
        self.remaining -= 1;
        match ty {
            Type::Unit
            | Type::Never
            | Type::Int
            | Type::Bool
            | Type::Float
            | Type::Text
            | Type::Char
            | Type::Prop => self.node(Symbol::Primitive(ty.clone()), Vec::new()),
            Type::Universe { level } => {
                let normal = level
                    .normalize()
                    .ok_or_else(|| self.refuse("unresolved universe"))?;
                self.node(Symbol::Universe(normal), Vec::new())
            }
            Type::RecordField {
                slot,
                receiver: Some(receiver),
                level,
            } => {
                let normal = level
                    .normalize()
                    .ok_or_else(|| self.refuse("unresolved projection universe"))?;
                let Some(Type::Record(fields)) = self
                    .checker
                    .ctx
                    .lookup_type(&format!("__struct_fields_{}", slot.owner))
                else {
                    return Err(self.refuse("unknown projected field owner"));
                };
                if !matches!(fields.get_index(slot.ordinal), Some((_, Type::Universe { level })) if level.normalize().as_ref() == Some(&normal))
                {
                    return Err(
                        self.refuse("projected slot is not a carrier at the declared universe")
                    );
                }
                self.node(
                    Symbol::Projection((**slot).clone(), (**receiver).clone(), normal),
                    Vec::new(),
                )
            }
            Type::Named { path, args } => {
                if path.segments.is_empty()
                    || !path
                        .segments
                        .iter()
                        .all(|segment| matches!(segment, verum_ast::ty::PathSegment::Name(_)))
                {
                    return Err(self.refuse("unresolved nominal path"));
                }
                self.nominal(self.checker.path_to_string(path), args, depth)
            }
            Type::Generic { name, args } => self.nominal(name.clone(), args, depth),
            Type::Tuple(args) if args.len() <= 32 => {
                let encoded: Result<Vec<_>> =
                    args.iter().map(|arg| self.encode(arg, depth + 1)).collect();
                self.node(Symbol::Tuple(args.len()), encoded?)
            }
            Type::Reference { mutable, inner }
            | Type::CheckedReference { mutable, inner }
            | Type::UnsafeReference { mutable, inner }
            | Type::Ownership { mutable, inner } => {
                let kind = match ty {
                    Type::Reference { .. } => "reference",
                    Type::CheckedReference { .. } => "checked",
                    Type::UnsafeReference { .. } => "unsafe",
                    _ => "ownership",
                };
                let inner = self.encode(inner, depth + 1)?;
                self.node(Symbol::Wrapper(kind, *mutable), vec![inner])
            }
            Type::Meta {
                ty,
                value: crate::ty::ValueTerm::Constant(value),
                refinement: None,
                ..
            } => {
                let symbol = match value {
                    ConstValue::Int(n) => Symbol::MetaInteger(*n),
                    ConstValue::UInt(n) => Symbol::MetaUnsigned(*n),
                    ConstValue::Bool(b) => Symbol::MetaBool(*b),
                    _ => return Err(self.refuse("unsupported meta argument")),
                };
                let base = self.encode(ty, depth + 1)?;
                self.node(symbol, vec![base])
            }
            _ => Err(self.refuse("unresolved, erased, or unsupported type in a dependent field")),
        }
    }
}

impl TypeChecker {
    /// Check conversion of the supported finite family-index fragment after
    /// ordinary value typing, with the same interning context on both sides.
    pub fn check_record_family_agreement(
        &self,
        actual: &Type,
        expected: &Type,
        span: Span,
    ) -> Result<()> {
        let mut encoding = Encoding {
            checker: self,
            context: Context::new().extend(
                "record.code".into(),
                CoreTerm::Universe(UniverseLevel::Concrete(0)),
            ),
            symbols: Vec::new(),
            remaining: 512,
            span,
        };
        let left = encoding.encode(&self.unifier.apply(actual), 0)?;
        let right = encoding.encode(&self.unifier.apply(expected), 0)?;
        let axioms = AxiomRegistry::new();
        let inductives = InductiveRegistry::new();
        // The native judgment supplied a value at the actual family. El is a
        // fresh local family with no equations, so the kernel can only use it
        // at a convertible code. Validate the code before assuming its value.
        verify_with_inductives(&encoding.context, &left, &sort(), &axioms, &inductives)
            .map_err(|_| encoding.refuse("invalid actual family code"))?;
        let el_type = CoreTerm::Pi {
            binder: "record.index".into(),
            domain: Heap::new(sort()),
            codomain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        };
        let el = |code| {
            CoreTerm::App(
                Heap::new(CoreTerm::Var("record.El".into())),
                Heap::new(code),
            )
        };
        let ctx = encoding
            .context
            .extend("record.El".into(), el_type)
            .extend("record.payload".into(), el(left));
        let goal = CoreTerm::Sigma {
            binder: "record.index".into(),
            fst_ty: Heap::new(sort()),
            snd_ty: Heap::new(el(CoreTerm::Var("record.index".into()))),
        };
        let pair = CoreTerm::Pair(
            Heap::new(right),
            Heap::new(CoreTerm::Var("record.payload".into())),
        );
        verify_with_inductives(&ctx, &pair, &goal, &axioms, &inductives).map_err(|_| {
            TypeError::Mismatch {
                expected: expected.to_text(),
                actual: actual.to_text(),
                span,
            }
        })
    }
}
