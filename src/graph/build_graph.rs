use crate::graph::component::{ComponentId, ComponentType, PackageId, Visibility};
use crate::graph::{Component, ComponentRef, DependencyEdge, Target};
use petgraph::Direction;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Error, Debug, Clone)]
pub enum GraphError {
	#[error("Component '{name}' not found")]
	ComponentNotFound { name: String },

	#[error("Target '{name}' not found")]
	TargetNotFound { name: String },

	#[error("Circular dependency detected: {cycle}")]
	CircularDependency { cycle: String },

	#[error("Duplicate component: '{name}' for target '{target}'")]
	DuplicateComponent { name: String, target: String },

	#[error("Dependency conflict: multiple components produce '{output}'")]
	OutputConflict { output: String },

	#[error(
		"Visibility violation: '{from}' (package '{from_pkg}') cannot depend on '{to}' \
		(visibility: {visibility})"
	)]
	VisibilityViolation {
		from: String,
		from_pkg: String,
		to: String,
		visibility: String,
	},

	#[error("Constraint violation: component '{component}' is not compatible with target '{target}' (required: {constraints:?})")]
	ConstraintViolation {
		component: String,
		target: String,
		constraints: Vec<String>,
	},
}

// ---------------------------------------------------------------------------
// DOT output options
// ---------------------------------------------------------------------------

/// Options for `BuildGraph::output_dot`.
#[derive(Debug, Clone, Default)]
pub struct DotOptions {
	/// Include edge labels describing the dependency kind.
	pub include_edge_labels: bool,
	/// Color nodes by component type.
	pub color_by_type: bool,
}

impl DotOptions {
	pub fn default_pretty() -> Self {
		Self {
			include_edge_labels: true,
			color_by_type: true,
		}
	}
}

// ---------------------------------------------------------------------------
// BuildGraph
// ---------------------------------------------------------------------------

pub struct BuildGraph {
	graph: DiGraph<ComponentId, DependencyEdge>,
	components: HashMap<ComponentId, Component>,
	/// `(name, target_name) → ComponentId`
	component_index: HashMap<(String, String), ComponentId>,
	targets: HashMap<String, Target>,
	output_map: HashMap<PathBuf, ComponentId>,
	/// `name → [ComponentId]` (one component per target)
	name_to_ids: HashMap<String, Vec<ComponentId>>,
}

impl BuildGraph {
	pub fn new() -> Self {
		Self {
			graph: DiGraph::new(),
			components: HashMap::new(),
			component_index: HashMap::new(),
			targets: HashMap::new(),
			output_map: HashMap::new(),
			name_to_ids: HashMap::new(),
		}
	}

	// -----------------------------------------------------------------------
	// Targets
	// -----------------------------------------------------------------------

	pub fn add_target(&mut self, target: Target) -> Result<(), GraphError> {
		if self.targets.contains_key(&target.name) {
			return Ok(());
		}
		self.targets.insert(target.name.clone(), target);
		Ok(())
	}

	pub fn get_target(&self, name: &str) -> Option<&Target> {
		self.targets.get(name)
	}

	pub fn targets(&self) -> impl Iterator<Item = &Target> {
		self.targets.values()
	}

	// -----------------------------------------------------------------------
	// Components
	// -----------------------------------------------------------------------

	pub fn add_component(&mut self, component: Component) -> Result<ComponentId, GraphError> {
		let key = (component.name.clone(), component.target_name.clone());

		let id = component.id;
		self.components.insert(id, component.clone());

		if self.component_index.contains_key(&key) {
			return Err(GraphError::DuplicateComponent {
				name: component.name.clone(),
				target: component.target_name.clone(),
			});
		}

		for output in &component.outputs {
			if let Some(_existing) = self.output_map.get(output) {
				return Err(GraphError::OutputConflict {
					output: output.display().to_string(),
				});
			}
		}

		let id = component.id;
		let _node_idx = self.graph.add_node(id);

		self.components.insert(id, component.clone());
		self.component_index.insert(key, id);
		self.name_to_ids.entry(component.name.clone()).or_default().push(id);

		for output in &component.outputs {
			self.output_map.insert(output.clone(), id);
		}

		Ok(id)
	}

	pub fn add_dependency(
		&mut self,
		from: ComponentId,
		to: ComponentRef,
		edge: DependencyEdge,
	) -> Result<(), GraphError> {
		if let Some(comp) = self.components.get_mut(&from) {
			comp.dependencies.push((to, edge));
			Ok(())
		} else {
			Err(GraphError::ComponentNotFound {
				name: format!("{:?}", from),
			})
		}
	}

