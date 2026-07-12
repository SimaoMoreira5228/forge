use std::collections::BTreeSet;
use std::fmt::Write;

use forge_core::BuildGraph;

pub enum OutputFormat {
	Label,
	Json,
	Dot,
	Count,
}

pub fn format_results(
	graph: &BuildGraph,
	ids: &BTreeSet<forge_core::ComponentId>,
	format: OutputFormat,
	expr: &str,
) -> String {
	match format {
		OutputFormat::Count => ids.len().to_string(),
		OutputFormat::Label => ids
			.iter()
			.map(|id| graph.component(*id).label.to_string())
			.collect::<Vec<_>>()
			.join("\n"),
		OutputFormat::Json => json_output(graph, ids),
		OutputFormat::Dot => dot_output(graph, ids, expr),
	}
}

fn json_output(graph: &BuildGraph, ids: &BTreeSet<forge_core::ComponentId>) -> String {
	let entries: Vec<serde_json::Value> = ids
		.iter()
		.map(|id| {
			let c = graph.component(*id);
			serde_json::json!({
				"label": c.label.to_string(),
				"kind": kind_string(c.kind.clone()),
			})
		})
		.collect();
	serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".into())
}

fn dot_output(graph: &BuildGraph, ids: &BTreeSet<forge_core::ComponentId>, expr: &str) -> String {
	let mut out = format!("digraph \"{}\" {{\n", expr);
	for id in ids {
		let c = graph.component(*id);
		let color = match c.kind {
			forge_core::ComponentKind::Library { .. } => "lightblue",
			forge_core::ComponentKind::Binary => "palegreen",
			forge_core::ComponentKind::Test => "gold",
			forge_core::ComponentKind::Generic { .. } => "lightgray",
		};
		let _ = writeln!(out, "  \"{}\" [fillcolor={color}, style=filled];", c.label);
	}
	for id in ids {
		for dep in graph.dependencies_of(*id) {
			if ids.contains(&dep) {
				let src = graph.component(*id).label.to_string();
				let dst = graph.component(dep).label.to_string();
				let _ = writeln!(out, "  \"{src}\" -> \"{dst}\";");
			}
		}
	}
	out.push_str("}\n");
	out
}

fn kind_string(kind: forge_core::ComponentKind) -> &'static str {
	match kind {
		forge_core::ComponentKind::Library { .. } => "library",
		forge_core::ComponentKind::Binary => "binary",
		forge_core::ComponentKind::Test => "test",
		forge_core::ComponentKind::Generic { .. } => "rule",
	}
}
