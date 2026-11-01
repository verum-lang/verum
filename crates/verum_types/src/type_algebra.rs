//! Bounded, proof-producing entailment in the free distributive lattice.
//!
//! Atoms are opaque identities assigned by the caller: this module knows nothing
//! about nominal names, numeric types, protocols, inference, or coercions. Equal
//! atom identities must mean the same semantic atom. No other atomic entailment
//! is assumed. Unions and intersections denote lattice joins and meets; they do
//! not describe runtime tags or representation changes.
//!
//! Search uses finite, multiple-conclusion sequents (`meet(left) <= join(right)`)
//! and only subformulas of the query. It memoizes sequents and decomposes them
//! incrementally, without constructing a distributive normal form. Both search
//! and certificate replay are iterative and explicitly bounded. Exhaustion and
//! unsuccessful search return `Undetermined`, never evidence of non-subtyping.
//! [`check`] independently checks local proof rules, exact graph contents and
//! endpoints. A caller may accept only certificates that pass that check.

use verum_common::{List, Map};

/// Semantic atom identity, supplied by the caller rather than derived from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtomId(pub u32);

/// Arena-local structural identity. IDs are deterministic for a construction
/// sequence; they are not portable between different expression graphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpressionId(pub usize);

/// A node in an acyclic, structurally interned expression graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Expression {
    Bottom,
    Top,
    Atom(AtomId),
    Union(ExpressionId, ExpressionId),
    Intersection(ExpressionId, ExpressionId),
}

/// Construction rejects invalid references and never grows past its node limit.
#[derive(Debug, Clone)]
pub struct ExpressionDag {
    nodes: List<Expression>,
    interned: Map<Expression, ExpressionId>,
    max_nodes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    NodeLimit,
    InvalidExpression,
}

impl ExpressionDag {
    pub fn new(max_nodes: usize) -> Self {
        Self {
            nodes: List::new(),
            interned: Map::new(),
            max_nodes,
        }
    }

    pub fn bottom(&mut self) -> Result<ExpressionId, BuildError> {
        self.intern(Expression::Bottom)
    }

    pub fn top(&mut self) -> Result<ExpressionId, BuildError> {
        self.intern(Expression::Top)
    }

    pub fn atom(&mut self, atom: AtomId) -> Result<ExpressionId, BuildError> {
        self.intern(Expression::Atom(atom))
    }

    pub fn union(
        &mut self,
        left: ExpressionId,
        right: ExpressionId,
    ) -> Result<ExpressionId, BuildError> {
        self.intern(Expression::Union(left, right))
    }

    pub fn intersection(
        &mut self,
        left: ExpressionId,
        right: ExpressionId,
    ) -> Result<ExpressionId, BuildError> {
        self.intern(Expression::Intersection(left, right))
    }

    pub fn expression(&self, id: ExpressionId) -> Option<&Expression> {
        self.nodes.get(id.0)
    }

