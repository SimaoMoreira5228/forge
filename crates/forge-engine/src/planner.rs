use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::{
	ActionSpec, ArgumentFile, BuildGraph, ComponentId, ComponentKind, EnvironmentFile, OutputDeclaration, OutputKind,
	Platform, Profile,
};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::TargetDecl;
use forge_script::cells::{ActionDecl, CellHooks, ComponentView, FetchedSource, lower};

use crate::hasher;
use crate::std_cells::StdCells;
use crate::toolchain::ResolvedToolchain;

#[derive(Debug, Default)]
pub struct ActionDag {
	pub specs: Vec<ActionSpec>,
	pub deps: Vec<Vec<usize>>,
}

impl ActionDag {
	pub fn dependents(&self) -> Vec<Vec<usize>> {
		let mut rev = vec![Vec::new(); self.specs.len()];
		for (i, ds) in self.deps.iter().enumerate() {
			for &d in ds {
				rev[d].push(i);
			}
		}
		rev
	}

	pub fn tests(&self) -> Vec<usize> {
		self.specs
			.iter()
			.enumerate()
			.filter(|(_, s)| s.is_test)
			.map(|(i, _)| i)
			.collect()
	}
}

pub struct PlanContext<'a> {
	pub graph: &'a BuildGraph,
	pub decls: &'a BTreeMap<String, TargetDecl>,
	pub profile: &'a Profile,
	pub platform: &'a Platform,
	pub toolchains: &'a BTreeMap<String, ResolvedToolchain>,
	pub cells: &'a StdCells,
	pub workspace: Option<&'a Path>,
	pub fetched_sources: &'a [FetchedSource],
	pub progress: Option<&'a crate::progress::Progress>,
}

pub type DeclMap = BTreeMap<String, TargetDecl>;

struct Planner<'a> {
	ctx: &'a PlanContext<'a>,
	dag: ActionDag,
	producer_of: BTreeMap<PathBuf, usize>,
	archive_of: BTreeMap<String, PathBuf>,
	last_action_of: BTreeMap<String, usize>,
	fetched_emitted: bool,
}

pub fn build_action_dag(ctx: &PlanContext<'_>) -> Result<ActionDag, ForgeDiagnostic> {
	let order = ctx.graph.topological_order().map_err(cycle_error)?;
	let mut planner = Planner {
		ctx,
		dag: ActionDag::default(),
		producer_of: BTreeMap::new(),
		archive_of: BTreeMap::new(),
		last_action_of: BTreeMap::new(),
		fetched_emitted: false,
	};
	for id in order {
		let component = ctx.graph.component(id);
		check_compatibility(component, ctx.platform)?;
		match component.kind {
			ComponentKind::Library { .. } => planner.plan_component(id, "library")?,
			ComponentKind::Binary => planner.plan_component(id, "binary")?,
			ComponentKind::Test => planner.plan_component(id, "test")?,
			ComponentKind::Generic { .. } => planner.plan_rule(id)?,
		}
	}
	planner.link_component_edges()?;
	planner.link_execution_edges()?;
	planner.validate_action_dag()?;
	Ok(planner.dag)
}

impl<'a> Planner<'a> {
	fn emit(&mut self, mut spec: ActionSpec) -> usize {
		let unique: BTreeSet<PathBuf> = spec.inputs.iter().cloned().collect();
		spec.inputs = unique.into_iter().collect();
		let unique: BTreeSet<PathBuf> = spec.execution_deps.iter().cloned().collect();
		spec.execution_deps = unique.into_iter().collect();

		let index = self.dag.specs.len();
		let deps = Vec::new();
		for output in &spec.outputs {
			self.producer_of.insert(output.path.clone(), index);
		}
		self.last_action_of.insert(spec.component.clone(), index);
		self.dag.specs.push(spec);
		self.dag.deps.push(deps);
		index
	}

