use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::{
	ActionSpec, ArgumentFile, BuildGraph, ComponentId, ComponentKind, EnvironmentFile, OutputDeclaration, OutputKind,
	Platform, Profile, WorkerBinding,
};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::TargetDecl;
use forge_script::cells::{
	ActionDecl, CellHooks, CellPlan, CellSession, ComponentView, FetchedSource, ProfileView, WorkspaceHooks, lower, plan,
};

use crate::std_cells::StdCells;
use crate::store::hasher;
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
	pub cell_config: &'a BTreeMap<String, toml::Table>,
	pub workspace: Option<&'a Path>,
	pub fetched_sources: &'a [FetchedSource],
	pub progress: Option<&'a crate::build::progress::Progress>,
}

pub type DeclMap = BTreeMap<String, TargetDecl>;

struct Planner<'a> {
	ctx: &'a PlanContext<'a>,
	dag: ActionDag,
	producer_of: BTreeMap<PathBuf, usize>,
	archive_of: BTreeMap<String, PathBuf>,
	last_action_of: BTreeMap<String, usize>,
	plans: BTreeMap<String, CellPlan>,
}

pub fn build_action_dag(ctx: &PlanContext<'_>) -> Result<ActionDag, ForgeDiagnostic> {
	let order = ctx.graph.topological_order().map_err(cycle_error)?;
	let mut planner = Planner {
		ctx,
		dag: ActionDag::default(),
		producer_of: BTreeMap::new(),
		archive_of: BTreeMap::new(),
		last_action_of: BTreeMap::new(),
		plans: BTreeMap::new(),
	};
	for id in order {
		let component = ctx.graph.component(id);
		let platform = component.configuration.resolve(ctx.platform);
		check_compatibility(component, &platform)?;
		match component.kind {
			ComponentKind::Library { .. } => planner.plan_component(id, "library", &platform)?,
			ComponentKind::Binary => planner.plan_component(id, "binary", &platform)?,
			ComponentKind::Test => planner.plan_component(id, "test", &platform)?,
			ComponentKind::Generic { .. } => planner.plan_rule(id)?,
		}
	}
	planner.link_component_edges()?;
	planner.adopt_component_inputs();
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

	fn adopt_component_inputs(&mut self) {
		let mut present: Vec<BTreeSet<PathBuf>> = self
			.dag
			.specs
			.iter()
			.map(|spec| spec.execution_deps.iter().cloned().collect())
			.collect();
		let mut queue: Vec<usize> = (0..self.dag.specs.len()).rev().collect();
		while let Some(index) = queue.pop() {
			let component = self.dag.specs[index].component.clone();
			let producers: Vec<usize> = self.dag.specs[index]
				.execution_deps
				.iter()
				.filter_map(|path| self.producer_of.get(path).copied())
				.filter(|&producer| self.dag.specs[producer].component != component)
				.collect();
			for producer in producers {
				let inherited = self.dag.specs[producer].execution_deps.clone();
				let mut added = false;
				for path in inherited {
					if present[index].insert(path.clone()) {
						self.dag.specs[index].execution_deps.push(path);
						added = true;
					}
				}
				if added {
					queue.push(producer);
					queue.push(index);
				}
			}
		}
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
		let label = &self.ctx.graph.component(id).label;
		let key = label.to_string();
		if let Some(decl) = self.ctx.decls.get(&key) {
			return Ok(decl);
		}
		if let Some((base, _)) = label.name().split_once("__") {
			let base_key = forge_core::Label::new(label.package(), base).to_string();
			if let Some(decl) = self.ctx.decls.get(&base_key) {
				return Ok(decl);
			}
		}
		Err(ForgeDiagnostic::error(
			codes::targets::UNKNOWN_TARGET,
			format!("no declaration found for component `{key}`"),
		))
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

	fn worker_binding(
		&self,
		subject: &str,
		variant: &str,
		toolchain: Option<&ResolvedToolchain>,
	) -> Result<WorkerBinding, ForgeDiagnostic> {
		let Some(toolchain) = toolchain else {
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("`{subject}` requests worker variant `{variant}` but names no toolchain"),
			)
			.with_help("a worker belongs to a toolchain: add `compiler = \"<toolchain>\"`"));
		};
		let Some(program) = &toolchain.worker else {
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!(
					"`{subject}` requests worker variant `{variant}` but toolchain `{}` declares no worker",
					toolchain.name
				),
			)
			.with_help("declare `[toolchains.<name>.worker]` in the toolchain catalog, or drop `worker`"));
		};
		if !program.accepts(variant) {
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("toolchain `{}` does not declare worker variant `{variant}`", toolchain.name),
			)
			.with_help(format!("declared variants: {}", program.variants.join(", "))));
		}
		let program = crate::toolchain::resolve_tool_reference(self.ctx.toolchains, &program.command)?;
		Ok(WorkerBinding {
			program,
			variant: variant.to_string(),
		})
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
		let command = if Path::new(&command)
			.parent()
			.is_some_and(|parent| !parent.as_os_str().is_empty())
		{
			command
		} else {
			crate::toolchain::resolve_tool_reference(self.ctx.toolchains, &command)?
		};
		let toolchain = decl.compiler.as_deref().and_then(|name| self.ctx.toolchains.get(name));
		let toolchain_id = toolchain.map(tool_id);
		let worker = match &decl.worker {
			Some(variant) => Some(self.worker_binding(&label, variant, toolchain)?),
			None => None,
		};
		let outputs = match &decl.output_dir {
			Some(dir) => vec![OutputDeclaration {
				path: PathBuf::from(dir),
				kind: OutputKind::Directory,
			}],
			None => decl
				.outputs
				.iter()
				.map(|(path, is_dir)| OutputDeclaration {
					path: path.clone(),
					kind: if *is_dir { OutputKind::Directory } else { OutputKind::File },
				})
				.collect(),
		};
		self.emit(ActionSpec {
			name: format!("rule {label}"),
			component: label,
			configuration: self.ctx.graph.component(id).configuration,
			command,
			args: decl.args.clone(),
			inputs: decl.resolved_inputs.clone(),
			execution_deps: Vec::new(),
			outputs,
			workdir: None,
			is_test: false,
			stdout: None,
			compile_command: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: decl.env.clone(),
			toolchain_id,
			worker,
		});
		Ok(())
	}

	fn plan_component(
		&mut self,
		id: ComponentId,
		kind: &'static str,
		platform: &forge_core::Platform,
	) -> Result<(), ForgeDiagnostic> {
		let component = self.ctx.graph.component(id);
		let label = component.label.to_string();
		let artifact_namespace = label.replace(['/', ':'], "_");
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

		let profile_name = self.ctx.profile.name.clone();
		let artifact_profile_name = profile_name.clone();
		let pkg_slug = self.package_slug(id);
		let name = component.label.name().to_string();
		let tool = self.tool_for(&decl, &language)?.clone();
		let worker = match &decl.worker {
			Some(variant) => Some(self.worker_binding(&label, variant, Some(&tool))?),
			None => None,
		};
		let tool_for_bin = tool.clone();
		let tool_digest = tool_id(&tool);
		let all_toolchains = self.ctx.toolchains.clone();
		let dir_toolchains = all_toolchains.clone();

		let hooks = CellHooks {
			workspace: self.workspace_hooks(),
			artifact_path: Box::new(move |src, category| {
				artifact_path(Path::new(src), &artifact_profile_name, &artifact_namespace, category)
			}),
			depfile_inputs: {
				let workspace = self.ctx.workspace.unwrap_or(Path::new(".")).to_path_buf();
				let roots: Vec<PathBuf> = self.ctx.toolchains.values().map(|tool| tool.root.clone()).collect();
				Box::new(move |path| depfile_inputs(&workspace, Path::new(path), &roots))
			},
			lib_path: Box::new(move |filename| lib_path(&profile_name, &pkg_slug, filename).to_string_lossy().into_owned()),
			bin: Box::new(move |binary_name| {
				tool_for_bin
					.binary(binary_name)
					.and_then(|p| tool_for_bin.reference(&p).ok())
					.unwrap_or_default()
			}),
			tool_bin: Box::new(move |binary_name| {
				crate::toolchain::resolve_tool_reference(&all_toolchains, binary_name).unwrap_or_default()
			}),
			toolchain_dir: Box::new(move |toolchain| {
				dir_toolchains
					.get(toolchain)
					.map(|tool| format!("{}/{}", crate::toolchain::TOOLCHAIN_TOKEN, tool.id()))
					.unwrap_or_default()
			}),
			tool_id: Box::new(move || tool_digest.clone()),
		};

		let dep_artifacts = self.collect_dep_artifacts(id);
		let mut session = self.session(&language);
		session.platform_os = platform.os.clone();
		session.platform_arch = platform.arch.clone();
		session.platform_abi = platform.abi.clone().unwrap_or_default();
		let cell_plan = self.plan_cell(&language, script, &session)?;

		let view = ComponentView {
			metadata: decl.metadata.clone(),
			session,
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
			linker: match &decl.linker {
				Some(spec) => crate::toolchain::resolve_tool_reference(self.ctx.toolchains, spec)?,
				None => String::new(),
			},
			link_flags: decl.link_flags.clone(),
		};

		let actions = lower(script, &cell_plan, &view, hooks)?;
		self.register_cell_actions(actions, id, kind, worker)
	}

	fn session(&self, language: &str) -> CellSession {
		CellSession {
			fetched_sources: self.ctx.fetched_sources.to_vec(),
			workspace: self
				.ctx
				.workspace
				.map(|p| p.to_string_lossy().into_owned())
				.unwrap_or_default(),
			platform_os: self.ctx.platform.os.clone(),
			platform_arch: self.ctx.platform.arch.clone(),
			platform_abi: self.ctx.platform.abi.clone().unwrap_or_default(),
			profile: ProfileView::from(self.ctx.profile),
			cell_config: self.ctx.cell_config.get(language).cloned().unwrap_or_default(),
			targets: self.ctx.decls.values().map(|decl| decl.metadata.clone()).collect(),
		}
	}

	fn plan_cell(&mut self, language: &str, script: &str, session: &CellSession) -> Result<CellPlan, ForgeDiagnostic> {
		if let Some(planned) = self.plans.get(language) {
			return Ok(planned.clone());
		}
		if let Some(progress) = self.ctx.progress {
			progress.phase(&format!("Planning {language}..."));
		}
		let planned = plan(script, session, self.workspace_hooks())?;
		self.plans.insert(language.to_string(), planned.clone());
		Ok(planned)
	}

	fn workspace_hooks(&self) -> WorkspaceHooks {
		let root = self.ctx.workspace.map(|p| p.to_path_buf()).unwrap_or_default();
		let reads = root.clone();
		let globs = root.clone();
		WorkspaceHooks {
			read_file: Box::new(move |path: &str| -> Result<String, String> {
				let full = if Path::new(path).is_absolute() {
					PathBuf::from(path)
				} else {
					reads.join(path)
				};
				std::fs::read_to_string(&full).map_err(|e| format!("cannot read {}: {e}", full.display()))
			}),
			glob: Box::new(move |pattern: &str| -> Result<Vec<String>, String> {
				let hits = forge_script::glob::expand_glob(&globs, pattern).map_err(|e| e.to_string())?;
				Ok(hits.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
			}),
		}
	}

	fn register_cell_actions(
		&mut self,
		actions: Vec<ActionDecl>,
		id: ComponentId,
		kind: &'static str,
		worker: Option<WorkerBinding>,
	) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let archive_path: Option<PathBuf> = actions
			.iter()
			.find(|action| action.artifact.is_some())
			.and_then(|action| action.artifact.clone())
			.map(PathBuf::from);

		for action in actions {
			let spec = ActionSpec {
				name: action.name.clone(),
				component: label.clone(),
				configuration: action.configuration,
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
				is_test: action.is_test,
				stdout: action.stdout.map(PathBuf::from),
				compile_command: action.compile_command.clone(),
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
				worker: worker
					.clone()
					.filter(|_| action.toolchain_id.as_ref().is_some_and(|id| !id.is_empty())),
			};
			self.emit(spec);
		}

		if kind == "library" {
			let Some(archive) = archive_path else {
				return Err(ForgeDiagnostic::error(
					codes::script::WRONG_TYPE,
					format!("cell for library `{label}` declared no `artifact`"),
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
	tool.id()
}

fn lib_path(profile: &str, package: &str, filename: &str) -> PathBuf {
	let mut path = PathBuf::from("forge-out/lib");
	path.push(profile);
	if !package.is_empty() {
		path.push(package);
	}
	path.push(filename);
	path
}

fn artifact_path(source: &Path, profile: &str, namespace: &str, category: &str) -> Result<String, String> {
	let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("src");
	for fragment in [profile, namespace, category, stem] {
		if fragment.is_empty() || matches!(fragment, "." | "..") || fragment.contains(['/', '\\', ':', '\0']) {
			return Err(format!("artifact_path requires single path components, got {fragment:?}"));
		}
	}
	let suffix = hasher::hex(blake3::hash(source.to_string_lossy().as_bytes()).as_bytes())[..8].to_string();
	Ok(format!("forge-out/{category}/{profile}/{namespace}_{stem}_{suffix}"))
}

fn depfile_inputs(workspace: &Path, depfile: &Path, toolchains: &[PathBuf]) -> Result<Vec<String>, String> {
	let workspace = std::path::absolute(workspace).map_err(|e| e.to_string())?;
	let path = workspace.join(depfile);
	let text = match std::fs::read_to_string(&path) {
		Ok(text) => text,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(e) => return Err(format!("cannot read depfile {}: {e}", path.display())),
	};
	forge_core::depfile::parse(&text)
		.into_iter()
		.filter_map(|input| depfile_input_path(&workspace, Path::new(&input), toolchains).transpose())
		.map(|result| result.map(|p| path_string(&p)))
		.collect()
}

fn depfile_input_path(workspace: &Path, input: &Path, toolchains: &[PathBuf]) -> Result<Option<PathBuf>, String> {
	if input.is_absolute() && toolchains.iter().any(|root| input.starts_with(root)) {
		return Ok(None);
	}
	let relative = if let Ok(path) = input.strip_prefix("FORGE_EXEC_ROOT") {
		path
	} else if input.is_absolute() {
		if let Ok(path) = input.strip_prefix(workspace.join("forge-out/exec")) {
			path
		} else if let Ok(path) = input.strip_prefix(workspace.join("forge-out/sandbox")) {
			let mut components = path.components();
			components.next();
			components.as_path()
		} else {
			input
				.strip_prefix(workspace)
				.map_err(|_| format!("depfile input is outside workspace: {}", input.display()))?
		}
	} else {
		input
	};
	let mut normalized = PathBuf::new();
	for component in relative.components() {
		match component {
			std::path::Component::CurDir => {}
			std::path::Component::Normal(part) => normalized.push(part),
			std::path::Component::ParentDir if normalized.pop() => {}
			_ => return Err(format!("depfile input escapes workspace: {}", input.display())),
		}
	}
	if normalized.as_os_str().is_empty() {
		return Err(format!("depfile input is not a file path: {}", input.display()));
	}
	Ok(Some(normalized))
}

#[cfg(test)]
mod tests;
