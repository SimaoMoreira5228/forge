use crate::graph::{Component, ComponentId, ComponentRef, DependencyEdge, Target};
use petgraph::Direction;
use petgraph::algo::{Cycle, toposort};
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphError {
	#[error("Component '{name}' not found")]
	ComponentNotFound {
		name: String,
	},

	#[error("Target '{name}' not found")]
	TargetNotFound {
		name: String,
	},

	#[error("Circular dependency detected: {cycle}")]
	CircularDependency {
		cycle: String,
	},

	#[error("Duplicate component: '{name}' for target '{target}'")]
	DuplicateComponent {
		name: String,
		target: String,
	},

	#[error("Dependency conflict: multiple components produce '{output}'")]
	OutputConflict {
		output: String,
	},
}

pub struct BuildGraph {
	graph: DiGraph<ComponentId, DependencyEdge>,
	components: HashMap<ComponentId, Component>,
	component_index: HashMap<(String, String), ComponentId>,
	targets: HashMap<String, Target>,
	output_map: HashMap<PathBuf, ComponentId>,
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

	pub fn add_component(&mut self, component: Component) -> Result<ComponentId, GraphError> {
		let key = (component.name.clone(), component.target_name.clone());

		if self.component_index.contains_key(&key) {
			return Err(GraphError::DuplicateComponent {
				name: component.name.clone(),
				target: component.target_name.clone(),
			});
		}

		for output in &component.outputs {
			if let Some(existing) = self.output_map.get(output) {
				let existing_comp = self.components.get(existing).unwrap();
				return Err(GraphError::OutputConflict {
					output: output.display().to_string(),
				});
			}
		}

		let id = component.id;
		let node_idx = self.graph.add_node(id);

		self.components.insert(id, component.clone());
		self.component_index.insert(key, id);

		self.name_to_ids.entry(component.name.clone()).or_default().push(id);

		for output in &component.outputs {
			self.output_map.insert(output.clone(), id);
		}

		Ok(id)
	}

	pub fn add_dependency(&mut self, from: ComponentId, to: ComponentRef, edge: DependencyEdge) -> Result<(), GraphError> {
		let from_idx = self.find_node_index(from)?;
		let to_id = self.resolve_component_ref(to)?;
		let to_idx = self.find_node_index(to_id)?;

		self.graph.add_edge(to_idx, from_idx, edge);
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

		// Edge goes from dependency to dependent
		self.graph.add_edge(to_idx, from_idx, edge);
		Ok(())
	}

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

	pub fn get_component(&self, id: ComponentId) -> Option<&Component> {
		self.components.get(&id)
	}

	pub fn get_component_by_name(&self, name: &str, target: &str) -> Option<&Component> {
		self.component_index
			.get(&(name.to_string(), target.to_string()))
			.and_then(|id| self.components.get(id))
	}

	pub fn dependencies_of(&self, id: ComponentId) -> Vec<ComponentId> {
		let Ok(idx) = self.find_node_index(id) else {
			return Vec::new();
		};

		// Dependencies are nodes that must come before this one (edges pointing TO this node)
		self.graph
			.neighbors_directed(idx, Direction::Incoming)
			.map(|n| self.graph[n])
			.collect()
	}

	pub fn dependents_of(&self, id: ComponentId) -> Vec<ComponentId> {
		let Ok(idx) = self.find_node_index(id) else {
			return Vec::new();
		};

		// Dependents are nodes that depend on this one (edges pointing FROM this node)
		self.graph
			.neighbors_directed(idx, Direction::Outgoing)
			.map(|n| self.graph[n])
			.collect()
	}

	pub fn transitive_dependencies(&self, id: ComponentId) -> Vec<ComponentId> {
		let mut result = Vec::new();
		let mut visited = std::collections::HashSet::new();
		self.collect_transitive_deps(id, &mut result, &mut visited);
		result
	}

