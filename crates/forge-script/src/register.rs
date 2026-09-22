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

pub struct LoadedWorkspace {
	pub graph: BuildGraph,
	pub decls: DeclMap,
	pub dependencies: Vec<forge_core::DependencyRequest>,
	pub requirements: Vec<forge_core::DependencyRequirement>,
	pub candidates: Vec<forge_core::PackageCandidate>,
	pub imported_lock: Option<String>,
	pub diagnostics: Vec<ForgeDiagnostic>,
}

pub fn load_workspace(
	workspace: &std::path::Path,
	packages: &[PackageSource],
	platform: &forge_core::Platform,
	declared_platforms: &std::collections::BTreeMap<String, forge_core::Platform>,
	bootstrap: &[(String, String)],
	cell_configs: &BTreeMap<String, toml::Table>,
) -> LoadedWorkspace {
	load_workspace_resolving(
		workspace,
		packages,
		platform,
		declared_platforms,
		bootstrap,
		cell_configs,
		&crate::rhai_rt::ResolutionContext::default(),
	)
}

pub fn load_workspace_resolving(
	workspace: &std::path::Path,
	packages: &[PackageSource],
	platform: &forge_core::Platform,
	declared_platforms: &std::collections::BTreeMap<String, forge_core::Platform>,
	bootstrap: &[(String, String)],
	cell_configs: &BTreeMap<String, toml::Table>,
	resolution: &crate::rhai_rt::ResolutionContext,
) -> LoadedWorkspace {
	let mut graph = BuildGraph::new();
	let mut files: Vec<(String, Vec<TargetDecl>)> = Vec::new();
	let mut pending_wires: Vec<(String, Vec<TargetDecl>)> = Vec::new();
	let mut dependencies = Vec::new();
	let mut requirements = Vec::new();
	let mut candidates = Vec::new();
	let mut imported_lock = None;
	let mut sink = Vec::new();
	let empty_config = toml::Table::new();

	for (cell, script) in bootstrap {
		let config = cell_configs.get(cell).unwrap_or(&empty_config);
		match crate::rhai_rt::run_forge_rhai_resolving(script, workspace, platform, config, resolution) {
			Ok(output) => {
				if let Some(claim) = output.imported_lock {
					if let Some(existing) = &imported_lock {
						if existing != &claim {
							sink.push(ForgeDiagnostic::error(
								101,
								format!("conflicting imported lock authorities `{existing}` and `{claim}` (cell `{cell}`)"),
							));
						}
					} else {
						imported_lock = Some(claim);
					}
				}
				dependencies.extend(output.dependencies);
				requirements.extend(output.requirements);
				candidates.extend(output.candidates);
				files.push((String::new(), output.targets));
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
			crate::rhai_rt::run_forge_rhai_resolving(&text, &package_dir, platform, &empty_config, resolution).and_then(
				|output| {
					if output.imported_lock.is_some() {
						return Err(ForgeDiagnostic::error(
							101,
							format!(
								"{}: imported lock authority may only be declared by a workspace cell",
								pkg.file.display()
							),
						));
					}
					dependencies.extend(output.dependencies);
					requirements.extend(output.requirements);
					candidates.extend(output.candidates);
					Ok(output.targets)
				},
			)
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

		files.push((pkg.package.clone(), file_decls));
	}

	if imported_lock.is_some() && (!requirements.is_empty() || !candidates.is_empty()) {
		sink.push(ForgeDiagnostic::error(
			101,
			"imported dependencies are externally managed; solver requirements and candidates are not allowed",
		));
	}
	let generated: Vec<_> = files
		.iter()
		.flat_map(|(package, decls)| {
			decls.iter().filter_map(move |decl| {
				if decl.kind != crate::document::TargetKind::Rule {
					return None;
				}
				decl.output_dir.as_ref().map(|dir| DynamicDir {
					dir: dir.trim_end_matches('/').to_string(),
					label: Label::new(package, decl.name.clone()),
				})
			})
		})
		.collect();
	for (package, file_decls) in files {
		let package_dir = workspace.join(&package);
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

		match register_components(&mut graph, usable.clone(), &package, &package_dir, &generated) {
			Ok(_) => pending_wires.push((package, usable)),
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
	if graph.node_count() == 0 && packages.is_empty() && sink.is_empty() {
		sink.push(ForgeDiagnostic::error(
			codes::targets::UNKNOWN_TARGET,
			"no targets registered by workspace cells and no FORGE.toml or FORGE.rhai found",
		));
	}
	LoadedWorkspace {
		graph,
		decls,
		dependencies,
		requirements,
		candidates,
		imported_lock,
		diagnostics: sink,
	}
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
		let ws = load_workspace(dir, &packages, active, &BTreeMap::new(), &[], &config.cell);
		ws.graph
	}

	#[test]
	fn resolution_context_reaches_bootstrap_and_packages() {
		let dir = fixture("resolution-context");
		let script = r#"if !resolving_dependencies { throw "not resolving"; } binary(fetch(cell_config.name), #{});"#;
		let scripts = vec![("demo".into(), script.into())];
		let config = crate::workspace::WorkspaceConfig::parse("[cell.demo]\nname = \"bootstrap\"\n").unwrap();
		let file = dir.join("FORGE.rhai");
		std::fs::write(
			&file,
			r#"if !resolving_dependencies { throw "not resolving"; } binary(fetch("package"), #{});"#,
		)
		.unwrap();
		let packages = vec![PackageSource {
			package: String::new(),
			file,
		}];
		let context = crate::rhai_rt::ResolutionContext {
			resolving: true,
			bytes: BTreeMap::from([
				("bootstrap".to_string(), "bootstrap".to_string()),
				("package".to_string(), "package".to_string()),
			]),
			..Default::default()
		};
		let platform = forge_core::Platform::host();
		let platforms = BTreeMap::new();
		let ws = load_workspace_resolving(&dir, &packages, &platform, &platforms, &scripts, &config.cell, &context);
		assert!(ws.diagnostics.is_empty(), "{:?}", ws.diagnostics);
		assert!(ws.graph.get(&Label::new("", "bootstrap")).is_some());
		assert!(ws.graph.get(&Label::new("", "package")).is_some());
		assert!(context.requested.borrow().is_empty());
		let ws = load_workspace(&dir, &packages, &platform, &platforms, &scripts, &config.cell);
		assert_eq!(ws.diagnostics.len(), 2);
		assert!(ws.diagnostics.iter().all(|error| error.to_string().contains("not resolving")));
		std::fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn bootstrap_targets_use_cell_config_and_wire_root_declarations() {
		let dir = fixture("bootstrap");
		let packages = vec![PackageSource {
			package: String::new(),
			file: dir.join("FORGE.toml"),
		}];
		let config = crate::workspace::WorkspaceConfig::parse("[cell.demo]\nname = \"boot\"\n").unwrap();
		let scripts = vec![(
			"demo".into(),
			r#"rule(cell_config.name, #{ command: "run", deps: ["app"], inputs: ["app.c"] });"#.into(),
		)];
		let LoadedWorkspace {
			graph,
			decls,
			imported_lock,
			diagnostics,
			..
		} = load_workspace(
			&dir,
			&packages,
			&forge_core::Platform::host(),
			&BTreeMap::new(),
			&scripts,
			&config.cell,
		);
		assert!(diagnostics.is_empty(), "{diagnostics:?}");
		assert!(imported_lock.is_none());
		let boot = graph.get(&Label::new("", "boot")).unwrap();
		let app = graph.get(&Label::new("", "app")).unwrap();
		assert!(graph.dependencies_of(boot).contains(&app));
		assert_eq!(decls["//:boot"].resolved_inputs, vec![PathBuf::from("app.c")]);
		std::fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn bootstrap_only_workspace_and_empty_package_discovery() {
		let dir = fixture("bootstrap-only");
		std::fs::remove_file(dir.join("FORGE.toml")).unwrap();
		let config = crate::workspace::WorkspaceConfig::load(&dir).unwrap();
		let packages = crate::discover::discover_packages(&dir, &config.discovery).unwrap();
		assert!(packages.is_empty());
		let scripts = vec![("demo".into(), r#"binary("boot", #{});"#.into())];
		let ws = load_workspace(
			&dir,
			&packages,
			&forge_core::Platform::host(),
			&BTreeMap::new(),
			&scripts,
			&config.cell,
		);
		assert!(ws.diagnostics.is_empty(), "{:?}", ws.diagnostics);
		assert_eq!(ws.graph.node_count(), 1);
		let ws = load_workspace(
			&dir,
			&packages,
			&forge_core::Platform::host(),
			&BTreeMap::new(),
			&[],
			&config.cell,
		);
		assert!(ws.diagnostics.iter().any(|d| d.to_string().contains("no targets registered")));
		std::fs::write(dir.join("FORGE.rhai"), "").unwrap();
		let packages = crate::discover::discover_packages(&dir, &config.discovery).unwrap();
		let ws = load_workspace(
			&dir,
			&packages,
			&forge_core::Platform::host(),
			&BTreeMap::new(),
			&[],
			&config.cell,
		);
		assert!(ws.diagnostics.is_empty(), "{:?}", ws.diagnostics);
		std::fs::remove_dir_all(dir).unwrap();
	}

	#[test]
	fn imported_authority_rejects_conflicts_package_claims_and_solver_inputs() {
		let dir = fixture("authority");
		let first = ("first".into(), r#"imported_lock("external.lock");"#.into());
		let second = ("second".into(), r#"imported_lock("other.lock");"#.into());
		let LoadedWorkspace {
			imported_lock: authority,
			diagnostics,
			..
		} = load_workspace(
			&dir,
			&[],
			&forge_core::Platform::host(),
			&BTreeMap::new(),
			&[first.clone(), second],
			&BTreeMap::new(),
		);
		assert_eq!(authority.as_deref(), Some("external.lock"));
		assert!(
			diagnostics
				.iter()
				.any(|d| d.to_string().contains("conflicting imported lock"))
		);
		let path = dir.join("FORGE.rhai");
		let packages = vec![PackageSource {
			package: String::new(),
			file: path.clone(),
		}];
		for (script, expected) in [
			(r#"imported_lock("external.lock");"#, "only be declared by a workspace cell"),
			(
				r#"dependency_require("demo", "1.0.0", "2.0.0");"#,
				"solver requirements and candidates",
			),
			(
				r#"dependency_candidate("demo", "1.0.0", "https://example.invalid/demo", "sha", []);"#,
				"solver requirements and candidates",
			),
		] {
			std::fs::write(&path, script).unwrap();
			let LoadedWorkspace { diagnostics, .. } = load_workspace(
				&dir,
				&packages,
				&forge_core::Platform::host(),
				&BTreeMap::new(),
				std::slice::from_ref(&first),
				&BTreeMap::new(),
			);
			assert!(
				diagnostics.iter().any(|d| d.to_string().contains(expected)),
				"{diagnostics:?}"
			);
		}
		std::fs::remove_dir_all(dir).unwrap();
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
