// SAT-based dependency resolver: encodes version constraints as boolean satisfiability problem
// Uses Boolean satisfiability to find compatible package versions

use crate::error::Result;
use verum_common::{List, Set, Text};

// Re-export types for tests
pub use super::types::{DependencySpec, CogMetadata, TierArtifacts};
pub use semver::{Version, VersionReq};
pub use verum_common::Map;

/// SAT solver for dependency resolution
///
/// This implements a DPLL-based SAT solver optimized for package dependency resolution.
/// Each package version is assigned a boolean variable, and constraints are expressed
/// as clauses in CNF (Conjunctive Normal Form).
pub struct SatResolver {
    /// Variable assignments (package version -> bool)
    pub assignments: Map<CogVar, Option<bool>>,

    /// CNF clauses representing constraints
    pub clauses: List<Clause>,

    /// Cog metadata cache
    pub metadata_cache: Map<CogVar, CogMetadata>,

    /// Decision stack for backtracking
    pub decision_stack: List<Decision>,
}

/// Cog variable (name + version)
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CogVar {
    pub name: Text,
    pub version: Version,
}

/// CNF clause (disjunction of literals)
#[derive(Debug, Clone)]
pub struct Clause {
    literals: List<Literal>,
    learned: bool, // Conflict-driven clause learning
}

/// Literal (variable or its negation)
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Literal {
    var: CogVar,
    positive: bool,
}

/// Decision in DPLL search
#[derive(Debug, Clone)]
pub struct Decision {
    pub var: CogVar,
    pub value: bool,
    pub level: usize,
}

/// Resolution result
#[derive(Debug, Clone)]
pub struct SatResolution {
    pub selected: Map<Text, Version>,
    pub conflicts: List<Conflict>,
}

/// Conflict information
#[derive(Debug, Clone)]
pub struct Conflict {
    pub cog: Text,
    pub required_by: List<Text>,
    pub versions: List<Text>,
    pub reason: String,
}

impl SatResolver {
    /// Create new SAT resolver
    pub fn new() -> Self {
        Self {
            assignments: Map::new(),
            clauses: List::new(),
            metadata_cache: Map::new(),
            decision_stack: List::new(),
        }
    }

    /// Add package metadata to cache
    pub fn add_metadata(&mut self, metadata: CogMetadata) {
        let var = CogVar {
            name: metadata.name.clone(),
            version: Version::parse(metadata.version.as_str()).unwrap(),
        };
        self.metadata_cache.insert(var.clone(), metadata);
        self.assignments.insert(var, None);
    }

    /// Add constraint: if package A is selected, it requires package B
    pub fn add_dependency_constraint(
        &mut self,
        from: &CogVar,
        to_name: &str,
        version_req: &VersionReq,
    ) {
        // Find all versions of 'to' that satisfy the requirement
        let compatible_versions: List<_> = self
            .metadata_cache
            .keys()
            .filter(|v| v.name == to_name && version_req.matches(&v.version))
            .cloned()
            .collect();

        if compatible_versions.is_empty() {
            // No compatible versions -> conflict
            // Add clause: ¬from (if from is true, we have conflict)
            self.clauses.push(Clause {
                literals: List::from(vec![Literal {
                    var: from.clone(),
                    positive: false,
                }]),
                learned: false,
            });
        } else {
            // Add clause: ¬from ∨ to₁ ∨ to₂ ∨ ... ∨ toₙ
            // (if from is selected, at least one compatible version of to must be selected)
            let mut literals = vec![Literal {
                var: from.clone(),
                positive: false,
            }];

            for compatible in compatible_versions {
                literals.push(Literal {
                    var: compatible,
                    positive: true,
                });
            }

            self.clauses.push(Clause {
                literals: literals.into(),
                learned: false,
            });
        }
    }