	fn link_component_edges(&mut self) -> Result<(), ForgeDiagnostic> {
		let mut additions: Vec<(usize, usize)> = Vec::new();
		for (index, spec) in self.dag.specs.iter().enumerate() {
			let Some(node) = self.ctx.graph.get(
				&forge_core::Label::parse(&spec.component, "")
					.unwrap_or_else(|_| forge_core::Label::new("", &spec.component)),
			) else {
				continue;
			};
			for dep in self.ctx.graph.dependencies_of(node) {
				let dep_label = self.ctx.graph.component(dep).label.to_string();
				if let Some(&last) = self.last_action_of.get(&dep_label) {
					additions.push((index, last));
				}
			}
		}
		for (index, predecessor) in additions {
			let slot = &mut self.dag.deps[index];
			if !slot.contains(&predecessor) {
				slot.push(predecessor);
			}
		}
		Ok(())
	}

	fn link_execution_edges(&mut self) -> Result<(), ForgeDiagnostic> {
		for (index, spec) in self.dag.specs.clone().iter().enumerate() {
			for dependency in &spec.execution_deps {
				let Some(&producer) = self.producer_of.get(dependency) else {
					return Err(ForgeDiagnostic::error(
						codes::inputs::MISSING_INPUT,
						format!(
							"action `{}` depends on unknown produced path `{}`",
							spec.name,
							dependency.display()
						),
					));
				};
				if producer == index {
					return Err(ForgeDiagnostic::error(
						codes::graph::CYCLE_DETECTED,
						format!("action `{}` depends on its own output `{}`", spec.name, dependency.display()),
					));
				}
				if !self.dag.deps[index].contains(&producer) {
					self.dag.deps[index].push(producer);
				}
			}
		}
		Ok(())
	}

	fn validate_action_dag(&self) -> Result<(), ForgeDiagnostic> {
		let mut indegree: Vec<usize> = self.dag.deps.iter().map(Vec::len).collect();
		let mut ready: Vec<usize> = indegree
			.iter()
			.enumerate()
			.filter_map(|(index, degree)| (*degree == 0).then_some(index))
			.collect();
		let dependents = self.dag.dependents();
		let mut visited = 0;
		while let Some(index) = ready.pop() {
			visited += 1;
			for &dependent in &dependents[index] {
				indegree[dependent] -= 1;
				if indegree[dependent] == 0 {
					ready.push(dependent);
				}
			}
		}
		if visited == self.dag.specs.len() {
			Ok(())
		} else {
			Err(ForgeDiagnostic::error(
				codes::graph::CYCLE_DETECTED,
				"action dependency cycle detected",
			))
		}
	}

