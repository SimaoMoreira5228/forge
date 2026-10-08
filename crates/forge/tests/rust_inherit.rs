use std::path::{Path, PathBuf};

use forge_core::ActionSpec;
use forge_engine::Engine;
use forge_engine::build::planner::ActionDag;

mod forge_cli;
mod rust_toolchain;
mod rust_workspace;

use forge_cli::run_forge;
use rust_toolchain::install_rust_toolchain;
use rust_workspace::{build_and_run_rust, rust_workspace};

const ANSWER_SOURCE: &str = "pub fn answer() -> u32 { 42 }\n";
const FEATURED_SOURCE: &str = r#"pub fn enabled() -> String {
    let mut names: Vec<&str> = Vec::new();
    if cfg!(feature = "base") { names.push("base"); }
    if cfg!(feature = "extra") { names.push("extra"); }
    names.join("+")
}
"#;

fn catalog(entry: &str) -> String {
	format!(
		"mode = \"native\"\n\n[cell.rust.dependencies.answer]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/answer.tar.gz\"\nchecksum = \"fixture\"\n{entry}"
	)
}

fn inherited_workspace(name: &str, entry: &str) -> Option<PathBuf> {
	let dir = rust_workspace(name, &catalog(entry));
	if !install_rust_toolchain(&dir) {
		return None;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n[patch.local.answer]\npath = \"vendor/answer\"\n"),
	)
	.unwrap();
	Some(dir)
}

fn vendor(dir: &Path, name: &str, source: &str) {
	let root = dir.join("vendor").join(name);
	std::fs::create_dir_all(root.join("src")).unwrap();
	std::fs::write(root.join("src/lib.rs"), source).unwrap();
}

fn extern_of(spec: &ActionSpec, crate_name: &str) -> String {
	spec.args
		.windows(2)
		.find(|args| args[0] == "--extern" && args[1].starts_with(&format!("{crate_name}=")))
		.unwrap_or_else(|| panic!("{crate_name} is not an extern of {}\n{:?}", spec.name, spec.args))[1]
		.clone()
}

fn crate_spec<'a>(dag: &'a ActionDag, component: &str) -> &'a ActionSpec {
	let prefix = format!("rustc {component} ->");
	dag.specs
		.iter()
		.find(|spec| spec.name.starts_with(&prefix))
		.unwrap_or_else(|| {
			let names: Vec<&str> = dag.specs.iter().map(|spec| spec.name.as_str()).collect();
			panic!("no action for {component}: {names:?}")
		})
}

fn build_dir(dir: &Path) -> String {
	let (ok, log) = run_forge(dir, &["build"]);
	assert!(ok, "build failed: {log}");
	log
}

