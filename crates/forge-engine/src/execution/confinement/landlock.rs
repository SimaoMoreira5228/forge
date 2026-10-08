use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;

use crate::execution::confinement::Policy;

const RULE_PATH_BENEATH: u32 = 1;
const CREATE_RULESET_VERSION: u32 = 1;

const EXECUTE: u64 = 1 << 0;
const WRITE_FILE: u64 = 1 << 1;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;
const REMOVE_DIR: u64 = 1 << 4;
const REMOVE_FILE: u64 = 1 << 5;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_DIR: u64 = 1 << 7;
const MAKE_REG: u64 = 1 << 8;
const MAKE_SOCK: u64 = 1 << 9;
const MAKE_FIFO: u64 = 1 << 10;
const MAKE_BLOCK: u64 = 1 << 11;
const MAKE_SYM: u64 = 1 << 12;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
const IOCTL_DEV: u64 = 1 << 15;

const READ: u64 = READ_FILE | READ_DIR;
const DEVICE: u64 = READ_FILE | WRITE_FILE | TRUNCATE;
const MUTATE: u64 = WRITE_FILE
	| REMOVE_DIR
	| REMOVE_FILE
	| MAKE_CHAR
	| MAKE_DIR
	| MAKE_REG
	| MAKE_SOCK
	| MAKE_FIFO
	| MAKE_BLOCK
	| MAKE_SYM
	| REFER
	| TRUNCATE;

const SYSTEM_READ: &[&str] = &[
	"/usr",
	"/bin",
	"/sbin",
	"/lib",
	"/lib64",
	"/lib32",
	"/libx32",
	"/opt",
	"/etc",
	"/dev/null",
	"/dev/zero",
	"/dev/random",
	"/dev/urandom",
	"/dev/tty",
	"/proc/self",
];

#[repr(C)]
struct RulesetAttr {
	handled_access_fs: u64,
	handled_access_net: u64,
	scoped: u64,
}

#[repr(C)]
struct PathBeneathAttr {
	allowed_access: u64,
	parent_fd: i32,
}

struct Rule {
	path: CString,
	access: u64,
}

pub struct Rules {
	ruleset: RulesetAttr,
	rules: Vec<Rule>,
}

pub fn abi() -> Option<u32> {
	static ABI: OnceLock<Option<u32>> = OnceLock::new();
	*ABI.get_or_init(|| {
		let version = unsafe {
			libc::syscall(
				libc::SYS_landlock_create_ruleset,
				std::ptr::null::<RulesetAttr>(),
				0usize,
				CREATE_RULESET_VERSION,
			)
		};
		(version > 0).then_some(version as u32)
	})
}

const DESIRED: u64 = EXECUTE | READ | MUTATE | IOCTL_DEV;

const LAST: [(u64, u32); 4] = [(MAKE_SYM, 1), (REFER, 2), (TRUNCATE, 3), (IOCTL_DEV, 5)];

fn mask(abi: u32) -> u64 {
	let last = LAST.iter().rev().find_map(|&(right, from)| (abi >= from).then_some(right));
	last.map_or(0, |last| (last << 1) - 1)
}

fn handled(abi: u32) -> u64 {
	DESIRED & mask(abi)
}

pub fn rules(policy: &Policy) -> Option<Rules> {
	let abi = abi()?;
	let handled = handled(abi);
	let mut rules = Vec::new();
	for path in SYSTEM_READ {
		push(&mut rules, path, READ | EXECUTE, handled);
	}
	if let Ok(resolved) = std::fs::read_link("/proc/self")
		&& resolved.is_absolute()
	{
		push(&mut rules, &resolved.to_string_lossy(), READ, handled);
	}
	for path in &policy.readable {
		push(&mut rules, &path.to_string_lossy(), READ | EXECUTE, handled);
	}
	for path in &policy.writable {
		push(&mut rules, &path.to_string_lossy(), READ | MUTATE | EXECUTE, handled);
	}
	Some(Rules {
		ruleset: RulesetAttr {
			handled_access_fs: handled,
			handled_access_net: 0,
			scoped: 0,
		},
		rules,
	})
}

fn push(rules: &mut Vec<Rule>, path: &str, access: u64, handled: u64) {
	let path = Path::new(path);
	if !path.exists() {
		return;
	}
	let access = if path.is_dir() { access } else { DEVICE };
	if let Ok(bytes) = CString::new(path.to_string_lossy().as_bytes()) {
		rules.push(Rule {
			path: bytes,
			access: access & handled,
		});
	}
}