	fn decl_for(&self, id: ComponentId) -> Result<&TargetDecl, ForgeDiagnostic> {
		let key = self.ctx.graph.component(id).label.to_string();
		self.ctx.decls.get(&key).ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::targets::UNKNOWN_TARGET,
				format!("no declaration found for component `{key}`"),
			)
		})
	}

	fn package_slug(&self, id: ComponentId) -> String {
		self.ctx.graph.component(id).label.package().replace('/', "_")
	}

	fn tool_for(&self, decl: &TargetDecl, language: &str) -> Result<&ResolvedToolchain, ForgeDiagnostic> {
		if let Some(name) = &decl.compiler
			&& let Some(tool) = self.ctx.toolchains.get(name.as_str())
		{
			return Ok(tool);
		}
		let candidates = self.ctx.cells.toolchain_candidates(language);
		for name in candidates {
			if let Some(tool) = self.ctx.toolchains.get(name.as_str()) {
				return Ok(tool);
			}
		}
		if let Some(tool) = self.ctx.toolchains.values().next() {
			return Ok(tool);
		}
		Err(ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!(
				"component `{}` needs compiler {:?} but FORGE_ROOT configures: {}",
				decl.name,
				decl.compiler,
				self.ctx.toolchains.keys().cloned().collect::<Vec<_>>().join(", ")
			),
		)
		.with_help("add a [toolchains.<name>] section to FORGE_ROOT"))
	}

	fn plan_rule(&mut self, id: ComponentId) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let decl = self.decl_for(id)?.clone();
		if decl.command.as_deref().is_none_or(str::is_empty) {
			return Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!("rule `{label}` has no command"),
			));
		}
		let command = decl.command.expect("checked above");
		let command = if command.contains('/') {
			command
		} else {
			crate::toolchain::resolve_tool_path(self.ctx.toolchains, &command)?
				.to_string_lossy()
				.into_owned()
		};
		let toolchain_id = decl
			.compiler
			.as_deref()
			.and_then(|name| self.ctx.toolchains.get(name))
			.map(tool_id);
		self.emit(ActionSpec {
			name: format!("rule {label}"),
			component: label,
			command,
			args: decl.args.clone(),
			inputs: decl.resolved_inputs.clone(),
			execution_deps: Vec::new(),
			outputs: decl
				.outputs
				.iter()
				.map(|(path, is_dir)| OutputDeclaration {
					path: path.clone(),
					kind: if *is_dir { OutputKind::Directory } else { OutputKind::File },
				})
				.collect(),
			workdir: None,
			is_test: false,
			stdout: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: decl.env.clone(),
			toolchain_id,
		});
		Ok(())
	}

	fn plan_component(&mut self, id: ComponentId, kind: &'static str) -> Result<(), ForgeDiagnostic> {
		let component = self.ctx.graph.component(id);
		let label = component.label.to_string();
		let object_namespace = label.replace(['/', ':'], "_");
		if let Some(progress) = self.ctx.progress {
			progress.phase(&format!("Lowering {label}..."));
		}
		if component.sources.is_empty() {
			return Err(ForgeDiagnostic::error(
				codes::inputs::MISSING_INPUT,
				format!("`{label}` declares no sources"),
			));
		}
		let language = cell_language(self.ctx.cells, &component.sources[0]).ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!(
					"`{label}` declares `{}`, which no std cell handles",
					component.sources[0].display()
				),
			)
			.with_help(format!(
				"supported source types: {}",
				render_supported_languages(self.ctx.cells)
			))
		})?;
		let script = self.ctx.cells.get(&language)?;

		let decl = self.decl_for(id)?.clone();
		let mut hdrs: Vec<String> = component.headers.iter().map(|p| path_string(p)).collect();
		let mut includes = decl.includes.clone();
		let dep_archives = if kind == "library" {
			Vec::new()
		} else {
			self.transitive_archives(id)
		};
		for &dep in self.ctx.graph.transitive_dependencies(id).iter() {
			hdrs.extend(self.ctx.graph.component(dep).headers.iter().map(|p| path_string(p)));
			if let Ok(dep_decl) = self.decl_for(dep) {
				includes.extend(dep_decl.includes.iter().cloned());
			}
		}

		let profile = self.ctx.profile.clone();
		let pkg_slug = self.package_slug(id);
		let name = component.label.name().to_string();
		let tool = self.tool_for(&decl, &language)?.clone();
		let tool_for_bin = tool.clone();
		let tool_digest = tool_id(&tool);

		let hooks = CellHooks {
			obj_path: Box::new(move |src| {
				object_path(Path::new(src), &profile, &object_namespace)
					.to_string_lossy()
					.into_owned()
			}),
			prior_depfile_headers: Box::new(|obj| {
				previous_depfile_headers(&PathBuf::from(format!("{obj}.d")))
					.into_iter()
					.map(|p| p.to_string_lossy().into_owned())
					.collect()
			}),
			lib_path: Box::new(move |component_name| {
				PathBuf::from(format!("forge-out/lib/{pkg_slug}/lib{component_name}.a"))
					.to_string_lossy()
					.into_owned()
			}),
			bin: Box::new(move |binary_name| {
				tool_for_bin
					.binary(binary_name)
					.map(|p| p.to_string_lossy().into_owned())
					.unwrap_or_default()
			}),
			tool_id: Box::new(move || tool_digest.clone()),
			read_file: {
				let ws = self.ctx.workspace.map(|p| p.to_path_buf()).unwrap_or_default();
				Box::new(move |path: &str| -> Result<String, String> {
					let full = if Path::new(path).is_absolute() {
						PathBuf::from(path)
					} else {
						ws.join(path)
					};
					std::fs::read_to_string(&full).map_err(|e| format!("cannot read {}: {e}", full.display()))
				})
			},
			glob: {
				let ws = self.ctx.workspace.map(|p| p.to_path_buf()).unwrap_or_default();
				Box::new(move |pattern: &str| -> Result<Vec<String>, String> {
					let hits = forge_script::glob::expand_glob(&ws, pattern).map_err(|e| e.to_string())?;
					Ok(hits.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
				})
			},
		};

		let dep_artifacts = self.collect_dep_artifacts(id);
		let fetch_owner = !self.fetched_emitted && !self.ctx.fetched_sources.is_empty();
		if fetch_owner {
			self.fetched_emitted = true;
		}

		let view = ComponentView {
			label,
			name,
			kind: kind.to_string(),
			compiler: decl.compiler.clone().unwrap_or_default(),
			srcs: component.sources.iter().map(|p| path_string(p)).collect(),
			hdrs,
			includes,
			defines: decl.defines.clone(),
			flags: decl.flags.clone(),
			standard: decl
				.standard
				.clone()
				.or_else(|| self.ctx.cells.standard_for(&language).map(|s| s.to_string())),
			system_libs: decl.system_libs.clone(),
			run_args: decl.args.clone(),
			data: decl.data.iter().map(|p| path_string(p)).collect(),
			env: decl.env.clone(),
			dep_archives: dep_archives.iter().map(|p: &PathBuf| path_string(p)).collect(),
			dep_artifacts,
			fetched_sources: self.ctx.fetched_sources.to_vec(),
			fetch_owner,
			workspace: self
				.ctx
				.workspace
				.map(|p| p.to_string_lossy().into_owned())
				.unwrap_or_default(),
			linker: match &decl.linker {
				Some(spec) => crate::toolchain::resolve_tool_path(self.ctx.toolchains, spec)?
					.to_string_lossy()
					.into_owned(),
				None => String::new(),
			},
			link_flags: decl.link_flags.clone(),
			platform_os: self.ctx.platform.os.clone(),
			platform_arch: self.ctx.platform.arch.clone(),
			platform_abi: self.ctx.platform.abi.clone().unwrap_or_default(),
			profile: forge_script::cells::ProfileView {
				name: self.ctx.profile.name.clone(),
				opt_level: i64::from(self.ctx.profile.opt_level),
				debug: self.ctx.profile.debug,
				lto: self.ctx.profile.lto,
				strip: self.ctx.profile.strip,
				coverage: self.ctx.profile.coverage,
				defines: self.ctx.profile.defines.clone(),
				sanitizers: self.ctx.profile.sanitizers.clone(),
			},
		};

		let actions = lower(script, &view, hooks)?;
		self.register_cell_actions(actions, id, kind)
	}

	fn register_cell_actions(
		&mut self,
		actions: Vec<ActionDecl>,
		id: ComponentId,
		kind: &'static str,
	) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let mut archive_path: Option<PathBuf> = None;

		for action in actions {
			if kind == "library" && archive_path.is_none() {
				for (out, _) in &action.outputs {
					let p = PathBuf::from(out);
					if p.starts_with("forge-out/lib") && !p.starts_with("forge-out/lib/deps") {
						archive_path = Some(p);
						break;
					}
				}
			}
			let spec = ActionSpec {
				name: action.name.clone(),
				component: label.clone(),
				command: action.command.clone(),
				args: action.args.clone(),
				inputs: action.inputs.iter().map(PathBuf::from).collect(),
				execution_deps: action.execution_deps.iter().map(PathBuf::from).collect(),
				outputs: action
					.outputs
					.iter()
					.map(|(path, is_dir)| OutputDeclaration {
						path: PathBuf::from(path),
						kind: if *is_dir { OutputKind::Directory } else { OutputKind::File },
					})
					.collect(),
				workdir: action.workdir.map(PathBuf::from),
				is_test: kind == "test" && action.name.starts_with("run "),
				stdout: action.stdout.map(PathBuf::from),
				environment_files: action
					.environment_files
					.into_iter()
					.map(|(key_prefix, path, line_prefix, ignored_keys)| EnvironmentFile {
						path: PathBuf::from(path),
						line_prefix,
						key_prefix,
						ignored_keys,
					})
					.collect(),
				argument_files: action
					.argument_files
					.into_iter()
					.map(|(path, line_prefix, flag, root_marker)| ArgumentFile {
						path: PathBuf::from(path),
						line_prefix,
						flag,
						root_marker,
					})
					.collect(),
				env: action.env.clone(),
				toolchain_id: action.toolchain_id.clone(),
			};
			self.emit(spec);
		}

		if kind == "library" {
			let Some(archive) = archive_path else {
				return Err(ForgeDiagnostic::error(
					codes::script::WRONG_TYPE,
					format!("cell for `{label}` did not declare any output under `forge-out/lib`"),
				));
			};
			self.archive_of.insert(label, archive);
		}
		Ok(())
	}

	fn collect_dep_artifacts(&self, id: ComponentId) -> Vec<(String, String)> {
		let mut artifacts = Vec::new();
		for dep in self.ctx.graph.transitive_dependencies(id) {
			let dep_label = self.ctx.graph.component(dep).label.clone();
			let label = dep_label.to_string();
			if let Some(path) = self.archive_of.get(&label) {
				artifacts.push((dep_label.name().to_string(), path.to_string_lossy().into_owned()));
			}
		}
		artifacts
	}

	fn transitive_archives(&self, id: ComponentId) -> Vec<PathBuf> {
		self.ctx
			.graph
			.transitive_dependencies(id)
			.iter()
			.filter_map(|&dep| {
				let dep_label = self.ctx.graph.component(dep).label.to_string();
				self.archive_of.get(&dep_label).cloned()
			})
			.collect()
	}
}

