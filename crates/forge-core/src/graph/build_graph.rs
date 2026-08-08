use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;

use forge_diagnostics::ForgeDiagnostic;
use petgraph::algo::tarjan_scc;
use petgraph::prelude::*;
use petgraph::visit::Topo;

use super::component::{Component, ComponentId, ComponentKind, Visibility, package_pattern_matches};
use super::edge::DependencyEdge;
use crate::label::Label;

#[derive(Debug, Default, Clone)]
pub struct BuildGraph {
	inner: DiGraph<Component, DependencyEdge>,
	by_label: BTreeMap<Label, ComponentId>,
}

impl BuildGraph {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn add_component(&mut self, component: Component) -> Result<ComponentId, ForgeDiagnostic> {
		if self.by_label.contains_key(&component.label) {
			return Err(
				ForgeDiagnostic::error(105, format!("duplicate component `{}`", component.label))
					.with_help("every component name must be unique within its package"),
			);
		}
		let id = self.inner.add_node(component);
		self.by_label.insert(self.inner[id].label.clone(), id);
		Ok(id)
	}

	pub fn add_dependency(
		&mut self,
		from: ComponentId,
		to_ref: &str,
		edge: DependencyEdge,
	) -> Result<ComponentId, ForgeDiagnostic> {
		let context = self.inner[from].label.package().to_string();
		let to_label = Label::parse(to_ref, &context)?;
		let Some(to) = self.get(&to_label) else {
			return Err(unknown_target(&to_label.to_string(), &self.inner[from].label));
		};
		self.inner.add_edge(from, to, edge);
		Ok(to)
	}

	pub fn get(&self, label: &Label) -> Option<ComponentId> {
		self.by_label.get(label).copied()
	}

	pub fn connect(&mut self, from: ComponentId, to: ComponentId, edge: DependencyEdge) {
		self.inner.add_edge(from, to, edge);
	}

	pub fn component(&self, id: ComponentId) -> &Component {
		&self.inner[id]
	}

	pub fn node_count(&self) -> usize {
		self.inner.node_count()
	}

	pub fn labels(&self) -> impl Iterator<Item = &Label> {
		self.by_label.keys()
	}

