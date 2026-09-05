use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use forge_core::{ActionSpec, ComponentKind};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::discover_packages;
use forge_script::register::{LoadedWorkspace, load_workspace_resolving};
use forge_script::rhai_rt::ResolutionContext;

use crate::cas::Cas;
use crate::db::CacheDb;
use crate::hasher;
use crate::lock::FileLock;
use crate::planner::{ActionDag, PlanContext, build_action_dag};
use crate::runner::SandboxRunner;
use crate::schedule::execute_dag;
use crate::toolchain::{ResolvedToolchain, ToolchainPaths, ToolchainStore};
use crate::worker::WorkerPool;

pub struct Engine {
	pub(crate) workspace: PathBuf,
}

#[derive(Debug, Default)]
pub struct BuildOutcome {
	pub actions: usize,
	pub cache_hits: usize,
	pub test_cache_hits: usize,
	pub executed: usize,
	pub binaries: BTreeMap<String, PathBuf>,
	pub tests: Vec<TestResult>,
}

#[derive(Debug, Clone)]
pub struct TestResult {
	pub component: String,
	pub verdict: &'static str,
	pub duration_ms: u128,
	pub stderr_tail: String,
}

pub struct Prepared {
	pub config: forge_script::WorkspaceConfig,
	pub graph: forge_core::BuildGraph,
	pub decls: forge_script::DeclMap,
	pub dependencies: Vec<forge_core::DependencyRequest>,
	pub requirements: Vec<forge_core::DependencyRequirement>,
	pub candidates: Vec<forge_core::PackageCandidate>,
	pub imported_lock: Option<String>,
}

impl Engine {
	pub fn open(workspace: &Path) -> Self {
		Self {
			workspace: workspace.to_path_buf(),
		}
	}

	pub(crate) fn out_dir(&self) -> PathBuf {
		self.workspace.join("forge-out")
	}

	pub(crate) fn lock_path(&self) -> PathBuf {
		self.workspace.join(".forge-lock")
	}

	pub(crate) fn exclusive_lock(&self) -> Result<FileLock, ForgeDiagnostic> {
		FileLock::exclusive(&self.lock_path(), "workspace build")
	}

	pub(crate) fn shared_lock(&self) -> Result<FileLock, ForgeDiagnostic> {
		FileLock::shared(&self.lock_path(), "workspace build")
	}

	pub(crate) fn prepare(&self) -> Result<Prepared, ForgeDiagnostic> {
		let config = forge_script::WorkspaceConfig::load(&self.workspace)?;
		self.prepare_with_context(config, &ResolutionContext::default())
	}

	pub(crate) fn prepare_for_resolution(&self) -> Result<Prepared, ForgeDiagnostic> {
		let config = forge_script::WorkspaceConfig::load(&self.workspace)?;
		let resolution = crate::metadata_fetch::resolution_context(config.resolution.clone());
		self.prepare_with_context(config, &resolution)
	}

	fn prepare_with_context(
		&self,
		config: forge_script::WorkspaceConfig,
		resolution: &ResolutionContext,
	) -> Result<Prepared, ForgeDiagnostic> {
		let packages = discover_packages(&self.workspace, &config.discovery)?;
		let platform = config.resolve_target()?;
		let cells = crate::std_cells::StdCells::load(&self.workspace, &config.std_patches)?;
		let LoadedWorkspace {
			graph,
			decls,
			mut dependencies,
			mut requirements,
			mut candidates,
			imported_lock,
			mut diagnostics,
		} = load_workspace_resolving(
			&self.workspace,
			&packages,
			&platform,
			&config.platforms,
			cells.workspace_scripts(),
			&config.cell,
			resolution,
		);

		if imported_lock.is_none() {
			let targets: Vec<toml::Table> = decls.values().map(|decl| decl.metadata.clone()).collect();
			for (cell, script) in cells.resolve_scripts() {
				let cell_config = config.cell.get(cell).cloned().unwrap_or_default();
				match forge_script::rhai_rt::run_forge_rhai_resolve(
					script,
					&self.workspace,
					&platform,
					&cell_config,
					resolution,
					&targets,
				) {
					Ok(output) => {
						dependencies.extend(output.dependencies);
						requirements.extend(output.requirements);
						candidates.extend(output.candidates);
					}
					Err(error) => diagnostics.push(error),
				}
			}
		}

		if let Err(visibility_errors) = graph.check_visibility() {
			diagnostics.extend(visibility_errors);
		}
		if let Err(cycle_error) = graph.topological_order() {
			diagnostics.push(cycles_diagnostic(cycle_error));
		}

		if !diagnostics.is_empty() {
			return Err(fatal_report(diagnostics));
		}
		Ok(Prepared {
			config,
			graph,
			decls,
			dependencies,
			requirements,
			candidates,
			imported_lock,
		})
	}

