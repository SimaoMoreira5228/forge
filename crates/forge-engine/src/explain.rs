use std::collections::BTreeMap;

use forge_core::{BuildGraph, ConfigTransition};
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::builder::Engine;
use crate::hasher;
use crate::planner::{PlanContext, build_action_dag};

pub struct Explanation {
	pub action: String,
	pub component: String,
	pub configuration: ConfigTransition,
	pub state: ActionState,
}

pub enum ActionState {
	NeverBuilt,
	Fresh,
	Stale {
		changed: Vec<String>,
		added: Vec<String>,
		removed: Vec<String>,
		reason_unknown: bool,
	},
}

impl Engine {
	pub fn explain(&self, target: &str, profile_name: &str) -> Result<Vec<Explanation>, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let (prepared, _dag) = self.plan_dag_locked(profile_name, None)?;
		let profile = prepared.config.resolve_profile(profile_name)?;
		let platform = prepared.config.resolve_target()?;
		let toolchains = crate::toolchain::ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;
		let cells = crate::std_cells::StdCells::load(&self.workspace, &prepared.config.std_patches)?;
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
			progress: None,
		};
		let dag = build_action_dag(&ctx)?;

		let label = resolve_label(&prepared.graph, target)?;
		let component_label = prepared.graph.component(label).label.to_string();

		let db = crate::db::CacheDb::open(&self.out_dir())?;
		let hash_cache = hasher::HashCache::new();
		let mut explanations = Vec::new();
		for spec in dag.specs.iter().filter(|s| s.component == component_label) {
			let hash_inputs = spec.inputs.iter().chain(&spec.execution_deps).cloned().collect::<Vec<_>>();
			let input_hashes = hasher::hash_inputs(&self.workspace, &hash_inputs, &hash_cache)
				.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, e.to_string()))?;
			let current_key = super::builder::compose_key(spec, &input_hashes, &profile.fingerprint(), &toolchains);

			let state = match db.latest_action_key(spec.component.as_str(), spec.name.as_str()) {
				None => ActionState::NeverBuilt,
				Some(previous) if previous == current_key => ActionState::Fresh,
				Some(previous) => {
					let previous_manifest: BTreeMap<String, String> = db.stored_inputs(&previous).into_iter().collect();
					let current_manifest: BTreeMap<String, String> = input_hashes
						.iter()
						.map(|(p, h)| (p.to_string_lossy().into_owned(), h.clone()))
						.collect();

					let mut changed = Vec::new();
					let mut added = Vec::new();
					for (path, hash) in &current_manifest {
						match previous_manifest.get(path) {
							Some(old) if old != hash => changed.push(path.clone()),
							None => added.push(path.clone()),
							_ => {}
						}
					}
					let removed: Vec<String> = previous_manifest
						.keys()
						.filter(|p| !current_manifest.contains_key(*p))
						.cloned()
						.collect();
					let reason_unknown = changed.is_empty() && added.is_empty() && removed.is_empty();
					ActionState::Stale {
						changed,
						added,
						removed,
						reason_unknown,
					}
				}
			};
			explanations.push(Explanation {
				action: spec.name.clone(),
				component: component_label.clone(),
				configuration: spec.configuration,
				state,
			});
		}

		if explanations.is_empty() {
			return Err(ForgeDiagnostic::error(
				codes::targets::UNKNOWN_TARGET,
				format!("`{target}` produces no actions"),
			));
		}
		Ok(explanations)
	}
}

fn resolve_label(graph: &BuildGraph, target: &str) -> Result<forge_core::ComponentId, ForgeDiagnostic> {
	let label = forge_core::Label::parse(target, "").or_else(|_| forge_core::Label::parse(target, "."))?;
	graph
		.get(&label)
		.ok_or_else(|| ForgeDiagnostic::error(codes::targets::UNKNOWN_TARGET, format!("unknown target `{target}`")))
}

pub fn render(explanations: &[Explanation]) -> String {
	let mut out = String::new();
	for explanation in explanations {
		let action = match explanation.configuration {
			ConfigTransition::Target => explanation.action.clone(),
			configuration => format!("{} [{}]", explanation.action, configuration.as_str()),
		};
		match &explanation.state {
			ActionState::NeverBuilt => out.push_str(&format!("{action}: never built\n")),
			ActionState::Fresh => out.push_str(&format!("{action}: up to date\n")),
			ActionState::Stale {
				changed,
				added,
				removed,
				reason_unknown,
			} => {
				out.push_str(&format!("{action}: STALE\n"));
				for path in changed {
					out.push_str(&format!("  changed:  {path}\n"));
				}
				for path in added {
					out.push_str(&format!("  added:    {path}\n"));
				}
				for path in removed {
					out.push_str(&format!("  removed:  {path}\n"));
				}
				if *reason_unknown {
					out.push_str("  reason:   command, flags, profile, or toolchain changed\n");
				}
			}
		}
	}
	out
}
