use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use forge_core::{ActionSpec, EnvironmentFile, OutputKind};
use forge_diagnostics::{ForgeDiagnostic, codes};

pub struct SandboxRunner {
	workspace: PathBuf,
	out_prefix: Option<PathBuf>,
	sandbox_root: PathBuf,
	exec_root: PathBuf,
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
		let exec_root = out_dir.join("exec");
		let _ = std::fs::create_dir_all(&exec_root);
		Self {
			workspace: workspace.to_path_buf(),
			out_prefix: out_dir.strip_prefix(workspace).ok().map(Path::to_path_buf),
			sandbox_root: out_dir.join("sandbox"),
			exec_root,
		}
	}

	pub fn prepare(&self, cache_key: &str, spec: &ActionSpec) -> Result<PathBuf, ForgeDiagnostic> {
		let dir = self.sandbox_root.join(short_key(cache_key));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).map_err(|e| io_err("create sandbox", &dir, e))?;
		std::fs::create_dir_all(dir.join("tmp")).map_err(|e| io_err("create sandbox tmp", &dir, e))?;
		for input in spec.inputs.iter().chain(&spec.execution_deps) {
			let src = self.workspace.join(input);
			let dst = dir.join(input);
			if !src.is_file() && !src.is_dir() {
				return Err(ForgeDiagnostic::error(
					codes::inputs::MISSING_INPUT,
					format!("action `{}` declared missing input `{}`", spec.name, input.display()),
				)
				.with_help(format!(
					"workspace path checked: {}\nworking directory: {}\noutputs declared by this action: {}",
					src.display(),
					spec.workdir.as_deref().unwrap_or(Path::new(".")).display(),
					spec.outputs
						.iter()
						.map(|output| output.path.display().to_string())
						.collect::<Vec<_>>()
						.join(", "),
				)));
			}
			let hardlink = self.out_prefix.as_ref().is_some_and(|prefix| !input.starts_with(prefix));
			if src.is_dir() {
				link_tree(&src, &dst, hardlink)?;
			} else {
				link_in(&src, &dst, hardlink)?;
			}
		}
		for output in &spec.outputs {
			let path = if output.kind == forge_core::OutputKind::Directory {
				dir.join(&output.path)
			} else {
				dir.join(output.path.parent().unwrap_or_else(|| Path::new("")))
			};
			std::fs::create_dir_all(&path).map_err(|e| io_err("prepare outputs", &path, e))?;
		}
		Ok(dir)
	}

	pub fn execute(&self, spec: &ActionSpec, sandbox: &Path, toolchain_bins: &[&Path]) -> ExecReport {
		#[cfg(target_os = "linux")]
		if should_use_namespaces()
			&& let Ok(report) = self.execute_namespaced(spec, sandbox, toolchain_bins)
		{
			return report;
		}
		self.execute_plain(spec, sandbox, toolchain_bins)
	}

	fn command_for(
		&self,
		spec: &ActionSpec,
		sandbox: &Path,
		exec_root: &Path,
		toolchain_bins: &[&Path],
		argument_files: &[String],
	) -> Command {
		let mut command = Command::new(expand_token(&spec.command, exec_root));
		for argument in &spec.args {
			command.arg(expand_token(argument, exec_root));
		}
		command.stdout(Stdio::piped()).stderr(Stdio::piped());
		command.env_clear();
		for (k, v) in RUNNER_LANG_ENV {
			command.env(k, v);
		}
		if !toolchain_bins.is_empty() {
			let mut joined = toolchain_bins
				.iter()
				.map(|p| p.to_string_lossy().into_owned())
				.collect::<Vec<_>>()
				.join(":");
			joined.push(':');
			joined.push_str(&fallback_path());
			command.env("PATH", joined);
		} else {
			command.env("PATH", fallback_path());
		}
		for (k, v) in &spec.env {
			command.env(k, expand_token(v, exec_root));
		}
		for file in &spec.environment_files {
			for (key, value) in read_environment_file(sandbox.join(&file.path), file) {
				command.env(format!("{}{}", file.key_prefix, key), &value);
			}
		}
		if !spec.env.contains_key("HOME") {
			command.env("HOME", &self.workspace);
		}
		if !spec.env.contains_key("TMPDIR") {
			command.env("TMPDIR", exec_root.join("tmp"));
		}
		for argument in argument_files {
			command.arg(argument);
		}
		command
	}

	#[cfg(target_os = "linux")]
	fn execute_namespaced(
		&self,
		spec: &ActionSpec,
		sandbox: &Path,
		toolchain_bins: &[&Path],
	) -> Result<ExecReport, std::io::Error> {
		let exec_root = &self.exec_root;
		let workdir = spec
			.workdir
			.as_ref()
			.map_or_else(|| exec_root.clone(), |path| exec_root.join(path));
		let argument_files = read_argument_files(sandbox, &spec.argument_files, exec_root);
		let mut command = self.command_for(spec, sandbox, exec_root, toolchain_bins, &argument_files);
		let sandbox_c = CString::new(sandbox.as_os_str().as_bytes()).map_err(io_other)?;
		let exec_root_c = CString::new(exec_root.as_os_str().as_bytes()).map_err(io_other)?;
		let workdir_c = CString::new(workdir.as_os_str().as_bytes()).map_err(io_other)?;
		unsafe {
			command.pre_exec(move || {
				if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) != 0 {
					return Err(std::io::Error::last_os_error());
				}
				let _ = std::fs::write("/proc/self/setgroups", b"deny");
				let _ = std::fs::write("/proc/self/uid_map", format!("0 {} 1", libc::getuid()));
				let _ = std::fs::write("/proc/self/gid_map", format!("0 {} 1", libc::getgid()));
				if libc::mount(
					std::ptr::null(),
					c"/".as_ptr(),
					std::ptr::null(),
					libc::MS_REC | libc::MS_PRIVATE,
					std::ptr::null(),
				) != 0
				{
					return Err(std::io::Error::last_os_error());
				}
				if libc::mount(
					sandbox_c.as_ptr(),
					exec_root_c.as_ptr(),
					std::ptr::null(),
					libc::MS_BIND | libc::MS_REC,
					std::ptr::null(),
				) != 0
				{
					return Err(std::io::Error::last_os_error());
				}
				if libc::chdir(workdir_c.as_ptr()) != 0 {
					return Err(std::io::Error::last_os_error());
				}
				Ok(())
			});
		}

		let started = Instant::now();
		let output = command.output();
		let duration = started.elapsed();
		let output = output?;
		if let Some(path) = &spec.stdout {
			let _ = std::fs::write(sandbox.join(path), &output.stdout);
		}
		Ok(ExecReport {
			success: output.status.success(),
			stdout_tail: tail(&output.stdout),
			stderr_tail: tail(&output.stderr),
			duration,
		})
	}

	fn execute_plain(&self, spec: &ActionSpec, sandbox: &Path, toolchain_bins: &[&Path]) -> ExecReport {
		let workdir = spec
			.workdir
			.as_ref()
			.map_or_else(|| sandbox.to_path_buf(), |path| sandbox.join(path));
		let argument_files = read_argument_files(sandbox, &spec.argument_files, sandbox);
		let build = || {
			let mut command = self.command_for(spec, sandbox, sandbox, toolchain_bins, &argument_files);
			command.current_dir(&workdir);
			command
		};

		let started = Instant::now();
		let mut output = build().output();
		// NOTE: ext4 can transiently report ETXTBSY when a file was just closed by a

		let mut attempts = 0;
		while let Err(e) = &output {
			if e.raw_os_error() != Some(26) || attempts >= 100 {
				break;
			}
			attempts += 1;
			std::thread::sleep(std::time::Duration::from_millis(20));
			output = build().output();
		}
		let duration = started.elapsed();
		match output {
			Ok(out) => {
				if let Some(path) = &spec.stdout {
					let _ = std::fs::write(sandbox.join(path), &out.stdout);
				}
				ExecReport {
					success: out.status.success(),
					stdout_tail: tail(&out.stdout),
					stderr_tail: tail(&out.stderr),
					duration,
				}
			}
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
				OutputKind::File => {
					crate::publish::publish_file(&src, &dst).map_err(|e| io_err("publish output", &dst, e))?
				}
				OutputKind::Directory => {
					crate::publish::publish_tree(&src, &dst).map_err(|e| io_err("publish output", &dst, e))?
				}
			}
		}
		Ok(produced)
	}

	pub fn discard(&self, cache_key: &str) {
		let dir = self.sandbox_root.join(short_key(cache_key));
		let _ = std::fs::remove_dir_all(dir);
	}
}

