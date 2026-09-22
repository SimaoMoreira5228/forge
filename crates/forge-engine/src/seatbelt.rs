use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::confine::Policy;
use crate::runner::Launch;

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

const SYSTEM_READ: &[&str] = &[
	"/usr",
	"/System",
	"/System/Volumes/Preboot/Cryptexes",
	"/Library",
	"/bin",
	"/sbin",
	"/opt",
	"/etc",
	"/private/var/db/dyld",
	"/private/var/db/CVMS",
	"/dev",
];

const SYSTEM_EXEC: &[&str] = &["/usr", "/System", "/Library", "/bin", "/sbin", "/opt"];

const SYSTEM_PREAMBLE: &str = "(version 1)\n\
	(deny default)\n\
	(allow file-read-metadata)\n\
	(allow process-fork)\n\
	(allow sysctl-read)\n\
	(allow mach-lookup)\n\
	(allow signal (target self))\n\
	(allow file-map-executable)\n";

const DEVICE_WRITES: &str = "(allow file-write* (literal \"/dev/null\") (literal \"/dev/zero\") \
	(literal \"/dev/random\") (literal \"/dev/urandom\") (literal \"/dev/stdout\") \
	(literal \"/dev/stderr\") (literal \"/dev/tty\") (regex #\"^/dev/ttys\"))\n";

pub fn profile(policy: &Policy) -> String {
	let mut out = String::from(SYSTEM_PREAMBLE);
	for path in SYSTEM_READ {
		push_path(&mut out, "file-read* file-map-executable", Path::new(path));
	}
	for path in SYSTEM_EXEC {
		push_path(&mut out, "process-exec*", Path::new(path));
	}
	for path in &policy.readable {
		push_path(&mut out, "file-read* file-map-executable", path);
		push_path(&mut out, "process-exec*", path);
	}
	for (index, path) in policy.writable.iter().enumerate() {
		let operation = if index == 0 { "file-read* file-write*" } else { "file-read*" };
		push_path(&mut out, operation, path);
	}
	out.push_str(DEVICE_WRITES);
	out
}

fn push_path(out: &mut String, operation: &str, path: &Path) {
	if path.as_os_str().is_empty() {
		return;
	}
	out.push_str(&format!("(allow {operation} (subpath \"{}\"))\n", quote(path)));
}

fn quote(path: &Path) -> String {
	let mut quoted = String::new();
	for character in path.to_string_lossy().chars() {
		match character {
			'"' => quoted.push_str("\\\""),
			'\\' => quoted.push_str("\\\\"),
			'\n' | '\r' => quoted.push(' '),
			other => quoted.push(other),
		}
	}
	quoted
}

pub fn spawn(launch: &Launch, policy: &Policy) -> std::io::Result<Output> {
	run(launch, policy, Path::new(SANDBOX_EXEC))
}

fn run(launch: &Launch, policy: &Policy, sandbox_exec: &Path) -> std::io::Result<Output> {
	let mut command = Command::new(sandbox_exec);
	command.arg("-p").arg(profile(policy));
	command.arg(&launch.program);
	command.args(&launch.args);
	command.current_dir(&launch.workdir);
	command.stdout(Stdio::piped()).stderr(Stdio::piped());
	command.env_clear();
	command.envs(&launch.env);
	command.output()
}

static PROBE_SERIAL: AtomicU32 = AtomicU32::new(0);

