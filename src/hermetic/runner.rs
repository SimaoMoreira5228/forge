use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::hermetic::{ActionSpec, Error as HermeticError, HermeticPolicy};

pub struct SandboxRunner {
	policy: HermeticPolicy,
	runfiles_dir: PathBuf,
	toolchain_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AccessTrace {
	pub reads: Vec<PathBuf>,
	pub writes: Vec<PathBuf>,
}

impl AccessTrace {
	pub fn new() -> Self {
		Self {
			reads: Vec::new(),
			writes: Vec::new(),
		}
	}

	pub fn check_violations(&self, spec: &ActionSpec, workdir: &Path) -> Vec<String> {
		let mut violations = Vec::new();

		let declared_inputs: Vec<PathBuf> = spec
			.inputs
			.iter()
			.map(|p| if p.is_absolute() { p.clone() } else { workdir.join(p) })
			.collect();

		let declared_outputs: Vec<PathBuf> = spec
			.outputs
			.iter()
			.map(|p| if p.is_absolute() { p.clone() } else { workdir.join(p) })
			.collect();

		for read in &self.reads {
			let is_declared = declared_inputs.iter().any(|d| read.starts_with(d));
			if !is_declared {
				violations.push(format!("Undeclared input read: {}", read.display()));
			}
		}

		for write in &self.writes {
			let is_declared = declared_outputs.iter().any(|d| write.starts_with(d));
			if !is_declared {
				violations.push(format!("Undeclared output write: {}", write.display()));
			}
		}

		violations
	}
}

impl Default for AccessTrace {
	fn default() -> Self {
		Self::new()
	}
}

impl SandboxRunner {
	pub fn new(policy: HermeticPolicy, runfiles_dir: PathBuf) -> Self {
		Self {
			policy,
			runfiles_dir,
			toolchain_paths: Vec::new(),
		}
	}

	pub fn with_toolchain_paths(mut self, paths: Vec<PathBuf>) -> Self {
		self.toolchain_paths = paths;
		self
	}

	pub fn execute(&self, spec: &ActionSpec) -> Result<ExecutionResult, HermeticError> {
		if let Err(e) = spec.validate() {
			self.policy.check_violation(&format!("Invalid action spec: {}", e))?;
		}

		if self.policy.mode.is_strict() {
			if let Err(e) = check_path_fallback(&self.policy, &self.toolchain_paths) {
				return Err(HermeticError::PolicyViolation(e));
			}
		}

		self.prepare_runfiles(spec)?;
		self.write_input_manifest(spec)?;

		let normalized_env = normalize_environment(&spec.env, &self.policy, &self.toolchain_paths)?;

		let mut access_trace = AccessTrace::new();
		let result = self.run_in_sandbox(spec, &normalized_env, &mut access_trace)?;

		if self.policy.trace_access || self.policy.mode.is_strict() {
			let violations = access_trace.check_violations(spec, &spec.workdir);
			if !violations.is_empty() {
				for v in &violations {
					if self.policy.mode.is_strict() {
						self.policy.check_violation(v)?;
					} else {
						log::warn!("Hermetic access violation: {}", v);
					}
				}
			}

			if self.policy.trace_access {
				log::debug!("Access trace for action {}:", spec.name);
				for read in &access_trace.reads {
					log::debug!("  READ: {}", read.display());
				}
				for write in &access_trace.writes {
					log::debug!("  WRITE: {}", write.display());
				}
			}
		}

		self.verify_outputs(spec)?;

		Ok(result)
	}

	fn prepare_runfiles(&self, spec: &ActionSpec) -> Result<(), HermeticError> {
		fs::create_dir_all(&self.runfiles_dir)?;

		for input in &spec.inputs {
			let filename = input.file_name().unwrap_or_else(|| input.as_os_str());
			let dest = self.runfiles_dir.join(filename);
			if let Some(parent) = dest.parent() {
				fs::create_dir_all(parent)?;
			}
			if input.exists() {
				fs::copy(input, &dest)?;
			}
		}

		Ok(())
	}

	fn write_input_manifest(&self, spec: &ActionSpec) -> Result<(), HermeticError> {
		let mut manifest_file = File::create(self.runfiles_dir.join(".forge-manifest"))?;

		writeln!(manifest_file, "# Forge Input Manifest")?;
		writeln!(manifest_file, "# Action: {}", spec.name)?;
		writeln!(manifest_file, "# Generated: {}", chrono_lite_timestamp())?;
		writeln!(manifest_file, "")?;
		writeln!(manifest_file, "[inputs]")?;

		let mut sorted_inputs: Vec<_> = spec.inputs.clone();
		sorted_inputs.sort();
		for input in sorted_inputs {
			writeln!(manifest_file, "{}", input.display())?;
		}

		writeln!(manifest_file, "")?;
		writeln!(manifest_file, "[outputs]")?;

		let mut sorted_outputs: Vec<_> = spec.outputs.clone();
		sorted_outputs.sort();
		for output in sorted_outputs {
			writeln!(manifest_file, "{}", output.display())?;
		}

		Ok(())
	}

