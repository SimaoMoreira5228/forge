#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use forge_core::Confinement;
use forge_engine::confine::{self, Backend};
#[cfg(unix)]
use forge_engine::runner::{self, Launch};

#[cfg(unix)]
fn scratch(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-confinement-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	dir
}

/// Runs a real action through the runner and reports whether it reached a path outside its own
/// sandbox root.
#[cfg(unix)]
fn reach_outside() -> bool {
	let root = scratch("root");
	let outside = scratch("outside");
	let secret = outside.join("forbidden");
	std::fs::write(&secret, b"secret").unwrap();
	let script = format!(
		"printf ok > {inside} || exit 20\n\
		 cat {secret} > /dev/null 2>&1 && exit 21\n\
		 printf escaped > {secret} 2>/dev/null && exit 22\n\
		 exit 0\n",
		inside = root.join("inside").display(),
		secret = secret.display(),
	);
	let launch = Launch {
		program: "/bin/sh".into(),
		args: vec!["-c".into(), script],
		env: BTreeMap::from([
			("PATH".to_string(), "/usr/bin:/bin".to_string()),
			("LANG".to_string(), "C.UTF-8".to_string()),
		]),
		workdir: root.clone(),
	};
	let confinement = Confinement::new(&root, vec![root.to_path_buf()]);
	let policy = confine::Policy::of(&confinement, None, &launch.program);
	let output = runner::spawn(&launch, None, &policy).expect("spawn the probe action");
	assert!(
		root.join("inside").exists(),
		"an action that cannot write inside its own sandbox cannot build"
	);
	let code = output.status.code();
	assert_ne!(code, Some(20), "the action could not write inside its sandbox");
	let escaped = matches!(code, Some(21) | Some(22));
	if !escaped {
		assert_eq!(
			std::fs::read_to_string(&secret).unwrap(),
			"secret",
			"a denied escape still modified a file outside the sandbox"
		);
	}
	let _ = std::fs::remove_dir_all(root);
	let _ = std::fs::remove_dir_all(outside);
	escaped
}

#[test]
fn the_active_backend_reports_exactly_what_it_enforces() {
	let report = confine::active();
	let rendered = report.renders();
	assert!(rendered.contains(report.backend.name()), "{rendered}");
	assert!(rendered.contains(std::env::consts::OS), "{rendered}");
	assert!(!report.probe.is_empty(), "a backend must state how it was probed: {rendered}");
	if !report.gates_paths {
		assert!(
			report.filesystem.starts_with("none:") || report.filesystem.contains("stays readable"),
			"a backend that confines nothing must say so in words, not in a field: {rendered}"
		);
	}
	if !report.gates_network {
		assert!(report.network.starts_with("not restricted:"), "{rendered}");
	}
	if report.gates_paths {
		assert!(
			!report.filesystem.starts_with("none:"),
			"a backend that gates paths cannot also report that it gates nothing: {rendered}"
		);
	}
}

#[test]
fn the_cli_reports_the_same_backend_the_engine_chose() {
	let reported = Command::new(env!("CARGO_BIN_EXE_forge"))
		.arg("confine")
		.output()
		.expect("run forge confine");
	let text = String::from_utf8_lossy(&reported.stdout).into_owned();
	assert!(
		reported.status.success(),
		"forge confine failed: {text}{}",
		String::from_utf8_lossy(&reported.stderr)
	);
	assert!(text.contains(confine::active().backend.name()), "{text}");
}

#[cfg(unix)]
#[test]
fn a_real_action_never_reaches_outside_a_sandbox_the_report_calls_confined() {
	let report = confine::active();
	if !report.gates_paths {
		return;
	}
	assert!(
		!reach_outside(),
		"forge reports that it gates paths, and a real action reached outside its sandbox root:\n{}",
		report.renders()
	);
}

#[cfg(all(unix, target_os = "linux"))]
#[test]
fn linux_selects_landlock_and_enforces_it() {
	let report = confine::active();
	assert_eq!(
		report.backend,
		Backend::Landlock,
		"forge has a linux backend and it did not run; a degraded host is a real failure, not a pass:\n{}",
		report.renders()
	);
	assert!(report.gates_paths, "{}", report.renders());
	assert!(
		!report.gates_network,
		"landlock filesystem rules do not cover sockets: {}",
		report.renders()
	);
}

#[cfg(all(unix, target_os = "macos"))]
#[test]
fn macos_selects_seatbelt_and_enforces_it() {
	let report = confine::active();
	assert_eq!(
		report.backend,
		Backend::Seatbelt,
		"forge has a macos backend and it did not run; a degraded host is a real failure, not a pass:\n{}",
		report.renders()
	);
	assert!(report.gates_paths, "{}", report.renders());
}

#[cfg(target_os = "windows")]
#[test]
fn windows_selects_job_objects_and_claims_no_path_confinement() {
	let report = confine::active();
	assert_eq!(
		report.backend,
		Backend::JobObjects,
		"forge has a windows backend and it did not run; a degraded host is a real failure, not a pass:\n{}",
		report.renders()
	);
	assert!(
		!report.gates_paths,
		"a job object contains the process tree, not paths: {}",
		report.renders()
	);
	assert!(!report.gates_network, "{}", report.renders());
	assert!(report.filesystem.starts_with("none:"), "{}", report.renders());
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
#[test]
fn this_platform_has_no_confinement_backend_at_all() {
	let report = confine::active();
	assert_eq!(report.backend, Backend::CopySandbox, "{}", report.renders());
	assert!(!report.gates_paths, "{}", report.renders());
	assert!(!report.gates_network, "{}", report.renders());
	panic!(
		"forge confines nothing on {}: there is no backend here, so this suite proves nothing. \
		 the build still runs, but every path confinement guarantee is unbacked.\n{}",
		std::env::consts::OS,
		report.renders()
	);
}
