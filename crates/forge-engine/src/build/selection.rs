use std::path::PathBuf;

use forge_diagnostics::{ForgeDiagnostic, codes};

use super::Prepared;
use crate::build::planner::ActionDag;

pub(super) fn select_dag(prepared: &Prepared, dag: &ActionDag, expression: &str) -> Result<ActionDag, ForgeDiagnostic> {
	let expr = forge_core::graph::query::parse(expression)?;
	let ids = forge_core::graph::query::evaluate(&prepared.graph, &expr)?;
	let selected: std::collections::BTreeSet<String> =
		ids.iter().map(|id| prepared.graph.component(*id).label.to_string()).collect();
	let mut keep = std::collections::BTreeSet::new();
	let mut pending: Vec<usize> = dag
		.specs
		.iter()
		.enumerate()
		.filter(|(_, spec)| selected.contains(&spec.component))
		.map(|(index, _)| index)
		.collect();
	if pending.is_empty() {
		return Err(ForgeDiagnostic::error(
			codes::targets::UNKNOWN_TARGET,
			format!("selection `{expression}` matched no components"),
		));
	}
	let producers: std::collections::BTreeMap<PathBuf, usize> = dag
		.specs
		.iter()
		.enumerate()
		.flat_map(|(index, spec)| spec.outputs.iter().map(move |output| (output.path.clone(), index)))
		.collect();
	while let Some(index) = pending.pop() {
		if keep.insert(index) {
			pending.extend(dag.deps[index].iter().copied());
			for input in dag.specs[index].inputs.iter().chain(&dag.specs[index].execution_deps) {
				let mut ancestor = Some(input.as_path());
				while let Some(path) = ancestor {
					if let Some(&producer) = producers.get(path) {
						pending.push(producer);
						break;
					}
					ancestor = path.parent().filter(|parent| !parent.as_os_str().is_empty());
				}
			}
		}
	}
	let remap: std::collections::BTreeMap<usize, usize> = keep.iter().enumerate().map(|(new, old)| (*old, new)).collect();
	let mut specs = Vec::with_capacity(keep.len());
	let mut deps = Vec::with_capacity(keep.len());
	for old in &keep {
		specs.push(dag.specs[*old].clone());
		deps.push(dag.deps[*old].iter().map(|dep| remap[dep]).collect());
	}
	Ok(ActionDag { specs, deps })
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use forge_core::{ActionSpec, Component, ComponentKind, ConfigTransition, Label, OutputDeclaration, OutputKind};

	use super::*;

	fn component(label: &str) -> Component {
		Component {
			label: Label::parse(label, "").unwrap(),
			kind: ComponentKind::Binary,
			visibility: forge_core::Visibility::Public,
			compatible_with: vec![],
			sources: vec![],
			headers: vec![],
			configuration: ConfigTransition::Target,
		}
	}

	fn action(component: &str, name: &str, inputs: &[&str], outputs: &[&str]) -> ActionSpec {
		ActionSpec {
			name: name.into(),
			component: component.into(),
			configuration: ConfigTransition::Target,
			command: "true".into(),
			args: Vec::new(),
			inputs: inputs.iter().map(PathBuf::from).collect(),
			execution_deps: Vec::new(),
			outputs: outputs
				.iter()
				.map(|path| OutputDeclaration {
					path: PathBuf::from(path),
					kind: OutputKind::File,
				})
				.collect(),
			workdir: None,
			is_test: false,
			stdout: None,
			compile_command: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::new(),
			toolchain_id: None,
			worker: None,
		}
	}

	fn prepared() -> Prepared {
		let mut graph = forge_core::BuildGraph::default();
		graph.add_component(component("//:gen")).unwrap();
		graph.add_component(component("//:app")).unwrap();
		Prepared {
			config: forge_script::WorkspaceConfig::parse("[project]\nname = \"t\"\n").unwrap(),
			graph,
			decls: Default::default(),
			dependencies: Vec::new(),
			requirements: Vec::new(),
			candidates: Vec::new(),
			imported_lock: None,
		}
	}

	fn dag() -> ActionDag {
		ActionDag {
			specs: vec![
				action("//:gen", "generate", &["ui/app.slint"], &["gen/app.h"]),
				action("//:app", "compile", &["src/main.c", "gen/app.h"], &["forge-out/bin/app"]),
			],
			deps: vec![vec![], vec![]],
		}
	}

	fn kept_names(dag: &ActionDag) -> Vec<String> {
		dag.specs.iter().map(|spec| spec.name.clone()).collect()
	}

	#[test]
	fn a_selection_keeps_the_producers_of_its_inputs() {
		let kept = select_dag(&prepared(), &dag(), "//:app").unwrap();
		assert_eq!(kept_names(&kept), vec!["generate".to_string(), "compile".to_string()]);
	}

	#[test]
	fn a_selection_keeps_the_producer_of_a_generated_directory() {
		let mut dag = dag();
		dag.specs[0].outputs[0].path = PathBuf::from("gen");
		dag.specs[0].outputs[0].kind = OutputKind::Directory;
		let kept = select_dag(&prepared(), &dag, "//:app").unwrap();
		assert_eq!(kept_names(&kept), vec!["generate".to_string(), "compile".to_string()]);
	}

	#[test]
	fn a_selection_without_producers_keeps_only_itself() {
		let kept = select_dag(&prepared(), &dag(), "//:gen").unwrap();
		assert_eq!(kept_names(&kept), vec!["generate".to_string()]);
	}
}
