use std::path::{Path, PathBuf};
use std::process::Command;

use forge_engine::Engine;

mod common;

use common::*;

fn workspace(name: &str, rust_config: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-rust-modes-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"[project]\nname = \"rust_modes\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.rust]\nfrom = \"version\"\nversion = \"1.98.0\"\n\n[cell.rust]\n{rust_config}\n"
		),
	)
	.unwrap();
	dir
}

fn install_toolchain_link(dir: &Path) {
	assert!(have_rustc(), "Rust mode execution tests require an installed rustc");
	if link_rust_toolchain(dir) {
		return;
	}
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
	let source = root.join(".forge/toolchains/rust/1.98.0");
	assert!(
		source.exists(),
		"Rust mode execution tests require the installed Rust 1.98.0 toolchain"
	);
	let link = dir.join(".forge/toolchains/rust/1.98.0");
	std::fs::create_dir_all(link.parent().unwrap()).unwrap();
	#[cfg(unix)]
	std::os::unix::fs::symlink(source, link).unwrap();
	#[cfg(not(unix))]
	panic!("Rust integration test requires a workspace toolchain symlink");
}

fn assert_rustc_plan(dir: &Path, count: usize) -> forge_engine::planner::ActionDag {
	let (_, dag) = Engine::open(dir).plan_dag("debug").expect("plan direct-rustc actions");
	assert_eq!(dag.specs.len(), count, "unexpected action plan: {dag:?}");
	for spec in &dag.specs {
		assert_eq!(Path::new(&spec.command).file_stem().unwrap(), "rustc", "{spec:?}");
	}
	dag
}