	/// Check whether component `from` is compatible with the target platform
	/// and any constraints declared by `to`.
	pub fn check_constraints(
		&self,
		from: ComponentId,
		to: ComponentId,
		registry: Option<&crate::platform::PlatformRegistry>,
	) -> Result<(), GraphError> {
		let from_comp = self.components.get(&from).ok_or_else(|| GraphError::ComponentNotFound {
			name: format!("{:?}", from),
		})?;
		let to_comp = self.components.get(&to).ok_or_else(|| GraphError::ComponentNotFound {
			name: format!("{:?}", to),
		})?;

		// Simple implementation: if 'to' has compatible_with requirements,
		// and they are not satisfied by the target, fail.
		// For now, we just check if any constraint is met by the target name
		// as a placeholder for a full platform/constraint engine.
		if !to_comp.compatible_with.is_empty() {
			let target = &from_comp.target_name;
			let satisfied = if let Some(reg) = registry {
				if let Some(platform) = reg.get(target) {
					// Use registry for robust check
					to_comp.compatible_with.iter().all(|c| {
						platform.constraint_values.contains(&c.0)
							|| platform.name == c.0
							|| platform.target.triple.contains(&c.0)
							|| c.0 == "all"
					})
				} else {
					// Fallback to simple string match if platform not found in registry
					to_comp.compatible_with.iter().any(|c| target.contains(&c.0) || c.0 == "all")
				}
			} else {
				// Fallback to simple string match if no registry provided
				to_comp.compatible_with.iter().any(|c| target.contains(&c.0) || c.0 == "all")
			};

			if !satisfied {
				return Err(GraphError::ConstraintViolation {
					component: to_comp.name.clone(),
					target: target.clone(),
					constraints: to_comp.compatible_with.iter().map(|c| c.0.clone()).collect(),
				});
			}
		}

		Ok(())
	}

	pub fn add_dependency_by_ids(
		&mut self,
		from: ComponentId,
		to: ComponentId,
		edge: DependencyEdge,
	) -> Result<(), GraphError> {
		let from_idx = self.find_node_index(from)?;
		let to_idx = self.find_node_index(to)?;
		self.graph.add_edge(to_idx, from_idx, edge);
		Ok(())
	}

	// -----------------------------------------------------------------------
	// Lookups
	// -----------------------------------------------------------------------

	pub fn get_component(&self, id: ComponentId) -> Option<&Component> {
		self.components.get(&id)
	}

	pub fn get_component_by_name(&self, name: &str, target: &str) -> Option<&Component> {
		self.component_index
			.get(&(name.to_string(), target.to_string()))
			.and_then(|id| self.components.get(id))
	}