#[test]
fn two_targets_inherit_one_catalog_entry_lock_link_and_run() {
	let Some(dir) = inherited_workspace("inherit-two", "") else {
		return;
	};
	vendor(&dir, "answer", ANSWER_SOURCE);
	std::fs::create_dir_all(dir.join("tool")).unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nworkspace = true\n\n[binary.tool]\nsrcs = [\"tool/main.rs\"]\n\n[binary.tool.metadata.rust.dependencies.answer]\nworkspace = true\n",
	)
	.unwrap();
	for entry in ["src/main.rs", "tool/main.rs"] {
		std::fs::write(dir.join(entry), "fn main() { println!(\"{}\", answer::answer()); }\n").unwrap();
	}
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?}: {log}");
	}
	let lock = std::fs::read_to_string(dir.join("forge.lock")).unwrap();
	assert!(lock.contains("answer") && lock.contains("1.0.0"), "{lock}");

	let (_, dag) = Engine::open(&dir).plan_dag("debug").expect("plan debug");
	let answer = "answer=forge-out/lib/deps/debug/libanswer-1.0.0.rlib".to_string();
	for component in ["//:app", "//:tool"] {
		assert_eq!(extern_of(crate_spec(&dag, component), "answer"), answer, "{component}");
	}

	let log = build_dir(&dir);
	assert!(log.contains("3 executed"), "{log}");
	for binary in ["app", "tool"] {
		let output = std::process::Command::new(dir.join(format!("forge-out/bin/debug/{binary}")))
			.output()
			.unwrap();
		assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n");
	}
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inherited_declaration_unions_catalog_features_into_the_built_crate() {
	let Some(dir) = inherited_workspace("inherit-features", "features = [\"base\"]\n") else {
		return;
	};
	vendor(&dir, "answer", FEATURED_SOURCE);
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nworkspace = true\nfeatures = [\"extra\"]\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/main.rs"),
		"fn main() { println!(\"{}\", answer::enabled()); }\n",
	)
	.unwrap();
	build_and_run_rust(&dir, "app", 2, "base+extra\n");

	let (_, dag) = Engine::open(&dir).plan_dag("debug").expect("plan debug");
	let unit = dag
		.specs
		.iter()
		.find(|spec| spec.name == "rustc dependency answer@1.0.0 (target)")
		.expect("dependency unit");
	for feature in ["base", "extra"] {
		assert!(
			unit.args
				.windows(2)
				.any(|args| args[0] == "--cfg" && args[1] == format!("feature=\"{feature}\"")),
			"missing --cfg feature=\"{feature}\": {:?}",
			unit.args
		);
	}
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_catalog_entry_and_ambiguous_conflicts_name_the_offending_declaration() {
	let Some(dir) = inherited_workspace("inherit-missing", "") else {
		return;
	};
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.ghost]\nworkspace = true\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "an undeclared workspace dependency must fail: {log}");
	assert!(
		log.contains("dependency ghost inherits cell.rust.dependencies, which does not declare ghost"),
		"{log}"
	);

	for conflict in ["version = \"1.0.0\"", "source = \"https://example.invalid/answer.tar.gz\""] {
		std::fs::write(
			dir.join("FORGE.toml"),
			format!("[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nworkspace = true\n{conflict}\n"),
		)
		.unwrap();
		let (ok, log) = run_forge(&dir, &["build"]);
		assert!(!ok, "a conflicting inherited declaration must fail: {log}");
		let key = conflict.split_whitespace().next().unwrap();
		assert!(
			log.contains(&format!(
				"dependency answer inherits cell.rust.dependencies and also sets {key}; an inherited declaration may only add features and default-features"
			)),
			"{log}"
		);
	}
	assert!(!dir.join("forge.lock").exists(), "{log}");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cargo_workspace_inherits_shared_dependencies_for_every_member() {
	let dir = rust_workspace("inherit-cargo", "mode = \"cargo\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let files = [
		(
			"Cargo.toml",
			"[workspace]\nresolver = \"2\"\nmembers = [\"crates/math\", \"crates/app\", \"crates/tool\"]\n\n[workspace.dependencies]\nmath-package = { path = \"crates/math\", version = \"2.3.4\" }\n",
		),
		(
			"Cargo.lock",
			"version = 4\n\n[[package]]\nname = \"math-package\"\nversion = \"2.3.4\"\n\n[[package]]\nname = \"workspace-app\"\nversion = \"5.6.7\"\n\n[[package]]\nname = \"workspace-tool\"\nversion = \"5.6.8\"\n",
		),
		(
			"crates/math/Cargo.toml",
			"[package]\nname = \"math-package\"\nversion = \"2.3.4\"\nedition = \"2021\"\n\n[lib]\nname = \"math_library\"\n",
		),
		(
			"crates/app/Cargo.toml",
			"[package]\nname = \"workspace-app\"\nversion = \"5.6.7\"\nedition = \"2021\"\n\n[dependencies]\nmath-package = { workspace = true, features = [\"extra\"] }\n",
		),
		(
			"crates/tool/Cargo.toml",
			"[package]\nname = \"workspace-tool\"\nversion = \"5.6.8\"\nedition = \"2021\"\n\n[dependencies]\nmath-package = { workspace = true }\n",
		),
		("crates/math/src/lib.rs", "pub fn answer() -> i32 { 42 }\n"),
		(
			"crates/app/src/main.rs",
			"fn main() { println!(\"app:{}\", math_package::answer()); }\n",
		),
		(
			"crates/tool/src/main.rs",
			"fn main() { println!(\"tool:{}\", math_package::answer()); }\n",
		),
	];
	for (path, contents) in files {
		let file = dir.join(path);
		std::fs::create_dir_all(file.parent().unwrap()).unwrap();
		std::fs::write(file, contents).unwrap();
	}
	let log = build_dir(&dir);
	assert!(log.contains("3 executed"), "{log}");
	for (binary, expected) in [("workspace_app", "app:42\n"), ("workspace_tool", "tool:42\n")] {
		let output = std::process::Command::new(dir.join(format!("forge-out/bin/debug/{binary}")))
			.output()
			.unwrap();
		assert!(output.status.success(), "{binary} failed: {output:?}");
		assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
	}
	for (path, contents) in files {
		assert_eq!(
			std::fs::read(dir.join(path)).unwrap(),
			contents.as_bytes(),
			"{path} was modified"
		);
	}
	assert!(!dir.join("target").exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inherited_dependencies_lock_the_same_way_from_the_resolve_hook_and_the_build() {
	let Some(dir) = inherited_workspace("inherit-lock", "") else {
		return;
	};
	vendor(&dir, "answer", ANSWER_SOURCE);
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.answer]\nworkspace = true\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", answer::answer()); }\n").unwrap();
	let (ok, log) = run_forge(&dir, &["deps", "lock"]);
	assert!(ok, "deps lock failed: {log}");
	let resolved = std::fs::read_to_string(dir.join("forge.lock")).unwrap();
	let (ok, log) = run_forge(&dir, &["deps", "sync"]);
	assert!(ok, "deps sync failed: {log}");
	build_and_run_rust(&dir, "app", 2, "42\n");
	assert_eq!(std::fs::read_to_string(dir.join("forge.lock")).unwrap(), resolved);

	std::fs::remove_file(dir.join("forge.lock")).unwrap();
	build_and_run_rust(&dir, "app", 0, "42\n");
	assert!(!dir.join("forge.lock").exists(), "the build path must not write a lock");
	let (ok, log) = run_forge(&dir, &["deps", "lock"]);
	assert!(ok, "deps lock after a build-path build failed: {log}");
	assert_eq!(
		std::fs::read_to_string(dir.join("forge.lock")).unwrap(),
		resolved,
		"both paths must resolve the inherited declaration identically"
	);
	std::fs::remove_dir_all(dir).unwrap();
}
