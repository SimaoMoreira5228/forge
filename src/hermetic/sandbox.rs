use std::path::{Path, PathBuf};
use std::process::Command;

use crate::hermetic::{ActionSpec, Error as HermeticError, HermeticPolicy};

/// Defines how a command should be isolated by the build system.
pub trait SandboxProvider {
	/// Constructs a `Command` pre-configured with the required isolation
	/// arguments (like `bwrap` mount namespaces) before argument and environment
	/// setup is applied.
	fn create_command(
		&self,
		spec: &ActionSpec,
		runfiles_dir: &Path,
		toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError>;
}

/// Linux isolation using `bwrap` (Bubblewrap) to safely manipulate namespaces.
pub struct LinuxBwrapSandbox;
/// Linux isolation using native `unshare` command.
pub struct LinuxUnshareSandbox;
/// macOS isolation using `sandbox-exec` and Seatbelt compilation profiles.
pub struct MacOsSeatbeltSandbox;
/// Windows isolation stub pointing towards Job Objects and Restricted Tokens.
pub struct WindowsSandbox;
/// Fallback for execution without strict constraints.
pub struct NullSandbox;

impl SandboxProvider for LinuxBwrapSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
		toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError> {
		// Use absolute path for bwrap to avoid PATH resolution issues after env_clear()
		let mut cmd = Command::new("/usr/bin/bwrap");
		
		cmd.args([
			"--unshare-all", 
			"--die-with-parent"
		]);
		
		// System paths (read-only)
		for p in &["/usr", "/bin", "/lib", "/lib64", "/nix", "/etc"] {
			if Path::new(p).exists() {
				cmd.args(["--ro-bind", p, p]);
			}
		}

		cmd.args(["--proc", "/proc"]);
		cmd.args(["--dev", "/dev"]);
		cmd.args(["--dir", "/tmp"]);

		// Emit --dir for each ancestor of the given path
		fn emit_parent_dirs(cmd: &mut Command, path: &Path) {
			let mut current = PathBuf::from("/");
			for component in path.parent().unwrap_or(Path::new("")).components() {
				if let std::path::Component::Normal(c) = component {
					current.push(c);
					cmd.args(["--dir", &current.to_string_lossy()]);
				}
			}
		}

		// Bind workdir (writable) — this covers all project files including .forge/toolchains
		if spec.workdir.exists() {
			emit_parent_dirs(&mut cmd, &spec.workdir);
			let p = spec.workdir.to_string_lossy();
			cmd.args(["--bind", &p, &p]);
		}

		// Only bind toolchain paths that are OUTSIDE the workdir tree
		for toolchain_path in toolchain_paths {
			if toolchain_path.exists() && !toolchain_path.starts_with(&spec.workdir) {
				let bind_target = if let Some(parent) = toolchain_path.parent() { parent.to_path_buf() } else { toolchain_path.to_path_buf() };
				if bind_target.exists() && !bind_target.starts_with(&spec.workdir) {
					emit_parent_dirs(&mut cmd, &bind_target);
					let p = bind_target.to_string_lossy().into_owned();
					cmd.args(["--ro-bind", &p, &p]);
				}
			}
		}

		cmd.arg("--");
		cmd.arg(&spec.command);
		
		Ok(cmd)
	}
}

impl SandboxProvider for LinuxUnshareSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
		_toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("/usr/bin/unshare");
		
		// Setup namespace isolation using unshare command:
		// --map-root-user: Map current user to root in the new namespace
		// --mount: New mount namespace
		// --pid: New PID namespace
		// --fork: Fork before executing to ensure PID 1 is handled
		cmd.args([
			"--map-root-user",
			"--mount",
			"--pid",
			"--fork",
		]);
		
		// Terminal arg separator
		cmd.arg("--");
		cmd.arg(&spec.command);
		
		// NOTE: unshare doesn't support bind-mounting as easily as bwrap via flags.
		// It would require running a script that performs mounts then execs.
		// For now, this provides basic process and mount isolation.
		
		Ok(cmd)
	}
}

impl SandboxProvider for MacOsSeatbeltSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		runfiles_dir: &Path,
		toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("sandbox-exec");
		cmd.arg("-p");
		
		let mut profile = String::from("(version 1)\n(allow default)\n(deny network*)\n");
		
		// Map standard system paths read-only
		profile.push_str("(allow file-read* (subpath \"/usr\") (subpath \"/bin\") (subpath \"/lib\") (subpath \"/etc\") (subpath \"/dev\"))\n");
		
		// Allow reading inputs
		for input in &spec.inputs {
			profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", input.src.display()));
		}
		
		// Allow reading/writing outputs
		for output in &spec.outputs {
			profile.push_str(&format!("(allow file-read* file-write* (subpath \"{}\"))\n", output.display()));
		}
		
		// Allow workdir, runfiles and toolchains
		profile.push_str(&format!("(allow file-read* file-write* (subpath \"{}\"))\n", spec.workdir.display()));
		profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", runfiles_dir.display()));
		for tp in toolchain_paths {
			profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", tp.display()));
		}

		cmd.arg(profile);
		cmd.arg(&spec.command);
		Ok(cmd)
	}
}

impl SandboxProvider for WindowsSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
		_toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError> {
		// Stub: Windows Job Objects apply after process start, requiring an OS-level Win32 spawn implementation hook.
		log::info!("Windows Sandbox uses a generic process for now. (Job Object execution stub)");
		Ok(Command::new(&spec.command))
	}
}

impl SandboxProvider for NullSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
		_toolchain_paths: &[PathBuf],
	) -> Result<Command, HermeticError> {
		Ok(Command::new(&spec.command))
	}
}

/// Retrieve the strategy instance for the active host and policy limit.
pub fn get_sandbox(policy: &HermeticPolicy) -> Box<dyn SandboxProvider> {
	if !policy.mode.is_strict() {
		return Box::new(NullSandbox);
	}

	if cfg!(target_os = "linux") {
		if command_exists("bwrap") {
			Box::new(LinuxBwrapSandbox)
		} else if command_exists("unshare") {
			Box::new(LinuxUnshareSandbox)
		} else {
			log::warn!("Neither 'bwrap' nor 'unshare' found for strict hermetic isolation. Falling back to NullSandbox.");
			Box::new(NullSandbox)
		}
	} else if cfg!(target_os = "macos") {
		Box::new(MacOsSeatbeltSandbox)
	} else if cfg!(target_os = "windows") {
		Box::new(WindowsSandbox)
	} else {
		Box::new(NullSandbox)
	}
}

fn command_exists(cmd: &str) -> bool {
	Command::new("which")
		.arg(cmd)
		.output()
		.map(|o| o.status.success())
		.unwrap_or(false)
}
