// Cog dependency resolver: semver constraint solving, conflict resolution, topological ordering

use super::types::*;
use crate::error::{CliError, Result};
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
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

    /// Check for version conflicts
    pub fn check_conflicts(&self) -> Result<()> {
        for (cog_name, nodes) in &self.package_map {
            if nodes.len() > 1 {
                let versions: List<Text> = nodes
                    .iter()
                    .map(|&idx| self.graph[idx].version.to_string().into())
                    .collect();

                return Err(CliError::VersionConflict {
                    package: cog_name.to_string(),
                    required: versions[0].to_string(),
                    found: versions[1].to_string(),
                });
            }
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

/// Resolve version requirement
pub fn resolve_version(requirement: &str, available: &[Version]) -> Result<Version> {
    let req = VersionReq::parse(requirement)
        .map_err(|e| CliError::Custom(format!("Invalid version requirement: {}", e)))?;

    // Find highest matching version
    let mut matching: List<_> = available.iter().filter(|v| req.matches(v)).collect();

    matching.sort();

    matching.last().map(|v| (*v).clone()).ok_or_else(|| {
        CliError::DependencyNotFound(format!("No matching version for {}", requirement))
    })
}
