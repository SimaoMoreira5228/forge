use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use forge_core::{Confinement, WorkerMount};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
	Namespaces,
	Landlock,
	Seatbelt,
	JobObjects,
	CopySandbox,
}

impl Backend {
	pub fn name(self) -> &'static str {
		match self {
			Self::Namespaces => "linux-namespaces",
			Self::Landlock => "linux-landlock",
			Self::Seatbelt => "darwin-seatbelt",
			Self::JobObjects => "windows-job-object",
			Self::CopySandbox => "copy-sandbox (no OS confinement)",
		}
	}
}

pub const FREEBSD_HAS_NO_CONFINEMENT: &str = "freebsd has no unprivileged per-process path confinement in forge: \
	unveil(2) and pledge(2) are openbsd syscalls freebsd has never implemented — checked on freebsd 15.0 \
	and 14.5, where they are absent from libc and from the headers and linking fails with \
	`undefined symbol: unveil` — and jail(2) needs root";

pub const NETBSD_HAS_NO_CONFINEMENT: &str = "forge ships no netbsd backend: no sandbox_init(2) policy was proven to \
	be enforced on the kernels forge targets, and an unproven policy is worse than an honest report that \
	the action is not confined";

pub struct Report {
	pub backend: Backend,
	pub filesystem: String,
	pub network: String,
	pub probe: String,
	pub gates_paths: bool,
	pub gates_network: bool,
}

impl Report {
	fn new(
		backend: Backend,
		gates_paths: bool,
		gates_network: bool,
		filesystem: impl Into<String>,
		probe: impl Into<String>,
	) -> Self {
		Self {
			backend,
			filesystem: filesystem.into(),
			network: if gates_network {
				"denied: the applied policy refuses socket, inet, and route operations"
			} else {
				"not restricted: this backend does not gate sockets"
			}
			.to_string(),
			probe: probe.into(),
			gates_paths,
			gates_network,
		}
	}

	fn copy_sandbox(reason: impl Into<String>) -> Self {
		Self::new(
			Backend::CopySandbox,
			false,
			false,
			"none: the action is not confined to its sandbox at all; only declared outputs are collected, \
			 so an undeclared write never reaches the workspace",
			reason,
		)
	}

	pub fn renders(&self) -> String {
		format!(
			"backend:    {}\nos:          {} ({})\nfilesystem:  {}\nnetwork:     {}\nprobe:       {}\n",
			self.backend.name(),
			std::env::consts::OS,
			std::env::consts::ARCH,
			self.filesystem,
			self.network,
			self.probe,
		)
	}
}

pub struct Policy {
	pub writable: Vec<PathBuf>,
	pub readable: Vec<PathBuf>,
}

impl Policy {
	pub fn of(confinement: &Confinement, mount: Option<&WorkerMount>, program: &str) -> Self {
		let mut writable = vec![confinement.root.clone()];
		if let Some(mount) = mount {
			writable.push(mount.source.clone());
		}
		let mut readable = confinement.read_only.clone();
		readable.extend(program_directory(program));
		dedup(&mut writable);
		dedup(&mut readable);
		Self { writable, readable }
	}
}

fn program_directory(program: &str) -> Option<PathBuf> {
	let program = Path::new(program);
	match program.parent() {
		Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
		_ => None,
	}
}

fn dedup(paths: &mut Vec<PathBuf>) {
	paths.sort();
	paths.dedup();
}

static ACTIVE: OnceLock<Report> = OnceLock::new();

pub fn active() -> &'static Report {
	ACTIVE.get_or_init(detect)
}

pub fn mounts_sandbox() -> bool {
	matches!(active().backend, Backend::Namespaces | Backend::Landlock)
}

fn detect() -> Report {
	if std::env::var_os("FORGE_NO_NS").is_some() {
		return Report::copy_sandbox("disabled by FORGE_NO_NS");
	}
	match std::env::consts::OS {
		"linux" => detect_linux(),
		"macos" => detect_seatbelt(),
		"windows" => detect_job_object(),
		"freebsd" => Report::copy_sandbox(FREEBSD_HAS_NO_CONFINEMENT),
		"netbsd" => Report::copy_sandbox(NETBSD_HAS_NO_CONFINEMENT),
		os => Report::copy_sandbox(format!(
			"forge confines nothing on {os}: there is no backend for this platform, so the action runs \
		 with the same filesystem reach as this process"
		)),
	}
}

#[cfg(target_os = "linux")]
fn detect_linux() -> Report {
	if !crate::namespace::probe_scratch() {
		return Report::copy_sandbox("user and mount namespaces are unavailable on this host");
	}
	match crate::landlock::probe() {
		Some(abi) => Report::new(
			Backend::Landlock,
			true,
			false,
			format!(
				"landlock ABI {abi}: reads and writes are limited to the action sandbox root, the toolchain, and the system directories"
			),
			"two forked probes: one entered a user and mount namespace and bind-mounted its sandbox, one applied the ruleset, wrote inside its own root, and was denied outside it",
		),
		None => Report::new(
			Backend::Namespaces,
			false,
			false,
			"user and mount namespaces only: the action gets a private mount view of its sandbox and cannot observe another action, but the host filesystem stays readable",
			"a forked child entered the namespace and bind-mounted its sandbox; this kernel has no landlock to gate paths with",
		),
	}
}

