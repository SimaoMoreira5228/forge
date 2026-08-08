use std::path::{Path, PathBuf};

use forge_core::BuildGraph;
use forge_core::graph::component::{Component, ComponentKind, LinkType};
use forge_core::label::Label;
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::document::{SourceExpr, TargetDecl};
use crate::glob;

pub fn register_package(
	graph: &mut BuildGraph,
	decls: Vec<TargetDecl>,
	package: &str,
	package_dir: &Path,
) -> Result<Vec<Label>, ForgeDiagnostic> {
	let labels = register_components(graph, decls.clone(), package, package_dir, &[])?;
	wire_dependencies(graph, decls, package, &forge_core::Platform::host())?;
	Ok(labels)
}

fn register_components(
	graph: &mut BuildGraph,
	decls: Vec<TargetDecl>,
	package: &str,
	package_dir: &Path,
	generated: &[DynamicDir],
) -> Result<Vec<Label>, ForgeDiagnostic> {
	let mut labels = Vec::new();
	for decl in &decls {
		let label = Label::new(package, decl.name.clone());
		let sources = resolve_sources(&decl.sources, package_dir, generated)?;
		let headers = resolve_sources(&decl.headers, package_dir, generated)?;
		let kind = component_kind(decl);
		graph.add_component(Component {
			label: label.clone(),
			kind,
			visibility: decl.visibility.clone(),
			compatible_with: decl.compatible_with.clone(),
			sources,
			headers,
			configuration: forge_core::ConfigTransition::Target,
		})?;
		labels.push(label);
	}
	Ok(labels)
}

pub fn wire_dependencies(
	graph: &mut BuildGraph,
	decls: Vec<TargetDecl>,
	package: &str,
	active: &forge_core::Platform,
) -> Result<(), ForgeDiagnostic> {
	for decl in decls {
		let from = graph
			.get(&Label::new(package, decl.name.clone()))
			.expect("registered just before");
		for dep in &decl.deps {
			wire_dependency(graph, from, dep, active)?;
		}
	}
	Ok(())
}

fn wire_dependency(
	graph: &mut BuildGraph,
	from: forge_core::ComponentId,
	dep: &forge_core::DependencyDecl,
	active: &forge_core::Platform,
) -> Result<(), ForgeDiagnostic> {
	let mut to = resolve_target(graph, from, &dep.label)?;
	if let forge_core::DependencyEdge::Transition(transition) = dep.edge
		&& transition.resolve(active) != *active
	{
		to = ensure_variant(graph, to, transition)?;
	}
	graph.connect(from, to, dep.edge.clone());
	Ok(())
}

fn resolve_target(
	graph: &BuildGraph,
	from: forge_core::ComponentId,
	reference: &str,
) -> Result<forge_core::ComponentId, ForgeDiagnostic> {
	let context = graph.component(from).label.package().to_string();
	if let Ok(label) = Label::parse(reference, &context)
		&& let Some(id) = graph.get(&label)
	{
		return Ok(id);
	}
	let matches: Vec<String> = graph
		.labels()
		.filter(|l| l.name() == reference.trim_start_matches(':'))
		.map(|l| l.to_string())
		.collect();
	match matches.len() {
		1 => Label::parse(&matches[0], &context)
			.ok()
			.and_then(|label| graph.get(&label))
			.ok_or_else(|| unknown_dep(reference, graph)),
		0 => Err(unknown_dep(reference, graph)),
		_ => Err(ForgeDiagnostic::error(2, format!("ambiguous dependency `{reference}`"))
			.with_help(format!("matches: {}", matches.join(", ")))),
	}
}

