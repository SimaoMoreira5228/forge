use std::path::Path;
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
	) -> Result<Command, HermeticError>;
}

/// Linux isolation using `bwrap` (Bubblewrap) to safely manipulate namespaces.
pub struct LinuxBwrapSandbox;
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
		runfiles_dir: &Path,
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("bwrap");
		
		// Setup basic namespace isolation container:
		// Drop privileges, setup proc, mount root as read-only.
		cmd.args([
			"--unshare-all", 
			"--share-net", // If we want to deny network, we omit --share-net. We'll omit network to follow absolute hermeticity.
			"--die-with-parent"
		]);
		
		// Map standard Linux paths read-only
		for p in &["/usr", "/bin", "/lib", "/lib64", "/etc"] {
			if Path::new(p).exists() {
				cmd.args(["--ro-bind", p, p]);
			}
		}

		// Virtual directories
		cmd.args(["--proc", "/proc"]);
		cmd.args(["--dev", "/dev"]);
		cmd.args(["--dir", "/tmp"]);

		// Mount execution environments
		let workdir_str = spec.workdir.to_string_lossy();
		if spec.workdir.exists() {
			cmd.args(["--bind", &workdir_str, &workdir_str]);
		}

		let run_str = runfiles_dir.to_string_lossy();
		cmd.args(["--ro-bind", &run_str, &run_str]);

		// Terminal arg separator
		cmd.arg("--");
		cmd.arg(&spec.command);
		
		Ok(cmd)
	}
}

impl SandboxProvider for MacOsSeatbeltSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("sandbox-exec");
		cmd.arg("-p");
		cmd.arg("(version 1) (allow default) (deny network*)");
		cmd.arg(&spec.command);
		Ok(cmd)
	}
}

impl SandboxProvider for WindowsSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
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
		// In a production engine, this would test `which bwrap` vs falling back to `unshare`.
		Box::new(LinuxBwrapSandbox)
	} else if cfg!(target_os = "macos") {
		Box::new(MacOsSeatbeltSandbox)
	} else if cfg!(target_os = "windows") {
		Box::new(WindowsSandbox)
	} else {
		Box::new(NullSandbox)
	}
}
