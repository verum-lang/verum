//! Safety of explicit recursive Horn models; source correspondence is external.
use super::{Atom, CHC, FixedPointEngine, RankingFunction, RankingTransition};
use verum_common::{List, Maybe, Set, Text};
use z3::{
    Context, FuncDecl, Params, SatResult, Sort,
    ast::{Ast, Bool, Dynamic},
};

/// A complete recursive model and universally quantified safety conditions.
/// Register all relations before rules, so mutual recursion is supported.
/// Every declared relation requires an explicit definition; an intentionally
/// empty relation can be defined by a clause with a false guard.
#[derive(Debug)]
pub struct RecursiveProgram {
    pub relations: List<FuncDecl>,
    pub rules: List<CHC>,
    pub verification_conditions: List<VerificationCondition>,
}

/// For all explicit variables: reachable hypotheses and path constraints imply
/// the assertion. Inputs, outputs and intermediate values are bound here; the
/// verifier never invents a function body or infers it from a function name.
#[derive(Debug)]
pub struct VerificationCondition {
    pub vars: List<(Text, Sort)>,
    pub hypothesis: List<Atom>,
    pub constraints: List<Bool>,
    pub assertion: Bool,
}

/// Partial correctness of the supplied Horn model, not a termination proof or
/// kernel certificate. A refutation's answer is raw backend derivation data.
#[derive(Debug)]
pub enum VerificationResult {
    Verified,
    Refuted {
        condition: usize,
        answer: Maybe<Bool>,
    },
}

/// Each verification call owns a fresh solver. No declarations, assumptions or
/// failed partial registrations leak into the next problem.
pub struct RecursiveProgramVerifier {
    ctx: Context,
}

impl RecursiveProgramVerifier {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        if ctx != Context::thread_local() {
            return Err("Recursive verifier requires the active Z3 context".into());
        }
        Ok(Self { ctx })
    }

    pub fn verify_recursive_program(
        &self,
        program: RecursiveProgram,
    ) -> Result<VerificationResult, Text> {
        let mut params = Params::new();
        params.set_symbol("engine", "spacer");
        if let Some(timeout) = crate::context::ContextConfig::default().timeout {
            params.set_u32(
                "timeout",
                timeout.as_millis().min(u128::from(u32::MAX)) as u32,
            );
        }
        self.verify_recursive_program_with_params(program, &params)
    }

    /// UNSAT of every reachable violation is the only accepting result.
    /// SAT refutes the corresponding condition; Unknown retains its reason as
    /// an error. Callers can supply fixedpoint time/resource limits in `params`.
    pub fn verify_recursive_program_with_params(
        &self,
        program: RecursiveProgram,
        params: &Params,
    ) -> Result<VerificationResult, Text> {
        let mut engine = FixedPointEngine::new(self.ctx.clone())?;
        if program.verification_conditions.is_empty() {
            return Err("Recursive verification requires explicit safety conditions".into());
        }
        engine.set_params(params);
        let defined: Set<_> = program
            .rules
            .iter()
            .map(|rule| rule.conclusion.predicate.clone())
            .collect();
        for relation in program.relations {
            let name = Text::from(relation.name());
            if !defined.contains(&name) {
                return Err(format!("Missing recursive relation definition: {name}").into());
            }
            engine.register_relation(relation)?;
        }
        for rule in program.rules {
            engine.add_chc(rule)?;
        }
        // Validate and lower every condition before asking the solver anything,
        // including conditions after an already reachable violation.
        let mut violations = List::new();
        for condition in program.verification_conditions {
            if condition.assertion.get_ctx() != &self.ctx {
                return Err("Safety assertion belongs to another Z3 context".into());
            }
            let violation = Bool::fresh_const("recursive_violation");
            engine.register_relation(violation.decl())?;
            let mut constraints = condition.constraints;
            constraints.push(condition.assertion.not());
            engine.add_chc(CHC {
                vars: condition.vars,
                hypothesis: condition.hypothesis,
                constraints,
                conclusion: Atom {
                    predicate: violation.decl().name().into(),
                    args: List::new(),
                },
            })?;
            violations.push(violation);
        }
        for (condition, violation) in violations.iter().enumerate() {
            match engine.query(violation)? {
                SatResult::Unsat => {}
                SatResult::Sat => {
                    return Ok(VerificationResult::Refuted {
                        condition,
                        answer: engine.fp.get_answer(),
                    });
                }
                SatResult::Unknown => {
                    return Err(format!(
                        "Recursive safety condition {condition} unknown: {}",
                        engine.get_reason_unknown()
                    )
                    .into());
                }
            }
        }
        Ok(VerificationResult::Verified)
    }

    /// Check actual guarded self-call arguments against a candidate rank.
    /// This is independent of partial correctness and requires the caller's
    /// complete call model; see `RankingFunction::verify` for its obligations.
    pub fn verify_termination(
        &self,
        func: RecursiveFunction,
        ranking: RankingFunction,
    ) -> Result<bool, Text> {
        if self.ctx != Context::thread_local() {
            return Err("Recursive verifier requires the active Z3 context".into());
        }
        ranking.verify(
            &func.parameters,
            &func.precondition,
            &func.recursive_calls,
            crate::context::ContextConfig::default(),
        )
    }
}

/// Explicit self-call model for ranking verification. Partial correctness uses
/// `RecursiveProgram`, including actual relation rules and safety conditions.
#[derive(Debug)]
pub struct RecursiveFunction {
    pub name: Text,
    /// Explicit symbolic parameters, in call argument order.
    pub parameters: List<Dynamic>,
    pub precondition: Bool,
    /// Actual path guard and simultaneous arguments for every self-call.
    pub recursive_calls: List<RankingTransition>,
}