#[cfg(target_os = "linux")]
fn should_use_namespaces() -> bool {
	std::env::var("FORGE_NO_NS").is_err()
}

#[cfg(not(target_os = "linux"))]
fn should_use_namespaces() -> bool {
	false
}

fn io_other(error: std::ffi::NulError) -> std::io::Error {
	std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

fn short_key(cache_key: &str) -> String {
	cache_key[..12.min(cache_key.len())].to_string()
}

fn read_environment_file(path: PathBuf, file: &EnvironmentFile) -> Vec<(String, String)> {
	std::fs::read_to_string(path)
		.ok()
		.into_iter()
		.flat_map(|text| {
			text.lines()
				.filter_map(|line| match file.line_prefix.as_deref() {
					Some(prefix) => line.strip_prefix(prefix),
					None => Some(line),
				})
				.filter_map(|line| line.split_once('='))
				.filter(|(key, _)| !file.ignored_keys.iter().any(|ignored| ignored == key))
				.map(|(key, value)| (key.to_string(), value.to_string()))
				.collect::<Vec<_>>()
		})
		.collect()
}

fn read_argument_files(sandbox: &Path, files: &[forge_core::ArgumentFile], exec_root: &Path) -> Vec<String> {
	let mut arguments = Vec::new();
	for file in files {
		let Ok(text) = std::fs::read_to_string(sandbox.join(&file.path)) else {
			continue;
		};
		for line in text.lines() {
			if let Some(value) = line.strip_prefix(&file.line_prefix)
				&& !value.is_empty()
			{
				let value = match &file.root_marker {
					Some(marker) => reanchor(value, marker, exec_root),
					None => value.to_string(),
				};
				arguments.push(file.flag.clone());
				arguments.push(expand_token(&value, exec_root));
			}
		}
	}
	arguments
}

const EXEC_ROOT_TOKEN: &str = "FORGE_EXEC_ROOT";

fn expand_token(value: &str, exec_root: &Path) -> String {
	value.replace(EXEC_ROOT_TOKEN, &exec_root.to_string_lossy())
}

fn reanchor(value: &str, marker: &str, exec_root: &Path) -> String {
	let Some(index) = value.rfind(marker) else {
		return value.to_string();
	};
	let path_start = value[..index].rfind('=').map_or(0, |position| position + 1);
	let mut result = String::with_capacity(value.len() + 64);
	result.push_str(&value[..path_start]);
	result.push_str(&exec_root.to_string_lossy());
	result.push('/');
	result.push_str(&value[index..]);
	result
}

fn link_in(src: &Path, dst: &Path, hardlink: bool) -> Result<(), ForgeDiagnostic> {
	if let Some(parent) = dst.parent() {
		std::fs::create_dir_all(parent).map_err(|e| io_err("prepare", parent, e))?;
	}
	if hardlink && std::fs::hard_link(src, dst).is_ok() {
		return Ok(());
	}
	std::fs::copy(src, dst).map(|_| ()).map_err(|e| io_err("copy input", src, e))
}

fn link_tree(src: &Path, dst: &Path, hardlink: bool) -> Result<(), ForgeDiagnostic> {
	std::fs::create_dir_all(dst).map_err(|e| io_err("create tree", dst, e))?;
	for entry in std::fs::read_dir(src).map_err(|e| io_err("read tree", src, e))?.flatten() {
		let from = entry.path();
		let to = dst.join(entry.file_name());
		if entry.file_type().map_err(|e| io_err("stat", &from, e))?.is_dir() {
			link_tree(&from, &to, hardlink)?;
		} else {
			link_in(&from, &to, hardlink)?;
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reanchors_out_dir_link_search() {
		let exec_root = Path::new("/ws/forge-out/sandbox/consumer");
		let value = "native=/ws/forge-out/sandbox/producer/forge-out/build/blake3-1.8.7";
		assert_eq!(
			reanchor(value, "forge-out/", exec_root),
			"native=/ws/forge-out/sandbox/consumer/forge-out/build/blake3-1.8.7"
		);
	}

	#[test]
	fn reanchor_keeps_bare_and_prefixed_paths() {
		let exec_root = Path::new("/exec/abc");
		assert_eq!(reanchor("/x/forge-out/f", "forge-out/", exec_root), "/exec/abc/forge-out/f");
		assert_eq!(reanchor("static=sqlite3", "forge-out/", exec_root), "static=sqlite3");
	}

	#[test]
	fn expands_exec_root_token() {
		assert_eq!(
			expand_token("FORGE_EXEC_ROOT/forge-out/build/x", Path::new("/exec/abc")),
			"/exec/abc/forge-out/build/x"
		);
	}
}
