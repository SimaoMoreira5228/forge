mod forge_cli;

use std::path::{Path, PathBuf};
use std::process::Command;

use forge_cli::{action_keys, run_forge};

fn workspace(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-contract-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"contract\"\n[discovery]\ninclude = [\".\"]\n",
	)
	.unwrap();
	dir
}

fn executable(dir: &Path, body: &str) -> PathBuf {
	let bin = dir.join("tools/bin");
	std::fs::create_dir_all(&bin).unwrap();
	let source = dir.join("helper.rs");
	let program = bin.join(format!("helper{}", std::env::consts::EXE_SUFFIX));
	std::fs::write(&source, format!("fn main() {{ {body} }}")).unwrap();
	let result = Command::new("rustc").arg(&source).arg("-o").arg(&program).output().unwrap();
	assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
	program
}

#[test]
fn writing_file_and_directory_inputs_cannot_change_workspace_sources() {
	let dir = workspace("input-writes");
	std::fs::create_dir_all(dir.join("sources")).unwrap();
	std::fs::write(dir.join("input.txt"), "original").unwrap();
	std::fs::write(dir.join("sources/nested.txt"), "original").unwrap();
	let program = executable(
		&dir,
		r#"std::fs::write("input.txt", "changed").unwrap(); std::fs::write("sources/nested.txt", "changed").unwrap(); std::fs::write("forge-out/result.txt", "ok").unwrap();"#,
	);
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			r#"
[rule.overwrite]
command = {program:?}
inputs = ["input.txt", "sources"]
outputs = ["forge-out/result.txt"]
"#
		),
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "{log}");
	assert_eq!(std::fs::read_to_string(dir.join("input.txt")).unwrap(), "original");
	assert_eq!(std::fs::read_to_string(dir.join("sources/nested.txt")).unwrap(), "original");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn changing_a_path_helper_invalidates_the_action_cache() {
	let dir = workspace("helper-key");
	std::fs::create_dir_all(dir.join("tools/bin")).unwrap();
	let helper = executable(&dir, r#"print!("first");"#);
	let driver_source = dir.join("driver.rs");
	let driver = dir.join(format!("driver{}", std::env::consts::EXE_SUFFIX));
	std::fs::write(&driver_source, r#"fn main() { let result = std::process::Command::new("helper").output().unwrap(); assert!(result.status.success()); std::fs::write("forge-out/result.txt", result.stdout).unwrap(); }"#).unwrap();
	let result = Command::new("rustc")
		.arg(&driver_source)
		.arg("-o")
		.arg(&driver)
		.output()
		.unwrap();
	assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
	let config = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"{config}\n[toolchains.helper]\nfrom = \"path\"\npath = {:?}\n",
			dir.join("tools")
		),
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			r#"
[rule.helper]
command = {driver:?}
outputs = ["forge-out/result.txt"]
"#
		),
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "{log}");
	let first = action_keys(&dir);
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok && log.contains("0 executed"), "{log}");
	assert_eq!(helper, executable(&dir, r#"print!("second");"#));
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "{log}");
	assert_ne!(first, action_keys(&dir));
	assert_eq!(std::fs::read_to_string(dir.join("forge-out/result.txt")).unwrap(), "second");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn syntax_errors_render_the_source_location_and_highlight() {
	let dir = workspace("diagnostic");
	std::fs::write(dir.join("FORGE.toml"), "[rule.bad]\ncommand = \"unterminated\n").unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "{log}");
	assert!(log.contains("FORGE.toml:2:"), "{log}");
	assert!(log.contains("command = \"unterminated"), "{log}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn selected_coverage_excludes_previous_targets_and_reports() {
	let dir = workspace("coverage-selection");
	std::fs::create_dir_all(dir.join("tools/bin")).unwrap();
	executable(
		&dir,
		r#"let args: Vec<_> = std::env::args().skip(1).collect(); let count = args.iter().filter(|arg| arg.ends_with(".raw")).count(); for arg in args { println!("SF:{arg}"); } println!("LF:{count}\nLH:{count}");"#,
	);
	let generator = executable(
		&dir.join("generator"),
		r#"let name = std::env::args().nth(1).unwrap(); std::fs::write(format!("forge-out/profile/coverage/{name}.raw"), "raw").unwrap(); std::fs::write(format!("forge-out/bin/coverage/{name}"), "object").unwrap();"#,
	);
	let config = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"{config}\n[toolchains.fake]\nfrom = \"path\"\npath = {:?}\n[catalog]\nfiles = [\"catalog.toml\"]\n",
			dir.join("tools")
		),
	)
	.unwrap();
	std::fs::write(
		dir.join("catalog.toml"),
		r#"
[toolchains.fake.coverage]
raw_extension = "raw"
format = "lcov"
commands = [{ tool = "helper", args = ["{raw}", "{objects}"], stdout = "{work}/coverage.info" }]
"#,
	)
	.unwrap();
	let mut declarations = String::new();
	for name in ["one", "two"] {
		declarations.push_str(&format!(
			r#"
[rule.{name}]
command = {generator:?}
args = ["{name}"]
outputs = ["forge-out/profile/coverage/{name}.raw", "forge-out/bin/coverage/{name}"]
"#
		));
	}
	std::fs::write(dir.join("FORGE.toml"), declarations).unwrap();
	let (ok, log) = run_forge(&dir, &["coverage"]);
	assert!(ok && log.contains("lines: 2/2"), "{log}");
	std::fs::write(dir.join("forge-out/coverage/stale.raw"), "stale").unwrap();
	let (ok, log) = run_forge(&dir, &["coverage", "//:one"]);
	assert!(ok && log.contains("lines: 1/1"), "{log}");
	assert!(!dir.join("forge-out/coverage/two.raw").exists());
	let report = std::fs::read_to_string(dir.join("forge-out/coverage/coverage.info")).unwrap();
	assert!(!report.contains("two") && !report.contains("stale"), "{report}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "linux")]
fn real_coverage(compiler: &str) {
	if !Path::new(&format!("/usr/bin/{compiler}")).is_file() {
		eprintln!("skipping: no {compiler}");
		return;
	}
	let dir = workspace(&format!("coverage-{compiler}"));
	let config = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{config}\n[toolchains.{compiler}]\nfrom = \"path\"\npath = \"/usr\"\n"),
	)
	.unwrap();
	std::fs::write(dir.join("shared.c"), "int shared(int x) { return x + 1; }\n").unwrap();
	let mut declarations =
		format!("[library.shared]\nsrcs = [\"shared.c\"]\nvisibility = \"public\"\ncompiler = \"{compiler}\"\n");
	for name in ["one", "two"] {
		std::fs::write(
			dir.join(format!("{name}.c")),
			format!(
				"int shared(int); int {name}(int x) {{ return shared(x); }}\nint main(void) {{ return {name}(1) != 2; }}\n"
			),
		)
		.unwrap();
		declarations.push_str(&format!(
			"[test.{name}]\nsrcs = [\"{name}.c\"]\ndeps = [\"shared\"]\ncompiler = \"{compiler}\"\n"
		));
	}
	std::fs::write(dir.join("FORGE.toml"), declarations).unwrap();
	let (ok, log) = run_forge(&dir, &["coverage"]);
	assert!(ok && log.contains("lines: 5/5"), "{log}");
	let (ok, log) = run_forge(&dir, &["coverage", "//:one"]);
	assert!(ok && log.contains("lines: 3/3"), "{log}");
	assert!(log.contains("0 executed, 1 cached"), "{log}");
	let mut pending = vec![dir.join("forge-out/coverage")];
	let mut reports = String::new();
	while let Some(path) = pending.pop() {
		if path.is_dir() {
			pending.extend(std::fs::read_dir(path).unwrap().map(|entry| entry.unwrap().path()));
		} else if path
			.extension()
			.is_some_and(|extension| extension == "info" || extension == "gcov")
		{
			reports.push_str(&std::fs::read_to_string(path).unwrap());
		}
	}
	assert!(
		reports.contains("one.c") && reports.contains("shared.c") && !reports.contains("two.c"),
		"{reports}"
	);
	std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn gcc_coverage_uses_only_selected_profiles() {
	real_coverage("gcc");
}

#[cfg(target_os = "linux")]
#[test]
fn llvm_coverage_uses_only_selected_profiles_and_objects() {
	real_coverage("clang");
}
