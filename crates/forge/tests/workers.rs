#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

mod c_workspace;
mod forge_cli;

use c_workspace::have_compiler;
use forge_cli::{diagnostic_text, forge_bin, run_forge};

fn worker_workspace(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-iworker-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	dir
}

fn write_executable(path: &Path, body: &str) {
	std::fs::write(path, body).unwrap();
	use std::os::unix::fs::PermissionsExt;
	let mut permissions = std::fs::metadata(path).unwrap().permissions();
	permissions.set_mode(0o755);
	std::fs::set_permissions(path, permissions).unwrap();
}

fn worker_spawn_log(dir: &Path) -> PathBuf {
	dir.join("worker-spawns")
}

fn write_worker(dir: &Path, body: &str) -> PathBuf {
	let worker = dir.join("worker.sh");
	write_executable(
		&worker,
		&format!("#!/bin/sh\necho $$ >> {}\n{body}\n", worker_spawn_log(dir).display()),
	);
	worker
}

fn worker_wrapper() -> String {
	format!("exec {} worker", forge_bin())
}

fn spawns(dir: &Path) -> Vec<String> {
	std::fs::read_to_string(worker_spawn_log(dir))
		.unwrap_or_default()
		.lines()
		.filter(|line| !line.trim().is_empty())
		.map(str::to_string)
		.collect()
}

fn assert_no_surviving_worker(dir: &Path) {
	for pid in spawns(dir) {
		let mut alive = true;
		for _ in 0..100 {
			if !Path::new(&format!("/proc/{pid}")).exists() {
				alive = false;
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		assert!(!alive, "worker {pid} outlived the build in {}", dir.display());
	}
}

fn write_toolchain_workspace(dir: &Path, worker: &Path) {
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"wk\"\n\n[discovery]\ninclude = [\".\"]\n\n[catalog]\nfiles = [\"workers.toml\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("workers.toml"),
		format!(
			"[toolchains.gcc.worker]\ncommand = \"{}\"\nvariants = [\"batch\"]\n",
			worker.display()
		),
	)
	.unwrap();
}

fn write_c_sources(dir: &Path) {
	std::fs::create_dir_all(dir.join("lib")).unwrap();
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b; }\n",
	)
	.unwrap();
	std::fs::write(dir.join("lib/math.h"), "int add(int a, int b);\n").unwrap();
	std::fs::write(
		dir.join("src/main.c"),
		"#include <stdio.h>\n#include \"math.h\"\nint main(void) { printf(\"%d\\n\", add(20, 22)); return 0; }\n",
	)
	.unwrap();
}