	pub fn node_ids(&self) -> impl Iterator<Item = ComponentId> + '_ {
		self.inner.node_indices()
	}

	pub fn dependencies_of(&self, id: ComponentId) -> Vec<ComponentId> {
		self.inner.neighbors_directed(id, Outgoing).collect()
	}

	pub fn reverse_dependencies_of(&self, id: ComponentId) -> Vec<ComponentId> {
		self.inner.neighbors_directed(id, Incoming).collect()
	}

	pub fn transitive_dependencies(&self, id: ComponentId) -> BTreeSet<ComponentId> {
		self.reachable(id, Direction::Outgoing)
	}

	pub fn transitive_dependents(&self, id: ComponentId) -> BTreeSet<ComponentId> {
		self.reachable(id, Direction::Incoming)
	}

	pub fn topological_order(&self) -> Result<Vec<ComponentId>, Vec<Vec<Label>>> {
		let mut topo = Topo::new(&self.inner);
		let mut order = Vec::with_capacity(self.inner.node_count());
		while let Some(node) = topo.next(&self.inner) {
			order.push(node);
		}
		order.reverse();
		if order.len() == self.inner.node_count() {
			Ok(order)
		} else {
			Err(self.cycle_paths())
		}
	}

	pub fn cycle_paths(&self) -> Vec<Vec<Label>> {
		tarjan_scc(&self.inner)
			.into_iter()
			.filter(|scc| scc.len() > 1 || self.inner.neighbors_directed(scc[0], Outgoing).any(|n| n == scc[0]))
			.map(|scc| {
				let mut labels: Vec<Label> = scc.iter().map(|id| self.inner[*id].label.clone()).collect();
				labels.sort();
				labels
			})
			.collect()
	}

	pub fn execution_batches(&self) -> Result<Vec<Vec<ComponentId>>, Vec<Vec<Label>>> {
		let order = self.topological_order()?;
		let mut depth: BTreeMap<ComponentId, usize> = BTreeMap::new();
		for id in &order {
			let layer = self
				.inner
				.neighbors_directed(*id, Outgoing)
				.filter_map(|p| depth.get(&p).copied())
				.max()
				.map_or(0, |d| d + 1);
			depth.insert(*id, layer);
		}
		let mut batches: Vec<Vec<ComponentId>> = Vec::new();
		for (id, layer) in depth {
			if batches.len() <= layer {
				batches.resize_with(layer + 1, Vec::new);
			}
			batches[layer].push(id);
		}
		Ok(batches)
	}

	pub fn all_paths(&self, from: ComponentId, to: ComponentId) -> Vec<Vec<ComponentId>> {
		let mut paths = Vec::new();
		let mut stack = vec![(from, vec![from])];
		while let Some((current, path)) = stack.pop() {
			if current == to {
				paths.push(path);
				continue;
			}
			if path.len() >= self.inner.node_count().max(1) {
				continue;
			}
			for next in self.inner.neighbors_directed(current, Outgoing) {
				if !path.contains(&next) {
					let mut extended = path.clone();
					extended.push(next);
					stack.push((next, extended));
				}
			}
		}
		paths
	}

	pub fn check_visibility(&self) -> Result<(), Vec<ForgeDiagnostic>> {
		let mut errors = Vec::new();
		for edge in self.inner.edge_references() {
			let consumer = &self.inner[edge.source()];
			let provider = &self.inner[edge.target()];
			let consumer_package = consumer.label.package();
			let allowed = match &provider.visibility {
				Visibility::Public => true,
				Visibility::Package => consumer_package == provider.label.package(),
				Visibility::Patterns(patterns) => patterns.iter().any(|p| package_pattern_matches(p, consumer_package)),
			};
			if !allowed {
				errors.push(
					ForgeDiagnostic::error(
						1,
						format!("`{}` is not permitted to depend on `{}`", consumer.label, provider.label),
					)
					.with_help(format!(
						"`{}` has restricted visibility ({:?}); adjust its visibility or restructure the dependency",
						provider.label, provider.visibility
					)),
				);
			}
		}
		if errors.is_empty() { Ok(()) } else { Err(errors) }
	}

	pub fn filter(&self, patterns: &[String]) -> Vec<ComponentId> {
		self.inner
			.node_indices()
			.filter(|id| patterns.iter().any(|p| self.matches_pattern(*id, p)))
			.collect()
	}

	fn matches_pattern(&self, id: ComponentId, pattern: &str) -> bool {
		let component = &self.inner[id];
		let label_string = component.label.to_string();
		let package = component.label.package().to_string();
		if let Some(rest) = pattern.strip_prefix("//") {
			if rest == "..." {
				return true;
			}
			if let Some(prefix) = rest.strip_suffix("/...") {
				return package == prefix || package.starts_with(&format!("{prefix}/"));
			}
			return label_string == pattern;
		}
		component.label.name() == pattern.trim_start_matches(':')
	}

	pub fn node_rows(&self) -> Vec<(String, String)> {
		self.inner
			.node_indices()
			.map(|id| {
				let c = &self.inner[id];
				(c.label.to_string(), c.kind.name().to_string())
			})
			.collect()
	}

	pub fn edge_rows(&self) -> Vec<(String, String)> {
		self.inner
			.edge_references()
			.map(|e| {
				(
					self.inner[e.source()].label.to_string(),
					self.inner[e.target()].label.to_string(),
				)
			})
			.collect()
	}

	pub fn components_containing_source(&self, needle: &str) -> Vec<ComponentId> {
		self.inner
			.node_indices()
			.filter(|id| self.inner[*id].sources.iter().any(|s| s.to_string_lossy().contains(needle)))
			.collect()
	}

	pub fn output_dot(&self) -> String {
		let mut out = String::from("digraph forge {\n  rankdir=LR;\n");
		for id in self.inner.node_indices() {
			let c = &self.inner[id];
			let color = match c.kind {
				ComponentKind::Library { .. } => "lightblue",
				ComponentKind::Binary => "palegreen",
				ComponentKind::Test => "gold",
				ComponentKind::Generic { .. } => "lightgray",
			};
			let _ = writeln!(out, "  \"{}\" [fillcolor={color}, style=filled];", c.label);
		}
		for e in self.inner.edge_references() {
			let _ = writeln!(
				out,
				"  \"{}\" -> \"{}\";",
				self.inner[e.source()].label,
				self.inner[e.target()].label
			);
		}
		out.push_str("}\n");
		out
	}

	fn reachable(&self, start: ComponentId, dir: Direction) -> BTreeSet<ComponentId> {
		let mut seen = BTreeSet::new();
		let mut queue = VecDeque::from([start]);
		while let Some(current) = queue.pop_front() {
			for next in self.inner.neighbors_directed(current, dir) {
				if seen.insert(next) {
					queue.push_back(next);
				}
			}
		}
		seen
	}
}