    /// Add constraint: at most one version of a package can be selected
    pub fn add_uniqueness_constraint(&mut self, cog_name: &str) {
        let versions: List<_> = self
            .metadata_cache
            .keys()
            .filter(|v| v.name == cog_name)
            .cloned()
            .collect();

        // For each pair of versions, add: ¬v₁ ∨ ¬v₂
        for i in 0..versions.len() {
            for j in (i + 1)..versions.len() {
                self.clauses.push(Clause {
                    literals: List::from(vec![
                        Literal {
                            var: versions[i].clone(),
                            positive: false,
                        },
                        Literal {
                            var: versions[j].clone(),
                            positive: false,
                        },
                    ]),
                    learned: false,
                });
            }
        }
    }

    /// Add constraint: root package must be selected
    pub fn add_root_constraint(&mut self, root: &CogVar) {
        self.clauses.push(Clause {
            literals: List::from(vec![Literal {
                var: root.clone(),
                positive: true,
            }]),
            learned: false,
        });
    }

    /// Solve SAT problem using DPLL with conflict-driven clause learning
    pub fn solve(&mut self) -> Result<SatResolution> {
        // Reset state
        self.decision_stack.clear();
        let keys: List<_> = self.assignments.keys().cloned().collect();
        for var in keys {
            self.assignments.insert(var, None);
        }

        // Run DPLL
        if self.dpll(0) {
            // Solution found
            let mut selected = Map::new();

            for (var, assigned) in &self.assignments {
                if let Some(true) = assigned {
                    selected.insert(var.name.clone(), var.version.clone());
                }
            }

            Ok(SatResolution {
                selected,
                conflicts: List::new(),
            })
        } else {
            // No solution - extract conflicts
            let conflicts = self.extract_conflicts();

            Ok(SatResolution {
                selected: Map::new(),
                conflicts,
            })
        }
    }

    /// DPLL algorithm with backtracking
    fn dpll(&mut self, level: usize) -> bool {
        // Unit propagation
        loop {
            let unit_clause = self.find_unit_clause();

            if let Some((var, value)) = unit_clause {
                self.assign(var.clone(), value, level);

                if self.has_conflict() {
                    return false;
                }
            } else {
                break;
            }
        }

        // Check if all variables assigned
        if self.all_assigned() {
            return true;
        }

        // Pure literal elimination
        if let Some((var, value)) = self.find_pure_literal() {
            self.assign(var.clone(), value, level);
            return self.dpll(level);
        }

        // Choose unassigned variable (pick highest version first)
        if let Some(var) = self.choose_variable() {
            // Try true
            self.assign(var.clone(), true, level + 1);
            self.decision_stack.push(Decision {
                var: var.clone(),
                value: true,
                level: level + 1,
            });

            if self.dpll(level + 1) {
                return true;
            }

            // Backtrack
            self.backtrack(level);

            // Try false
            self.assign(var.clone(), false, level + 1);
            self.decision_stack.push(Decision {
                var: var.clone(),
                value: false,
                level: level + 1,
            });

            if self.dpll(level + 1) {
                return true;
            }

            // Backtrack again
            self.backtrack(level);

            false
        } else {
            false
        }
    }

    /// Find unit clause (clause with only one unassigned literal)
    fn find_unit_clause(&self) -> Option<(CogVar, bool)> {
        for clause in &self.clauses {
            let mut unassigned = None;
            let mut satisfied = false;

            for literal in &clause.literals {
                let assignment = self.assignments.get(&literal.var);

                match assignment {
                    Some(Some(true)) if literal.positive => {
                        satisfied = true;
                        break;
                    }
                    Some(Some(false)) if !literal.positive => {
                        satisfied = true;
                        break;
                    }
                    Some(None) => {
                        if unassigned.is_some() {
                            // More than one unassigned
                            unassigned = None;
                            break;
                        }
                        unassigned = Some((literal.var.clone(), literal.positive));
                    }
                    _ => {}
                }
            }

            if !satisfied && unassigned.is_some() {
                return unassigned;
            }
        }

        None
    }

