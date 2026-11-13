//! Structural translation of closed intensional judgments to Lean kernel input.
//! Emission is not a verdict. Lean checks the original term against the statement.
use crate::{
    admission::{AdmissionLimits, decode_judgment},
    proof_checker::{Foundation, Level, Term},
};
use std::{collections::HashMap, fmt::Write, ops::Range};

const PRELUDE: &str = "import Init
set_option autoImplicit false
set_option warningAsError true
set_option maxRecDepth 4096
namespace VerumCertificate
universe u v
inductive Identity {A : Type u} (x : A) : A → Type u where
  | refl : Identity x x

def transport {A : Type u} {x y : A} (P : A → Type v) (b : P x)
    (p : Identity x y) : P y := match p with | .refl => b

def induction {A : Type u} {x y : A} (P : (z : A) → Identity x z → Type v)
    (b : P x (.refl)) (p : Identity x y) : P y p := match p with | .refl => b

";

struct Encoder {
    // Wire names never enter Lean source. Index in first-statement-occurrence
    // order makes a consistent alpha-renaming produce identical output.
    levels: HashMap<String, usize>,
    in_statement: bool,
    // Lexical de Bruijn slots point into the ordered list of generated names.
    // A reference claims its binder. All others print as ordinary anonymous binders.
    bindings: Vec<usize>,
    unused_names: Vec<Option<Range<usize>>>,
    out: String,
}

impl Encoder {
    fn level(&mut self, level: &Level) -> Result<(), String> {
        match level {
            Level::Concrete(n) => write!(self.out, "{n}").unwrap(),
            Level::Var(name) => {
                let next = self.levels.len();
                let index = if self.in_statement {
                    *self.levels.entry(name.clone()).or_insert(next)
                } else {
                    *self
                        .levels
                        .get(name)
                        .ok_or("proof universe is outside the statement scope")?
                };
                write!(self.out, "w{index}").unwrap();
            }
            Level::Succ(inner) | Level::Offset(inner, _) => {
                let offset = if let Level::Offset(_, n) = level {
                    *n
                } else {
                    1
                };
                self.out.push('(');
                self.level(inner)?;
                write!(self.out, " + {offset})").unwrap();
            }
            Level::Max(a, b) => {
                self.out.push_str("(max ");
                self.level(a)?;
                self.out.push(' ');
                self.level(b)?;
                self.out.push(')');
            }
        }
        Ok(())
    }

    fn binder_name(&mut self) -> usize {
        let start = self.out.len();
        write!(self.out, "x{}", self.bindings.len()).unwrap();
        let slot = self.unused_names.len();
        self.unused_names.push(Some(start..self.out.len()));
        slot
    }

    fn binder_body(&mut self, slot: usize, body: &Term) -> Result<(), String> {
        self.bindings.push(slot);
        let result = self.term(body);
        self.bindings.pop();
        result
    }

    fn render(&mut self, term: &Term) -> Result<String, String> {
        self.term(term)?;
        let named = std::mem::take(&mut self.out);
        let mut result = String::with_capacity(named.len());
        let mut cursor = 0;
        for range in self.unused_names.drain(..).flatten() {
            result.push_str(&named[cursor..range.start]);
            result.push('_');
            cursor = range.end;
        }
        result.push_str(&named[cursor..]);
        Ok(result)
    }

    fn term(&mut self, term: &Term) -> Result<(), String> {
        self.out.push('(');
        match term {
            Term::Var(i) => {
                let depth = self
                    .bindings
                    .len()
                    .checked_sub(i.checked_add(1).ok_or("unbound term variable")?)
                    .ok_or("unbound term variable")?;
                self.unused_names[self.bindings[depth]] = None;
                write!(self.out, "x{depth}").unwrap();
            }
            Term::Universe(u) => {
                self.out.push_str("Type ");
                self.level(u)?;
            }
            Term::Pi(a, b) | Term::Lam(a, b) | Term::Sigma(a, b) => {
                let (open, close) = match term {
                    Term::Pi(..) => ("", ") → "),
                    Term::Lam(..) => ("fun ", ") => "),
                    _ => ("Sigma (fun ", ") => "),
                };
                self.out.push_str(open);
                self.out.push('(');
                let slot = self.binder_name();
                self.out.push_str(" : ");
                self.term(a)?;
                self.out.push_str(close);
                self.binder_body(slot, b)?;
                if matches!(term, Term::Sigma(..)) {
                    self.out.push(')');
                }
            }
            Term::Let { value, body } => {
                self.out.push_str("let ");
                let slot = self.binder_name();
                self.out.push_str(" := ");
                self.term(value)?;
                self.out.push_str("; ");
                self.binder_body(slot, body)?;
            }
            Term::App(f, a) => {
                self.term(f)?;
                self.out.push(' ');
                self.term(a)?;
            }
            Term::Pair(a, b) => {
                self.out.push('⟨');
                self.term(a)?;
                self.out.push_str(", ");
                self.term(b)?;
                self.out.push('⟩');
            }
            Term::Fst(p) | Term::Snd(p) => {
                self.term(p)?;
                self.out.push_str(if matches!(term, Term::Fst(_)) {
                    ".1"
                } else {
                    ".2"
                });
            }
            Term::Id { ty, lhs, rhs } => {
                self.out.push_str("@Identity ");
                self.term(ty)?;
                self.out.push(' ');
                self.term(lhs)?;
                self.out.push(' ');
                self.term(rhs)?;
            }
            Term::Refl(x) => {
                self.out.push_str("Identity.refl (x := ");
                self.term(x)?;
                self.out.push(')');
            }
            Term::J {
                motive,
                base,
                scrutinee,
            }
            | Term::IdElim {
                motive,
                base,
                scrutinee,
            } => {
                self.out.push_str(if matches!(term, Term::J { .. }) {
                    "transport"
                } else {
                    "induction"
                });
                for t in [motive, base, scrutinee] {
                    self.out.push(' ');
                    self.term(t)?;
                }
            }
            Term::FunExt { .. } => {
                return Err("extensionality is outside the intensional profile".into());
            }
            Term::Data { .. } | Term::Construct { .. } | Term::Induct { .. } => {
                return Err(
                    "native inductive declarations are outside this Lean export fragment".into(),
                );
            }
        }
        self.out.push(')');
        Ok(())
    }
}

pub fn source(bytes: &[u8]) -> Result<String, String> {
    let judgment = decode_judgment(bytes, Foundation::Intensional, AdmissionLimits::DEFAULT)
        .map_err(|r| {
            r.diagnostic
                .map(|d| d.message)
                .unwrap_or_else(|| "invalid judgment".into())
        })?;
    let mut encoder = Encoder {
        levels: HashMap::new(),
        in_statement: true,
        bindings: Vec::new(),
        unused_names: Vec::new(),
        out: String::new(),
    };
    let statement = encoder.render(&judgment.statement)?;
    encoder.in_statement = false;
    let proof = encoder.render(&judgment.certificate.term)?;
    let mut source = String::from(PRELUDE);
    if !encoder.levels.is_empty() {
        source.push_str("universe");
        for index in 0..encoder.levels.len() {
            write!(source, " w{index}").unwrap();
        }
        source.push('\n');
    }
    writeln!(
        source,
        "def checked : {statement} :=\n{proof}\n#print axioms checked\nend VerumCertificate"
    )
    .unwrap();
    Ok(source)
}