	pub fn plan_dag(&self, profile_name: &str) -> Result<(Prepared, ActionDag), ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		self.plan_dag_locked(profile_name, None)
	}

	pub(crate) fn plan_dag_locked(
		&self,
		profile_name: &str,
		mut progress: Option<&crate::progress::Progress>,
	) -> Result<(Prepared, ActionDag), ForgeDiagnostic> {
		if let Some(progress) = &mut progress {
			progress.phase("Loading workspace files...");
		}
		let prepared = self.prepare()?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let platform = prepared.config.resolve_target()?;
		if let Some(progress) = &mut progress {
			progress.phase("Resolving toolchains...");
		}
		let toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;
		if let Some(progress) = &mut progress {
			progress.phase("Loading standard cells...");
		}
		let cells = crate::std_cells::StdCells::load(&self.workspace, &prepared.config.std_patches)?;
		if let Some(progress) = &mut progress {
			progress.phase("Fetching locked dependencies...");
		}
		let fetched_sources = self.fetch_sources(&prepared)?;

		let ctx = PlanContext {
			graph: &prepared.graph,
			decls: &prepared.decls,
			profile: &profile,
			platform: &platform,
			toolchains: &toolchains,
			cells: &cells,
			cell_config: &prepared.config.cell,
			workspace: Some(&self.workspace),
			fetched_sources: &fetched_sources,
			progress,
		};
		if let Some(progress) = &mut progress {
			progress.phase("Lowering actions...");
		}
		let dag = build_action_dag(&ctx)?;
		Ok((prepared, dag))
	}

	pub fn build(&self, profile_name: &str, selection: Option<&str>) -> Result<BuildOutcome, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		self.execute_locked(profile_name, false, selection, false)
	}

	pub fn test(&self, profile_name: &str, selection: Option<&str>) -> Result<BuildOutcome, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		self.execute_locked(profile_name, true, selection, false)
	}

	pub fn replay(
		&self,
		profile_name: &str,
		selection: Option<&str>,
		proof_path: &Path,
	) -> Result<(usize, Vec<crate::proof::ReplayDivergence>), ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		let recorded = crate::proof::Proof::load(proof_path)?;
		recorded.check_seal()?;
		self.execute_locked(profile_name, true, selection, true)?;
		let replayed = crate::proof::Proof::load(&self.out_dir().join("forge.proof"))?;
		Ok((recorded.entries.len(), crate::proof::compare(&recorded, &replayed)))
	}

	pub fn compile_commands(&self, profile_name: &str) -> Result<String, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let (_prepared, dag) = self.plan_dag_locked(profile_name, None)?;

		let workspace_abs =
			std::fs::canonicalize(&self.workspace).map_err(|e| ForgeDiagnostic::error(8, format!("workspace: {e}")))?;

		let entries: Vec<CompileCommandEntry> = dag
			.specs
			.iter()
			.filter(|s| s.name.starts_with("compile "))
			.filter_map(|s| compile_entry(s, &workspace_abs))
			.collect();

		serde_json::to_string_pretty(&entries).map_err(|e| ForgeDiagnostic::error(8, format!("json: {e}")))
	}

	pub fn coverage(&self, output: Option<&str>, selection: Option<&str>) -> Result<(), ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		eprintln!("coverage: building with coverage flags...");
		let build_outcome = self.execute_locked("coverage", true, selection, false)?;
		eprintln!(
			"coverage: tests ok ({} executed, {} cached)",
			build_outcome.executed, build_outcome.test_cache_hits
		);

		let out_dir = self.out_dir();
		let toolchains =
			ToolchainStore::load(&self.workspace, forge_script::WorkspaceConfig::load(&self.workspace)?)?.resolve_all()?;

		eprintln!("coverage: collecting profiles...");
		let report = crate::coverage::collect(&self.workspace, &out_dir, &toolchains, None, "coverage")?;

		match output {
			Some(path) if path.ends_with(".info") || path == "lcov" => match &report.lcov_path {
				Some(lcov) if path != "lcov" => {
					let target = PathBuf::from(path);
					std::fs::copy(lcov, &target)
						.map_err(|e| ForgeDiagnostic::error(8, format!("failed to copy {}: {e}", target.display())))?;
					eprintln!("coverage: wrote {}", target.display());
				}
				Some(lcov) => eprintln!("coverage: wrote {}", lcov.display()),
				None => eprintln!("coverage: {}", report.summary),
			},
			Some(path) => eprintln!(
				"coverage: {} (report format `{path}` not supported by the coverage backend, printed summary)",
				report.summary
			),
			None => println!("{}", report.summary),
		}

		Ok(())
	}

	fn execute_locked(
		&self,
		profile_name: &str,
		run_tests: bool,
		selection: Option<&str>,
		force: bool,
	) -> Result<BuildOutcome, ForgeDiagnostic> {
		let confinement = crate::confine::active();
		if !confinement.gates_paths {
			eprintln!(
				"confinement: {} — run `forge confine` for the full report",
				confinement.backend.name()
			);
		}
		let mut progress = crate::progress::Progress::new(0, profile_name);
		progress.header(env!("CARGO_PKG_VERSION"));
		progress.phase("Loading workspace...");
		progress.phase("Resolving graph and dependencies...");
		let (mut prepared, mut dag) = self.plan_dag_locked(profile_name, Some(&progress))?;
		if let Some(expression) = selection {
			dag = select_dag(&prepared, &dag, expression)?;
		}
		let mut profile = prepared.config.resolve_profile(profile_name)?;
		let mut toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;

		let cas = Cas::open();
		let store = crate::store::Store::open();
		let registry = prepared.config.registry_url.clone().map(crate::registry::Registry::open);
		let lease = store.lock_shared("lease")?;
		let db = CacheDb::open(&self.out_dir())?;
		let runner = SandboxRunner::new(&self.workspace, &self.out_dir());

		if !dynamic_components(&prepared.decls).is_empty() {
			progress.phase("Running dynamic actions...");
			let dynamic = dynamic_components(&prepared.decls);
			let toolchains_paths = ToolchainPaths::of(&toolchains);
			let discovery_workers = WorkerPool::new(&runner, &toolchains_paths);
			let discovery = ExecContext {
				run_tests: false,
				discovery: true,
				record_proofs: false,
				force: false,
				store: &store,
				registry: None,
				dynamic: &dynamic,
				workspace: self.workspace.clone(),
				specs: &dag.specs,
				cas: &cas,
				db: &db,
				runner: &runner,
				workers: &discovery_workers,
				toolchains_paths: &toolchains_paths,
				toolchains: &toolchains,
				profile_fingerprint: profile.fingerprint(),
				outcome: parking_lot::Mutex::new(BuildOutcome::default()),
				progress: &progress,
				hash_cache: hasher::HashCache::new(),
				proofs: parking_lot::Mutex::new(vec![None; dag.specs.len()]),
			};
			execute_dag(&dag, &discovery, |ctx, index| ctx.run(index))?;
			let (next_prepared, next_dag) = self.plan_dag_locked(profile_name, Some(&progress))?;
			prepared = next_prepared;
			dag = next_dag;
			if let Some(expression) = selection {
				dag = select_dag(&prepared, &dag, expression)?;
			}
			profile = prepared.config.resolve_profile(profile_name)?;
			toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;
		}

		progress.set_total(dag.specs.len());
		progress.analyzing(prepared.graph.node_count());
		progress.planned(dag.specs.len());

		db.replace_graph(&prepared.graph.node_rows(), &prepared.graph.edge_rows());

		let dynamic = dynamic_components(&prepared.decls);
		let toolchains_paths = ToolchainPaths::of(&toolchains);
		let workers = WorkerPool::new(&runner, &toolchains_paths);
		let exec = ExecContext {
			run_tests,
			discovery: false,
			record_proofs: true,
			force,
			store: &store,
			registry: registry.as_ref(),
			dynamic: &dynamic,
			workspace: self.workspace.clone(),
			specs: &dag.specs,
			cas: &cas,
			db: &db,
			runner: &runner,
			workers: &workers,
			toolchains_paths: &toolchains_paths,
			toolchains: &toolchains,
			profile_fingerprint: profile.fingerprint(),
			outcome: parking_lot::Mutex::new(BuildOutcome::default()),
			progress: &progress,
			hash_cache: hasher::HashCache::new(),
			proofs: parking_lot::Mutex::new(vec![None; dag.specs.len()]),
		};

		execute_dag(&dag, &exec, |ctx, index| ctx.run(index))?;
		{
			let outcome = exec.outcome.lock();
			progress.finished(outcome.executed, outcome.cache_hits);
		}

		drop(lease);
		if let Some(max_bytes) = prepared.config.max_cache_bytes {
			let evicted = Cas::open().gc(max_bytes)?;
			if evicted > 0 {
				eprintln!("gc: evicted {:.1} MB of cached actions", evicted as f64 / (1024.0 * 1024.0));
			}
		}

		let entries: Vec<crate::proof::ActionProof> = exec.proofs.into_inner().into_iter().flatten().collect();
		if !entries.is_empty() {
			crate::proof::Proof::seal(entries)?.write(&self.out_dir().join("forge.proof"))?;
			crate::time_travel::record_revision(&self.workspace, &self.out_dir())?;
		}

		let mut outcome = exec.outcome.into_inner();
		for id in prepared.graph.node_ids() {
			let component = prepared.graph.component(id);
			if matches!(component.kind, ComponentKind::Binary) {
				let path = self.out_dir().join("bin").join(&profile.name).join(component.label.name());
				if path.exists() {
					outcome.binaries.insert(component.label.to_string(), path);
				}
			}
		}
		Ok(outcome)
	}

	pub fn clean(&self) -> Result<(), ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		let out = self.out_dir();
		if out.exists() {
			std::fs::remove_dir_all(&out)
				.map_err(|e| ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("clean failed: {e}")))?;
		}
		Ok(())
	}
}

