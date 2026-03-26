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
		runfiles_dir: &Path,
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("bwrap");
		
		// Setup basic namespace isolation container:
		// Drop privileges, setup proc, mount root as read-only.
		cmd.args([
			"--unshare-all", 
			"--die-with-parent"
		]);
		
		// Map standard Linux paths read-only
		for p in &["/usr", "/bin", "/lib", "/lib64", "/nix", "/etc"] {
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

impl SandboxProvider for LinuxUnshareSandbox {
	fn create_command(
		&self,
		spec: &ActionSpec,
		_runfiles_dir: &Path,
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("unshare");
		
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
	) -> Result<Command, HermeticError> {
		let mut cmd = Command::new("sandbox-exec");
		cmd.arg("-p");
		
		let mut profile = String::from("(version 1)\n(allow default)\n(deny network*)\n");
		
		// Map standard system paths read-only
		profile.push_str("(allow file-read* (subpath \"/usr\") (subpath \"/bin\") (subpath \"/lib\") (subpath \"/etc\") (subpath \"/dev\"))\n");
		
		// Allow reading inputs
		for input in &spec.inputs {
			profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", input.display()));
		}
		
		// Allow reading/writing outputs
		for output in &spec.outputs {
			profile.push_str(&format!("(allow file-read* file-write* (subpath \"{}\"))\n", output.display()));
		}
		
		// Allow workdir and runfiles
		profile.push_str(&format!("(allow file-read* file-write* (subpath \"{}\"))\n", spec.workdir.display()));
		profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", runfiles_dir.display()));

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