fn build_and_run(dir: &Path, binary: &str, executed: usize, expected: &str) {
	let (ok, log) = run_forge(dir, &["build"]);
	assert!(ok, "Rust mode build failed in {}: {log}", dir.display());
	assert!(
		log.contains(&format!("{executed} executed")),
		"expected actual compilation: {log}"
	);
	let path = dir.join(format!("forge-out/bin/debug/{binary}"));
	let output = Command::new(&path).output().expect("execute the Forge-built Rust binary");
	assert!(output.status.success(), "{} failed: {output:?}", path.display());
	assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn cargo_root_imports_library_and_binary_without_forge_targets_and_preserves_cargo_files() {
	let dir = workspace("cargo-root", "mode = \"cargo\"");
	install_toolchain_link(&dir);
	let manifest = "[package]\nname = \"cargo-root\"\nversion = \"1.2.3\"\nedition = \"2021\"\n";
	let lock = "version = 4\n\n[[package]]\nname = \"cargo-root\"\nversion = \"1.2.3\"\n";
	let invalid_lock = "this is deliberately not valid forge.lock TOML [";
	std::fs::write(dir.join("Cargo.toml"), manifest).unwrap();
	std::fs::write(dir.join("Cargo.lock"), lock).unwrap();
	std::fs::write(dir.join("forge.lock"), invalid_lock).unwrap();
	std::fs::write(
		dir.join("src/lib.rs"),
		"pub fn version() -> &'static str { env!(\"CARGO_PKG_VERSION\") }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/main.rs"),
		"fn main() { println!(\"{}:{}\", cargo_root::version(), env!(\"CARGO_PKG_NAME\")); }\n",
	)
	.unwrap();

	let dag = assert_rustc_plan(&dir, 2);
	assert!(
		dag.specs
			.iter()
			.any(|spec| spec.args.windows(2).any(|args| args == ["--crate-type", "lib"]))
	);
	assert!(
		dag.specs
			.iter()
			.any(|spec| spec.args.windows(2).any(|args| args == ["--crate-type", "bin"]))
	);
	build_and_run(&dir, "cargo_root_bin", 2, "1.2.3:cargo-root\n");
	for (path, contents) in [("Cargo.toml", manifest), ("Cargo.lock", lock), ("forge.lock", invalid_lock)] {
		assert_eq!(
			std::fs::read(dir.join(path)).unwrap(),
			contents.as_bytes(),
			"{path} was modified"
		);
	}
	assert!(!dir.join("FORGE.toml").exists());
	assert!(!dir.join("FORGE.rhai").exists());
	assert!(!dir.join("target").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn native_metadata_builds_custom_crate_root_and_ignores_added_invalid_cargo_files() {
	let dir = workspace("native", "mode = \"native\"");
	install_toolchain_link(&dir);
	std::fs::create_dir_all(dir.join("custom")).unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.native_app]\nsrcs = [\"src/main.rs\", \"custom/entry.rs\"]\n\n[binary.native_app.metadata.rust]\nroot = \"custom\"\ncrate_root = \"custom/entry.rs\"\nname = \"native_crate\"\n\n[binary.native_app.metadata.rust.package]\nname = \"native-package\"\nedition = \"2021\"\nversion = \"7.8.9\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/main.rs"),
		"compile_error!(\"the conventional crate root must not be used\");\n",
	)
	.unwrap();
	assert!(!dir.join("Cargo.toml").exists());
	assert!(!dir.join("Cargo.lock").exists());
	for phase in ["without-cargo", "invalid-cargo"] {
		if phase == "invalid-cargo" {
			std::fs::write(dir.join("Cargo.toml"), "deliberately invalid Cargo manifest [").unwrap();
			std::fs::write(dir.join("Cargo.lock"), "deliberately invalid Cargo lock [").unwrap();
		}
		std::fs::write(
			dir.join("custom/entry.rs"),
			format!(
				"fn main() {{ let values: Vec<i32> = [20, 22].into_iter().collect(); println!(\"{{}}:{{}}:{{}}:{{}}:{phase}\", env!(\"CARGO_PKG_VERSION\"), env!(\"CARGO_PKG_NAME\"), env!(\"CARGO_CRATE_NAME\"), values.iter().sum::<i32>()); }}\n"
			),
		)
		.unwrap();
		let dag = assert_rustc_plan(&dir, 1);
		let spec = &dag.specs[0];
		assert_eq!(spec.args.last().unwrap(), "custom/entry.rs");
		assert!(spec.args.windows(2).any(|args| args == ["--crate-name", "native_crate"]));
		assert!(spec.args.windows(2).any(|args| args == ["--edition", "2021"]));
		assert_eq!(spec.env.get("CARGO_PKG_VERSION").unwrap(), "7.8.9");
		assert!(!spec.inputs.iter().any(|path| {
			path.file_name()
				.is_some_and(|name| name == "Cargo.toml" || name == "Cargo.lock")
		}));
		build_and_run(
			&dir,
			"native_app",
			1,
			&format!("7.8.9:native-package:native_crate:42:{phase}\n"),
		);
		if phase == "without-cargo" {
			assert!(!dir.join("Cargo.toml").exists());
			assert!(!dir.join("Cargo.lock").exists());
		}
	}
	assert_eq!(
		std::fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
		"deliberately invalid Cargo manifest ["
	);
	assert_eq!(
		std::fs::read_to_string(dir.join("Cargo.lock")).unwrap(),
		"deliberately invalid Cargo lock ["
	);
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_workspace_builds_members_with_normal_aliased_path_dependency() {
	let dir = workspace("cargo-workspace", "mode = \"cargo\"");
	install_toolchain_link(&dir);
	std::fs::create_dir_all(dir.join("crates/math/src")).unwrap();
	std::fs::create_dir_all(dir.join("crates/app/src")).unwrap();
	let files = [
		(
			"Cargo.toml",
			"[workspace]\nresolver = \"2\"\nmembers = [\"crates/math\", \"crates/app\"]\n",
		),
		(
			"Cargo.lock",
			"version = 4\n\n[[package]]\nname = \"math-package\"\nversion = \"2.3.4\"\n\n[[package]]\nname = \"workspace-app\"\nversion = \"5.6.7\"\ndependencies = [\"math-package\"]\n",
		),
		(
			"crates/math/Cargo.toml",
			"[package]\nname = \"math-package\"\nversion = \"2.3.4\"\nedition = \"2021\"\n\n[lib]\nname = \"math_library\"\n",
		),
		(
			"crates/app/Cargo.toml",
			"[package]\nname = \"workspace-app\"\nversion = \"5.6.7\"\nedition = \"2021\"\n\n[dependencies]\narithmetic = { package = \"math-package\", path = \"../math\" }\n",
		),
		(
			"crates/math/src/lib.rs",
			"pub fn answer() -> i32 { 42 }\npub fn version() -> &'static str { env!(\"CARGO_PKG_VERSION\") }\n",
		),
		(
			"crates/app/src/main.rs",
			"fn main() { println!(\"{}:{}:{}\", arithmetic::answer(), arithmetic::version(), env!(\"CARGO_PKG_VERSION\")); }\n",
		),
	];
	for (path, contents) in files {
		std::fs::write(dir.join(path), contents).unwrap();
	}
	let dag = assert_rustc_plan(&dir, 2);
	assert!(dag.specs.iter().any(|spec| {
		spec.args
			.windows(2)
			.any(|args| args[0] == "--extern" && args[1].starts_with("arithmetic="))
	}));
	build_and_run(&dir, "workspace_app", 2, "42:2.3.4:5.6.7\n");
	for (path, contents) in files {
		assert_eq!(
			std::fs::read(dir.join(path)).unwrap(),
			contents.as_bytes(),
			"{path} was modified"
		);
	}
	assert!(!dir.join("FORGE.toml").exists());
	assert!(!dir.join("forge.lock").exists());
	assert!(!dir.join("target").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn native_dependencies_lock_sync_and_build_without_cargo_metadata() {
	let dir = workspace(
		"native-dependencies",
		"mode = \"native\"\n[cell.rust.dependencies.answer]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/answer.tar.gz\"\nchecksum = \"fixture\"",
	);
	install_toolchain_link(&dir);
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n[patch.local.answer]\npath = \"vendor/answer\"\n"),
	)
	.unwrap();
	std::fs::create_dir_all(dir.join("vendor/answer/src")).unwrap();
	std::fs::write(dir.join("vendor/answer/src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
	std::fs::write(dir.join("FORGE.toml"), "[binary.app]\nsrcs = [\"src/main.rs\"]\n").unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", answer::answer()); }\n").unwrap();
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?}: {log}");
	}
	let lock = std::fs::read_to_string(dir.join("forge.lock")).unwrap();
	assert!(lock.contains("answer"));
	assert_rustc_plan(&dir, 2);
	build_and_run(&dir, "app", 2, "42\n");
	assert_eq!(std::fs::read_to_string(dir.join("forge.lock")).unwrap(), lock);
	assert!(!dir.join("Cargo.toml").exists());
	assert!(!dir.join("Cargo.lock").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_mode_requires_explicit_lock_without_creating_one() {
	let dir = workspace("missing-lock", "mode = \"cargo\"");
	let manifest = "[package]\nname = \"missing-lock\"\nversion = \"1.0.0\"\nedition = \"2021\"\n";
	std::fs::write(dir.join("Cargo.toml"), manifest).unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "missing Cargo.lock must fail: {log}");
	assert!(
		log.contains("cargo mode requires an explicit Cargo.lock"),
		"wrong diagnostic: {log}"
	);
	assert!(
		log.contains("Forge does not run Cargo or solve Cargo versions"),
		"missing remedy context: {log}"
	);
	assert_eq!(std::fs::read(dir.join("Cargo.toml")).unwrap(), manifest.as_bytes());
	assert!(!dir.join("Cargo.lock").exists());
	assert!(!dir.join("forge.lock").exists());
	assert!(!dir.join("forge-out/bin").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_mode_requires_root_manifest() {
	let dir = workspace("missing-manifest", "mode = \"cargo\"");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "missing Cargo.toml must fail: {log}");
	assert!(log.contains("cargo mode requires Cargo.toml"), "wrong diagnostic: {log}");
	assert!(!dir.join("Cargo.toml").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_rust_mode_is_rejected_before_toolchain_resolution() {
	let dir = workspace("unknown-mode", "mode = \"automatic\"");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "unknown mode must fail: {log}");
	assert!(log.contains("unknown cell.rust mode: automatic"), "wrong diagnostic: {log}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_mode_rejects_native_dependency_configuration() {
	let dir = workspace("mixed-modes", "mode = \"cargo\"\ndependencies = {}");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "mixed dependency modes must fail: {log}");
	assert!(
		log.contains("cell.rust.dependencies is native-only"),
		"wrong diagnostic: {log}"
	);
	std::fs::remove_dir_all(dir).unwrap();
}