	pub fn component_ids(&self) -> impl Iterator<Item = ComponentId> + '_ {
		self.components.keys().copied()
	}

	pub fn component_count(&self) -> usize {
		self.components.len()
	}

	pub fn target_count(&self) -> usize {
		self.targets.len()
	}

	pub fn edge_count(&self) -> usize {
		self.graph.edge_count()
	}

	pub fn output_map(&self) -> &HashMap<PathBuf, ComponentId> {
		&self.output_map
	}

	// -----------------------------------------------------------------------
	// Dependency queries
	// -----------------------------------------------------------------------

	/// All components that this component directly depends on.
	pub fn dependencies_of(&self, id: ComponentId) -> Vec<ComponentId> {
		let Ok(idx) = self.find_node_index(id) else { return Vec::new() };
		self.graph
			.neighbors_directed(idx, Direction::Incoming)
			.map(|n| self.graph[n])
			.collect()
	}

	/// All components that directly depend on this component.
	///
	/// This is the reverse of `dependencies_of`.  Also exposed as
	/// `dependents_of` for internal use.
	pub fn reverse_dependencies(&self, id: ComponentId) -> Vec<ComponentId> {
		self.dependents_of(id)
	}

	/// Alias for `reverse_dependencies`.
	pub fn dependents_of(&self, id: ComponentId) -> Vec<ComponentId> {
		let Ok(idx) = self.find_node_index(id) else { return Vec::new() };
		self.graph
			.neighbors_directed(idx, Direction::Outgoing)
			.map(|n| self.graph[n])
			.collect()
	}

	/// Full transitive dependency closure of a component.
	pub fn transitive_dependencies(&self, id: ComponentId) -> Vec<ComponentId> {
		let mut result = Vec::new();
		let mut visited = HashSet::new();
		self.collect_transitive_deps(id, &mut result, &mut visited);
		result
	}

	fn collect_transitive_deps(
		&self,
		id: ComponentId,
		result: &mut Vec<ComponentId>,
		visited: &mut HashSet<ComponentId>,
	) {
		if visited.contains(&id) {
			return;
		}
		visited.insert(id);
		for dep_id in self.dependencies_of(id) {
			self.collect_transitive_deps(dep_id, result, visited);
			if !result.contains(&dep_id) {
				result.push(dep_id);
			}
		}
	}

	/// All simple paths in the dependency graph from `from` to `to`.
	///
	/// `from` is the dependent (e.g. a binary), `to` is the transitive
	/// dependency (e.g. a base library).  The path is expressed as an ordered
	/// list of `ComponentId`s starting at `from` and ending at `to`.
	///
	/// Returns an empty `Vec` if no such path exists.  Paths are found by
	/// iterative DFS following `dependencies_of` (incoming edges).
	pub fn all_paths(&self, from: ComponentId, to: ComponentId) -> Vec<Vec<ComponentId>> {
		let mut results = Vec::new();
		let mut path = vec![from];
		let mut visited = HashSet::new();
		visited.insert(from);
		self.dfs_all_paths(from, to, &mut path, &mut visited, &mut results);
		results
	}

	fn dfs_all_paths(
		&self,
		current: ComponentId,
		target: ComponentId,
		path: &mut Vec<ComponentId>,
		visited: &mut HashSet<ComponentId>,
		results: &mut Vec<Vec<ComponentId>>,
	) {
		if current == target {
			results.push(path.clone());
			return;
		}

		for dep_id in self.dependencies_of(current) {
			if visited.contains(&dep_id) {
				continue;
			}
			visited.insert(dep_id);
			path.push(dep_id);
			self.dfs_all_paths(dep_id, target, path, visited, results);
			path.pop();
			visited.remove(&dep_id);
		}
	}

	// -----------------------------------------------------------------------
	// Visibility checking
	// -----------------------------------------------------------------------

	/// Check whether component `from` is permitted to depend on `to`.
	///
	/// Returns `Ok(())` if the dependency is allowed, or a
	/// `GraphError::VisibilityViolation` with a human-readable explanation
	/// if it is not.
	pub fn check_visibility(
		&self,
		from: ComponentId,
		to: ComponentId,
	) -> Result<(), GraphError> {
		let from_comp = self.components.get(&from).ok_or_else(|| GraphError::ComponentNotFound {
			name: format!("{:?}", from),
		})?;
		let to_comp = self.components.get(&to).ok_or_else(|| GraphError::ComponentNotFound {
			name: format!("{:?}", to),
		})?;

		log::debug!(
			"Checking visibility: '{}' ({}) -> '{}' ({}) [vis: {}]",
			from_comp.name, from_comp.package,
			to_comp.name, to_comp.package,
			to_comp.visibility
		);

		if to_comp.visibility.allows(&from_comp.package, &to_comp.package) {
			Ok(())
		} else {
			Err(GraphError::VisibilityViolation {
				from: from_comp.name.clone(),
				from_pkg: from_comp.package.to_string(),
				to: to_comp.name.clone(),
				visibility: to_comp.visibility.to_string(),
			})
		}
	}

	/// Check visibility for every declared dependency in the graph and collect
	/// all violations.  Returns `Ok(())` if all dependencies are allowed.
	pub fn check_all_visibility(&self) -> Result<(), Vec<GraphError>> {
		let mut violations = Vec::new();

		for edge_idx in self.graph.edge_indices() {
			let (dep_idx, dependent_idx) = self.graph.edge_endpoints(edge_idx).unwrap();
			let dep_id = self.graph[dep_idx];
			let dependent_id = self.graph[dependent_idx];

			// `dependent` depends on `dep`: check if that is allowed
			if let Err(e) = self.check_visibility(dependent_id, dep_id) {
				violations.push(e);
			}
		}

		if violations.is_empty() {
			Ok(())
		} else {
			Err(violations)
		}
	}

	/// Resolve all deferred dependencies and add them to the graph.
	/// This should be called after all FORGE files have been loaded.
	pub fn resolve_all_dependencies(
		&mut self,
		registry: Option<&crate::platform::PlatformRegistry>,
	) -> Result<(), Vec<GraphError>> {
		let mut errors = Vec::new();

		let component_ids: Vec<_> = self.components.keys().copied().collect();

		for from_id in component_ids {
			let (name, dependencies) = {
				let comp = self.components.get(&from_id).unwrap();
				(comp.name.clone(), comp.dependencies.clone())
			};

			for (to_ref, edge) in dependencies {
				// We need to clone to_ref because it's used in both resolution and error reporting
				let to_ref_owned: crate::graph::ComponentRef = to_ref.clone();
				
				match self.resolve_component_ref(to_ref_owned) {
					Ok(to_id) => {
						// 1. Check visibility
						if let Err(e) = self.check_visibility(from_id, to_id) {
							errors.push(e);
						}

						// 2. Check platform constraints
						if let Err(e) = self.check_constraints(from_id, to_id, registry) {
							errors.push(e);
						}

						// 3. Add edge to graph
						if let (Ok(from_idx), Ok(to_idx)) =
							(self.find_node_index(from_id), self.find_node_index(to_id))
						{
							self.graph.add_edge(to_idx, from_idx, edge);
						}
					}
					Err(_e) => {
						errors.push(GraphError::ComponentNotFound {
							name: match to_ref {
								crate::graph::ComponentRef::Local { name } => name,
								crate::graph::ComponentRef::WithTarget { name, target } => {
									format!("{}:{}", name, target)
								}
							},
						});
					}
				}
			}
		}

		if errors.is_empty() {
			Ok(())
		} else {
			Err(errors)
		}
	}

	// -----------------------------------------------------------------------
	// Pattern matching
	// -----------------------------------------------------------------------

	/// Return all components whose name matches the given glob pattern.
	///
	/// Uses `globset` for full glob semantics (`*`, `?`, `[...]`, `**`).
	pub fn components_matching(&self, pattern: &str) -> Vec<ComponentId> {
		use globset::{Glob, GlobSetBuilder};

		let Ok(glob) = Glob::new(pattern) else {
			return Vec::new();
		};
		let mut builder = GlobSetBuilder::new();
		builder.add(glob);
		let Ok(set) = builder.build() else {
			return Vec::new();
		};

		self.components.values().filter(|c| set.is_match(&c.name)).map(|c| c.id).collect()
	}

	/// Return all components of a specific `kind_name` (e.g. `"binary"`,
	/// `"library"`, `"test"`, `"module"`, `"custom"`).
	pub fn components_of_kind(&self, kind: &str) -> Vec<ComponentId> {
		self.components
			.values()
			.filter(|c| c.component_type.kind_name() == kind)
			.map(|c| c.id)
			.collect()
	}

	/// Return all components in the given package.
	pub fn components_in_package(&self, package: &PackageId) -> Vec<ComponentId> {
		self.components
			.values()
			.filter(|c| &c.package == package)
			.map(|c| c.id)
			.collect()
	}

	// -----------------------------------------------------------------------
	// Topological / scheduling
	// -----------------------------------------------------------------------

	pub fn topological_order(&self) -> Result<Vec<ComponentId>, GraphError> {
		match toposort(&self.graph, None) {
			Ok(indices) => Ok(indices.into_iter().map(|idx| self.graph[idx]).collect()),
			Err(cycle) => {
				let cycle_path = self.format_cycle(cycle);
				Err(GraphError::CircularDependency { cycle: cycle_path })
			}
		}
	}

	pub fn find_cycles(&self) -> Vec<Vec<ComponentId>> {
		let mut cycles = Vec::new();
		let mut visited = HashSet::new();
		let mut rec_stack = HashSet::new();
		let mut path = Vec::new();

		for node_idx in self.graph.node_indices() {
			self.find_cycles_dfs(node_idx, &mut visited, &mut rec_stack, &mut path, &mut cycles);
		}

		cycles
	}

	pub fn execution_batches(&self) -> Result<Vec<Vec<ComponentId>>, GraphError> {
		let mut batches: Vec<Vec<ComponentId>> = Vec::new();
		let mut completed: HashSet<ComponentId> = HashSet::new();
		let mut remaining: HashSet<ComponentId> =
			self.graph.node_indices().map(|idx| self.graph[idx]).collect();

		while !remaining.is_empty() {
			let mut batch = Vec::new();

			for &id in &remaining {
				let deps = self.dependencies_of(id);
				if deps.iter().all(|dep| completed.contains(dep)) {
					batch.push(id);
				}
			}

			if batch.is_empty() && !remaining.is_empty() {
				let cycle = self.find_cycles().first().cloned().unwrap_or_default();
				return Err(GraphError::CircularDependency {
					cycle: cycle
						.iter()
						.filter_map(|id| self.components.get(id).map(|c| c.name.clone()))
						.collect::<Vec<_>>()
						.join(" -> "),
				});
			}

			for id in &batch {
				remaining.remove(id);
				completed.insert(*id);
			}

			batches.push(batch);
		}

		Ok(batches)
	}

	// -----------------------------------------------------------------------
	// Filtering
	// -----------------------------------------------------------------------

	pub fn filter_by_target(&self, targets: &[String]) -> Vec<ComponentId> {
		if targets.is_empty() {
			return self.components.keys().copied().collect();
		}
		self.components
			.values()
			.filter(|c| targets.contains(&c.target_name))
			.map(|c| c.id)
			.collect()
	}

	pub fn filter_by_component(&self, component_names: &[String]) -> Vec<ComponentId> {
		if component_names.is_empty() {
			return self.components.keys().copied().collect();
		}
		component_names
			.iter()
			.filter_map(|name| self.name_to_ids.get(name))
			.flatten()
			.copied()
			.collect()
	}

	pub fn filter(
		&self,
		targets: &[String],
		component_names: &[String],
	) -> Vec<ComponentId> {
		let by_target: HashSet<_> = self.filter_by_target(targets).into_iter().collect();
		let by_component: HashSet<_> =
			self.filter_by_component(component_names).into_iter().collect();

		if targets.is_empty() && component_names.is_empty() {
			self.components.keys().copied().collect()
		} else if targets.is_empty() {
			by_component.into_iter().collect()
		} else if component_names.is_empty() {
			by_target.into_iter().collect()
		} else {
			by_target.intersection(&by_component).copied().collect()
		}
	}

	// -----------------------------------------------------------------------
	// DOT output
	// -----------------------------------------------------------------------

	/// Render the build graph as a Graphviz DOT string.
	///
	/// ```bash
	/// forge graph --output dot > graph.dot
	/// xdot graph.dot
	/// ```
	pub fn output_dot(&self, opts: &DotOptions) -> String {
		self.output_dot_subset(&self.components.keys().copied().collect::<HashSet<_>>(), opts)
	}

	pub fn output_dot_subset(&self, ids: &HashSet<ComponentId>, opts: &DotOptions) -> String {
		let mut dot = String::from("digraph forge {\n");
		dot.push_str("    rankdir=LR;\n");
		dot.push_str("    node [shape=box fontname=\"monospace\" style=filled];\n");
		dot.push_str("    edge [fontname=\"monospace\" fontsize=10];\n");
		dot.push('\n');

		// Nodes
		for id in ids {
			if let Some(comp) = self.components.get(id) {
				let (color, shape) = if opts.color_by_type {
					match comp.component_type {
						ComponentType::Library { .. } => ("#4A90D9", "box"),
						ComponentType::Binary => ("#5CB85C", "oval"),
						ComponentType::Test { .. } => ("#F0AD4E", "diamond"),
						ComponentType::Module { .. } => ("#9B59B6", "box"),
						ComponentType::Custom { .. } => ("#95A5A6", "box"),
					}
				} else {
					("#CCCCCC", "box")
				};

				let vis_suffix = match &comp.visibility {
					Visibility::Public => "",
					Visibility::Package => " 📦",
					Visibility::Private => " 🔒",
					Visibility::Restricted(_) => " 🔐",
				};

				let label = format!("{}:{}{}", comp.target_name, comp.name, vis_suffix);
				dot.push_str(&format!(
					"    \"{}:{}\" [label={:?} fillcolor={:?} shape=\"{}\"];\n",
					comp.target_name, comp.name, label, color, shape
				));
			}
		}

		dot.push('\n');

		// Edges
		for edge_idx in self.graph.edge_indices() {
			let (dep_idx, dependent_idx) = self.graph.edge_endpoints(edge_idx).unwrap();
			let dep_id = self.graph[dep_idx];
			let dependent_id = self.graph[dependent_idx];

			let (Some(dep_comp), Some(dependent_comp)) = (
				self.components.get(&dep_id),
				self.components.get(&dependent_id),
			) else {
				continue;
			};

			let edge_data = &self.graph[edge_idx];
			let edge_attrs = if opts.include_edge_labels && edge_data != &DependencyEdge::Hard {
				let label = edge_data.dot_label();
				let style = match edge_data {
					DependencyEdge::OrderOnly => "dashed",
					DependencyEdge::ModuleImport => "bold",
					DependencyEdge::ProcMacro | DependencyEdge::BuildScript => "dotted",
					DependencyEdge::Transition(_) => "bold",
					DependencyEdge::Hard => "solid",
				};
				format!("[label={:?} style=\"{}\"]", label, style)
			} else {
				String::new()
			};

			// Arrow goes from dependent to dependency (visual reads: "A depends on B")
			dot.push_str(&format!(
				"    \"{}:{}\" -> \"{}:{}\" {};\n",
				dependent_comp.target_name,
				dependent_comp.name,
				dep_comp.target_name,
				dep_comp.name,
				edge_attrs,
			));
		}

		dot.push_str("}\n");
		dot
	}

	// -----------------------------------------------------------------------
	// Private helpers
	// -----------------------------------------------------------------------

	fn find_node_index(&self, id: ComponentId) -> Result<NodeIndex, GraphError> {
		self.graph
			.node_indices()
			.find(|&idx| self.graph[idx] == id)
			.ok_or_else(|| GraphError::ComponentNotFound {
				name: format!("{:?}", id),
			})
	}

	fn resolve_component_ref(&self, comp_ref: ComponentRef) -> Result<ComponentId, GraphError> {
		match comp_ref {
			ComponentRef::Local { name } => self
				.name_to_ids
				.get(&name)
				.and_then(|ids| ids.first().copied())
				.ok_or_else(|| GraphError::ComponentNotFound { name }),
			ComponentRef::WithTarget { name, target } => self
				.component_index
				.get(&(name.clone(), target.clone()))
				.copied()
				.ok_or_else(|| GraphError::ComponentNotFound {
					name: format!("{}:{}", name, target),
				}),
		}
	}

	fn format_cycle(&self, cycle: petgraph::algo::Cycle<NodeIndex>) -> String {
		let node_id = cycle.node_id();
		if let Some(component) = self.components.get(&self.graph[node_id]) {
			let mut path = vec![component.name.clone()];
			let current = node_id;
			let mut visited = HashSet::new();
			visited.insert(current);

			for neighbor in self.graph.neighbors_directed(current, Direction::Outgoing) {
				if visited.contains(&neighbor) {
					if let Some(comp) = self.components.get(&self.graph[neighbor]) {
						path.push(comp.name.clone());
					}
					break;
				}
				visited.insert(neighbor);
			}

			path.join(" -> ")
		} else {
			"unknown cycle".to_string()
		}
	}

	fn find_cycles_dfs(
		&self,
		node: NodeIndex,
		visited: &mut HashSet<NodeIndex>,
		rec_stack: &mut HashSet<NodeIndex>,
		path: &mut Vec<ComponentId>,
		cycles: &mut Vec<Vec<ComponentId>>,
	) {
		if rec_stack.contains(&node) {
			if let Some(start_idx) = path
				.iter()
				.position(|&id| self.graph.node_indices().any(|idx| self.graph[idx] == id && idx == node))
			{
				cycles.push(path[start_idx..].to_vec());
			}
			return;
		}

		if visited.contains(&node) {
			return;
		}

		visited.insert(node);
		rec_stack.insert(node);
		path.push(self.graph[node]);

		for neighbor in self.graph.neighbors_directed(node, Direction::Outgoing) {
			self.find_cycles_dfs(neighbor, visited, rec_stack, path, cycles);
		}

		rec_stack.remove(&node);
		path.pop();
	}
}

