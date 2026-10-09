use std::path::{Path, PathBuf};

use forge_core::ActionSpec;
use forge_diagnostics::ForgeDiagnostic;

use crate::build::Engine;
use crate::toolchain::{ToolchainPaths, ToolchainStore};

#[derive(serde::Serialize)]
struct CompileCommandEntry {
	directory: PathBuf,
	command: String,
	file: String,
}

impl Engine {
	pub fn compile_commands(&self, profile_name: &str) -> Result<String, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let (prepared, dag) = self.plan_dag_locked(profile_name, None)?;
		let toolchains = ToolchainStore::load(&self.workspace, prepared.config.clone())?.resolve_all()?;
		let toolchains = ToolchainPaths::of(&toolchains);

		let directory =
			std::fs::canonicalize(&self.workspace).map_err(|e| ForgeDiagnostic::error(8, format!("workspace: {e}")))?;

		let entries: Vec<CompileCommandEntry> = dag
			.specs
			.iter()
			.filter_map(|spec| compile_entry(spec, &directory, &toolchains))
			.collect();

		serde_json::to_string_pretty(&entries).map_err(|e| ForgeDiagnostic::error(8, format!("json: {e}")))
	}
}

fn compile_entry(spec: &ActionSpec, directory: &Path, toolchains: &ToolchainPaths) -> Option<CompileCommandEntry> {
	let file = spec.compile_command.clone()?;
	Some(CompileCommandEntry {
		directory: directory.to_path_buf(),
		command: format!(
			"{} {}",
			toolchains.expand(&spec.command),
			spec.args
				.iter()
				.map(|arg| toolchains.expand(arg))
				.collect::<Vec<_>>()
				.join(" ")
		),
		file,
	})
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use forge_core::ConfigTransition;

	use super::*;
	use crate::toolchain::ResolvedToolchain;

	fn paths() -> ToolchainPaths {
		ToolchainPaths::of(&BTreeMap::from([(
			"rust".to_string(),
			ResolvedToolchain {
				name: "rust".into(),
				root: "/store/toolchains/rust/1.98.0".into(),
				bin_dir: "/store/toolchains/rust/1.98.0/bin".into(),
				path_dirs: vec!["/store/toolchains/rust/1.98.0/bin".into()],
				digest: "0123456789ab".into(),
				coverage: None,
				worker: None,
			},
		)]))
	}

	fn spec(args: Vec<String>) -> ActionSpec {
		ActionSpec {
			name: "compile src/math.rs".into(),
			component: "//:forge".into(),
			configuration: ConfigTransition::Target,
			command: "FORGE_TOOLCHAIN/rust@0123456789ab/bin/rustc".into(),
			args,
			inputs: Vec::new(),
			execution_deps: Vec::new(),
			outputs: Vec::new(),
			workdir: None,
			is_test: false,
			stdout: None,
			compile_command: Some("src/math.rs".into()),
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::new(),
			toolchain_ids: Vec::new(),
			toolchain_id: Some("rust@0123456789ab".into()),
			worker: None,
		}
	}

	#[test]
	fn an_entry_names_the_compiler_that_will_actually_run() {
		let entry = compile_entry(
			&spec(vec![
				"-O2".into(),
				"-c".into(),
				"src/math.rs".into(),
				"-o".into(),
				"forge-out/obj/math.o".into(),
			]),
			Path::new("/ws"),
			&paths(),
		)
		.expect("a compile action is an entry");

		assert_eq!(entry.file, "src/math.rs");
		assert_eq!(entry.directory, PathBuf::from("/ws"));
		assert_eq!(
			entry.command,
			"/store/toolchains/rust/1.98.0/bin/rustc -O2 -c src/math.rs -o forge-out/obj/math.o"
		);
	}

	#[test]
	fn an_action_without_a_compiled_source_is_not_a_compile_command() {
		let mut link = spec(vec!["-o".into(), "forge-out/bin/debug/app".into()]);
		link.compile_command = None;
		assert!(compile_entry(&link, Path::new("/ws"), &paths()).is_none());
	}
}