fn ensure_variant(
	graph: &mut BuildGraph,
	target: forge_core::ComponentId,
	transition: forge_core::ConfigTransition,
) -> Result<forge_core::ComponentId, ForgeDiagnostic> {
	let mut variant = graph.component(target).clone();
	variant.label = Label::new(
		variant.label.package(),
		format!("{}__{}", variant.label.name(), transition.as_str()),
	);
	if let Some(existing) = graph.get(&variant.label) {
		return Ok(existing);
	}
	variant.configuration = transition;
	graph.add_component(variant)
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

pub fn resolve_sources(
	sources: &[SourceExpr],
	root: &Path,
	generated: &[DynamicDir],
) -> Result<Vec<PathBuf>, ForgeDiagnostic> {
	let mut resolved = Vec::new();
	for source in sources {
		match source {
			SourceExpr::File(p) => resolved.push(p.clone()),
			SourceExpr::Glob(pattern) => {
				let hits = glob::expand_glob(root, pattern).map_err(|e| {
					ForgeDiagnostic::error(codes::script::BAD_EXPRESSION, format!("glob `{pattern}` failed: {e}"))
				})?;
				if hits.is_empty() && !targets_generated_dir(pattern, generated) {
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

#[derive(Debug, Clone)]
pub struct DynamicDir {
	pub dir: String,
	pub label: Label,
}

fn targets_generated_dir(pattern: &str, generated: &[DynamicDir]) -> bool {
	let literal = pattern.split(['*', '?']).next().unwrap_or("");
	let literal = literal.trim_start_matches("./").trim_end_matches('/');
	!literal.is_empty()
		&& generated
			.iter()
			.any(|d| literal == d.dir || literal.starts_with(&format!("{}/", d.dir)))
}

pub fn collect_generated_dirs(
	workspace: &Path,
	packages: &[PackageSource],
	platform: &forge_core::Platform,
) -> Vec<DynamicDir> {
	let mut dirs = Vec::new();
	for pkg in packages {
		let Ok(text) = std::fs::read_to_string(&pkg.file) else {
			continue;
		};
		let package_dir = workspace.join(&pkg.package);
		let decls = if pkg.file.extension().is_some_and(|e| e == "rhai") {
			crate::rhai_rt::run_forge_rhai(&text, &package_dir, platform)
				.map(|output| output.targets)
				.unwrap_or_default()
		} else {
			crate::parser::parse_forge_toml(&text).unwrap_or_default()
		};
		for decl in decls {
			if decl.kind == crate::document::TargetKind::Rule
				&& let Some(dir) = decl.output_dir
			{
				dirs.push(DynamicDir {
					dir: dir.trim_end_matches('/').to_string(),
					label: Label::new(&pkg.package, decl.name),
				});
			}
		}
	}
	dirs
}

fn wire_generated_sources(graph: &mut BuildGraph, generated: &[DynamicDir]) {
	if generated.is_empty() {
		return;
	}
	for id in graph.node_ids().collect::<Vec<_>>() {
		for source in graph.component(id).sources.clone() {
			let text = source.to_string_lossy();
			let Some(owner) = generated.iter().find(|d| text.starts_with(&format!("{}/", d.dir))) else {
				continue;
			};
			let Some(rule) = graph.get(&owner.label) else {
				continue;
			};
			if rule != id && !graph.dependencies_of(id).contains(&rule) {
				graph.connect(id, rule, forge_core::DependencyEdge::OrderOnly);
			}
		}
	}
}

use std::collections::BTreeMap;

use crate::discover::PackageSource;

pub type DeclMap = BTreeMap<String, TargetDecl>;

pub fn load_workspace(
	workspace: &std::path::Path,
	packages: &[PackageSource],
	platform: &forge_core::Platform,
	declared_platforms: &std::collections::BTreeMap<String, forge_core::Platform>,
	bootstrap: &[String],
) -> (
	BuildGraph,
	DeclMap,
	Vec<forge_core::DependencyRequest>,
	Vec<forge_core::DependencyRequirement>,
	Vec<forge_core::PackageCandidate>,
	Vec<ForgeDiagnostic>,
) {
	let mut graph = BuildGraph::new();
	let generated = collect_generated_dirs(workspace, packages, platform);
	let mut pending_wires: Vec<(String, Vec<TargetDecl>)> = Vec::new();
	let mut dependencies = Vec::new();
	let mut requirements = Vec::new();
	let mut candidates = Vec::new();
	let mut sink = Vec::new();

	for script in bootstrap {
		match crate::rhai_rt::run_forge_rhai(script, workspace, platform) {
			Ok(output) => {
				dependencies.extend(output.dependencies);
				requirements.extend(output.requirements);
				candidates.extend(output.candidates);
			}
			Err(d) => sink.push(d),
		}
	}

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
			crate::rhai_rt::run_forge_rhai(&text, &package_dir, platform).map(|output| {
				dependencies.extend(output.dependencies);
				requirements.extend(output.requirements);
				candidates.extend(output.candidates);
				output.targets
			})
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
			match resolve_sources(&decl.inputs, &package_dir, &generated) {
				Ok(resolved) => {
					decl.resolved_inputs = resolved;
					usable.push(decl);
				}
				Err(e) => sink.push(e),
			}
		}

		match register_components(&mut graph, usable.clone(), &pkg.package, &package_dir, &generated) {
			Ok(_) => pending_wires.push((pkg.package.clone(), usable)),
			Err(e) => sink.push(e),
		}
	}

	let mut decls: DeclMap = BTreeMap::new();
	for (package, package_decls) in pending_wires {
		wire_dependencies(&mut graph, package_decls.clone(), &package, platform).unwrap_or_else(|e| sink.push(e));
		for decl in package_decls {
			decls.insert(Label::new(&package, decl.name.clone()).to_string(), decl);
		}
	}
	wire_generated_sources(&mut graph, &generated);
	(graph, decls, dependencies, requirements, candidates, sink)
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::*;

	fn fixture(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("forge-register-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(
			dir.join("FORGE_ROOT"),
			"[project]\nname = \"t\"\n\n[discovery]\ninclude = [\".\"]\n",
		)
		.unwrap();
		std::fs::write(
			dir.join("FORGE.toml"),
			"[library.gen]\nsrcs = [\"gen.c\"]\n\n[binary.app]\ndeps = [{ target = \"gen\", edge = \"host\" }]\nsrcs = [\"app.c\"]\n",
		)
		.unwrap();
		std::fs::write(dir.join("gen.c"), "int g;\n").unwrap();
		std::fs::write(dir.join("app.c"), "int main(void){return 0;}\n").unwrap();
		dir
	}

	fn graph_for(dir: &Path, active: &forge_core::Platform) -> BuildGraph {
		let config = crate::workspace::WorkspaceConfig::load(dir).unwrap();
		let packages = crate::discover::discover_packages(dir, &config.discovery).unwrap();
		let (graph, ..) = load_workspace(dir, &packages, active, &BTreeMap::new(), &[]);
		graph
	}

	#[test]
	fn host_transition_splits_configuration_under_a_different_target() {
		let dir = fixture("split");
		let target = forge_core::Platform {
			os: "none".into(),
			arch: "armv7".into(),
			abi: None,
			cpu: None,
		};
		let graph = graph_for(&dir, &target);
		let variant = graph
			.get(&Label::new("", "gen__host"))
			.expect("a host-configured variant must exist");
		assert_eq!(graph.component(variant).configuration, forge_core::ConfigTransition::Host);
		let app = graph.get(&Label::new("", "app")).unwrap();
		assert!(graph.dependencies_of(app).contains(&variant));
	}

	#[test]
	fn transition_coalesces_when_it_resolves_to_the_active_platform() {
		let dir = fixture("coalesce");
		let graph = graph_for(&dir, &forge_core::Platform::host());
		assert!(
			graph.get(&Label::new("", "gen__host")).is_none(),
			"host and active platform coincide, so no duplicate variant is built"
		);
		let app = graph.get(&Label::new("", "app")).unwrap();
		let target_gen = graph.get(&Label::new("", "gen")).unwrap();
		assert!(graph.dependencies_of(app).contains(&target_gen));
	}
}