impl Rules {
	pub unsafe fn apply(&self) -> std::io::Result<()> {
		if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
			return Err(std::io::Error::last_os_error());
		}
		let set = unsafe {
			libc::syscall(
				libc::SYS_landlock_create_ruleset,
				&self.ruleset,
				std::mem::size_of::<RulesetAttr>(),
				0u32,
			)
		};
		if set < 0 {
			return Err(std::io::Error::last_os_error());
		}
		let set = set as i32;
		let result = self
			.rules
			.iter()
			.try_for_each(|rule| unsafe { add_rule(set, rule) })
			.and_then(|()| {
				let restricted = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, set, 0u32) };
				if restricted < 0 {
					Err(std::io::Error::last_os_error())
				} else {
					Ok(())
				}
			});
		unsafe { libc::close(set) };
		result
	}
}

unsafe fn add_rule(set: i32, rule: &Rule) -> std::io::Result<()> {
	let fd = unsafe { libc::open(rule.path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
	if fd < 0 {
		return Err(std::io::Error::last_os_error());
	}
	let beneath = PathBeneathAttr {
		allowed_access: rule.access,
		parent_fd: fd,
	};
	let added = unsafe { libc::syscall(libc::SYS_landlock_add_rule, set, RULE_PATH_BENEATH, &beneath, 0u32) };
	unsafe { libc::close(fd) };
	if added < 0 {
		return Err(std::io::Error::last_os_error());
	}
	Ok(())
}

fn escapes_denied(secret: &Path) -> bool {
	std::fs::read(secret).is_err() && std::fs::write(secret, b"escaped").is_err()
}

pub fn probe() -> Option<u32> {
	let abi = abi()?;
	let scratch = std::env::temp_dir().join(format!("forge-landlock-probe-{}", std::process::id()));
	let secret = scratch.with_extension("secret");
	let _ = std::fs::remove_dir_all(&scratch);
	let _ = std::fs::remove_file(&secret);
	if std::fs::create_dir_all(&scratch).is_err() || std::fs::write(&secret, b"secret").is_err() {
		return None;
	}
	let policy = Policy {
		writable: vec![scratch.clone()],
		readable: Vec::new(),
	};
	let applied = fork_probe(|| {
		let Some(rules) = rules(&policy) else {
			return false;
		};
		unsafe { rules.apply() }.is_ok() && std::fs::write(scratch.join("inside"), b"ok").is_ok() && escapes_denied(&secret)
	});
	let _ = std::fs::remove_dir_all(&scratch);
	let _ = std::fs::remove_file(&secret);
	applied.then_some(abi)
}

fn fork_probe(body: impl FnOnce() -> bool) -> bool {
	unsafe {
		let pid = libc::fork();
		if pid == 0 {
			let passed = body();
			libc::_exit(i32::from(!passed));
		}
		if pid < 0 {
			return false;
		}
		let mut status = 0;
		libc::waitpid(pid, &mut status, 0);
		libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	fn scratch(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("forge-landlock-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	fn rule_for<'a>(rules: &'a [Rule], path: &str) -> Option<&'a Rule> {
		rules.iter().find(|rule| rule.path.as_bytes() == path.as_bytes())
	}

	#[test]
	fn a_kernel_without_landlock_reports_none_rather_than_guessing() {
		let empty = Policy {
			writable: Vec::new(),
			readable: Vec::new(),
		};
		assert_eq!(abi().is_some(), rules(&empty).is_some());
	}

	#[test]
	fn the_ruleset_grants_write_only_under_the_sandbox_and_reads_the_toolchain() {
		let root = scratch("policy");
		let toolchain = scratch("toolchain");
		let Some(rules) = rules(&Policy {
			writable: vec![root.clone()],
			readable: vec![toolchain.clone()],
		}) else {
			return;
		};
		let writable = rule_for(&rules.rules, &root.to_string_lossy()).expect("the sandbox root is a rule");
		assert!(writable.access & WRITE_FILE != 0, "the action must write its outputs");
		assert!(writable.access & READ_FILE != 0, "the action must read its inputs");
		assert!(writable.access & EXECUTE != 0, "the action may run what it built");
		let read_only = rule_for(&rules.rules, &toolchain.to_string_lossy()).expect("the toolchain is a rule");
		assert!(read_only.access & WRITE_FILE == 0, "the toolchain must be read-only");
		assert!(read_only.access & EXECUTE != 0, "the toolchain must be executable");
		let _ = std::fs::remove_dir_all(&root);
		let _ = std::fs::remove_dir_all(&toolchain);
	}

	#[test]
	fn a_rule_never_asks_for_a_right_the_ruleset_does_not_handle() {
		let root = scratch("handled");
		let Some(rules) = rules(&Policy {
			writable: vec![root.clone()],
			readable: Vec::new(),
		}) else {
			return;
		};
		assert_eq!(
			rules.ruleset.handled_access_net, 0,
			"landlock only gates what this backend gates"
		);
		assert_eq!(rules.ruleset.scoped, 0, "no ipc scope is gated");
		assert!(
			rules
				.rules
				.iter()
				.all(|rule| rule.access & !rules.ruleset.handled_access_fs == 0),
			"a rule must not request an unhandled right"
		);
		let _ = std::fs::remove_dir_all(&root);
	}

	fn create_ruleset(handled_access_fs: u64) -> std::io::Result<()> {
		let attr = RulesetAttr {
			handled_access_fs,
			handled_access_net: 0,
			scoped: 0,
		};
		let set = unsafe {
			libc::syscall(
				libc::SYS_landlock_create_ruleset,
				&attr,
				std::mem::size_of::<RulesetAttr>(),
				0u32,
			)
		};
		if set < 0 {
			return Err(std::io::Error::last_os_error());
		}
		unsafe { libc::close(set as i32) };
		Ok(())
	}

	#[test]
	fn every_abi_this_backend_can_meet_produces_a_ruleset_the_running_kernel_accepts() {
		let running = abi().expect("this test needs a kernel with landlock");
		let undefined = 1u64 << 63;
		assert_eq!(
			create_ruleset(undefined).unwrap_err().raw_os_error(),
			Some(libc::EINVAL),
			"this kernel accepts a right it does not define, so nothing below proves the mask is in range"
		);
		for version in 1..=running {
			let handled = handled(version);
			assert_eq!(
				handled & !mask(version),
				0,
				"abi {version} asks for a right outside its own ABI"
			);
			assert!(handled & WRITE_FILE != 0 && handled & READ_FILE != 0, "abi {version}");
			create_ruleset(handled).unwrap_or_else(|e| {
				panic!("abi {version}: the running kernel refused forge's handled mask {handled:#x}: {e}")
			});
		}
	}

	#[test]
	fn an_action_may_reparent_inside_its_sandbox_and_never_out_of_it() {
		if abi().is_none() {
			eprintln!("NOTE: this kernel has no landlock, so there is no granted set to reparent under");
			return;
		}
		let root = scratch("reparent");
		let output = root.join("lib/debug");
		let staging = root.join("scratch");
		let outside = scratch("reparent-outside");
		let secret = outside.join("forbidden");
		std::fs::create_dir_all(&output).unwrap();
		std::fs::create_dir_all(&staging).unwrap();
		std::fs::write(&secret, b"secret").unwrap();
		let policy = Policy {
			writable: vec![root.clone()],
			readable: Vec::new(),
		};

		let staged = staging.join("archive.tmp");
		let reparented_inside = fork_probe(|| {
			let Some(rules) = rules(&policy) else {
				return false;
			};
			unsafe { rules.apply() }.is_ok()
				&& std::fs::write(&staged, b"archive").is_ok()
				&& std::fs::rename(&staged, output.join("libarchive.a")).is_ok()
		});
		assert!(
			reparented_inside,
			"an action could not move a file it wrote from one directory of its own sandbox to another; \
			 a backend that breaks that write is worse than no backend"
		);

		let escape = root.join("escape-me");
		std::fs::write(&escape, b"x").unwrap();
		let reparented_outside = fork_probe(|| {
			let Some(rules) = rules(&policy) else {
				return false;
			};
			unsafe { rules.apply() }.is_ok()
				&& std::fs::write(&escape, b"x").is_ok()
				&& std::fs::rename(&escape, outside.join("stolen")).is_ok()
		});
		assert!(
			!reparented_outside && !outside.join("stolen").exists(),
			"the granted set reached a path outside the sandbox root"
		);

		let read_outside = fork_probe(|| {
			let Some(rules) = rules(&policy) else {
				return false;
			};
			unsafe { rules.apply() }.is_ok() && std::fs::read(&secret).is_err()
		});
		assert!(
			read_outside,
			"the granted set let an action read a path outside the sandbox root"
		);

		let _ = std::fs::remove_dir_all(&root);
		let _ = std::fs::remove_dir_all(&outside);
	}
}
