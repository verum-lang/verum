// Cog dependency resolver: semver constraint solving, conflict resolution, topological ordering

use super::resolver_errors::{RequirementOrigin, RequirerSpec, ResolverError};
use super::types::*;
use crate::error::{CliError, Result};
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::VecDeque;
use verum_common::{List, Map, Set, Text};

// Import and re-export semver types for tests
pub use semver::{Version, VersionReq};

/// Dependency resolver
pub struct DependencyResolver {
    /// Dependency graph
    pub graph: DiGraph<CogNode, DependencyEdge>,

    /// Cog name to node index
    pub package_map: Map<Text, List<NodeIndex>>,
}

/// Cog node in dependency graph
#[derive(Debug, Clone)]
pub struct CogNode {
    pub name: Text,
    pub version: Version,
    pub source: CogSource,
    pub features: Set<Text>,
}

/// Dependency edge
#[derive(Debug, Clone)]
pub struct DependencyEdge {
    pub version_req: VersionReq,
    pub features: List<Text>,
    pub optional: bool,
}

/// Resolved dependency
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    pub name: Text,
    pub version: Version,
    pub source: CogSource,
    pub features: Set<Text>,
    pub dependencies: List<Text>,
}

impl DependencyResolver {
    /// Create new resolver
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            package_map: Map::new(),
        }
    }

    /// Add package to resolver
    pub fn add_cog(
        &mut self,
        name: Text,
        version: Version,
        source: CogSource,
        features: Set<Text>,
    ) -> NodeIndex {
        let node = CogNode {
            name: name.clone(),
            version,
            source,
            features,
        };

        let index = self.graph.add_node(node);
        self.package_map
            .entry(name)
            .or_insert_with(List::new)
            .push(index);

        index
    }

    /// Add dependency between packages
    pub fn add_dependency(
        &mut self,
        from: NodeIndex,
        to: NodeIndex,
        version_req: VersionReq,
        features: List<Text>,
        optional: bool,
    ) {
        let edge = DependencyEdge {
            version_req,
            features,
            optional,
        };

        self.graph.add_edge(from, to, edge);
    }

    /// Resolve dependencies
    pub fn resolve(&self, root: NodeIndex) -> Result<List<ResolvedDependency>> {
        let mut resolved = List::new();
        let mut visited = Set::new();
        let mut queue = VecDeque::new();

        queue.push_back(root);

        while let Some(node_idx) = queue.pop_front() {
            if visited.contains(&node_idx) {
                continue;
            }

            visited.insert(node_idx);

            let node = &self.graph[node_idx];
            let mut dependencies = List::new();

            // Collect dependencies
            for neighbor in self.graph.neighbors_directed(node_idx, Direction::Outgoing) {
                let target = neighbor;
                let target_node = &self.graph[target];

                dependencies.push(target_node.name.clone());
                queue.push_back(target);
            }

            resolved.push(ResolvedDependency {
                name: node.name.clone(),
                version: node.version.clone(),
                source: node.source.clone(),
                features: node.features.clone(),
                dependencies,
            });
        }

        Ok(resolved)
    }

    /// Check for version conflicts. On failure, the returned
    /// [`CliError::VersionConflict`] carries a one-line summary; the
    /// richer multi-line diagnostic (with the full requirement chain)
    /// is available via [`Self::check_conflicts_structured`] for
    /// callers that want to print it directly.
    pub fn check_conflicts(&self) -> Result<()> {
        match self.check_conflicts_structured() {
            Ok(()) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Same as [`Self::check_conflicts`] but returns the structured
    /// [`ResolverError`] so callers can render the full chain via
    /// `Display` before converting (or in addition to converting) to
    /// [`CliError`].
    pub fn check_conflicts_structured(&self) -> std::result::Result<(), ResolverError> {
        for (cog_name, nodes) in &self.package_map {
            if nodes.len() <= 1 {
                continue;
            }
            let present_versions: Vec<String> = nodes
                .iter()
                .map(|&idx| self.graph[idx].version.to_string())
                .collect();

            // Walk incoming edges to every node in the duplicate
            // group. Each edge is a (requirer → cog_name) relationship
            // with its declared version requirement.
            let mut requirements: Vec<RequirementOrigin> = Vec::new();
            for &target in nodes.iter() {
                let edges = self.graph.edges_directed(target, Direction::Incoming);
                for edge in edges {
                    let source_idx = edge.source();
                    let source = &self.graph[source_idx];
                    let requirement = edge.weight().version_req.to_string();
                    requirements.push(RequirementOrigin {
                        requirer: Some(RequirerSpec {
                            name: source.name.to_string(),
                            version: source.version.to_string(),
                        }),
                        requirement,
                    });
                }
            }
            // Stable order — by requirer (name, version) for diff-
            // friendly diagnostics.
            requirements.sort_by(|a, b| match (&a.requirer, &b.requirer) {
                (Some(ra), Some(rb)) => (&ra.name, &ra.version, &a.requirement)
                    .cmp(&(&rb.name, &rb.version, &b.requirement)),
                _ => a.requirement.cmp(&b.requirement),
            });
            requirements.dedup();

            return Err(ResolverError::version_conflict(
                cog_name.to_string(),
                requirements,
                present_versions,
            ));
        }

        Ok(())
    }

    /// Detect cycles in dependency graph
    pub fn detect_cycles(&self) -> Option<List<Text>> {
        use petgraph::algo::kosaraju_scc;

        let sccs = kosaraju_scc(&self.graph);

        for scc in sccs {
            if scc.len() > 1 {
                let cycle: List<_> = scc
                    .iter()
                    .map(|&idx| self.graph[idx].name.clone())
                    .collect();
                return Some(cycle);
            }
        }

        None
    }

    /// Get dependency tree for visualization
    pub fn get_tree(&self, root: NodeIndex, max_depth: Option<usize>) -> DependencyTree {
        self.build_tree(root, 0, max_depth, &mut Set::new())
    }

    fn build_tree(
        &self,
        node: NodeIndex,
        depth: usize,
        max_depth: Option<usize>,
        visited: &mut Set<NodeIndex>,
    ) -> DependencyTree {
        let pkg = &self.graph[node];

        let mut children = List::new();

        if max_depth.is_none_or(|max| depth < max) && !visited.contains(&node) {
            visited.insert(node);

            for target in self.graph.neighbors_directed(node, Direction::Outgoing) {
                children.push(self.build_tree(target, depth + 1, max_depth, visited));
            }
        }

        DependencyTree {
            name: pkg.name.clone(),
            version: pkg.version.to_string().into(),
            features: pkg.features.iter().cloned().collect(),
            children,
        }
    }
}

/// Dependency tree for visualization
#[derive(Debug, Clone)]
pub struct DependencyTree {
    pub name: Text,
    pub version: Text,
    pub features: List<Text>,
    pub children: List<DependencyTree>,
}

impl DependencyTree {
    /// Print tree with indentation
    pub fn print(&self, indent: usize, last: bool, prefix: &str) {
        let connector = if last { "└─" } else { "├─" };
        let new_prefix = if last {
            format!("{}  ", prefix)
        } else {
            format!("{}│ ", prefix)
        };

        println!("{}{}─ {} {}", prefix, connector, self.name, self.version);

        if !self.features.is_empty() {
            println!("{}   features: [{}]", new_prefix, self.features.join(", "));
        }

        for (i, child) in self.children.iter().enumerate() {
            let is_last = i == self.children.len() - 1;
            child.print(indent + 2, is_last, &new_prefix);
        }
    }
}

/// Resolve a version requirement against an available-version list.
/// Returns the highest matching version. Failures carry a structured
/// [`ResolverError`] (mapped to [`CliError`] via `From`) so the caller
/// gets the available-version list, the parse-error reason, etc.
pub fn resolve_version(requirement: &str, available: &[Version]) -> Result<Version> {
    let req = VersionReq::parse(requirement).map_err(|e| {
        let err: CliError = ResolverError::invalid_requirement(requirement, e.to_string()).into();
        err
    })?;
    let mut matching: List<_> = available.iter().filter(|v| req.matches(v)).collect();
    matching.sort();
    match matching.last().map(|v| (*v).clone()) {
        Some(v) => Ok(v),
        None => {
            // Sort by descending version so the most-recent versions
            // are shown first when the list is truncated.
            let mut all: Vec<Version> = available.to_vec();
            all.sort();
            all.reverse();
            let names: Vec<String> = all.iter().map(|v| v.to_string()).collect();
            // Conventionally the package name isn't known here — the
            // caller should rewrap if it needs to surface the package.
            // For the standalone helper, "<unknown>" matches the legacy
            // shape ("No matching version for <requirement>").
            let err: CliError =
                ResolverError::no_matching_version("<unknown>", requirement, names).into();
            Err(err)
        }
    }
}

/// Resolve a version requirement, naming the package in the error
/// message. Preferred over [`resolve_version`] when the caller knows
/// the package name — produces a more useful error.
pub fn resolve_version_for(
    package: &str,
    requirement: &str,
    available: &[Version],
) -> Result<Version> {
    let req = VersionReq::parse(requirement).map_err(|e| {
        let err: CliError = ResolverError::invalid_requirement(requirement, e.to_string()).into();
        err
    })?;
    let mut matching: List<_> = available.iter().filter(|v| req.matches(v)).collect();
    matching.sort();
    match matching.last().map(|v| (*v).clone()) {
        Some(v) => Ok(v),
        None => {
            let mut all: Vec<Version> = available.to_vec();
            all.sort();
            all.reverse();
            let names: Vec<String> = all.iter().map(|v| v.to_string()).collect();
            let err: CliError =
                ResolverError::no_matching_version(package, requirement, names).into();
            Err(err)
        }
    }
}

#[cfg(test)]
mod resolver_diagnostic_tests {
    use super::*;

    fn ver(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn req(s: &str) -> VersionReq {
        VersionReq::parse(s).unwrap()
    }

    #[test]
    fn check_conflicts_structured_collects_full_chain() {
        let mut r = DependencyResolver::new();
        // package_map: foo has two versions in the graph; a@1 requires
        // foo ^1, b@2 requires foo ^2.
        let foo1 = r.add_cog(
            Text::from("foo"),
            ver("1.0.0"),
            CogSource::Registry { registry: Text::new(), version: Text::new() },
            Set::new(),
        );
        let foo2 = r.add_cog(
            Text::from("foo"),
            ver("2.0.0"),
            CogSource::Registry { registry: Text::new(), version: Text::new() },
            Set::new(),
        );
        let a = r.add_cog(
            Text::from("a"),
            ver("1.0.0"),
            CogSource::Registry { registry: Text::new(), version: Text::new() },
            Set::new(),
        );
        let b = r.add_cog(
            Text::from("b"),
            ver("2.0.0"),
            CogSource::Registry { registry: Text::new(), version: Text::new() },
            Set::new(),
        );
        r.graph.add_edge(
            a,
            foo1,
            DependencyEdge {
                version_req: req("^1"),
                features: List::new(),
                optional: false,
            },
        );
        r.graph.add_edge(
            b,
            foo2,
            DependencyEdge {
                version_req: req("^2"),
                features: List::new(),
                optional: false,
            },
        );

        let err = r.check_conflicts_structured().unwrap_err();
        match err {
            crate::registry::resolver_errors::ResolverError::VersionConflict {
                package,
                requirements,
                present_versions,
            } => {
                assert_eq!(package, "foo");
                let mut seen: Vec<String> = present_versions.clone();
                seen.sort();
                assert_eq!(seen, vec!["1.0.0".to_string(), "2.0.0".to_string()]);
                assert_eq!(requirements.len(), 2);
                let names_reqs: Vec<(String, String)> = requirements
                    .iter()
                    .map(|o| {
                        (
                            o.requirer.as_ref().unwrap().name.clone(),
                            o.requirement.clone(),
                        )
                    })
                    .collect();
                assert!(names_reqs.contains(&("a".to_string(), "^1".to_string())));
                assert!(names_reqs.contains(&("b".to_string(), "^2".to_string())));
            }
            other => panic!("expected VersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn resolve_version_for_lists_available_on_miss() {
        let avail = vec![ver("1.0.0"), ver("1.1.0"), ver("2.0.0")];
        let err = resolve_version_for("widget", "^99", &avail).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("widget"), "{s}");
        assert!(s.contains("^99"), "{s}");
        assert!(s.contains("available"), "{s}");
        assert!(s.contains("2.0.0"), "{s}");
    }

    #[test]
    fn resolve_version_for_returns_highest_match() {
        let avail = vec![ver("1.0.0"), ver("1.4.0"), ver("1.2.0")];
        let v = resolve_version_for("widget", "^1", &avail).unwrap();
        assert_eq!(v, ver("1.4.0"));
    }

    #[test]
    fn resolve_version_invalid_requirement_surfaces_position_aware_message() {
        let err = resolve_version_for("widget", "not-a-semver", &[]).unwrap_err();
        assert!(err.to_string().contains("invalid version requirement"));
    }
}
