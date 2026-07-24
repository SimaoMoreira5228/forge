use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use forge_core::{ActionSpec, ComponentKind, Platform};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::discover_packages;
use forge_script::register::load_workspace;

use crate::cas::Cas;
use crate::db::CacheDb;
use crate::hasher;
use crate::lock::FileLock;
use crate::planner::{ActionDag, PlanContext, build_action_dag};
use crate::runner::SandboxRunner;
use crate::schedule::execute_dag;
use crate::toolchain::{ResolvedToolchain, ToolchainStore};

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
		let packages = discover_packages(&self.workspace, &config.discovery)?;
		let platform = Platform::host();
		let cells = crate::std_cells::StdCells::load(&self.workspace, &config.std_patches)?;
		let (graph, decls, dependencies, requirements, candidates, mut diagnostics) = load_workspace(
			&self.workspace,
			&packages,
			&platform,
			&config.platforms,
			cells.workspace_scripts(),
		);

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
		let platform = Platform::host();
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

	pub fn dependency_lock(&self) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let prepared = self.prepare()?;
		self.dependency_lock_for(&prepared, false)
	}

	fn dependency_lock_for(
		&self,
		prepared: &Prepared,
		use_existing: bool,
	) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		let path = self.workspace.join("forge.lock");
		if use_existing && path.is_file() {
			let text =
				std::fs::read_to_string(&path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
			return forge_core::resolver::ForgeLock::parse(&text).map_err(|e| ForgeDiagnostic::error(101, e));
		}
		if prepared.requirements.is_empty() && prepared.candidates.is_empty() {
			return Ok(forge_core::resolver::ForgeLock::from_requests(prepared.dependencies.clone()));
		}
		let resolved = forge_core::solve(
			prepared.config.name.clone(),
			prepared.requirements.clone(),
			prepared.candidates.clone(),
		)
		.map_err(|error| ForgeDiagnostic::error(101, error.to_string()))?;
		let lock = forge_core::resolver::ForgeLock::from_resolved(&resolved);
		lock.sources().map_err(|error| ForgeDiagnostic::error(101, error))?;
		Ok(lock)
	}

	pub(crate) fn fetch_sources(
		&self,
		prepared: &Prepared,
	) -> Result<Vec<forge_script::cells::FetchedSource>, ForgeDiagnostic> {
		if prepared.dependencies.is_empty() && prepared.requirements.is_empty() && prepared.candidates.is_empty() {
			return Ok(Vec::new());
		}
		let lock = self.dependency_lock_for(prepared, true)?;
		if lock.packages.is_empty() {
			return Ok(Vec::new());
		}
		let store = crate::source_store::SourceStore::open(&self.workspace);
		let fetched = store.fetch_lock(&lock)?;
		let mut roots = BTreeMap::new();
		for (package, root) in fetched {
			let relative = root.strip_prefix(&self.workspace).map_err(|_| {
				ForgeDiagnostic::error(101, format!("dependency source escaped workspace: {}", root.display()))
			})?;
			roots.insert(package, relative.to_string_lossy().into_owned());
		}
		let mut packages = Vec::new();
		for package in lock.dependency_order().map_err(|e| ForgeDiagnostic::error(101, e))? {
			let key = format!("{}@{}", package.name, package.version);
			let root = roots
				.get(&key)
				.ok_or_else(|| ForgeDiagnostic::error(101, format!("missing fetched dependency `{key}`")))?;
			packages.push(forge_script::cells::FetchedSource {
				name: package.name.clone(),
				version: package.version.clone(),
				root: root.clone(),
				dependencies: package.dependencies.clone(),
			});
		}
		Ok(packages)
	}

	pub fn build(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		self.execute_locked(profile_name, false)
	}

	pub fn test(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		self.execute_locked(profile_name, true)
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

	pub fn coverage(&self, output: Option<&str>) -> Result<(), ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		eprintln!("coverage: building with coverage flags...");
		let build_outcome = self.execute_locked("coverage", true)?;
		eprintln!(
			"coverage: tests ok ({} executed, {} cached)",
			build_outcome.executed, build_outcome.test_cache_hits
		);

		let out_dir = self.out_dir();
		let toolchains =
			ToolchainStore::load(&self.workspace, forge_script::WorkspaceConfig::load(&self.workspace)?)?.resolve_all()?;

		eprintln!("coverage: merging raw profiles...");
		let profiles = crate::coverage::merge_profiles(&self.workspace, &out_dir, &toolchains, None)?;
		if profiles.is_empty() {
			eprintln!("coverage: no profiling data found; did tests run with coverage enabled?");
			return Ok(());
		}

		eprintln!("coverage: generating report...");
		let report = crate::coverage::generate_report(
			&self.workspace,
			&out_dir,
			&profiles,
			&toolchains,
			match output {
				Some(s) if s.ends_with(".info") || s == "lcov" => crate::coverage::CoverageFormat::Lcov,
				_ => crate::coverage::CoverageFormat::Text,
			},
			None,
		)?;

		if let Some(path) = output {
			if path.ends_with(".info") || path == "lcov" {
				let lcov_path = out_dir.join("coverage.info");
				let target = if path == "lcov" { &lcov_path } else { &PathBuf::from(path) };
				if target != &lcov_path && lcov_path.exists() {
					std::fs::copy(&lcov_path, target)
						.map_err(|e| ForgeDiagnostic::error(8, format!("failed to copy {}: {e}", target.display())))?;
					eprintln!("coverage: wrote {}", target.display());
				} else {
					eprintln!("coverage: {}", report.summary);
				}
			} else {
				eprintln!(
					"coverage: {} (report format `{}` not implemented yet, wrote text instead)",
					report.summary, path
				);
			}
		} else {
			println!("{}", report.summary);
		}

		Ok(())
	}

	fn execute_locked(&self, profile_name: &str, run_tests: bool) -> Result<BuildOutcome, ForgeDiagnostic> {
		let mut progress = crate::progress::Progress::new(0, profile_name);
		progress.header(env!("CARGO_PKG_VERSION"));
		progress.phase("Loading workspace...");
		progress.phase("Resolving graph and dependencies...");
		let (prepared, dag) = self.plan_dag_locked(profile_name, Some(&progress))?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;

		progress.set_total(dag.specs.len());
		progress.analyzing(prepared.graph.node_count());
		progress.planned(dag.specs.len());

		let cas = Cas::open(&self.out_dir());
		let db = CacheDb::open(&self.out_dir())?;
		db.replace_graph(&prepared.graph.node_rows(), &prepared.graph.edge_rows());
		let runner = SandboxRunner::new(&self.workspace, &self.out_dir());

		let exec = ExecContext {
			run_tests,
			workspace: self.workspace.clone(),
			specs: &dag.specs,
			cas: &cas,
			db: &db,
			runner: &runner,
			toolchains: &toolchains,
			profile_fingerprint: profile.fingerprint(),
			outcome: parking_lot::Mutex::new(BuildOutcome::default()),
			progress: &progress,
			hash_cache: hasher::HashCache::new(),
		};

		execute_dag(&dag, &exec, |ctx, index| ctx.run(index))?;
		{
			let outcome = exec.outcome.lock();
			progress.finished(outcome.executed, outcome.cache_hits);
		}

		if let Some(max_bytes) = prepared.config.max_cache_bytes {
			let evicted = Cas::open(&self.out_dir()).gc(max_bytes)?;
			if evicted > 0 {
				eprintln!("gc: evicted {:.1} MB of cached actions", evicted as f64 / (1024.0 * 1024.0));
			}
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
	workspace: PathBuf,
	specs: &'a [ActionSpec],
	cas: &'a Cas,
	db: &'a CacheDb,
	runner: &'a SandboxRunner,
	toolchains: &'a BTreeMap<String, ResolvedToolchain>,
	profile_fingerprint: String,
	outcome: parking_lot::Mutex<BuildOutcome>,
	progress: &'a crate::progress::Progress,
	hash_cache: hasher::HashCache,
}

impl ExecContext<'_> {
	fn run(&self, index: usize) -> Result<(), ForgeDiagnostic> {
		let spec = &self.specs[index];
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

		if is_test_run
			&& let Some(verdict) = self.db.prior_test_verdict(&key)
			&& verdict == "PASSED"
		{
			self.outcome.lock().test_cache_hits += 1;
			self.progress
				.action_finished(&spec.name, true, Some(action_started.elapsed().as_millis()));
			return Ok(());
		}

		if self.cas.contains(&key) && !is_test_run {
			let out_tuples: Vec<(PathBuf, forge_core::OutputKind)> =
				spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
			self.cas.restore(&key, &out_tuples, &self.workspace)?;
			self.db.record_action(&key, &spec.component, &spec.name);
			self.db.mark_cache_hit(&key);
			self.outcome.lock().cache_hits += 1;
			self.progress
				.action_finished(&spec.name, true, Some(action_started.elapsed().as_millis()));
			return Ok(());
		}

		let sandbox = self.runner.prepare(&key, spec)?;
		let mut bins: Vec<&Path> = self
			.toolchains
			.values()
			.flat_map(|t| t.path_dirs.iter().map(|p| p.as_path()))
			.collect();
		bins.sort();
		bins.dedup();
		let report = self.runner.execute(spec, &sandbox, &bins);

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
		self.progress
			.action_finished(&spec.name, false, Some(action_started.elapsed().as_millis()));
		Ok(())
	}
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