#[test]
fn a_toolchain_action_runs_through_one_worker_and_caches_away_from_it() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = worker_workspace("gcc");
	write_c_sources(&dir);
	write_toolchain_workspace(&dir, &write_worker(&dir, &worker_wrapper()));
	std::fs::write(
		dir.join("FORGE.toml"),
		"[library.math]\nsrcs = [\"lib/math.c\"]\nhdrs = [\"lib/math.h\"]\nincludes = [\"lib\"]\nworker = \"batch\"\n\n[binary.app]\ndeps = [\"math\"]\nsrcs = [\"src/main.c\"]\nworker = \"batch\"\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "worker build failed: {log}");
	assert!(log.contains("4 executed"), "{log}");
	assert_eq!(spawns(&dir).len(), 1, "four actions must share one worker process: {log}");
	assert_no_surviving_worker(&dir);

	let binary = dir.join("forge-out/bin/debug/app");
	let output = Command::new(&binary).output().expect("run the worker-built binary");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

	let (_, cached) = run_forge(&dir, &["build"]);
	assert!(cached.contains("4 cache hits") && cached.contains("0 executed"), "{cached}");
	assert_eq!(spawns(&dir).len(), 1, "a cache hit must never reach a worker: {cached}");

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b + 1; }\n",
	)
	.unwrap();
	let (_, rebuilt) = run_forge(&dir, &["build"]);
	assert!(
		rebuilt.contains("3 executed") && rebuilt.contains("1 cache hits"),
		"{rebuilt}"
	);
	assert_eq!(
		spawns(&dir).len(),
		2,
		"a fresh build starts exactly one new worker: {rebuilt}"
	);
	assert_no_surviving_worker(&dir);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_user_tool_is_served_by_a_worker_without_interleaving_requests() {
	let dir = worker_workspace("tool");
	let tool = dir.join("fake-tool");
	write_executable(
		&tool,
		"#!/bin/sh\nmkdir -p \"$(dirname \"$2\")\"\ndate +%s.%N > \"$2\"\nsleep 0.3\ndate +%s.%N >> \"$2\"\nprintf 'tool ran %s\\n' \"$1\" >> \"$2\"\necho \"tool diagnostic for $1\"\n",
	);
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"wktool\"\n\n[discovery]\ninclude = [\".\"]\n\n[catalog]\nfiles = [\"workers.toml\"]\n\n[toolchains.fake]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("workers.toml"),
		format!(
			"[toolchains.fake.worker]\ncommand = \"{}\"\n",
			write_worker(&dir, &worker_wrapper()).display()
		),
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[rule.one]\ncommand = \"{}\"\nargs = [\"one\", \"out/one.txt\"]\noutputs = [\"out/one.txt\"]\ncompiler = \"fake\"\nworker = \"batch\"\n\n[rule.two]\ncommand = \"{}\"\nargs = [\"two\", \"out/two.txt\"]\noutputs = [\"out/two.txt\"]\ncompiler = \"fake\"\nworker = \"batch\"\n",
			tool.display(),
			tool.display()
		),
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "tool build failed: {log}");
	let one = std::fs::read_to_string(dir.join("out/one.txt")).unwrap();
	let two = std::fs::read_to_string(dir.join("out/two.txt")).unwrap();
	assert!(one.ends_with("tool ran one\n"), "{one}");
	assert!(two.ends_with("tool ran two\n"), "{two}");
	let window = |report: &str| {
		let stamps: Vec<f64> = report.lines().filter_map(|line| line.parse::<f64>().ok()).collect();
		assert_eq!(
			stamps.len(),
			2,
			"a tool run must record when it started and stopped: {report}"
		);
		(stamps[0], stamps[1])
	};
	let (one_start, one_end) = window(&one);
	let (two_start, two_end) = window(&two);
	assert!(
		one_end <= two_start || two_end <= one_start,
		"two requests reached one worker at once: {one}{two}\n{log}"
	);
	assert_eq!(spawns(&dir).len(), 1, "{log}");
	assert_no_surviving_worker(&dir);

	let (_, cached) = run_forge(&dir, &["build"]);
	assert!(cached.contains("2 cache hits"), "{cached}");
	assert_eq!(spawns(&dir).len(), 1, "{cached}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_worker_action_takes_its_worker_process_with_it() {
	let dir = worker_workspace("fail");
	let tool = dir.join("fake-tool");
	write_executable(
		&tool,
		"#!/bin/sh\nsleep 0.3\nif [ \"$1\" = \"two\" ]; then echo \"tool refuses $1\" >&2; exit 3; fi\nmkdir -p \"$(dirname \"$2\")\"\nprintf 'tool ran %s\\n' \"$1\" > \"$2\"\n",
	);
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"wkfail\"\n\n[discovery]\ninclude = [\".\"]\n\n[catalog]\nfiles = [\"workers.toml\"]\n\n[toolchains.fake]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("workers.toml"),
		format!(
			"[toolchains.fake.worker]\ncommand = \"{}\"\n",
			write_worker(&dir, &worker_wrapper()).display()
		),
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[rule.one]\ncommand = \"{}\"\nargs = [\"one\", \"out/one.txt\"]\noutputs = [\"out/one.txt\"]\ncompiler = \"fake\"\nworker = \"batch\"\n\n[rule.two]\ncommand = \"{}\"\nargs = [\"two\", \"out/two.txt\"]\noutputs = [\"out/two.txt\"]\ncompiler = \"fake\"\nworker = \"batch\"\n",
			tool.display(),
			tool.display()
		),
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "a failing action must fail the build");
	assert!(
		log.contains("tool refuses two"),
		"the worker must relay the tool diagnostic: {log}"
	);
	assert!(
		!dir.join("out/two.txt").exists(),
		"an undeclared output must not be published"
	);
	assert!(!spawns(&dir).is_empty(), "the failing action must have used a worker");
	assert_no_surviving_worker(&dir);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_crashed_worker_fails_the_action_and_the_next_build_gets_a_fresh_one() {
	let dir = worker_workspace("crash");
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("src/main.c"),
		"#include <stdio.h>\nint main(void) { printf(\"built\\n\"); return 0; }\n",
	)
	.unwrap();
	let crash_flag = dir.join("crash-once");
	let worker = write_worker(
		&dir,
		&format!(
			"if [ -f {} ]; then rm -f {}; echo 'worker lost its compiler' >&2; exit 9; fi\n{}",
			crash_flag.display(),
			crash_flag.display(),
			worker_wrapper()
		),
	);
	write_toolchain_workspace(&dir, &worker);
	std::fs::write(&crash_flag, b"1").unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.c\"]\nworker = \"batch\"\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "a crashed worker must fail the build: {log}");
	assert!(log.contains("worker") && log.contains("closed the connection"), "{log}");
	assert_eq!(spawns(&dir).len(), 1, "a crashed worker must not be retried in place");
	assert_no_surviving_worker(&dir);

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "the next build must start a fresh worker: {log}");
	assert_eq!(spawns(&dir).len(), 2, "{log}");
	assert!(dir.join("forge-out/bin/debug/app").is_file(), "{log}");
	assert_no_surviving_worker(&dir);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_worker_declaration_without_a_worker_program_is_rejected() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = worker_workspace("reject");
	write_c_sources(&dir);
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"wkreject\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.c\"]\nworker = \"batch\"\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "an undeclared worker must not be silently ignored: {log}");
	assert!(diagnostic_text(&log).contains("toolchain `gcc` declares no worker"), "{log}");

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"wkreject\"\n\n[discovery]\ninclude = [\".\"]\n\n[catalog]\nfiles = [\"workers.toml\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("workers.toml"),
		format!(
			"[toolchains.gcc.worker]\ncommand = \"{}\"\nvariants = [\"incremental\"]\n",
			write_worker(&dir, &worker_wrapper()).display()
		),
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "an undeclared variant must be rejected: {log}");
	assert!(
		diagnostic_text(&log).contains("does not declare worker variant `batch`"),
		"{log}"
	);

	let _ = std::fs::remove_dir_all(&dir);
}
