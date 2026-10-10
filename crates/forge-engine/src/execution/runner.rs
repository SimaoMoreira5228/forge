use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use forge_core::{ActionSpec, Confinement, EnvironmentFile, OutputKind, WorkRequest, WorkerMount};
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::execution::confinement::Policy;
use crate::toolchain::ToolchainPaths;

pub struct SandboxRunner {
	workspace: PathBuf,
	sandbox_root: PathBuf,
	exec_root: PathBuf,
	isolated: bool,
}

#[derive(Debug, Clone)]
pub struct Launch {
	pub program: String,
	pub args: Vec<String>,
	pub env: BTreeMap<String, String>,
	pub workdir: PathBuf,
}

struct Execution {
	launch: Launch,
	mount: Option<WorkerMount>,
	confinement: Confinement,
}

pub struct ExecReport {
	pub success: bool,
	pub stdout_tail: String,
	pub stderr_tail: String,
	pub duration: std::time::Duration,
}

const TAIL_BYTES: usize = 4000;
const RUNNER_LANG_ENV: [(&str, &str); 2] = [("LANG", "C.UTF-8"), ("LC_ALL", "C.UTF-8")];
const ETXTBSY: i32 = 26;

impl SandboxRunner {
	pub fn new(workspace: &Path, out_dir: &Path) -> Self {
		let exec_root = out_dir.join("exec");
		let _ = std::fs::create_dir_all(&exec_root);
		let sandbox_root = out_dir.join("sandbox");
		let _ = std::fs::create_dir_all(&sandbox_root);
		Self {
			workspace: workspace.to_path_buf(),
			isolated: crate::execution::confinement::mounts_sandbox(),
			sandbox_root,
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
			if src.is_dir() {
				copy_tree(&src, &dst)?;
			} else {
				copy_in(&src, &dst)?;
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

	pub fn execute(&self, spec: &ActionSpec, sandbox: &Path, toolchains: &ToolchainPaths) -> ExecReport {
		let execution = self.execution_for(spec, sandbox, toolchains);
		let started = Instant::now();
		let output = self.spawn(&execution);
		let duration = started.elapsed();
		self.report(spec, sandbox, output, duration)
	}

	fn spawn(&self, execution: &Execution) -> std::io::Result<Output> {
		let policy = Policy::of(&execution.confinement, execution.mount.as_ref(), &execution.launch.program);
		spawn(&execution.launch, execution.mount.as_ref(), &policy)
	}

	pub fn work_request(&self, spec: &ActionSpec, sandbox: &Path, toolchains: &ToolchainPaths) -> WorkRequest {
		let execution = self.execution_for(spec, sandbox, toolchains);
		WorkRequest {
			program: execution.launch.program,
			args: execution.launch.args,
			env: execution.launch.env,
			workdir: execution.launch.workdir,
			mount: execution.mount,
			confinement: Some(execution.confinement),
			outputs: spec.output_paths().map(Path::to_path_buf).collect(),
		}
	}

	pub fn record_stdout(&self, spec: &ActionSpec, sandbox: &Path, bytes: &[u8]) {
		if let Some(path) = &spec.stdout {
			let _ = std::fs::write(sandbox.join(path), bytes);
		}
	}

	fn execution_for(&self, spec: &ActionSpec, sandbox: &Path, toolchains: &ToolchainPaths) -> Execution {
		let toolchains = toolchains.scoped(&spec.toolchain_ids);
		let (root, mount) = if self.isolated {
			(
				self.exec_root.clone(),
				Some(WorkerMount {
					source: sandbox.to_path_buf(),
					target: self.exec_root.clone(),
				}),
			)
		} else {
			(sandbox.to_path_buf(), None)
		};
		Execution {
			launch: self.launch_for(spec, sandbox, &root, &toolchains),
			confinement: Confinement::new(root, toolchains.read_only.clone()),
			mount,
		}
	}

	fn launch_for(&self, spec: &ActionSpec, sandbox: &Path, root: &Path, toolchains: &ToolchainPaths) -> Launch {
		let mut env: BTreeMap<String, String> =
			RUNNER_LANG_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
		env.insert("PATH".into(), toolchain_path(&toolchains.bin_refs()));
		for (k, v) in &spec.env {
			env.insert(k.clone(), expand(v, root, toolchains));
		}
		for file in &spec.environment_files {
			for (key, value) in read_environment_file(sandbox.join(&file.path), file) {
				env.insert(metadata_env_name(&file.key_prefix, &key), value);
			}
		}
		if !spec.env.contains_key("HOME") {
			env.insert("HOME".into(), self.workspace.to_string_lossy().into_owned());
		}
		if !spec.env.contains_key("TMPDIR") {
			env.insert("TMPDIR".into(), root.join("tmp").to_string_lossy().into_owned());
		}
		#[cfg(target_os = "windows")]
		inherit_windows_host_env(&mut env, root);
		let mut args: Vec<String> = spec.args.iter().map(|argument| expand(argument, root, toolchains)).collect();
		args.extend(read_argument_files(sandbox, &spec.argument_files, root, toolchains));
		Launch {
			program: expand(&spec.command, root, toolchains),
			args,
			env,
			workdir: spec
				.workdir
				.as_ref()
				.map_or_else(|| root.to_path_buf(), |path| root.join(path)),
		}
	}

	fn report(
		&self,
		spec: &ActionSpec,
		sandbox: &Path,
		output: std::io::Result<Output>,
		duration: std::time::Duration,
	) -> ExecReport {
		match output {
			Ok(out) => {
				self.record_stdout(spec, sandbox, &out.stdout);
				ExecReport {
					success: out.status.success(),
					stdout_tail: output_tail(&out.stdout),
					stderr_tail: output_tail(&out.stderr),
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
					crate::store::publish::publish_file(&src, &dst).map_err(|e| io_err("publish output", &dst, e))?
				}
				OutputKind::Directory => {
					crate::store::publish::publish_tree(&src, &dst).map_err(|e| io_err("publish output", &dst, e))?
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

pub fn spawn(launch: &Launch, mount: Option<&WorkerMount>, policy: &Policy) -> std::io::Result<Output> {
	let mut attempts = 0;
	loop {
		match spawn_once(launch, mount, policy) {
			Err(e) if e.raw_os_error() == Some(ETXTBSY) && attempts < 100 => {
				attempts += 1;
				std::thread::sleep(std::time::Duration::from_millis(20));
			}
			outcome => return outcome,
		}
	}
}

fn spawn_once(launch: &Launch, mount: Option<&WorkerMount>, policy: &Policy) -> std::io::Result<Output> {
	#[cfg(not(target_os = "linux"))]
	let _ = mount;
	match crate::execution::confinement::active().backend {
		crate::execution::confinement::Backend::CopySandbox => spawn_plain(launch),
		#[cfg(target_os = "linux")]
		crate::execution::confinement::Backend::Landlock => {
			crate::execution::confinement::namespace::spawn_landlocked(launch, mount, policy)
		}
		#[cfg(target_os = "linux")]
		crate::execution::confinement::Backend::Namespaces => match mount {
			Some(mount) => crate::execution::confinement::namespace::spawn_mounted(launch, mount),
			None => spawn_plain(launch),
		},
		crate::execution::confinement::Backend::Seatbelt => crate::execution::confinement::seatbelt::spawn(launch, policy),
		#[cfg(target_os = "windows")]
		crate::execution::confinement::Backend::JobObjects => crate::execution::confinement::job_object::spawn(launch, policy),
		#[cfg(not(target_os = "linux"))]
		crate::execution::confinement::Backend::Landlock | crate::execution::confinement::Backend::Namespaces => {
			unreachable!("landlock and namespaces are only ever detected on linux")
		}
		#[cfg(not(target_os = "windows"))]
		crate::execution::confinement::Backend::JobObjects => unreachable!("job objects are only ever detected on windows"),
	}
}

fn spawn_plain(launch: &Launch) -> std::io::Result<Output> {
	let mut command = base_command(launch);
	command.current_dir(&launch.workdir);
	command.output()
}

pub(crate) fn base_command(launch: &Launch) -> Command {
	let mut command = Command::new(&launch.program);
	command.args(&launch.args);
	command.stdout(Stdio::piped()).stderr(Stdio::piped());
	command.env_clear();
	command.envs(&launch.env);
	command
}

fn toolchain_path(toolchain_bins: &[&Path]) -> String {
	let mut dirs: Vec<PathBuf> = toolchain_bins.iter().map(|dir| dir.to_path_buf()).collect();
	#[cfg(target_os = "windows")]
	dirs.extend(windows_msvc_bin_dirs());
	dirs.extend(fallback_path_dirs());
	std::env::join_paths(dirs)
		.map(|joined| joined.to_string_lossy().into_owned())
		.unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn inherit_windows_host_env(env: &mut BTreeMap<String, String>, root: &Path) {
	for (key_os, value_os) in std::env::vars_os() {
		let Ok(key) = key_os.into_string() else { continue };
		let Ok(value) = value_os.into_string() else { continue };
		if key.eq_ignore_ascii_case("PATH") {
			continue;
		}
		env.entry(key).or_insert(value);
	}
	let tmp = root.join("tmp").to_string_lossy().into_owned();
	for key in ["TEMP", "TMP"] {
		if !env.keys().any(|k| k.eq_ignore_ascii_case(key)) {
			env.insert(key.into(), tmp.clone());
		}
	}
}

#[cfg(target_os = "windows")]
fn windows_msvc_bin_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	for key in ["VCToolsInstallDir", "VCINSTALLDIR"] {
		if let Some(dir) = std::env::var_os(key) {
			let dir = PathBuf::from(dir);
			for candidate in [dir.join("bin/HostX64/x64"), dir.join("bin/Hostx64/x64")] {
				if candidate.join("link.exe").is_file() && !dirs.contains(&candidate) {
					dirs.push(candidate);
				}
			}
		}
	}
	if let Some(vs) = std::env::var_os("VSINSTALLDIR") {
		for candidate in find_msvc_under(Path::new(&vs)) {
			if !dirs.contains(&candidate) {
				dirs.push(candidate);
			}
		}
	}
	for program_files in [
		std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
		std::env::var_os("ProgramFiles").map(PathBuf::from),
	]
	.into_iter()
	.flatten()
	{
		let vswhere = program_files.join("Microsoft Visual Studio/Installer/vswhere.exe");
		if !vswhere.is_file() {
			continue;
		}
		let Ok(output) = std::process::Command::new(&vswhere)
			.args(["-latest", "-property", "installationPath"])
			.output()
		else {
			continue;
		};
		if !output.status.success() {
			continue;
		}
		let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
		if path.is_empty() {
			continue;
		}
		for candidate in find_msvc_under(Path::new(&path)) {
			if !dirs.contains(&candidate) {
				dirs.push(candidate);
			}
		}
	}
	for root in windows_vs_roots() {
		for candidate in find_msvc_under(&root) {
			if !dirs.contains(&candidate) {
				dirs.push(candidate);
			}
			break;
		}
	}
	dirs
}

#[cfg(target_os = "windows")]
fn windows_vs_roots() -> Vec<PathBuf> {
	let mut roots = Vec::new();
	for base in [
		std::env::var_os("ProgramFiles").map(PathBuf::from),
		std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
	]
	.into_iter()
	.flatten()
	{
		for year in ["2022", "2019"] {
			for edition in ["Enterprise", "Professional", "Community", "BuildTools"] {
				let root = base.join("Microsoft Visual Studio").join(year).join(edition);
				if root.is_dir() {
					roots.push(root);
				}
			}
		}
	}
	roots
}

#[cfg(target_os = "windows")]
fn find_msvc_under(vs_root: &Path) -> Vec<PathBuf> {
	let mut out = Vec::new();
	let Ok(entries) = std::fs::read_dir(vs_root.join("VC/Tools/MSVC")) else {
		return out;
	};
	let mut versions: Vec<PathBuf> = entries
		.flatten()
		.map(|entry| entry.path())
		.filter(|path| path.is_dir())
		.collect();
	versions.sort();
	for version in versions.into_iter().rev() {
		for host in ["HostX64/x64", "HostX64/x86", "Hostx64/x64"] {
			let candidate = version.join(host);
			if candidate.join("link.exe").is_file() {
				out.push(candidate);
				break;
			}
		}
	}
	out
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

fn metadata_env_name(key_prefix: &str, key: &str) -> String {
	format!("{key_prefix}{}", key.to_uppercase().replace('-', "_"))
}

fn read_argument_files(
	sandbox: &Path,
	files: &[forge_core::ArgumentFile],
	exec_root: &Path,
	toolchains: &ToolchainPaths,
) -> Vec<String> {
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
				arguments.push(expand(&value, exec_root, toolchains));
			}
		}
	}
	arguments
}

const EXEC_ROOT_TOKEN: &str = "FORGE_EXEC_ROOT";

fn expand(value: &str, exec_root: &Path, toolchains: &ToolchainPaths) -> String {
	toolchains.expand(&value.replace(EXEC_ROOT_TOKEN, &exec_root.to_string_lossy()))
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

fn copy_in(src: &Path, dst: &Path) -> Result<(), ForgeDiagnostic> {
	if let Some(parent) = dst.parent() {
		std::fs::create_dir_all(parent).map_err(|e| io_err("prepare", parent, e))?;
	}
	if dst.exists() {
		std::fs::remove_file(dst).map_err(|e| io_err("replace input", dst, e))?;
	}
	std::fs::copy(src, dst).map(|_| ()).map_err(|e| io_err("copy input", src, e))
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), ForgeDiagnostic> {
	std::fs::create_dir_all(dst).map_err(|e| io_err("create tree", dst, e))?;
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

pub fn output_tail(bytes: &[u8]) -> String {
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

fn fallback_path_dirs() -> Vec<PathBuf> {
	match std::env::consts::OS {
		"macos" => ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
			.into_iter()
			.map(PathBuf::from)
			.collect(),
		"windows" => std::env::var_os("PATH")
			.map(|host| std::env::split_paths(&host).collect())
			.unwrap_or_default(),
		_ => ["/usr/sbin", "/usr/bin", "/sbin", "/bin"]
			.into_iter()
			.map(PathBuf::from)
			.collect(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn repeated_materialization_preserves_the_workspace_input() {
		let dir = std::env::temp_dir().join(format!("forge-repeated-input-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let src = dir.join("source");
		let dst = dir.join("sandbox/input");
		std::fs::write(&src, b"archive contents").unwrap();
		for _ in 0..4 {
			copy_in(&src, &dst).unwrap();
			assert_eq!(std::fs::read(&src).unwrap(), b"archive contents");
			assert_eq!(std::fs::read(&dst).unwrap(), b"archive contents");
		}
		std::fs::remove_dir_all(dir).unwrap();
	}

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
			expand(
				"FORGE_EXEC_ROOT/forge-out/build/x",
				Path::new("/exec/abc"),
				&ToolchainPaths::default()
			),
			"/exec/abc/forge-out/build/x"
		);
	}

	#[test]
	fn metadata_env_names_follow_the_links_convention() {
		assert_eq!(
			metadata_env_name("DEP_AWS_LC_0_45_0_", "include"),
			"DEP_AWS_LC_0_45_0_INCLUDE"
		);
		assert_eq!(metadata_env_name("DEP_FOO_", "rustc-link-lib"), "DEP_FOO_RUSTC_LINK_LIB");
	}

	#[test]
	fn environment_files_parse_metadata_and_skip_ignored_keys() {
		let path = std::env::temp_dir().join(format!("forge-envfile-{}.txt", std::process::id()));
		std::fs::write(
			&path,
			"cargo:include=/out/include\ncargo:rustc-link-lib=static=aws_lc\ncargo:rerun-if-changed=src\nplain line\n",
		)
		.unwrap();
		let file = EnvironmentFile {
			path: PathBuf::new(),
			line_prefix: Some("cargo:".into()),
			key_prefix: "DEP_AWS_LC_0_45_0_".into(),
			ignored_keys: vec!["rustc-link-lib".into(), "rerun-if-changed".into()],
		};
		let parsed = read_environment_file(path.clone(), &file);
		let _ = std::fs::remove_file(&path);
		assert_eq!(parsed, vec![("include".to_string(), "/out/include".to_string())]);
		assert_eq!(metadata_env_name(&file.key_prefix, &parsed[0].0), "DEP_AWS_LC_0_45_0_INCLUDE");
	}
}