pub fn probe() -> Result<(), String> {
	let serial = PROBE_SERIAL.fetch_add(1, Ordering::Relaxed);
	let scratch = std::env::temp_dir().join(format!("forge-seatbelt-probe-{}-{serial}", std::process::id()));
	let secret = scratch.with_extension("secret");
	let _ = std::fs::remove_dir_all(&scratch);
	let _ = std::fs::remove_file(&secret);
	std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
	std::fs::write(&secret, b"secret").map_err(|e| e.to_string())?;
	let policy = Policy {
		writable: vec![scratch.clone()],
		readable: vec![scratch.clone()],
	};
	let script = format!(
		"printf ok > {inside} || exit 10\n\
		 cat {secret} > /dev/null 2>&1 && exit 11\n\
		 printf escaped > {secret} 2>/dev/null && exit 12\n\
		 exit 0\n",
		inside = scratch.join("inside").display(),
		secret = secret.display(),
	);
	let outcome = Command::new(SANDBOX_EXEC)
		.arg("-p")
		.arg(profile(&policy))
		.arg("/bin/sh")
		.arg("-c")
		.arg(&script)
		.output();
	let wrote_inside = scratch.join("inside").exists();
	let _ = std::fs::remove_dir_all(&scratch);
	let _ = std::fs::remove_file(&secret);
	let output = outcome.map_err(|e| format!("{e}"))?;
	match output.status.code() {
		Some(0) if wrote_inside => Ok(()),
		Some(11) => Err("the profile let the probe read a path outside its root".into()),
		Some(12) => Err("the profile let the probe write a path outside its root".into()),
		Some(code) => Err(format!(
			"sandbox-exec probe exited with {code}: {}",
			String::from_utf8_lossy(&output.stderr).trim()
		)),
		None => Err("the sandbox-exec probe was killed by a signal".into()),
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	fn policy() -> Policy {
		Policy {
			writable: vec![PathBuf::from("/ws/forge-out/exec")],
			readable: vec![
				PathBuf::from("/ws/forge-out/sandbox/abc"),
				PathBuf::from("/cache/toolchains/clang/bin"),
			],
		}
	}

	#[test]
	fn the_profile_denies_by_default_and_grants_the_network_nothing() {
		let rendered = profile(&policy());
		assert!(rendered.starts_with("(version 1)\n(deny default)\n"), "{rendered}");
		assert!(!rendered.contains("network"), "{rendered}");
	}

	#[test]
	fn the_profile_is_derived_from_the_sandbox_root_the_toolchain_and_the_system() {
		let rendered = profile(&policy());
		assert!(
			rendered.contains(r#"(allow file-read* file-write* (subpath "/ws/forge-out/exec"))"#),
			"{rendered}"
		);
		assert!(
			rendered.contains(r#"(allow file-read* file-map-executable (subpath "/cache/toolchains/clang/bin"))"#),
			"{rendered}"
		);
		assert!(
			rendered.contains(r#"(allow process-exec* (subpath "/cache/toolchains/clang/bin"))"#),
			"the toolchain the action runs has to be executable: {rendered}"
		);
		assert!(
			rendered.contains(r#"(allow file-read* file-map-executable (subpath "/usr"))"#),
			"{rendered}"
		);
		assert!(rendered.contains(r#"(allow process-exec* (subpath "/usr"))"#), "{rendered}");
	}

	#[test]
	fn only_the_root_the_action_runs_in_is_writable() {
		let mut two_roots = policy();
		two_roots.writable.push(PathBuf::from("/ws/forge-out/sandbox/abc"));
		let rendered = profile(&two_roots);
		assert!(
			rendered.contains(r#"(allow file-read* file-write* (subpath "/ws/forge-out/exec"))"#),
			"{rendered}"
		);
		assert!(
			rendered.contains(r#"(allow file-read* (subpath "/ws/forge-out/sandbox/abc"))"#),
			"{rendered}"
		);
		assert_eq!(
			rendered.matches("file-read* file-write*").count(),
			1,
			"only the root the action runs in is writable: {rendered}"
		);
	}

	#[test]
	fn a_path_cannot_break_out_of_the_profile_language() {
		let rendered = profile(&Policy {
			writable: vec![PathBuf::from("/ws/\" (allow default) \"/")],
			readable: Vec::new(),
		});
		assert!(rendered.contains(r#"/ws/\" (allow default) \"/"#), "{rendered}");
		assert_eq!(rendered.matches("(deny default)").count(), 1, "{rendered}");
	}

	#[cfg(unix)]
	#[test]
	fn the_action_runs_under_sandbox_exec_and_keeps_its_own_identity() {
		use std::os::unix::fs::PermissionsExt;

		let dir = std::env::temp_dir().join(format!("forge-seatbelt-stub-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let stub = dir.join("sandbox-exec");
		let argv = dir.join("argv");
		std::fs::write(
			&stub,
			format!(
				"#!/bin/sh\nprintf '%s\\0' \"$@\" > {}\nshift 2\nexec \"$@\"\n",
				argv.display()
			),
		)
		.unwrap();
		std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

		let launch = Launch {
			program: "/bin/echo".into(),
			args: vec!["compiled".into()],
			env: [("LANG".to_string(), "C.UTF-8".to_string())].into(),
			workdir: dir.clone(),
		};
		let output = run(&launch, &policy(), &stub).expect("run the stub");
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "compiled");

		let recorded: Vec<String> = std::fs::read_to_string(&argv)
			.unwrap()
			.split('\0')
			.filter(|argument| !argument.is_empty())
			.map(str::to_string)
			.collect();
		assert_eq!(recorded[0], "-p");
		assert!(recorded[1].starts_with("(version 1)\n(deny default)"), "{}", recorded[1]);
		assert_eq!(&recorded[2..], &["/bin/echo", "compiled"]);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