fn unknown_target(target: &str, from: &Label) -> ForgeDiagnostic {
	ForgeDiagnostic::error(2, format!("unknown target `{target}`")).related_note(format!("referenced from `{from}`"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::graph::component::LinkType;

	fn library(name: &str, visibility: Visibility) -> Component {
		Component {
			label: Label::parse(name, "").unwrap(),
			kind: ComponentKind::Library { link: LinkType::Static },
			visibility,
			compatible_with: vec![],
			sources: vec![],
			headers: vec![],
			configuration: Default::default(),
		}
	}

	fn binary(name: &str) -> Component {
		Component {
			label: Label::parse(name, "").unwrap(),
			kind: ComponentKind::Binary,
			visibility: Visibility::Public,
			compatible_with: vec![],
			sources: vec![],
			headers: vec![],
			configuration: Default::default(),
		}
	}

	fn chain_graph() -> BuildGraph {
		let mut g = BuildGraph::new();
		g.add_component(library("//lib:base", Visibility::Public)).unwrap();
		let math = g.add_component(library("//lib:math", Visibility::Public)).unwrap();
		let app = g.add_component(binary("//app:demo")).unwrap();
		g.add_dependency(math, "//lib:base", DependencyEdge::Hard).unwrap();
		g.add_dependency(app, "//lib:math", DependencyEdge::Hard).unwrap();
		g
	}

	#[test]
	fn batches_respect_dependencies() {
		let g = chain_graph();
		let batches = g.execution_batches().unwrap();
		assert_eq!(batches.len(), 3);
		assert_eq!(batches[0].len(), 1);
		assert_eq!(batches[1].len(), 1);
		assert_eq!(batches[2].len(), 1);
	}

	#[test]
	fn parallel_nodes_share_a_batch() {
		let mut g = BuildGraph::new();
		g.add_component(library("//p:a", Visibility::Public)).unwrap();
		g.add_component(library("//p:b", Visibility::Public)).unwrap();
		let c = g.add_component(binary("//p:c")).unwrap();
		g.add_dependency(c, "//p:a", DependencyEdge::Hard).unwrap();
		g.add_dependency(c, ":b", DependencyEdge::Hard).unwrap();
		let batches = g.execution_batches().unwrap();
		assert_eq!(batches.len(), 2);
		assert_eq!(batches[0].len(), 2);
	}

	#[test]
	fn cycles_report_full_path() {
		let mut g = BuildGraph::new();
		g.add_component(library("//x:a", Visibility::Public)).unwrap();
		g.add_component(library("//x:b", Visibility::Public)).unwrap();
		let ids: Vec<_> = g.node_ids().collect();
		g.add_dependency(ids[0], "//x:b", DependencyEdge::Hard).unwrap();
		g.add_dependency(ids[1], "//x:a", DependencyEdge::Hard).unwrap();
		let cycles = g.cycle_paths();
		assert_eq!(cycles.len(), 1);
		assert_eq!(cycles[0].len(), 2);
		assert!(g.execution_batches().is_err());
	}

	#[test]
	fn transitive_queries() {
		let g = chain_graph();
		let app = g.filter(&["//app:demo".into()]);
		let deps = g.transitive_dependencies(app[0]);
		assert_eq!(deps.len(), 2);
		let base = g.filter(&["//lib:base".into()]);
		let rdeps = g.transitive_dependents(base[0]);
		assert_eq!(rdeps.len(), 2);
	}

	#[test]
	fn all_paths_finds_both_routes() {
		let mut g = BuildGraph::new();
		g.add_component(library("//l:base", Visibility::Public)).unwrap();
		g.add_component(library("//l:left", Visibility::Public)).unwrap();
		g.add_component(library("//l:right", Visibility::Public)).unwrap();
		g.add_component(binary("//l:app")).unwrap();
		let ids: Vec<ComponentId> = g.node_ids().collect();
		g.add_dependency(ids[1], "//l:base", DependencyEdge::Hard).unwrap();
		g.add_dependency(ids[2], "//l:base", DependencyEdge::Hard).unwrap();
		g.add_dependency(ids[3], "//l:left", DependencyEdge::Hard).unwrap();
		g.add_dependency(ids[3], "//l:right", DependencyEdge::Hard).unwrap();
		let paths = g.all_paths(ids[3], ids[0]);
		assert_eq!(paths.len(), 2);
	}

	#[test]
	fn visibility_enforced_at_construction_time() {
		let mut g = BuildGraph::new();
		g.add_component(library("//internal:secret", Visibility::Package)).unwrap();
		g.add_component(binary("//bin:app")).unwrap();
		let ids: Vec<ComponentId> = g.node_ids().collect();
		g.add_dependency(ids[1], "//internal:secret", DependencyEdge::Hard).unwrap();
		let errs = g.check_visibility().unwrap_err();
		assert_eq!(errs.len(), 1);
		assert!(format!("{}", errs[0]).contains("not permitted"));
	}

	#[test]
	fn visibility_allows_same_package_and_patterns() {
		let mut g = BuildGraph::new();
		g.add_component(library("//pkg:impl", Visibility::Package)).unwrap();
		g.add_component(binary("//pkg:main")).unwrap();
		let ids: Vec<ComponentId> = g.node_ids().collect();
		g.add_dependency(ids[1], ":impl", DependencyEdge::Hard).unwrap();
		assert!(g.check_visibility().is_ok());

		let mut g2 = BuildGraph::new();
		g2.add_component(library("//shared:util", Visibility::Patterns(vec!["//tools/...".into()])))
			.unwrap();
		g2.add_component(binary("//tools/gen")).unwrap();
		let ids2: Vec<ComponentId> = g2.node_ids().collect();
		g2.add_dependency(ids2[1], "//shared:util", DependencyEdge::Hard).unwrap();
		assert!(g2.check_visibility().is_ok());
	}

	#[test]
	fn duplicate_components_rejected() {
		let mut g = BuildGraph::new();
		g.add_component(library("//a:x", Visibility::Public)).unwrap();
		assert!(g.add_component(library("//a:x", Visibility::Public)).is_err());
	}

	#[test]
	fn dot_output_lists_nodes_and_edges() {
		let g = chain_graph();
		let dot = g.output_dot();
		assert!(dot.contains("//lib:base"));
		assert!(dot.contains("\"//app:demo\" -> \"//lib:math\""));
	}

	#[test]
	fn unknown_dependency_names_the_referencing_component() {
		let mut g = BuildGraph::new();
		g.add_component(binary("//app:demo")).unwrap();
		let app = g.node_ids().next().unwrap();
		let err = g.add_dependency(app, "//lib:nope", DependencyEdge::Hard).unwrap_err();
		assert!(format!("{err}").contains("unknown target"));
	}
}
