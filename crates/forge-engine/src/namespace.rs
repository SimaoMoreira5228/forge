use std::ffi::{CStr, CString};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicU32, Ordering};

use forge_core::WorkerMount;

use crate::confine::Policy;
use crate::runner::{Launch, base_command};

pub fn spawn_mounted(launch: &Launch, mount: &WorkerMount) -> std::io::Result<Output> {
	let mut command = base_command(launch);
	let call = call_for(launch, mount)?;
	unsafe {
		command.pre_exec(move || call.apply());
	}
	command.output()
}

pub fn spawn_landlocked(launch: &Launch, mount: Option<&WorkerMount>, policy: &Policy) -> std::io::Result<Output> {
	let rules = crate::landlock::rules(policy);
	let call = match mount {
		Some(mount) => Some(call_for(launch, mount)?),
		None => None,
	};
	let mut command = base_command(launch);
	unsafe {
		command.pre_exec(move || {
			if let Some(call) = &call {
				call.apply()?;
			}
			match &rules {
				Some(rules) => rules.apply(),
				None => Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
			}
		});
	}
	command.output()
}

fn call_for(launch: &Launch, mount: &WorkerMount) -> std::io::Result<SandboxCall> {
	SandboxCall::new(Some(&mount.source), &mount.target, &launch.workdir)
		.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

static PROBE_SERIAL: AtomicU32 = AtomicU32::new(0);

pub fn probe_scratch() -> bool {
	let serial = PROBE_SERIAL.fetch_add(1, Ordering::Relaxed);
	let scratch = std::env::temp_dir().join(format!("forge-namespace-probe-{}-{serial}", std::process::id()));
	let _ = std::fs::remove_dir_all(&scratch);
	if std::fs::create_dir_all(&scratch).is_err() {
		return false;
	}
	let entered = probe(&scratch, &scratch);
	let _ = std::fs::remove_dir_all(&scratch);
	entered
}

pub fn probe(root: &Path, workdir: &Path) -> bool {
	let Some(call) = SandboxCall::new(Some(root), root, workdir) else {
		return false;
	};
	unsafe {
		let pid = libc::fork();
		if pid == 0 {
			let entered = call.apply().is_ok() && libc::chdir(c"/".as_ptr()) == 0;
			libc::_exit(i32::from(!entered));
		}
		if pid < 0 {
			return false;
		}
		let mut status = 0;
		libc::waitpid(pid, &mut status, 0);
		libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
	}
}

struct SandboxCall {
	source: CString,
	target: CString,
	workdir: CString,
	uid_map: CString,
	gid_map: CString,
}

impl SandboxCall {
	fn new(mount: Option<&Path>, target: &Path, workdir: &Path) -> Option<Self> {
		let text = |bytes: Vec<u8>| CString::new(bytes).ok();
		Some(Self {
			source: text(mount?.as_os_str().as_encoded_bytes().to_vec())?,
			target: text(target.as_os_str().as_encoded_bytes().to_vec())?,
			workdir: text(workdir.as_os_str().as_encoded_bytes().to_vec())?,
			uid_map: text(format!("0 {} 1", unsafe { libc::getuid() }).into_bytes())?,
			gid_map: text(format!("0 {} 1", unsafe { libc::getgid() }).into_bytes())?,
		})
	}

	pub unsafe fn apply(&self) -> std::io::Result<()> {
		unsafe {
			if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) != 0 {
				return Err(std::io::Error::last_os_error());
			}
			let _ = write_proc(c"/proc/self/setgroups", b"deny");
			let _ = write_proc(c"/proc/self/uid_map", self.uid_map.as_bytes());
			let _ = write_proc(c"/proc/self/gid_map", self.gid_map.as_bytes());
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
				self.source.as_ptr(),
				self.target.as_ptr(),
				std::ptr::null(),
				libc::MS_BIND | libc::MS_REC,
				std::ptr::null(),
			) != 0
			{
				return Err(std::io::Error::last_os_error());
			}
			if libc::chdir(self.workdir.as_ptr()) != 0 {
				return Err(std::io::Error::last_os_error());
			}
		}
		Ok(())
	}
}

unsafe fn write_proc(path: &CStr, value: &[u8]) -> std::io::Result<()> {
	let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY) };
	if fd < 0 {
		return Err(std::io::Error::last_os_error());
	}
	let written = unsafe { libc::write(fd, value.as_ptr().cast(), value.len()) };
	unsafe { libc::close(fd) };
	if written < 0 {
		return Err(std::io::Error::last_os_error());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_host_that_refuses_namespaces_is_reported_rather_than_assumed() {
		assert_eq!(probe_scratch(), probe_scratch(), "the probe must be stable within a process");
	}

	#[test]
	fn concurrent_probes_do_not_share_a_scratch_directory() {
		let expected = probe_scratch();
		let results: Vec<bool> = std::thread::scope(|scope| {
			let probes: Vec<_> = (0..8)
				.map(|_| scope.spawn(|| (0..4).map(|_| probe_scratch()).collect::<Vec<_>>()))
				.collect();
			probes
				.into_iter()
				.flat_map(|probe| probe.join().expect("probe thread"))
				.collect()
		});
		assert_eq!(results.len(), 32);
		assert!(
			results.iter().all(|entered| *entered == expected),
			"a probe that deleted another probe's scratch reports {expected} and {results:?}"
		);
	}
}