fn cell_language(cells: &StdCells, source: &Path) -> Option<String> {
	let extension = source.extension()?.to_str()?;
	cells.cell_for_extension(extension).map(|s| s.to_string())
}

fn render_supported_languages(cells: &StdCells) -> String {
	cells.supported_languages()
}

fn path_string(path: &Path) -> String {
	path.to_string_lossy().into_owned()
}

fn check_compatibility(component: &forge_core::Component, platform: &Platform) -> Result<(), ForgeDiagnostic> {
	if component.is_compatible_with(platform) {
		return Ok(());
	}
	Err(ForgeDiagnostic::error(
		codes::platform::CONSTRAINT_VIOLATION,
		format!(
			"`{}` is incompatible with platform {}",
			component.label,
			platform.catalog_key()
		),
	)
	.with_help(format!("requires {}", component.compatible_with.join(", "))))
}

fn cycle_error(cycles: Vec<Vec<forge_core::Label>>) -> ForgeDiagnostic {
	let rendered: Vec<String> = cycles
		.iter()
		.map(|cycle| cycle.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" -> "))
		.collect();
	ForgeDiagnostic::error(
		codes::graph::CYCLE_DETECTED,
		format!("dependency cycle detected: {}", rendered.join("; ")),
	)
}

fn tool_id(tool: &ResolvedToolchain) -> String {
	format!("{}@{}", tool.name, &tool.digest[..12.min(tool.digest.len())])
}