impl Default for BuildGraph {
	fn default() -> Self {
		Self::new()
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::graph::component::{ConstraintRef, PackageId, Visibility};
	use crate::graph::target::Target;

	fn make_lib(name: &str, target: &str) -> Component {
		Component::library(name, target)
	}

	fn make_bin(name: &str, target: &str) -> Component {
		Component::binary(name, target)
	}

	// -----------------------------------------------------------------------
	// Existing tests (unchanged)
	// -----------------------------------------------------------------------

	#[test]
	fn test_add_target() {
		let mut graph = BuildGraph::new();
		let target = Target::new("linux_x64", "x86_64-unknown-linux-gnu");
		assert!(graph.add_target(target).is_ok());
		assert_eq!(graph.target_count(), 1);
	}

	#[test]
	fn test_add_component() {
		let mut graph = BuildGraph::new();
		let component = make_lib("mylib", "linux_x64");
		let id = graph.add_component(component).unwrap();
		assert_eq!(graph.component_count(), 1);
		assert!(graph.get_component(id).is_some());
	}

	#[test]
	fn test_add_dependency() {
		let mut graph = BuildGraph::new();
		let lib = make_lib("mylib", "linux_x64");
		let lib_id = graph.add_component(lib).unwrap();
		let bin = make_bin("myapp", "linux_x64");
		let bin_id = graph.add_component(bin).unwrap();
		graph.add_dependency(bin_id, ComponentRef::new("mylib"), DependencyEdge::Hard).unwrap();
		let deps = graph.dependencies_of(bin_id);
		assert_eq!(deps.len(), 1);
		assert_eq!(deps[0], lib_id);
	}

	#[test]
	fn test_transitive_dependencies() {
		let mut graph = BuildGraph::new();
		let c = make_lib("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();
		let b = make_lib("b", "linux_x64");
		let b_id = graph.add_component(b).unwrap();
		graph.add_dependency(b_id, ComponentRef::new("c"), DependencyEdge::Hard).unwrap();
		let a = make_bin("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();
		graph.add_dependency(a_id, ComponentRef::new("b"), DependencyEdge::Hard).unwrap();
		let transitive = graph.transitive_dependencies(a_id);
		assert_eq!(transitive.len(), 2);
		assert!(transitive.contains(&b_id));
		assert!(transitive.contains(&c_id));
	}

	#[test]
	fn test_topological_order() {
		let mut graph = BuildGraph::new();
		let a = make_lib("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();
		let b = make_lib("b", "linux_x64");
		let b_id = graph.add_component(b).unwrap();
		graph.add_dependency(b_id, ComponentRef::new("a"), DependencyEdge::Hard).unwrap();
		let c = make_bin("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();
		graph.add_dependency(c_id, ComponentRef::new("b"), DependencyEdge::Hard).unwrap();
		let order = graph.topological_order().unwrap();
		let a_pos = order.iter().position(|&id| id == a_id).unwrap();
		let b_pos = order.iter().position(|&id| id == b_id).unwrap();
		let c_pos = order.iter().position(|&id| id == c_id).unwrap();
		assert!(a_pos < b_pos);
		assert!(b_pos < c_pos);
	}

	#[test]
	fn test_execution_batches() {
		let mut graph = BuildGraph::new();
		let a = make_lib("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();
		let b = make_lib("b", "linux_x64");
		let b_id = graph.add_component(b).unwrap();
		let c = make_bin("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();
		graph.add_dependency(c_id, ComponentRef::new("a"), DependencyEdge::Hard).unwrap();
		graph.add_dependency(c_id, ComponentRef::new("b"), DependencyEdge::Hard).unwrap();
		let batches = graph.execution_batches().unwrap();
		assert_eq!(batches.len(), 2);
		assert_eq!(batches[0].len(), 2);
		assert_eq!(batches[1].len(), 1);
		let first: HashSet<_> = batches[0].iter().copied().collect();
		assert!(first.contains(&a_id));
		assert!(first.contains(&b_id));
		assert_eq!(batches[1][0], c_id);
	}

	// -----------------------------------------------------------------------
	// New tests
	// -----------------------------------------------------------------------

	#[test]
	fn test_reverse_dependencies() {
		let mut graph = BuildGraph::new();
		let lib_id = graph.add_component(make_lib("lib", "t")).unwrap();
		let bin_id = graph.add_component(make_bin("bin", "t")).unwrap();
		graph.add_dependency(bin_id, ComponentRef::new("lib"), DependencyEdge::Hard).unwrap();

		// lib's reverse-deps should include bin
		let rdeps = graph.reverse_dependencies(lib_id);
		assert_eq!(rdeps.len(), 1);
		assert_eq!(rdeps[0], bin_id);

		// bin has no reverse-deps (nobody depends on it)
		assert!(graph.reverse_dependencies(bin_id).is_empty());
	}

	#[test]
	fn test_all_paths_diamond() {
		// Diamond: app → (left, right) → base
		let mut graph = BuildGraph::new();
		let base_id = graph.add_component(make_lib("base", "t")).unwrap();
		let left_id = graph.add_component(make_lib("left", "t")).unwrap();
		let right_id = graph.add_component(make_lib("right", "t")).unwrap();
		let app_id = graph.add_component(make_bin("app", "t")).unwrap();

		graph.add_dependency(left_id, ComponentRef::new("base"), DependencyEdge::Hard).unwrap();
		graph.add_dependency(right_id, ComponentRef::new("base"), DependencyEdge::Hard).unwrap();
		graph.add_dependency(app_id, ComponentRef::new("left"), DependencyEdge::Hard).unwrap();
		graph.add_dependency(app_id, ComponentRef::new("right"), DependencyEdge::Hard).unwrap();

		let paths = graph.all_paths(app_id, base_id);
		// Should find exactly two paths: app→left→base and app→right→base
		assert_eq!(paths.len(), 2, "expected 2 paths, got {:?}", paths.len());
		for path in &paths {
			assert_eq!(*path.first().unwrap(), app_id);
			assert_eq!(*path.last().unwrap(), base_id);
			assert!(path.contains(&left_id) || path.contains(&right_id));
		}

		// Paths to a component with no connection
		let unrelated_id = graph.add_component(make_lib("unrelated", "t")).unwrap();
		assert!(graph.all_paths(app_id, unrelated_id).is_empty());

		// Path from a node to itself
		let self_paths = graph.all_paths(app_id, app_id);
		assert_eq!(self_paths.len(), 1);
		assert_eq!(self_paths[0], vec![app_id]);
	}

	#[test]
	fn test_check_visibility_public() {
		let mut graph = BuildGraph::new();
		let pkg_a = PackageId::new("/workspace/a");
		let pkg_b = PackageId::new("/workspace/b");

		let mut lib = make_lib("mylib", "t");
		lib.package = pkg_b.clone();
		lib.visibility = Visibility::Public;
		let lib_id = graph.add_component(lib).unwrap();

		let mut bin = make_bin("myapp", "t");
		bin.package = pkg_a.clone();
		let bin_id = graph.add_component(bin).unwrap();

		assert!(graph.check_visibility(bin_id, lib_id).is_ok());
	}

	#[test]
	fn test_check_visibility_private_same_package() {
		let mut graph = BuildGraph::new();
		let pkg = PackageId::new("/workspace/foo");

		let mut lib = make_lib("internal", "t");
		lib.package = pkg.clone();
		lib.visibility = Visibility::Private;
		let lib_id = graph.add_component(lib).unwrap();

		let mut bin = make_bin("bin", "t");
		bin.package = pkg.clone();
		let bin_id = graph.add_component(bin).unwrap();

		// Same package → allowed
		assert!(graph.check_visibility(bin_id, lib_id).is_ok());
	}

	#[test]
	fn test_check_visibility_private_cross_package() {
		let mut graph = BuildGraph::new();
		let pkg_a = PackageId::new("/workspace/a");
		let pkg_b = PackageId::new("/workspace/b");

		let mut lib = make_lib("secret", "t");
		lib.package = pkg_b.clone();
		lib.visibility = Visibility::Private;
		let lib_id = graph.add_component(lib).unwrap();

		let mut bin = make_bin("outsider", "t");
		bin.package = pkg_a.clone();
		let bin_id = graph.add_component(bin).unwrap();

		// Different package → violation
		assert!(graph.check_visibility(bin_id, lib_id).is_err());
	}

	#[test]
	fn test_check_visibility_restricted() {
		let mut graph = BuildGraph::new();
		let pkg_allowed = PackageId::new("lib/allowed");
		let pkg_denied = PackageId::new("lib/denied");
		let pkg_owner = PackageId::new("lib/core");

		let mut lib = make_lib("core_lib", "t");
		lib.package = pkg_owner.clone();
		lib.visibility = Visibility::Restricted(vec!["//lib/allowed/...".into()]);
		let lib_id = graph.add_component(lib).unwrap();

		let mut allowed_bin = make_bin("allowed_bin", "t");
		allowed_bin.package = pkg_allowed.clone();
		let allowed_id = graph.add_component(allowed_bin).unwrap();

		let mut denied_bin = make_bin("denied_bin", "t");
		denied_bin.package = pkg_denied.clone();
		let denied_id = graph.add_component(denied_bin).unwrap();

		assert!(graph.check_visibility(allowed_id, lib_id).is_ok());
		assert!(graph.check_visibility(denied_id, lib_id).is_err());
	}

	#[test]
	fn test_components_matching() {
		let mut graph = BuildGraph::new();
		graph.add_component(make_lib("math_core", "t")).unwrap();
		graph.add_component(make_lib("math_utils", "t")).unwrap();
		graph.add_component(make_bin("my_app", "t")).unwrap();
		graph.add_component(make_lib("unrelated", "t")).unwrap();

		let matches = graph.components_matching("math_*");
		assert_eq!(matches.len(), 2);

		let all = graph.components_matching("*");
		assert_eq!(all.len(), 4);

		let none = graph.components_matching("does_not_exist");
		assert!(none.is_empty());
	}

	#[test]
	fn test_output_dot_contains_expected_strings() {
		let mut graph = BuildGraph::new();
		let lib_id = graph.add_component(make_lib("mylib", "linux_x64")).unwrap();
		let bin_id = graph.add_component(make_bin("myapp", "linux_x64")).unwrap();
		graph.add_dependency(bin_id, ComponentRef::new("mylib"), DependencyEdge::Hard).unwrap();

		let dot = graph.output_dot(&DotOptions::default_pretty());

		assert!(dot.contains("digraph forge"), "missing digraph header");
		assert!(dot.contains("mylib"), "missing library node");
		assert!(dot.contains("myapp"), "missing binary node");
		assert!(dot.contains("->"), "missing edge");
		assert!(dot.contains("rankdir"), "missing layout hint");
	}

	#[test]
	fn test_dependency_edge_variants() {
		use crate::graph::dependency::ConfigTransition;

		let edges = [
			DependencyEdge::Hard,
			DependencyEdge::OrderOnly,
			DependencyEdge::ModuleImport,
			DependencyEdge::ProcMacro,
			DependencyEdge::BuildScript,
			DependencyEdge::Transition(ConfigTransition::Host),
			DependencyEdge::Transition(ConfigTransition::Exec),
			DependencyEdge::Transition(ConfigTransition::Target),
		];

		// All variants should be Copy and serializable
		for edge in &edges {
			let cloned = *edge; // Copy
			assert_eq!(*edge, cloned);
		}

		assert!(DependencyEdge::ProcMacro.is_proc_macro());
		assert!(DependencyEdge::BuildScript.is_build_script());
		assert!(DependencyEdge::Transition(ConfigTransition::Host).is_transition());
		assert_eq!(
			DependencyEdge::Transition(ConfigTransition::Host).transition(),
			Some(ConfigTransition::Host)
		);
	}

	#[test]
	fn test_new_component_fields() {
		let comp = Component::library("mylib", "linux_x64");
		// Defaults
		assert!(matches!(comp.visibility, Visibility::Public));
		assert!(comp.compatible_with.is_empty());

		// Builder
		let comp = Component::library("mylib2", "linux_x64")
			.with_visibility(Visibility::Package)
			.with_compatible_with(vec![ConstraintRef::new("//platforms:linux")]);
		assert!(matches!(comp.visibility, Visibility::Package));
		assert_eq!(comp.compatible_with.len(), 1);
	}
}