	fn run_in_sandbox(
		&self,
		spec: &ActionSpec,
		env: &HashMap<String, String>,
		_access_trace: &mut AccessTrace,
	) -> Result<ExecutionResult, HermeticError> {
		let path = env.get("PATH").cloned().unwrap_or_default();
		log::debug!("Running command: {} with PATH: {}", spec.command, path);

		let mut cmd = Command::new(&spec.command);

		if spec.workdir.exists() {
			cmd.current_dir(&spec.workdir);
		} else {
			cmd.current_dir(&self.runfiles_dir);
		}

		cmd.args(&spec.args);

		for (key, value) in env {
			cmd.env(key, value);
		}

		let output = cmd.output()?;

		Ok(ExecutionResult {
			success: output.status.success(),
			stdout: String::from_utf8_lossy(&output.stdout).to_string(),
			stderr: String::from_utf8_lossy(&output.stderr).to_string(),
			exit_code: output.status.code(),
		})
	}

	fn verify_outputs(&self, spec: &ActionSpec) -> Result<(), HermeticError> {
		let mut missing = Vec::new();

		for output in &spec.outputs {
			let filename = output.file_name().unwrap_or_else(|| output.as_os_str());
			let path = self.runfiles_dir.join(filename);
			if !path.exists() && !output.exists() {
				missing.push(output.clone());
			}
		}

		if !missing.is_empty() {
			let msg = format!("Action {} missing declared outputs: {:?}", spec.name, missing);
			self.policy.check_violation(&msg)?;
		}

		Ok(())
	}
}

pub struct ExecutionResult {
	pub success: bool,
	pub stdout: String,
	pub stderr: String,
	pub exit_code: Option<i32>,
}

fn chrono_lite_timestamp() -> String {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default();
	format!("{}", now.as_secs())
}

fn normalize_environment(
	user_env: &HashMap<String, String>,
	policy: &HermeticPolicy,
	toolchain_paths: &[PathBuf],
) -> Result<HashMap<String, String>, String> {
	let mut normalized = HashMap::new();

	for (key, value) in user_env {
		normalized.insert(key.clone(), value.clone());
	}

	if policy.mode.is_strict() {
		let mut final_path_parts: Vec<PathBuf> = Vec::new();

		for path in toolchain_paths {
			if path.exists() {
				final_path_parts.push(path.clone());
			}
		}

		if let Ok(system_path) = std::env::var("PATH") {
			for p in std::env::split_paths(&system_path) {
				if !final_path_parts.iter().any(|existing| existing == &p) {
					final_path_parts.push(p);
				}
			}
		}

		if final_path_parts.is_empty() {
			log::warn!("Hermetic strict mode: PATH is empty after normalization");
			normalized.insert("PATH".to_string(), String::new());
		} else {
			normalized.insert(
				"PATH".to_string(),
				std::env::join_paths(&final_path_parts)
					.map(|p| p.to_string_lossy().to_string())
					.unwrap_or_default(),
			);
		}
	} else {
		if let Ok(system_path) = std::env::var("PATH") {
			if user_env.contains_key("PATH") {
				normalized.insert("PATH".to_string(), system_path);
			}
		}
	}

	normalized.insert("LANG".to_string(), "C.UTF-8".to_string());
	normalized.insert("LC_ALL".to_string(), "C.UTF-8".to_string());

	Ok(normalized)
}

fn check_path_fallback(policy: &HermeticPolicy, toolchain_paths: &[PathBuf]) -> Result<(), String> {
	let path = std::env::var_os("PATH");

	if toolchain_paths.is_empty() {
		log::debug!("Hermetic: no explicit toolchain paths discovered; using system PATH fallback");
		return Ok(());
	}

	if let Some(path) = path {
		let paths: Vec<_> = std::env::split_paths(&path).collect();

		let allowed_paths: Vec<String> = toolchain_paths.iter().map(|p| p.to_string_lossy().to_string()).collect();

		log::debug!("Allowed toolchain paths: {:?}", allowed_paths);

		let suspicious: Vec<_> = paths
			.iter()
			.filter(|p| {
				let p_str = p.to_string_lossy();
				let p_owned = p_str.to_string();
				let is_allowed = allowed_paths.iter().any(|allowed| {
					if allowed.is_empty() {
						return false;
					}
					p_str.starts_with(allowed) || allowed.starts_with(&p_owned)
				});
				!is_allowed && !p_str.starts_with("/usr") && !p_str.starts_with("/bin") && !p_str.starts_with("/lib")
			})
			.collect();

		log::debug!("Suspicious paths: {:?}", suspicious);

		if !suspicious.is_empty() && policy.mode.is_strict() {
			log::warn!("Hermetic strict mode: PATH contains non-system directories: {:?}", suspicious);
		}
	}

	Ok(())
}
