//! Structural translation of closed intensional judgments to Lean kernel input.
//! Emission is not a verdict. Lean checks the original term against the statement.
use crate::{
    admission::{AdmissionLimits, decode_judgment},
    proof_checker::{Foundation, Level, Term},
};
use std::{collections::HashMap, fmt::Write};

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

    fn term(&mut self, term: &Term, depth: usize) -> Result<(), String> {
        self.out.push('(');
        match term {
            Term::Var(i) => write!(
                self.out,
                "x{}",
                depth.checked_sub(i + 1).ok_or("unbound term variable")?
            )
            .unwrap(),
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
                write!(self.out, "{open}(x{depth} : ").unwrap();
                self.term(a, depth)?;
                self.out.push_str(close);
                self.term(b, depth + 1)?;
                if matches!(term, Term::Sigma(..)) {
                    self.out.push(')');
                }
            }
            Term::Let { value, body } => {
                write!(self.out, "let x{depth} := ").unwrap();
                self.term(value, depth)?;
                self.out.push_str("; ");
                self.term(body, depth + 1)?;
            }
            Term::App(f, a) => {
                self.term(f, depth)?;
                self.out.push(' ');
                self.term(a, depth)?;
            }
            Term::Pair(a, b) => {
                self.out.push('⟨');
                self.term(a, depth)?;
                self.out.push_str(", ");
                self.term(b, depth)?;
                self.out.push('⟩');
            }
            Term::Fst(p) | Term::Snd(p) => {
                self.term(p, depth)?;
                self.out.push_str(if matches!(term, Term::Fst(_)) {
                    ".1"
                } else {
                    ".2"
                });
            }
            Term::Id { ty, lhs, rhs } => {
                self.out.push_str("@Identity ");
                self.term(ty, depth)?;
                self.out.push(' ');
                self.term(lhs, depth)?;
                self.out.push(' ');
                self.term(rhs, depth)?;
            }
            Term::Refl(x) => {
                self.out.push_str("Identity.refl (x := ");
                self.term(x, depth)?;
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
                    self.term(t, depth)?;
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
        out: String::new(),
    };
    encoder.term(&judgment.statement, 0)?;
    let statement = std::mem::take(&mut encoder.out);
    encoder.in_statement = false;
    encoder.term(&judgment.certificate.term, 0)?;
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
        "def checked : {statement} :=\n{}\n#print axioms checked\nend VerumCertificate",
        encoder.out
    )
    .unwrap();
    Ok(source)
}
