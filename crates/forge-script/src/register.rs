use std::path::{Path, PathBuf};

use forge_core::graph::component::{Component, ComponentKind, LinkType};
use forge_core::label::Label;
use forge_core::{BuildGraph, DependencyEdge};
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::document::{SourceExpr, TargetDecl};
use crate::glob;

pub fn register_package(
	graph: &mut BuildGraph,
	decls: Vec<TargetDecl>,
	package: &str,
	package_dir: &Path,
) -> Result<Vec<Label>, ForgeDiagnostic> {
	let labels = register_components(graph, decls.clone(), package, package_dir)?;
	wire_dependencies(graph, decls, package)?;
	Ok(labels)
}

fn register_components(
	graph: &mut BuildGraph,
	decls: Vec<TargetDecl>,
	package: &str,
	package_dir: &Path,
) -> Result<Vec<Label>, ForgeDiagnostic> {
	let mut labels = Vec::new();
	for decl in &decls {
		let label = Label::new(package, decl.name.clone());
		let sources = resolve_sources(&decl.sources, package_dir)?;
		let headers = resolve_sources(&decl.headers, package_dir)?;
		let kind = component_kind(decl);
		graph.add_component(Component {
			label: label.clone(),
			kind,
			visibility: decl.visibility.clone(),
			compatible_with: decl.compatible_with.clone(),
			sources,
			headers,
		})?;
		labels.push(label);
	}
	Ok(labels)
}

pub fn wire_dependencies(graph: &mut BuildGraph, decls: Vec<TargetDecl>, package: &str) -> Result<(), ForgeDiagnostic> {
	for decl in decls {
		let from = graph
			.get(&Label::new(package, decl.name.clone()))
			.expect("registered just before");
		for dep in &decl.deps {
			wire_dependency(graph, from, dep)?;
		}
	}
	Ok(())
}

fn wire_dependency(graph: &mut BuildGraph, from: forge_core::ComponentId, dep: &str) -> Result<(), ForgeDiagnostic> {
	if let Err(first_err) = graph.add_dependency(from, dep, DependencyEdge::Hard) {
		let matches: Vec<String> = graph
			.labels()
			.filter(|l| l.name() == dep.trim_start_matches(':'))
			.map(|l| l.to_string())
			.collect();
		match matches.len() {
			1 => {
				graph
					.add_dependency(from, &matches[0], DependencyEdge::Hard)
					.map_err(|_| first_err)?;
				return Ok(());
			}
			0 => return Err(unknown_dep(dep, graph)),
			_ => {
				return Err(ForgeDiagnostic::error(2, format!("ambiguous dependency `{dep}`"))
					.with_help(format!("matches: {}", matches.join(", "))));
			}
		}
	}
	Ok(())
}

fn unknown_dep(dep: &str, graph: &BuildGraph) -> ForgeDiagnostic {
	let candidates: Vec<&str> = graph.labels().map(|l| l.name()).collect();
	let suggestion = forge_diagnostics::suggest::closest(dep, candidates);
	let d = ForgeDiagnostic::error(codes::targets::UNKNOWN_TARGET, format!("unknown dependency `{dep}`"));
	match suggestion {
		Some(s) => d.with_help(format!("did you mean `{s}`?")),
		None => d.with_help("the target must be declared before it can be referenced"),
	}
}

fn component_kind(decl: &TargetDecl) -> ComponentKind {
	match decl.kind {
		crate::document::TargetKind::Library => ComponentKind::Library { link: LinkType::Static },
		crate::document::TargetKind::Binary => ComponentKind::Binary,
		crate::document::TargetKind::Test => ComponentKind::Test,
		crate::document::TargetKind::Rule => ComponentKind::Generic {
			command: decl.command.clone().unwrap_or_default(),
		},
	}
}

pub fn resolve_sources(sources: &[SourceExpr], root: &Path) -> Result<Vec<PathBuf>, ForgeDiagnostic> {
	let mut resolved = Vec::new();
	for source in sources {
		match source {
			SourceExpr::File(p) => resolved.push(p.clone()),
			SourceExpr::Glob(pattern) => {
				let hits = glob::expand_glob(root, pattern).map_err(|e| {
					ForgeDiagnostic::error(codes::script::BAD_EXPRESSION, format!("glob `{pattern}` failed: {e}"))
				})?;
				if hits.is_empty() {
					return Err(ForgeDiagnostic::error(
						codes::inputs::MISSING_INPUT,
						format!("glob `{pattern}` matched nothing under {}", root.display()),
					));
				}
				resolved.extend(hits);
			}
		}
	}
	Ok(resolved)
}

use std::collections::BTreeMap;

use crate::discover::PackageSource;

pub type DeclMap = BTreeMap<String, TargetDecl>;

pub fn load_workspace(
	workspace: &std::path::Path,
	packages: &[PackageSource],
	platform: &forge_core::Platform,
	declared_platforms: &std::collections::BTreeMap<String, forge_core::Platform>,
) -> (BuildGraph, DeclMap, Vec<ForgeDiagnostic>) {
	let mut graph = BuildGraph::new();
	let mut pending_wires: Vec<(String, Vec<TargetDecl>)> = Vec::new();
	let mut sink = Vec::new();

	for pkg in packages {
		let package_dir = workspace.join(&pkg.package);
		let text = match std::fs::read_to_string(&pkg.file) {
			Ok(text) => text,
			Err(e) => {
				sink.push(ForgeDiagnostic::error(
					codes::inputs::MISSING_INPUT,
					format!("{}: {e}", pkg.file.display()),
				));
				continue;
			}
		};
		let parsed = if pkg.file.extension().is_some_and(|e| e == "rhai") {
			crate::rhai_rt::run_forge_rhai(&text, &package_dir, platform)
		} else {
			crate::parser::parse_forge_toml(&text)
		};
		let file_decls = match parsed {
			Ok(decls) => decls,
			Err(e) => {
				sink.push(e);
				continue;
			}
		};

		let mut usable = Vec::new();
		for mut decl in file_decls {
			decl.apply_platform_overrides(platform, declared_platforms);
			match resolve_sources(&decl.inputs, &package_dir) {
				Ok(resolved) => {
					decl.resolved_inputs = resolved;
					usable.push(decl);
				}
				Err(e) => sink.push(e),
			}
		}

		match register_components(&mut graph, usable.clone(), &pkg.package, &package_dir) {
			Ok(_) => pending_wires.push((pkg.package.clone(), usable)),
			Err(e) => sink.push(e),
		}
	}

	let mut decls: DeclMap = BTreeMap::new();
	for (package, package_decls) in pending_wires {
		wire_dependencies(&mut graph, package_decls.clone(), &package).unwrap_or_else(|e| sink.push(e));
		for decl in package_decls {
			decls.insert(Label::new(&package, decl.name.clone()).to_string(), decl);
		}
	}
	(graph, decls, sink)
}