	fn collect_transitive_deps(
		&self,
		id: ComponentId,
		result: &mut Vec<ComponentId>,
		visited: &mut std::collections::HashSet<ComponentId>,
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

	pub fn topological_order(&self) -> Result<Vec<ComponentId>, GraphError> {
		match toposort(&self.graph, None) {
			Ok(indices) => Ok(indices.into_iter().map(|idx| self.graph[idx]).collect()),
			Err(cycle) => {
				let cycle_path = self.format_cycle(cycle);
				Err(GraphError::CircularDependency { cycle: cycle_path })
			}
		}
	}

	fn format_cycle(&self, cycle: Cycle<NodeIndex>) -> String {
		let node_id = cycle.node_id();
		if let Some(component) = self.components.get(&self.graph[node_id]) {
			let mut path = vec![component.name.clone()];

			let mut current = node_id;
			let mut visited = std::collections::HashSet::new();
			visited.insert(current);

			for neighbor in self.graph.neighbors_directed(current, Direction::Outgoing) {
				if visited.contains(&neighbor) {
					if let Some(comp) = self.components.get(&self.graph[neighbor]) {
						path.push(comp.name.clone());
					}
					break;
				}
				visited.insert(neighbor);
				current = neighbor;
			}

			path.join(" -> ")
		} else {
			"unknown cycle".to_string()
		}
	}

	pub fn find_cycles(&self) -> Vec<Vec<ComponentId>> {
		let mut cycles = Vec::new();
		let mut visited = std::collections::HashSet::new();
		let mut rec_stack = std::collections::HashSet::new();
		let mut path = Vec::new();

		for node_idx in self.graph.node_indices() {
			self.find_cycles_dfs(node_idx, &mut visited, &mut rec_stack, &mut path, &mut cycles);
		}

		cycles
	}

	fn find_cycles_dfs(
		&self,
		node: NodeIndex,
		visited: &mut std::collections::HashSet<NodeIndex>,
		rec_stack: &mut std::collections::HashSet<NodeIndex>,
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

	pub fn execution_batches(&self) -> Result<Vec<Vec<ComponentId>>, GraphError> {
		let mut batches = Vec::new();
		let mut completed: std::collections::HashSet<ComponentId> = std::collections::HashSet::new();
		let mut remaining: std::collections::HashSet<ComponentId> =
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

	pub fn filter(&self, targets: &[String], component_names: &[String]) -> Vec<ComponentId> {
		let by_target: std::collections::HashSet<_> = self.filter_by_target(targets).into_iter().collect();
		let by_component: std::collections::HashSet<_> = self.filter_by_component(component_names).into_iter().collect();

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

	pub fn component_ids(&self) -> impl Iterator<Item = ComponentId> + '_ {
		self.components.keys().copied()
	}
}

impl Default for BuildGraph {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
		let component = Component::library("mylib", "linux_x64");
		let id = graph.add_component(component).unwrap();
		assert_eq!(graph.component_count(), 1);
		assert!(graph.get_component(id).is_some());
	}

	#[test]
	fn test_add_dependency() {
		let mut graph = BuildGraph::new();

		let lib = Component::library("mylib", "linux_x64");
		let lib_id = graph.add_component(lib).unwrap();

		let bin = Component::binary("myapp", "linux_x64");
		let bin_id = graph.add_component(bin).unwrap();

		graph
			.add_dependency(bin_id, ComponentRef::new("mylib"), DependencyEdge::Hard)
			.unwrap();

		let deps = graph.dependencies_of(bin_id);
		assert_eq!(deps.len(), 1);
		assert_eq!(deps[0], lib_id);
	}

	#[test]
	fn test_transitive_dependencies() {
		let mut graph = BuildGraph::new();

		let c = Component::library("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();

		let b = Component::library("b", "linux_x64");
		let b_id = graph.add_component(b.clone()).unwrap();
		graph
			.add_dependency(b_id, ComponentRef::new("c"), DependencyEdge::Hard)
			.unwrap();

		let a = Component::binary("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();
		graph
			.add_dependency(a_id, ComponentRef::new("b"), DependencyEdge::Hard)
			.unwrap();

		let transitive = graph.transitive_dependencies(a_id);
		assert_eq!(transitive.len(), 2);
		assert!(transitive.contains(&b_id));
		assert!(transitive.contains(&c_id));
	}

	#[test]
	fn test_topological_order() {
		let mut graph = BuildGraph::new();

		let a = Component::library("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();

		let b = Component::library("b", "linux_x64");
		let b_id = graph.add_component(b.clone()).unwrap();
		graph
			.add_dependency(b_id, ComponentRef::new("a"), DependencyEdge::Hard)
			.unwrap();

		let c = Component::binary("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();
		graph
			.add_dependency(c_id, ComponentRef::new("b"), DependencyEdge::Hard)
			.unwrap();

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

		let a = Component::library("a", "linux_x64");
		let a_id = graph.add_component(a).unwrap();

		let b = Component::library("b", "linux_x64");
		let b_id = graph.add_component(b).unwrap();

		let c = Component::binary("c", "linux_x64");
		let c_id = graph.add_component(c).unwrap();
		graph
			.add_dependency(c_id, ComponentRef::new("a"), DependencyEdge::Hard)
			.unwrap();
		graph
			.add_dependency(c_id, ComponentRef::new("b"), DependencyEdge::Hard)
			.unwrap();

		let batches = graph.execution_batches().unwrap();

		assert_eq!(batches.len(), 2);
		assert_eq!(batches[0].len(), 2);
		assert_eq!(batches[1].len(), 1);

		let first_batch: std::collections::HashSet<_> = batches[0].iter().copied().collect();
		assert!(first_batch.contains(&a_id));
		assert!(first_batch.contains(&b_id));
		assert_eq!(batches[1][0], c_id);
	}
}
