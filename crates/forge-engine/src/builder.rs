use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::{ActionSpec, Platform, action::compose_cache_key};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::{discover_packages, load_workspace};

use crate::cas::Cas;
use crate::db::CacheDb;
use crate::hasher;
use crate::planner::{PlanContext, build_action_dag};
use crate::runner::SandboxRunner;
use crate::schedule::execute_dag;
use crate::toolchain::{ResolvedToolchain, ToolchainStore};

pub struct Engine {
	workspace: PathBuf,
}

#[derive(Debug, Default)]
pub struct BuildOutcome {
	pub actions: usize,
	pub cache_hits: usize,
	pub test_cache_hits: usize,
	pub executed: usize,
	pub binaries: BTreeMap<String, PathBuf>,
}

impl Engine {
	pub fn open(workspace: &Path) -> Self {
		Self {
			workspace: workspace.to_path_buf(),
		}
	}

	fn out_dir(&self) -> PathBuf {
		self.workspace.join("forge-out")
	}

	fn prepare(&self) -> Result<Prepared, ForgeDiagnostic> {
		let config = forge_script::WorkspaceConfig::load(&self.workspace)?;
		let packages = discover_packages(&self.workspace, &config.discovery)?;
		let (graph, decls) = load_workspace(&self.workspace, &packages)?;
		graph
			.check_visibility()
			.map_err(|errs| errs.into_iter().next().expect("non-empty"))?;
		Ok(Prepared { config, graph, decls })
	}

	pub fn build(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		self.execute(profile_name, false)
	}

	pub fn test(&self, profile_name: &str) -> Result<BuildOutcome, ForgeDiagnostic> {
		self.execute(profile_name, true)
	}

	fn execute(&self, profile_name: &str, run_tests: bool) -> Result<BuildOutcome, ForgeDiagnostic> {
		let prepared = self.prepare()?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let platform = Platform::host();
		let toolchains = ToolchainStore::new(&self.workspace, prepared.config.clone()).resolve_all()?;

		let ctx = PlanContext {
			graph: &prepared.graph,
			decls: &prepared.decls,
			profile: &profile,
			platform: &platform,
			toolchains: &toolchains,
		};
		let dag = build_action_dag(&ctx)?;

		let cas = Cas::open(&self.out_dir());
		let db = CacheDb::open(&self.out_dir())?;
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

		let mut outcome = exec.outcome.into_inner();
		for id in prepared.graph.node_ids() {
			let component = prepared.graph.component(id);
			if matches!(component.kind, forge_core::ComponentKind::Binary) {
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

struct Prepared {
	config: forge_script::WorkspaceConfig,
	graph: forge_core::BuildGraph,
	decls: forge_script::DeclMap,
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
		if std::env::var_os("FORGE_TRACE").is_some() {
			eprintln!("[trace] {} key={key}", spec.name);
		}

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
			let outputs: Vec<(std::path::PathBuf, forge_core::OutputKind)> =
				spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
			self.cas.restore(&key, &outputs, &self.workspace)?;
			self.db.record_action(&key, &spec.component, &spec.name);
			self.outcome.lock().cache_hits += 1;
			return Ok(());
		}

		let sandbox = self.runner.prepare(&key, spec)?;
		let report = self.runner.execute(
			spec,
			&sandbox,
			spec.toolchain_id
				.as_ref()
				.and_then(|id| id.split('@').next())
				.and_then(|name| self.toolchains.get(name))
				.map(|t| t.bin_dir.as_path()),
		);

		if !report.success {
			if is_test_run {
				self.db
					.record_test(&key, &spec.component, "FAILED", report.duration.as_millis());
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
		let stored: Vec<(std::path::PathBuf, forge_core::OutputKind)> =
			spec.outputs.iter().map(|o| (o.path.clone(), o.kind)).collect();
		self.cas.store(&key, &stored, &sandbox)?;
		self.runner.discard(&key);

		self.db.record_action(&key, &spec.component, &spec.name);
		if is_test_run {
			self.db
				.record_test(&key, &spec.component, "PASSED", report.duration.as_millis());
		}
		{
			let mut outcome = self.outcome.lock();
			outcome.executed += 1;
		}
		Ok(())
	}
}

fn compose_key(
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

	compose_cache_key(
		env!("CARGO_PKG_VERSION"),
		profile_fingerprint,
		toolchain_digest,
		spec.fingerprint(),
		input_hashes,
	)
}
