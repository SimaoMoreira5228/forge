use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::{ActionSpec, ComponentKind, Platform};
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::cas::Cas;
use crate::db::CacheDb;
use crate::hasher;
use crate::planner::{ActionDag, PlanContext, build_action_dag};
use crate::runner::SandboxRunner;
use crate::schedule::execute_dag;
use crate::toolchain::{ResolvedToolchain, ToolchainStore};

use forge_script::discover_packages;
use forge_script::register::load_workspace;

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
	pub(crate) fn prepare(&self) -> Result<Prepared, ForgeDiagnostic> {
		let config = forge_script::WorkspaceConfig::load(&self.workspace)?;
		let packages = discover_packages(&self.workspace, &config.discovery)?;
		let platform = Platform::host();
		let (graph, decls, mut diagnostics) = load_workspace(&self.workspace, &packages, &platform, &config.platforms);

		if let Err(visibility_errors) = graph.check_visibility() {
			diagnostics.extend(visibility_errors);
		}
		if let Err(cycle_error) = graph.topological_order() {
			diagnostics.push(cycles_diagnostic(cycle_error));
		}

		if !diagnostics.is_empty() {
			return Err(fatal_report(diagnostics));
		}
		Ok(Prepared { config, graph, decls })
	}

	pub fn plan_dag(&self, profile_name: &str) -> Result<(Prepared, ActionDag), ForgeDiagnostic> {
		let prepared = self.prepare()?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let platform = Platform::host();
		let toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;
		let cells = crate::std_cells::StdCells::load(&self.workspace, &prepared.config.std_patches)?;

		let ctx = PlanContext {
			graph: &prepared.graph,
			decls: &prepared.decls,
			profile: &profile,
			platform: &platform,
			toolchains: &toolchains,
			cells: &cells,
		};
		let dag = build_action_dag(&ctx)?;
		Ok((prepared, dag))
	}

	pub fn build(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		self.execute(profile_name, false)
	}

	pub fn test(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		self.execute(profile_name, true)
	}

	pub fn compile_commands(&self, profile_name: &str) -> Result<String, ForgeDiagnostic> {
		let (_prepared, dag) = self.plan_dag(profile_name)?;

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

	fn execute(&self, profile_name: &str, run_tests: bool) -> Result<BuildOutcome, ForgeDiagnostic> {
		let (prepared, dag) = self.plan_dag(profile_name)?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;

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
		};

		execute_dag(&dag, &exec, |ctx, index| ctx.run(index))?;

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

	pub fn clean(&self, expunge: bool) -> Result<(), ForgeDiagnostic> {
		let out = self.out_dir();
		if out.exists() {
			std::fs::remove_dir_all(&out)
				.map_err(|e| ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("clean failed: {e}")))?;
		}
		if expunge {
			let tools = self.workspace.join(".forge");
			if tools.exists() {
				std::fs::remove_dir_all(&tools).map_err(|e| {
					ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("expunge failed: {e}"))
				})?;
			}
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
}

impl ExecContext<'_> {
	fn run(&self, index: usize) -> Result<(), ForgeDiagnostic> {
		let spec = &self.specs[index];

		let input_hashes = hasher::hash_inputs(&self.workspace, &spec.inputs)
			.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, e.to_string()))?;
		let key = compose_key(spec, &input_hashes, &self.profile_fingerprint, self.toolchains);
		let manifest: Vec<(String, String)> = input_hashes
			.iter()
			.map(|(path, hash)| (path.to_string_lossy().into_owned(), hash.clone()))
			.collect();
		self.db.record_action_inputs(&key, &manifest);

		let is_test_run = spec.name.starts_with("run ");
		if is_test_run && !self.run_tests {
			return Ok(());
		}
		if is_test_run
			&& let Some(verdict) = self.db.prior_test_verdict(&key)
			&& verdict == "PASSED"
		{
			self.outcome.lock().test_cache_hits += 1;
			return Ok(());
		}

		if self.cas.contains(&key) && !is_test_run {
			let out_tuples: Vec<(PathBuf, forge_core::OutputKind)> =
				spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
			self.cas.restore(&key, &out_tuples, &self.workspace)?;
			self.db.record_action(&key, &spec.component, &spec.name);
			self.db.mark_cache_hit(&key);
			self.outcome.lock().cache_hits += 1;
			return Ok(());
		}

		let sandbox = self.runner.prepare(&key, spec)?;
		let bins: Vec<&Path> = self.toolchains.values().map(|t| t.bin_dir.as_path()).collect();
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
				"exit code nonzero\nstderr:\n{}",
				if report.stderr_tail.trim().is_empty() {
					"(empty)"
				} else {
					report.stderr_tail.trim()
				}
			)));
		}

		self.runner.collect(spec, &sandbox)?;
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
	ForgeDiagnostic::error(code, parts.join("\n\n"))
}