#[cfg(not(target_os = "linux"))]
fn detect_linux() -> Report {
	Report::copy_sandbox("namespaces are a Linux primitive")
}

fn detect_seatbelt() -> Report {
	if std::env::consts::OS != "macos" {
		return Report::copy_sandbox("seatbelt is a macOS primitive");
	}
	match crate::seatbelt::probe() {
		Ok(()) => Report::new(
			Backend::Seatbelt,
			true,
			true,
			"a seatbelt profile generated per action: deny by default, allow the sandbox root, the toolchain, and the system directories",
			"/usr/bin/sandbox-exec applied a generated profile to a probe that wrote inside its root and was denied outside it",
		),
		Err(reason) => Report::copy_sandbox(format!("sandbox-exec is unusable here: {reason}")),
	}
}

#[cfg(target_os = "windows")]
fn detect_job_object() -> Report {
	match crate::job_object::probe() {
		Ok(()) => Report::new(
			Backend::JobObjects,
			false,
			false,
			"none: windows offers no path confinement for a build action, so the copy sandbox is the only filesystem boundary",
			"an unnamed job object was created with kill-on-close and die-on-unhandled-exception, and then closed; \
		 breakaway is not enabled, so nothing in the job can leave it. this proves the primitive exists, not that it confines paths",
		),
		Err(reason) => Report::copy_sandbox(format!("job objects are unusable here: {reason}")),
	}
}

#[cfg(not(target_os = "windows"))]
fn detect_job_object() -> Report {
	Report::copy_sandbox("job objects are a windows primitive")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_network_line_is_a_function_of_what_the_probe_measured() {
		let gated = Report::new(Backend::Seatbelt, true, true, "fs", "probe");
		assert!(gated.network.starts_with("denied:"), "{}", gated.network);
		let open = Report::new(Backend::Landlock, true, false, "fs", "probe");
		assert!(open.network.starts_with("not restricted:"), "{}", open.network);
	}

	#[test]
	fn a_degraded_host_says_so_instead_of_claiming_hermeticity() {
		let report = Report::copy_sandbox("namespaces are unavailable");
		assert_eq!(report.backend, Backend::CopySandbox);
		assert!(report.backend.name().contains("no OS confinement"));
		assert!(!report.gates_paths);
		assert!(!report.gates_network);
		assert!(report.filesystem.starts_with("none:"), "{}", report.filesystem);
		assert!(report.probe.contains("namespaces are unavailable"));
	}

	#[test]
	fn no_backend_claims_enforcement_without_a_probe_that_demonstrated_it() {
		let report = active();
		let rendered = report.renders();
		assert!(!report.probe.is_empty(), "{rendered}");
		if report.gates_paths {
			assert!(
				!report.filesystem.starts_with("none:"),
				"a backend that gates paths cannot also report no filesystem confinement: {rendered}"
			);
		}
		if report.backend == Backend::CopySandbox {
			assert!(!report.gates_paths, "{rendered}");
			assert!(!report.gates_network, "{rendered}");
		}
	}

	#[test]
	fn the_policy_writes_only_the_sandbox_and_reads_the_toolchain_and_the_program() {
		let confinement = Confinement::new(
			"/out/exec",
			vec![PathBuf::from("/toolchains/clang/bin"), PathBuf::from("/toolchains/clang/bin")],
		);
		let mount = WorkerMount {
			source: PathBuf::from("/out/sandbox/abc"),
			target: PathBuf::from("/out/exec"),
		};
		let policy = Policy::of(&confinement, Some(&mount), "/toolchains/clang/bin/clang");
		assert_eq!(
			policy.writable,
			vec![PathBuf::from("/out/exec"), PathBuf::from("/out/sandbox/abc")]
		);
		assert_eq!(policy.readable, vec![PathBuf::from("/toolchains/clang/bin")]);
	}

	#[test]
	fn a_program_on_the_path_has_no_extra_readable_directory() {
		let confinement = Confinement::new("/out/exec", vec![]);
		let policy = Policy::of(&confinement, None, "clang");
		assert_eq!(policy.writable, vec![PathBuf::from("/out/exec")]);
		assert!(policy.readable.is_empty(), "{:?}", policy.readable);
	}

	#[test]
	fn the_decision_is_made_once_per_process() {
		assert!(std::ptr::eq(active(), active()));
		assert_eq!(
			mounts_sandbox(),
			matches!(active().backend, Backend::Namespaces | Backend::Landlock)
		);
	}
}
