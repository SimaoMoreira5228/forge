use std::path::Path;
use std::process::Command;

use forge_engine::Engine;

mod forge_cli;
mod rust_toolchain;
mod rust_workspace;

use forge_cli::{diagnostic_text, run_forge};
use rust_toolchain::install_rust_toolchain;
use rust_workspace::{build_and_run_rust, rust_workspace};

fn assert_rustc_plan(dir: &Path, count: usize) -> forge_engine::build::planner::ActionDag {
	let (_, dag) = Engine::open(dir).plan_dag("debug").expect("plan direct-rustc actions");
	assert_eq!(dag.specs.len(), count, "unexpected action plan: {dag:?}");
	for spec in &dag.specs {
		assert_eq!(Path::new(&spec.command).file_stem().unwrap(), "rustc", "{spec:?}");
	}
	dag
}

#[test]
fn cargo_root_imports_library_and_binary_without_forge_targets_and_preserves_cargo_files() {
	let dir = rust_workspace("cargo-root", "mode = \"cargo\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
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
	build_and_run_rust(&dir, "cargo_root_bin", 2, "1.2.3:cargo-root\n");
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
	let dir = rust_workspace("native", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
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
		build_and_run_rust(
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
	let dir = rust_workspace("cargo-workspace", "mode = \"cargo\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
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
	build_and_run_rust(&dir, "workspace_app", 2, "42:2.3.4:5.6.7\n");
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
	let dir = rust_workspace("native-dependencies", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n[patch.local.answer]\npath = \"vendor/answer\"\n"),
	)
	.unwrap();
	std::fs::create_dir_all(dir.join("vendor/answer/src")).unwrap();
	std::fs::write(dir.join("vendor/answer/src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/answer.tar.gz\"\nchecksum = \"fixture\"\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", answer::answer()); }\n").unwrap();
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?}: {log}");
	}
	let lock = std::fs::read_to_string(dir.join("forge.lock")).unwrap();
	assert!(lock.contains("answer"));
	assert_rustc_plan(&dir, 2);
	build_and_run_rust(&dir, "app", 2, "42\n");
	assert_eq!(std::fs::read_to_string(dir.join("forge.lock")).unwrap(), lock);
	assert!(!dir.join("Cargo.toml").exists());
	assert!(!dir.join("Cargo.lock").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_mode_requires_explicit_lock_without_creating_one() {
	let dir = rust_workspace("missing-lock", "mode = \"cargo\"");
	let manifest = "[package]\nname = \"missing-lock\"\nversion = \"1.0.0\"\nedition = \"2021\"\n";
	std::fs::write(dir.join("Cargo.toml"), manifest).unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "missing Cargo.lock must fail: {log}");
	assert!(
		diagnostic_text(&log).contains("cargo mode requires an explicit Cargo.lock"),
		"wrong diagnostic: {log}"
	);
	assert!(
		diagnostic_text(&log).contains("Forge does not run Cargo or solve Cargo versions"),
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
	let dir = rust_workspace("missing-manifest", "mode = \"cargo\"");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "missing Cargo.toml must fail: {log}");
	assert!(log.contains("cargo mode requires Cargo.toml"), "wrong diagnostic: {log}");
	assert!(!dir.join("Cargo.toml").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_rust_mode_is_rejected_before_toolchain_resolution() {
	let dir = rust_workspace("unknown-mode", "mode = \"automatic\"");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "unknown mode must fail: {log}");
	assert!(log.contains("unknown cell.rust mode: automatic"), "wrong diagnostic: {log}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_cell_rust_key_is_rejected() {
	let dir = rust_workspace("mixed-modes", "mode = \"cargo\"\nregistries = {}");
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "unknown cell key must fail: {log}");
	assert!(log.contains("unknown cell.rust key: registries"), "wrong diagnostic: {log}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn native_dependency_with_custom_lib_path_builds() {
	let dir = rust_workspace("custom-lib-path", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n[patch.local.answer]\npath = \"vendor/answer\"\n"),
	)
	.unwrap();
	std::fs::create_dir_all(dir.join("vendor/answer/src")).unwrap();
	std::fs::write(
		dir.join("vendor/answer/Cargo.toml"),
		"[package]\nname = \"answer\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/custom_entry.rs\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("vendor/answer/src/custom_entry.rs"),
		"pub fn answer() -> u32 { 42 }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/answer.tar.gz\"\nchecksum = \"fixture\"\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", answer::answer()); }\n").unwrap();
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?}: {log}");
	}
	build_and_run_rust(&dir, "app", 2, "42\n");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn library_and_binary_outputs_are_profile_distinct_and_cached_per_profile() {
	let dir = rust_workspace("profile-outputs", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	std::fs::write(
		dir.join("FORGE.toml"),
		"[library.math]\nvisibility = \"public\"\nsrcs = [\"src/lib.rs\"]\n\n[binary.app]\ndeps = [\"math\"]\nsrcs = [\"src/main.rs\"]\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", math::answer()); }\n").unwrap();

	let planned = |profile: &str| {
		let (_, dag) = Engine::open(&dir).plan_dag(profile).expect("plan Rust library");
		let outputs: Vec<String> = dag
			.specs
			.iter()
			.flat_map(|spec| spec.outputs.iter().map(|out| out.path.display().to_string()))
			.collect();
		outputs
	};
	let debug = planned("debug");
	assert_eq!(debug, planned("debug"), "a profile must plan stable output paths");
	let release = planned("release");
	assert_ne!(debug, release, "each profile must own its output files");
	for (profile, outputs) in [("debug", &debug), ("release", &release)] {
		for expected in [
			format!("forge-out/lib/{profile}/libmath.rlib"),
			format!("forge-out/bin/{profile}/app"),
		] {
			assert!(outputs.contains(&expected), "{profile} plan lacks {expected}: {outputs:?}");
		}
	}

	for profile in ["debug", "release"] {
		let (ok, log) = run_forge(&dir, &["build", "--profile", profile]);
		assert!(ok, "{profile} build failed: {log}");
		assert!(dir.join(format!("forge-out/lib/{profile}/libmath.rlib")).is_file(), "{log}");
		assert!(dir.join(format!("forge-out/bin/{profile}/app")).is_file(), "{log}");
	}
	let output = Command::new(dir.join("forge-out/bin/release/app"))
		.output()
		.expect("run the release binary");
	assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n");

	let (ok, log) = run_forge(&dir, &["build", "--profile", "release"]);
	assert!(ok, "{log}");
	assert!(log.contains("cache hit"), "repeat release build must hit the cache: {log}");
	assert!(
		dir.join("forge-out/lib/debug/libmath.rlib").is_file(),
		"the release build must leave the debug artifact in place: {log}"
	);
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_override_applies_to_build_scripts_but_not_targets() {
	let dir = rust_workspace("build-override", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n[profile.release.build]\nopt_level = 0\ndebug = 1\n"),
	)
	.unwrap();
	std::fs::write(dir.join("build.rs"), "fn main() {}\n").unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust]\nbuild = true\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"ok\"); }\n").unwrap();

	let (_, dag) = Engine::open(&dir).plan_dag("release").expect("plan release");
	let script = dag
		.specs
		.iter()
		.find(|spec| spec.name.starts_with("rustc build script"))
		.unwrap_or_else(|| {
			let names: Vec<&str> = dag.specs.iter().map(|spec| spec.name.as_str()).collect();
			panic!("build script action not found: {names:?}")
		});
	assert!(
		script.args.windows(2).any(|args| args == ["-C", "opt-level=0"]),
		"{:?}",
		script.args
	);
	assert!(
		script.args.windows(2).any(|args| args == ["-C", "debuginfo=1"]),
		"{:?}",
		script.args
	);
	let target = dag
		.specs
		.iter()
		.find(|spec| spec.name.starts_with("rustc //:app"))
		.expect("binary action");
	assert!(
		target.args.windows(2).any(|args| args == ["-C", "opt-level=3"]),
		"{:?}",
		target.args
	);
	assert!(
		!target.args.windows(2).any(|args| args == ["-C", "debuginfo=1"]),
		"{:?}",
		target.args
	);
	std::fs::remove_dir_all(dir).unwrap();
}