fn object_path(source: &Path, profile: &Profile, namespace: &str) -> PathBuf {
	let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("src");
	let suffix = hasher::hex(blake3::hash(source.to_string_lossy().as_bytes()).as_bytes())[..8].to_string();
	let directory = if profile.coverage { "profile" } else { "obj" };
	PathBuf::from(format!(
		"forge-out/{directory}/{}/{}_{}_{suffix}.o",
		profile.name, namespace, stem
	))
}

fn previous_depfile_headers(depfile: &Path) -> Vec<PathBuf> {
	let Ok(text) = std::fs::read_to_string(depfile) else {
		return Vec::new();
	};
	parse_depfile(&text)
}

fn parse_depfile(text: &str) -> Vec<PathBuf> {
	const HEADER_EXTS: &[&str] = &["h", "hh", "hpp", "hxx", "inc", "inl"];
	let flattened = text.replace("\\\n", " ");
	let Some((_, rest)) = flattened.split_once(':') else {
		return Vec::new();
	};
	rest.split_whitespace()
		.filter(|token| *token != "\\")
		.filter_map(|token| {
			let path = PathBuf::from(token.replace('\\', ""));
			let is_header = path
				.extension()
				.and_then(|e| e.to_str())
				.map(|e| HEADER_EXTS.contains(&e))
				.unwrap_or(false);
			(is_header && path.is_file()).then_some(path)
		})
		.collect()
}