struct ExecContext<'a> {
	run_tests: bool,
	discovery: bool,
	record_proofs: bool,
	force: bool,
	store: &'a crate::store::Store,
	registry: Option<&'a crate::registry::Registry>,
	dynamic: &'a std::collections::BTreeSet<String>,
	workspace: PathBuf,
	specs: &'a [ActionSpec],
	cas: &'a Cas,
	db: &'a CacheDb,
	runner: &'a SandboxRunner,
	workers: &'a WorkerPool<'a>,
	toolchains_paths: &'a ToolchainPaths,
	toolchains: &'a BTreeMap<String, ResolvedToolchain>,
	profile_fingerprint: String,
	outcome: parking_lot::Mutex<BuildOutcome>,
	progress: &'a crate::progress::Progress,
	hash_cache: hasher::HashCache,
	proofs: parking_lot::Mutex<Vec<Option<crate::proof::ActionProof>>>,
}

impl ExecContext<'_> {
	fn run(&self, index: usize) -> Result<(), ForgeDiagnostic> {
		let spec = &self.specs[index];
		if self.discovery && !self.dynamic.contains(&spec.component) {
			return Ok(());
		}
		let is_test_run = spec.is_test;
		if is_test_run && !self.run_tests {
			return Ok(());
		}

		self.progress.started(&spec.name);
		let action_started = Instant::now();
		let hash_inputs = spec.inputs.iter().chain(&spec.execution_deps).cloned().collect::<Vec<_>>();
		let hash_started = Instant::now();
		let input_hashes = hasher::hash_inputs(&self.workspace, &hash_inputs, &self.hash_cache)
			.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, format!("action `{}`: {e}", spec.name)))?;
		let hash_duration = hash_started.elapsed();
		if hash_duration.as_secs() >= 10 {
			eprintln!(
				"hashing `{}` took {:.1}s ({} inputs, {} execution deps)",
				spec.name,
				hash_duration.as_secs_f64(),
				spec.inputs.len(),
				spec.execution_deps.len()
			);
		}
		let key = compose_key(spec, &input_hashes, &self.profile_fingerprint, self.toolchains);
		let manifest: Vec<(String, String)> = input_hashes
			.iter()
			.map(|(path, hash)| (path.to_string_lossy().into_owned(), hash.clone()))
			.collect();
		self.db.record_action_inputs(&key, &manifest);

		if !self.force
			&& is_test_run
			&& let Some(verdict) = self.db.prior_test_verdict(&key)
			&& verdict == "PASSED"
		{
			self.outcome.lock().test_cache_hits += 1;
			self.record_proof(index, spec, &input_hashes, &key)?;
			self.progress
				.action_finished(&spec.name, true, Some(action_started.elapsed().as_millis()));
			return Ok(());
		}

		let mut cached = !self.force && !is_test_run && self.cas.contains(&key);
		if !cached
			&& !self.force
			&& !is_test_run
			&& let Some(registry) = self.registry
		{
			cached = registry.fetch(&key, self.store, self.cas)?;
		}
		if cached {
			let out_tuples: Vec<(PathBuf, forge_core::OutputKind)> =
				spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
			self.cas.restore(&key, &out_tuples, &self.workspace)?;
			self.db.record_action(&key, &spec.component, &spec.name);
			self.db.mark_cache_hit(&key);
			self.outcome.lock().cache_hits += 1;
			self.record_proof(index, spec, &input_hashes, &key)?;
			self.progress
				.action_finished(&spec.name, true, Some(action_started.elapsed().as_millis()));
			return Ok(());
		}

		let sandbox = self.runner.prepare(&key, spec)?;
		let report = match &spec.worker {
			Some(binding) => self.workers.execute(spec, binding, &sandbox),
			None => self.runner.execute(spec, &sandbox, self.toolchains_paths),
		};

		if !report.success {
			if is_test_run {
				self.db.record_test(
					&key,
					&spec.component,
					"FAILED",
					report.duration.as_millis(),
					&report.stderr_tail,
				);
			}
			return Err(ForgeDiagnostic::error(
				codes::hermetic::HERMETIC_VIOLATION,
				format!("action `{}` failed", spec.name),
			)
			.with_help(format!(
				"command: {} {}\nworking directory: {}\nstdout:\n{}\nstderr:\n{}",
				spec.command,
				spec.args.join(" "),
				spec.workdir.as_deref().unwrap_or(Path::new(".")).display(),
				if report.stdout_tail.trim().is_empty() {
					"(empty)"
				} else {
					report.stdout_tail.trim()
				},
				if report.stderr_tail.trim().is_empty() {
					"(empty)"
				} else {
					report.stderr_tail.trim()
				},
			)));
		}

		if let Err(e) = self.runner.collect(spec, &sandbox) {
			return Err(e.with_help(format!(
				"action stderr:n{}",
				if report.stderr_tail.trim().is_empty() {
					"(empty)"
				} else {
					report.stderr_tail.trim()
				}
			)));
		}
		let out_tuples: Vec<(PathBuf, forge_core::OutputKind)> =
			spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
		self.cas.store(&key, &out_tuples, &sandbox)?;
		self.runner.discard(&key);

		self.db.record_action(&key, &spec.component, &spec.name);
		self.db.record_duration(&key, report.duration.as_millis());
		if is_test_run {
			self.db.record_test(
				&key,
				&spec.component,
				"PASSED",
				report.duration.as_millis(),
				&report.stderr_tail,
			);
		}
		{
			let mut outcome = self.outcome.lock();
			outcome.executed += 1;
		}
		self.record_proof(index, spec, &input_hashes, &key)?;
		self.progress
			.action_finished(&spec.name, false, Some(action_started.elapsed().as_millis()));
		Ok(())
	}

	fn record_proof(
		&self,
		index: usize,
		spec: &ActionSpec,
		input_hashes: &BTreeMap<PathBuf, String>,
		key: &str,
	) -> Result<(), ForgeDiagnostic> {
		if !self.record_proofs || spec.is_test {
			return Ok(());
		}
		let outputs = spec.outputs.iter().map(|output| output.path.clone()).collect::<Vec<_>>();
		let proof = crate::proof::ActionProof {
			action: spec.name.clone(),
			component: spec.component.clone(),
			key: key.to_string(),
			toolchain: spec.toolchain_id.clone(),
			inputs: input_hashes
				.iter()
				.map(|(path, hash)| (path.to_string_lossy().into_owned(), hash.clone()))
				.collect(),
			outputs: crate::proof::hash_records(&self.workspace, &outputs)?,
		};
		self.proofs.lock()[index] = Some(proof);
		Ok(())
	}
}

