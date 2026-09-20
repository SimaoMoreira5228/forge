use std::path::{Path, PathBuf};
use std::process::Command;

mod forge_cli;
mod rust_toolchain;

use forge_cli::run_forge;
use rust_toolchain::link_rust_toolchain;

fn copy_tree(source: &Path, destination: &Path) {
	std::fs::create_dir_all(destination).unwrap();
	for entry in std::fs::read_dir(source).unwrap() {
		let entry = entry.unwrap();
		let name = entry.file_name();
		if name == "forge-out" || name == ".forge" {
			continue;
		}
		let target = destination.join(&name);
		if entry.file_type().unwrap().is_dir() {
			copy_tree(&entry.path(), &target);
		} else {
			std::fs::copy(entry.path(), target).unwrap();
		}
	}
}

fn install_toolchain_link(dir: &Path) {
	if link_rust_toolchain(dir) {
		return;
	}
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
	let source = root.join(".forge/toolchains/rust/1.98.0");
	assert!(
		source.exists(),
		"Rust example tests require the installed Rust 1.98.0 toolchain"
	);
	let link = dir.join(".forge/toolchains/rust/1.98.0");
	std::fs::create_dir_all(link.parent().unwrap()).unwrap();
	#[cfg(unix)]
	std::os::unix::fs::symlink(source, link).unwrap();
	#[cfg(not(unix))]
	std::fs::copy(source, link).unwrap();
}

fn example(name: &str, variant: &str) -> PathBuf {
	let source = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.parent()
		.unwrap()
		.join("examples")
		.join(name);
	let dir = std::env::temp_dir().join(format!("forge-example-{name}-{variant}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	copy_tree(&source, &dir);
	install_toolchain_link(&dir);
	dir
}

fn dependencies_synced(dir: &Path) -> bool {
	if std::env::var_os("FORGE_EXAMPLE_DEPS").is_none() {
		eprintln!("skipping: set FORGE_EXAMPLE_DEPS=1 to sync blake3 and aws-lc-rs");
		return false;
	}
	let (ok, log) = run_forge(dir, &["deps", "sync"]);
	if !ok {
		eprintln!("skipping: rust-native dependencies unavailable ({log})");
	}
	ok
}

fn listed_tests(binary: &Path) -> String {
	let output = Command::new(binary)
		.arg("--list")
		.output()
		.expect("list Rust test harness cases");
	assert!(output.status.success(), "{} --list failed", binary.display());
	String::from_utf8(output.stdout).unwrap()
}

#[test]
fn rust_native_example_builds_runs_and_tests_without_cargo() {
	let dir = example("rust-native", "build");
	if !dependencies_synced(&dir) {
		std::fs::remove_dir_all(&dir).unwrap();
		return;
	}
	assert!(!dir.join("Cargo.toml").exists());
	assert!(!dir.join("Cargo.lock").exists());

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "rust-native build failed: {log}");
	assert!(dir.join("forge-out/bin/debug/app").is_file(), "{log}");
	assert!(dir.join("forge-out/lib/debug/libmath.rlib").is_file(), "{log}");

	let (ok, log) = run_forge(&dir, &["run", "app"]);
	assert!(ok, "rust-native run failed: {log}");
	assert!(
		log.contains("math: 42") && log.contains("blake3: 2e3c0f4f") && log.contains("sha256: 32 bytes"),
		"{log}"
	);

	let unit = listed_tests(&dir.join("forge-out/test/debug/math_unit"));
	assert!(
		unit.contains("tests::adds_values") && unit.contains("tests::negative_operands"),
		"{unit}"
	);
	let integration = listed_tests(&dir.join("forge-out/test/debug/math_integration"));
	assert!(
		integration.contains("public_api_answers") && integration.contains("handles_edges"),
		"{integration}"
	);

	let (ok, log) = run_forge(&dir, &["test"]);
	assert!(ok, "rust-native test failed: {log}");
	assert!(log.contains("2 executed"), "{log}");

	assert!(!dir.join("target").exists());
	assert!(!dir.join("Cargo.toml").exists());
	std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rust_native_test_failure_is_reported() {
	let dir = example("rust-native", "failure");
	if !dependencies_synced(&dir) {
		std::fs::remove_dir_all(&dir).unwrap();
		return;
	}
	let lib = dir.join("src/lib.rs");
	let source = std::fs::read_to_string(&lib).unwrap();
	std::fs::write(
		&lib,
		source.replace("assert_eq!(add(20, 22), 42)", "assert_eq!(add(20, 22), 41)"),
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["test"]);
	assert!(!ok, "failing Rust test must fail the run: {log}");
	assert!(log.contains("action `run //:math_unit` failed"), "{log}");
	assert!(log.contains("FAILED") || log.contains("panicked"), "{log}");
	std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rust_native_release_profile_builds_and_runs_without_declaration() {
	let dir = example("rust-native", "release");
	if !dependencies_synced(&dir) {
		std::fs::remove_dir_all(&dir).unwrap();
		return;
	}
	assert!(
		!std::fs::read_to_string(dir.join("FORGE_ROOT"))
			.unwrap()
			.contains("profile.release"),
		"release must be a built-in profile"
	);

	let (ok, log) = run_forge(&dir, &["build", "--profile", "release"]);
	assert!(ok, "rust-native release build failed: {log}");
	assert!(dir.join("forge-out/bin/release/app").is_file(), "{log}");
	assert!(dir.join("forge-out/test/release/math_unit").is_file(), "{log}");

	let (ok, log) = run_forge(&dir, &["run", "app", "--profile", "release"]);
	assert!(ok, "rust-native release run failed: {log}");
	assert!(log.contains("42"), "{log}");

	let (ok, log) = run_forge(&dir, &["test", "--profile", "release"]);
	assert!(ok, "rust-native release test failed: {log}");
	assert!(log.contains("2 executed"), "{log}");
	std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rust_cargo_example_imports_cargo_files_and_runs_without_cargo() {
	let dir = example("rust-cargo", "build");
	let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
	let lock = std::fs::read_to_string(dir.join("Cargo.lock")).unwrap();
	assert!(!dir.join("FORGE.toml").exists());
	assert!(!dir.join("FORGE.rhai").exists());

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "rust-cargo build failed: {log}");
	assert!(dir.join("forge-out/bin/debug/rust_cargo_bin").is_file(), "{log}");

	let (ok, log) = run_forge(&dir, &["run", "rust_cargo_bin"]);
	assert!(ok, "rust-cargo run failed: {log}");
	assert!(log.contains("42"), "{log}");

	assert_eq!(std::fs::read_to_string(dir.join("Cargo.toml")).unwrap(), manifest);
	assert_eq!(std::fs::read_to_string(dir.join("Cargo.lock")).unwrap(), lock);
	assert!(!dir.join("target").exists());
	std::fs::remove_dir_all(&dir).unwrap();
}