    /// Find pure literal (appears only positive or only negative)
    fn find_pure_literal(&self) -> Option<(CogVar, bool)> {
        let mut positive_vars = Set::new();
        let mut negative_vars = Set::new();

        for clause in &self.clauses {
            for literal in &clause.literals {
                if self.assignments.get(&literal.var) == Some(&None) {
                    if literal.positive {
                        positive_vars.insert(literal.var.clone());
                    } else {
                        negative_vars.insert(literal.var.clone());
                    }
                }
            }
        }

        // Find variables that appear only positive or only negative
        for var in positive_vars.iter() {
            if !negative_vars.contains(var) {
                return Some((var.clone(), true));
            }
        }

        for var in negative_vars.iter() {
            if !positive_vars.contains(var) {
                return Some((var.clone(), false));
            }
        }

        None
    }

    /// Choose next unassigned variable
    fn choose_variable(&self) -> Option<CogVar> {
        // Choose highest version of any unassigned package
        let mut candidates: List<_> = self
            .assignments
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.clone())
            .collect();

        candidates.sort_by(|a, b| b.version.cmp(&a.version));
        candidates.first().cloned()
    }

    /// Assign value to variable
    fn assign(&mut self, var: CogVar, value: bool, _level: usize) {
        self.assignments.insert(var, Some(value));
    }

    /// Backtrack to level
    fn backtrack(&mut self, level: usize) {
        while let Some(decision) = self.decision_stack.last() {
            if decision.level <= level {
                break;
            }

            let decision = self.decision_stack.pop().unwrap();
            self.assignments.insert(decision.var, None);
        }
    }

    /// Check if all variables are assigned
    fn all_assigned(&self) -> bool {
        self.assignments.values().all(|v| v.is_some())
    }

    /// Check if there's a conflict (unsatisfied clause)
    fn has_conflict(&self) -> bool {
        for clause in &self.clauses {
            let mut satisfied = false;
            let mut all_assigned = true;

            for literal in &clause.literals {
                let assignment = self.assignments.get(&literal.var);

                match assignment {
                    Some(Some(true)) if literal.positive => satisfied = true,
                    Some(Some(false)) if !literal.positive => satisfied = true,
                    Some(None) => all_assigned = false,
                    _ => {}
                }
            }

            if all_assigned && !satisfied {
                return true;
            }
        }

        false
    }

    /// Extract conflict information for error reporting
    fn extract_conflicts(&self) -> List<Conflict> {
        let mut conflicts = List::new();

        // Group by package name
        let mut package_groups: Map<Text, List<CogVar>> = Map::new();

        for var in self.metadata_cache.keys() {
            package_groups
                .entry(var.name.clone())
                .or_default()
                .push(var.clone());
        }

        // Check each package for conflicts
        for (cog_name, versions) in package_groups {
            if versions.len() > 1 {
                // Multiple versions exist - check if any caused conflict
                let required_by = self.find_dependent_cogs(cog_name.as_str());

                if !required_by.is_empty() {
                    conflicts.push(Conflict {
                        cog: cog_name,
                        required_by,
                        versions: versions
                            .iter()
                            .map(|v| v.version.to_string().into())
                            .collect(),
                        reason: "Multiple incompatible versions required".into(),
                    });
                }
            }
        }

        conflicts
    }

    /// Find packages that depend on given package
    fn find_dependent_cogs(&self, cog_name: &str) -> List<Text> {
        let mut dependents = Set::new();

        for metadata in self.metadata_cache.values() {
            if metadata.dependencies.contains_key(&Text::from(cog_name)) {
                dependents.insert(metadata.name.clone());
            }
        }

        dependents.into_iter().collect()
    }
}

impl Default for SatResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CogVar {
    pub fn new(name: impl Into<Text>, version: Version) -> Self {
        Self {
            name: name.into(),
            version,
        }
    }
}