fn select_dag(prepared: &Prepared, dag: &ActionDag, expression: &str) -> Result<ActionDag, ForgeDiagnostic> {
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
	while let Some(index) = pending.pop() {
		if keep.insert(index) {
			pending.extend(dag.deps[index].iter().copied());
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

fn dynamic_components(decls: &forge_script::DeclMap) -> std::collections::BTreeSet<String> {
	decls
		.iter()
		.filter(|(_, decl)| decl.output_dir.is_some())
		.map(|(label, _)| label.clone())
		.collect()
}

pub(crate) fn compose_key(
	spec: &ActionSpec,
	input_hashes: &BTreeMap<PathBuf, String>,
	profile_fingerprint: &str,
	toolchains: &BTreeMap<String, ResolvedToolchain>,
) -> String {
	let toolchain_digest: Option<&str> = spec
		.toolchain_id
		.as_ref()
		.and_then(|id| id.split('@').next())
		.and_then(|name| toolchains.get(name))
		.map(|t| t.digest.as_str());

	forge_core::action::compose_cache_key(
		env!("CARGO_PKG_VERSION"),
		profile_fingerprint,
		toolchain_digest,
		spec.fingerprint(),
		input_hashes,
	)
}

#[derive(serde::Serialize)]
struct CompileCommandEntry {
	directory: PathBuf,
	command: String,
	file: String,
}

fn compile_entry(spec: &ActionSpec, workspace: &Path) -> Option<CompileCommandEntry> {
	let file = find_source_in_args(&spec.args)?;
	Some(CompileCommandEntry {
		directory: workspace.to_path_buf(),
		command: format!("{} {}", spec.command, spec.args.join(" ")),
		file,
	})
}

fn find_source_in_args(args: &[String]) -> Option<String> {
	args.windows(3).find(|w| w[0] == "-c").map(|w| w[1].clone())
}

fn cycles_diagnostic(cycles: Vec<Vec<forge_core::Label>>) -> ForgeDiagnostic {
	let rendered: Vec<String> = cycles
		.iter()
		.map(|cycle| cycle.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" -> "))
		.collect();
	ForgeDiagnostic::error(
		codes::graph::CYCLE_DETECTED,
		format!("dependency cycle detected: {}", rendered.join("; ")),
	)
}

fn fatal_report(diagnostics: Vec<ForgeDiagnostic>) -> ForgeDiagnostic {
	let code = diagnostics.first().map(|d| d.code.0).unwrap_or(codes::script::PARSE_ERROR);
	let mut parts: Vec<String> = Vec::new();
	for (index, d) in diagnostics.iter().enumerate() {
		let rendered = d.to_string();
		if index == 0
			&& let Some(rest) = rendered.strip_prefix(&format!("{}[{:03}] ", d.severity.prefix(), d.code.0))
		{
			parts.push(rest.to_string());
			continue;
		}
		parts.push(rendered);
	}
	ForgeDiagnostic::error(code, parts.join("nn"))
}
