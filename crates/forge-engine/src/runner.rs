use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use forge_core::{ActionSpec, OutputKind};
use forge_diagnostics::{ForgeDiagnostic, codes};

pub struct SandboxRunner {
	workspace: PathBuf,
	sandbox_root: PathBuf,
}

pub struct ExecReport {
	pub success: bool,
	pub stdout_tail: String,
	pub stderr_tail: String,
	pub duration: std::time::Duration,
}

const TAIL_BYTES: usize = 4000;
const RUNNER_LANG_ENV: [(&str, &str); 2] = [("LANG", "C.UTF-8"), ("LC_ALL", "C.UTF-8")];

impl SandboxRunner {
	pub fn new(workspace: &Path, out_dir: &Path) -> Self {
		Self {
			workspace: workspace.to_path_buf(),
			sandbox_root: out_dir.join("sandbox"),
		}
	}

	pub fn prepare(&self, cache_key: &str, spec: &ActionSpec) -> Result<PathBuf, ForgeDiagnostic> {
		let dir = self.sandbox_root.join(short_key(cache_key));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).map_err(|e| io_err("create sandbox", &dir, e))?;
		for input in &spec.inputs {
			let src = self.workspace.join(input);
			let dst = dir.join(input);
			if !src.is_file() {
				return Err(ForgeDiagnostic::error(
					codes::inputs::MISSING_INPUT,
					format!("declared input `{}` does not exist", input.display()),
				));
			}
			copy_in(&src, &dst)?;
		}
		for output in &spec.outputs {
			if let Some(parent) = output.path.parent() {
				std::fs::create_dir_all(dir.join(parent)).map_err(|e| io_err("prepare outputs", &dir.join(parent), e))?;
			}
		}
		Ok(dir)
	}

	pub fn execute(&self, spec: &ActionSpec, sandbox: &Path, toolchain_bins: &[&Path]) -> ExecReport {
		let mut command = Command::new(&spec.command);
		command
			.args(&spec.args)
			.current_dir(sandbox)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());

		command.env_clear();
		for (k, v) in RUNNER_LANG_ENV {
			command.env(k, v);
		}
		if !toolchain_bins.is_empty() {
			let joined = toolchain_bins
				.iter()
				.map(|p| p.to_string_lossy().into_owned())
				.collect::<Vec<_>>()
				.join(":");
			command.env("PATH", joined);
		} else {
			command.env("PATH", fallback_path());
		}
		for (k, v) in &spec.env {
			command.env(k, v);
		}

		let started = Instant::now();
		let output = command.output();
		let duration = started.elapsed();
		match output {
			Ok(out) => ExecReport {
				success: out.status.success(),
				stdout_tail: tail(&out.stdout),
				stderr_tail: tail(&out.stderr),
				duration,
			},
			Err(e) => ExecReport {
				success: false,
				stdout_tail: String::new(),
				stderr_tail: format!("failed to launch `{}`: {e}", spec.command),
				duration,
			},
		}
	}

	pub fn collect(&self, spec: &ActionSpec, sandbox: &Path) -> Result<Vec<(PathBuf, OutputKind)>, ForgeDiagnostic> {
		let mut produced = Vec::new();
		for output in &spec.outputs {
			let produced_path = sandbox.join(&output.path);
			match output.kind {
				OutputKind::File => {
					if !produced_path.is_file() {
						return Err(ForgeDiagnostic::error(
							codes::hermetic::HERMETIC_VIOLATION,
							format!(
								"action `{}` did not produce declared output `{}`",
								spec.name,
								output.path.display()
							),
						));
					}
				}
				OutputKind::Directory => {
					if !produced_path.is_dir() {
						return Err(ForgeDiagnostic::error(
							codes::hermetic::HERMETIC_VIOLATION,
							format!(
								"action `{}` did not produce declared directory `{}`",
								spec.name,
								output.path.display()
							),
						));
					}
				}
			}
			produced.push((output.path.clone(), output.kind));
		}

		for (rel, kind) in &produced {
			let src = sandbox.join(rel);
			let dst = self.workspace.join(rel);
			match kind {
				OutputKind::File => copy_in(&src, &dst)?,
				OutputKind::Directory => copy_tree(&src, &dst)?,
			}
		}
		Ok(produced)
	}

	pub fn discard(&self, cache_key: &str) {
		let dir = self.sandbox_root.join(short_key(cache_key));
		let _ = std::fs::remove_dir_all(dir);
	}
}

fn short_key(cache_key: &str) -> String {
	cache_key[..12.min(cache_key.len())].to_string()
}

fn copy_in(src: &Path, dst: &Path) -> Result<(), ForgeDiagnostic> {
	if let Some(parent) = dst.parent() {
		std::fs::create_dir_all(parent).map_err(|e| io_err("prepare", parent, e))?;
	}
	std::fs::copy(src, dst).map(|_| ()).map_err(|e| io_err("copy input", src, e))
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), ForgeDiagnostic> {
	std::fs::create_dir_all(dst).map_err(|e| io_err("copy tree", dst, e))?;
	for entry in std::fs::read_dir(src).map_err(|e| io_err("read tree", src, e))?.flatten() {
		let from = entry.path();
		let to = dst.join(entry.file_name());
		if entry.file_type().map_err(|e| io_err("stat", &from, e))?.is_dir() {
			copy_tree(&from, &to)?;
		} else {
			copy_in(&from, &to)?;
		}
	}
	Ok(())
}

fn tail(bytes: &[u8]) -> String {
	let text = String::from_utf8_lossy(bytes);
	let len = text.len();
	let start = len.saturating_sub(TAIL_BYTES);
	let sliced = &text[start..];
	if start > 0 {
		format!("…{sliced}")
	} else {
		sliced.to_owned()
	}
}

fn io_err(stage: &str, path: &Path, e: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("{stage} `{}`: {e}", path.display()),
	)
}

fn fallback_path() -> String {
	match std::env::consts::OS {
		"macos" => "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
		"windows" => std::env::var("PATH").unwrap_or_default(),
		_ => "/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
	}
}