    pub fn nodes(&self) -> &[Expression] {
        &self.nodes
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn intern(&mut self, expression: Expression) -> Result<ExpressionId, BuildError> {
        match expression {
            Expression::Union(a, b) | Expression::Intersection(a, b)
                if a.0 >= self.nodes.len() || b.0 >= self.nodes.len() =>
            {
                return Err(BuildError::InvalidExpression);
            }
            _ => {}
        }
        if let Some(id) = self.interned.get(&expression) {
            return Ok(*id);
        }
        if self.nodes.len() >= self.max_nodes {
            return Err(BuildError::NodeLimit);
        }
        let id = ExpressionId(self.nodes.len());
        self.nodes.push(expression);
        self.interned.insert(expression, id);
        Ok(id)
    }
}

/// Limits count all graph nodes, distinct visited sequents, the sum of their
/// context lengths, and the longest proof path (an axiom has depth one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_expression_nodes: usize,
    pub max_sequents: usize,
    pub max_context_entries: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_expression_nodes: 512,
            max_sequents: 4096,
            max_context_entries: 131_072,
            max_depth: 512,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndeterminedReason {
    InvalidExpression,
    ResourceLimit,
    /// The equality-only lattice fragment found no derivation. This does not
    /// refute entailment in the caller's richer type system.
    NoDerivation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Proven(Certificate),
    Undetermined(UndeterminedReason),
}

/// Contexts are sorted, duplicate-free sets of graph references.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sequent {
    pub left: List<ExpressionId>,
    pub right: List<ExpressionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofId(pub usize);

/// Each premise must precede its conclusion in the certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Identity(ExpressionId),
    BottomLeft(ExpressionId),
    TopRight(ExpressionId),
    IntersectionLeft(ExpressionId, ProofId),
    UnionRight(ExpressionId, ProofId),
    TopLeft(ExpressionId, ProofId),
    BottomRight(ExpressionId, ProofId),
    UnionLeft(ExpressionId, ProofId, ProofId),
    IntersectionRight(ExpressionId, ProofId, ProofId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofStep {
    pub sequent: Sequent,
    pub rule: Rule,
}

/// Public data may be inspected, stored, or tampered with. Only successful
/// [`check`] gives this data evidentiary meaning for a specific query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub expressions: List<Expression>,
    pub source: ExpressionId,
    pub target: ExpressionId,
    pub steps: List<ProofStep>,
    pub root: ProofId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckError {
    ResourceLimit,
    QueryMismatch,
    InvalidExpression,
    InvalidContext,
    InvalidRule,
    InvalidPremise,
    InvalidRoot,
}

#[derive(Debug, Clone, Copy)]
enum Decomposition {
    IntersectionLeft(ExpressionId),
    UnionRight(ExpressionId),
    TopLeft(ExpressionId),
    BottomRight(ExpressionId),
    UnionLeft(ExpressionId),
    IntersectionRight(ExpressionId),
}

struct Expansion {
    kind: Decomposition,
    premises: List<Sequent>,
}

enum Work {
    Enter(Sequent, usize),
    Finish(Sequent, Expansion),
}

/// Searches only the equality-only finite lattice fragment. Even a generated
/// proof must be checked with [`check`] before the caller relies on it.
pub fn prove(
    dag: &ExpressionDag,
    source: ExpressionId,
    target: ExpressionId,
    limits: Limits,
) -> Outcome {
    if dag.len() > limits.max_expression_nodes {
        return Outcome::Undetermined(UndeterminedReason::ResourceLimit);
    }
    if dag.expression(source).is_none() || dag.expression(target).is_none() {
        return Outcome::Undetermined(UndeterminedReason::InvalidExpression);
    }
    let root = Sequent {
        left: List::from_single(source),
        right: List::from_single(target),
    };
    let mut work = List::from_single(Work::Enter(root.clone(), 1));
    let mut memo: Map<Sequent, ProofId> = Map::new();
    let mut steps = List::new();
    let mut proof_depths: List<usize> = List::new();
    let mut remaining_sequents = limits.max_sequents;
    let mut remaining_entries = limits.max_context_entries;
    while let Some(item) = work.pop() {
        let (sequent, rule) = match item {
            Work::Enter(sequent, depth) => {
                if memo.contains_key(&sequent) {
                    continue;
                }
                let entries = sequent.left.len().saturating_add(sequent.right.len());
                if depth > limits.max_depth
                    || remaining_sequents == 0
                    || entries > remaining_entries
                {
                    return Outcome::Undetermined(UndeterminedReason::ResourceLimit);
                }
                remaining_sequents -= 1;
                remaining_entries -= entries;
                if let Some(rule) = search_axiom(dag, &sequent) {
                    (sequent, rule)
                } else if let Some(expansion) = expand(dag, &sequent) {
                    // No further allocation or increment once the path is full.
                    if depth >= limits.max_depth {
                        return Outcome::Undetermined(UndeterminedReason::ResourceLimit);
                    }
                    let children = expansion.premises.clone();
                    work.push(Work::Finish(sequent, expansion));
                    for child in children.into_iter().rev() {
                        work.push(Work::Enter(child, depth + 1));
                    }
                    continue;
                } else {
                    return Outcome::Undetermined(UndeterminedReason::NoDerivation);
                }
            }
            Work::Finish(sequent, expansion) => {
                let first = memo[&expansion.premises[0]];
                let rule = match expansion.kind {
                    Decomposition::IntersectionLeft(id) => Rule::IntersectionLeft(id, first),
                    Decomposition::UnionRight(id) => Rule::UnionRight(id, first),
                    Decomposition::TopLeft(id) => Rule::TopLeft(id, first),
                    Decomposition::BottomRight(id) => Rule::BottomRight(id, first),
                    Decomposition::UnionLeft(id) => {
                        Rule::UnionLeft(id, first, memo[&expansion.premises[1]])
                    }
                    Decomposition::IntersectionRight(id) => {
                        Rule::IntersectionRight(id, first, memo[&expansion.premises[1]])
                    }
                };
                (sequent, rule)
            }
        };
        // A memoized proof can be reused below a deeper search path. Check the
        // actual proof DAG depth as well as the work-stack depth above.
        let depth = match rule {
            Rule::Identity(_) | Rule::BottomLeft(_) | Rule::TopRight(_) => 1,
            Rule::IntersectionLeft(_, child)
            | Rule::UnionRight(_, child)
            | Rule::TopLeft(_, child)
            | Rule::BottomRight(_, child) => proof_depths[child.0].saturating_add(1),
            Rule::UnionLeft(_, first, second) | Rule::IntersectionRight(_, first, second) => {
                proof_depths[first.0]
                    .max(proof_depths[second.0])
                    .saturating_add(1)
            }
        };
        if depth > limits.max_depth {
            return Outcome::Undetermined(UndeterminedReason::ResourceLimit);
        }
        let id = ProofId(steps.len());
        memo.insert(sequent.clone(), id);
        steps.push(ProofStep { sequent, rule });
        proof_depths.push(depth);
    }
    Outcome::Proven(Certificate {
        expressions: dag.nodes.clone(),
        source,
        target,
        root: memo[&root],
        steps,
    })
}

fn search_axiom(dag: &ExpressionDag, sequent: &Sequent) -> Option<Rule> {
    for id in &sequent.left {
        if sequent.right.binary_search(id).is_ok() {
            return Some(Rule::Identity(*id));
        }
        if dag.expression(*id) == Some(&Expression::Bottom) {
            return Some(Rule::BottomLeft(*id));
        }
    }
    for id in &sequent.right {
        if dag.expression(*id) == Some(&Expression::Top) {
            return Some(Rule::TopRight(*id));
        }
    }
    None
}

fn replaced(
    context: &List<ExpressionId>,
    id: ExpressionId,
    with: &[ExpressionId],
) -> List<ExpressionId> {
    let mut result: List<_> = context.iter().copied().filter(|old| *old != id).collect();
    result.extend(with.iter().copied());
    result.sort_unstable();
    result.dedup();
    result
}

fn expand(dag: &ExpressionDag, sequent: &Sequent) -> Option<Expansion> {
    // Nonbranching rules come first, keeping contexts compact before splitting.
    for id in &sequent.left {
        let (kind, with) = match *dag.expression(*id)? {
            Expression::Intersection(a, b) => (
                Decomposition::IntersectionLeft(*id),
                List::from(&[a, b][..]),
            ),
            Expression::Top => (Decomposition::TopLeft(*id), List::new()),
            _ => continue,
        };
        return Some(Expansion {
            kind,
            premises: List::from_single(Sequent {
                left: replaced(&sequent.left, *id, &with),
                right: sequent.right.clone(),
            }),
        });
    }
    for id in &sequent.right {
        let (kind, with) = match *dag.expression(*id)? {
            Expression::Union(a, b) => (Decomposition::UnionRight(*id), List::from(&[a, b][..])),
            Expression::Bottom => (Decomposition::BottomRight(*id), List::new()),
            _ => continue,
        };
        return Some(Expansion {
            kind,
            premises: List::from_single(Sequent {
                left: sequent.left.clone(),
                right: replaced(&sequent.right, *id, &with),
            }),
        });
    }
    for id in &sequent.left {
        if let Expression::Union(a, b) = *dag.expression(*id)? {
            return Some(Expansion {
                kind: Decomposition::UnionLeft(*id),
                premises: [a, b]
                    .into_iter()
                    .map(|branch| Sequent {
                        left: replaced(&sequent.left, *id, &[branch]),
                        right: sequent.right.clone(),
                    })
                    .collect(),
            });
        }
    }
    for id in &sequent.right {
        if let Expression::Intersection(a, b) = *dag.expression(*id)? {
            return Some(Expansion {
                kind: Decomposition::IntersectionRight(*id),
                premises: [a, b]
                    .into_iter()
                    .map(|branch| Sequent {
                        left: sequent.left.clone(),
                        right: replaced(&sequent.right, *id, &[branch]),
                    })
                    .collect(),
            });
        }
    }
    None
}

/// Independently replays a certificate against exactly this graph and query.
/// No search, normalization, atomic oracle, or cached search answer is trusted.
pub fn check(
    dag: &ExpressionDag,
    source: ExpressionId,
    target: ExpressionId,
    certificate: &Certificate,
    limits: Limits,
) -> Result<(), CheckError> {
    if dag.len() > limits.max_expression_nodes
        || certificate.expressions.len() > limits.max_expression_nodes
        || certificate.steps.len() > limits.max_sequents
    {
        return Err(CheckError::ResourceLimit);
    }
    if certificate.source != source
        || certificate.target != target
        || certificate.expressions != dag.nodes
    {
        return Err(CheckError::QueryMismatch);
    }
    if source.0 >= dag.len() || target.0 >= dag.len() {
        return Err(CheckError::InvalidExpression);
    }
    if certificate.steps.is_empty() || certificate.root.0 != certificate.steps.len() - 1 {
        return Err(CheckError::InvalidRoot);
    }
    let conclusion = &certificate.steps[certificate.root.0].sequent;
    if conclusion.left.as_slice() != [source] || conclusion.right.as_slice() != [target] {
        return Err(CheckError::InvalidRoot);
    }

    // Mark the exact query's subformula scope. Graph construction guarantees
    // children precede parents, so one descending pass suffices without a stack.
    let mut in_scope = List::from_elem(false, dag.len());
    in_scope[source.0] = true;
    in_scope[target.0] = true;
    for index in (0..dag.len()).rev() {
        if in_scope[index] {
            match dag.nodes[index] {
                Expression::Union(a, b) | Expression::Intersection(a, b) => {
                    in_scope[a.0] = true;
                    in_scope[b.0] = true;
                }
                _ => {}
            }
        }
    }
    let mut remaining_entries = limits.max_context_entries;
    let mut depths: List<usize> = List::new();
    for (index, step) in certificate.steps.iter().enumerate() {
        for context in [&step.sequent.left, &step.sequent.right] {
            if context.len() > remaining_entries {
                return Err(CheckError::ResourceLimit);
            }
            remaining_entries -= context.len();
            if context.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(CheckError::InvalidContext);
            }
            if context
                .iter()
                .any(|id| !in_scope.get(id.0).copied().unwrap_or(false))
            {
                return Err(CheckError::InvalidExpression);
            }
        }
        let mut depth = 1;
        let mut premise = |id: ProofId| -> Result<&Sequent, CheckError> {
            if id.0 >= index {
                return Err(CheckError::InvalidPremise);
            }
            depth = depth.max(depths[id.0].saturating_add(1));
            Ok(&certificate.steps[id.0].sequent)
        };
        let left = &step.sequent.left;
        let right = &step.sequent.right;
        let valid = match step.rule {
            Rule::Identity(id) => left.contains(&id) && right.contains(&id),
            Rule::BottomLeft(id) => {
                left.contains(&id) && dag.expression(id) == Some(&Expression::Bottom)
            }
            Rule::TopRight(id) => {
                right.contains(&id) && dag.expression(id) == Some(&Expression::Top)
            }
            Rule::IntersectionLeft(id, proof) => {
                let child = premise(proof)?;
                match dag.expression(id) {
                    Some(Expression::Intersection(a, b)) => {
                        child.right == *right
                            && checks_replacement(left, &child.left, id, &[*a, *b])
                    }
                    _ => false,
                }
            }
            Rule::UnionRight(id, proof) => {
                let child = premise(proof)?;
                match dag.expression(id) {
                    Some(Expression::Union(a, b)) => {
                        child.left == *left
                            && checks_replacement(right, &child.right, id, &[*a, *b])
                    }
                    _ => false,
                }
            }
            Rule::TopLeft(id, proof) => {
                let child = premise(proof)?;
                dag.expression(id) == Some(&Expression::Top)
                    && child.right == *right
                    && checks_replacement(left, &child.left, id, &[])
            }
            Rule::BottomRight(id, proof) => {
                let child = premise(proof)?;
                dag.expression(id) == Some(&Expression::Bottom)
                    && child.left == *left
                    && checks_replacement(right, &child.right, id, &[])
            }
            Rule::UnionLeft(id, first, second) => {
                let first = premise(first)?;
                let second = premise(second)?;
                match dag.expression(id) {
                    Some(Expression::Union(a, b)) => {
                        first.right == *right
                            && second.right == *right
                            && checks_replacement(left, &first.left, id, &[*a])
                            && checks_replacement(left, &second.left, id, &[*b])
                    }
                    _ => false,
                }
            }
            Rule::IntersectionRight(id, first, second) => {
                let first = premise(first)?;
                let second = premise(second)?;
                match dag.expression(id) {
                    Some(Expression::Intersection(a, b)) => {
                        first.left == *left
                            && second.left == *left
                            && checks_replacement(right, &first.right, id, &[*a])
                            && checks_replacement(right, &second.right, id, &[*b])
                    }
                    _ => false,
                }
            }
        };
        if !valid {
            return Err(CheckError::InvalidRule);
        }
        if depth > limits.max_depth {
            return Err(CheckError::ResourceLimit);
        }
        depths.push(depth);
    }
    Ok(())
}

// Deliberately does not use the searcher's context-replacement constructor.
// Equality of finite sets is checked by membership in both directions.
fn checks_replacement(
    parent: &[ExpressionId],
    child: &[ExpressionId],
    principal: ExpressionId,
    additions: &[ExpressionId],
) -> bool {
    parent.binary_search(&principal).is_ok()
        && parent
            .iter()
            .all(|id| *id == principal || child.binary_search(id).is_ok())
        && additions.iter().all(|id| child.binary_search(id).is_ok())
        && child.iter().all(|id| {
            additions.contains(id) || (*id != principal && parent.binary_search(id).is_ok())
        })
}
