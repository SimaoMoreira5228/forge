use std::path::{Path, PathBuf};

use forge_core::ActionSpec;
use forge_engine::Engine;
use forge_engine::planner::ActionDag;

mod forge_cli;
mod rust_toolchain;
mod rust_workspace;

use forge_cli::run_forge;
use rust_toolchain::install_rust_toolchain;
use rust_workspace::{build_and_run_rust, rust_workspace};

const HELPER_SOURCE: &str = "pub fn value() -> u32 { 42 }\n";
const HELPER_BUILD_SCRIPT: &str = "fn main() {\n    let generated = std::path::Path::new(&std::env::var(\"OUT_DIR\").unwrap()).join(\"generated.rs\");\n    std::fs::write(generated, format!(\"pub const FROM_BUILD: u32 = {};\\n\", helper::value())).unwrap();\n}\n";

fn build_dependency_workspace(name: &str, also_normal: bool) -> Option<PathBuf> {
	let dir = rust_workspace(name, "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return None;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"{root}\n[patch.local.helper]\npath = \"vendor/helper\"\n\n[profile.release.build]\nopt_level = 0\ndebug = 1\nstrip = \"symbols\"\n"
		),
	)
	.unwrap();
	std::fs::create_dir_all(dir.join("vendor/helper/src")).unwrap();
	std::fs::write(dir.join("vendor/helper/src/lib.rs"), HELPER_SOURCE).unwrap();
	let normal = if also_normal {
		"\n[binary.app.metadata.rust.dependencies.helper]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/helper.tar.gz\"\nchecksum = \"fixture\"\n"
	} else {
		""
	};
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust]\nbuild = true\n\n[binary.app.metadata.rust.build-dependencies.helper]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/helper.tar.gz\"\nchecksum = \"fixture\"\n{normal}"
		),
	)
	.unwrap();
	std::fs::write(dir.join("build.rs"), HELPER_BUILD_SCRIPT).unwrap();
	std::fs::write(
		dir.join("src/main.rs"),
		if also_normal {
			"include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));\nfn main() { println!(\"{}:{}\", helper::value(), FROM_BUILD); }\n"
		} else {
			"include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));\nfn main() { println!(\"{FROM_BUILD}\"); }\n"
		},
	)
	.unwrap();
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?}: {log}");
	}
	Some(dir)
}

fn rlib(crate_name: &str, profile: &str, host: bool) -> String {
	let deps = if host {
		format!("forge-out/lib/deps/host/{profile}")
	} else {
		format!("forge-out/lib/deps/{profile}")
	};
	format!("{deps}/lib{crate_name}-1.0.0.rlib")
}

fn dependency_unit<'a>(dag: &'a ActionDag, transition: &str) -> &'a ActionSpec {
	dag.specs
		.iter()
		.find(|spec| spec.name == format!("rustc dependency helper@1.0.0 ({transition})"))
		.unwrap_or_else(|| {
			let names: Vec<&str> = dag.specs.iter().map(|spec| spec.name.as_str()).collect();
			panic!("no {transition} unit for the build dependency: {names:?}")
		})
}

fn extern_of(spec: &ActionSpec, crate_name: &str) -> String {
	spec.args
		.windows(2)
		.find(|args| args[0] == "--extern" && args[1].starts_with(&format!("{crate_name}=")))
		.unwrap_or_else(|| panic!("{crate_name} is not an extern of {}", spec.name))[1]
		.clone()
}

fn vendored_crate(dir: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) {
	let root = dir.join("vendor").join(name);
	std::fs::create_dir_all(root.join("src")).unwrap();
	std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
	for (path, contents) in files {
		let file = root.join(path);
		std::fs::create_dir_all(file.parent().unwrap()).unwrap();
		std::fs::write(file, contents).unwrap();
	}
}

fn patch_local(names: &[&str]) -> String {
	names
		.iter()
		.map(|name| format!("[patch.local.{name}]\npath = \"vendor/{name}\"\n"))
		.collect::<Vec<_>>()
		.join("\n")
}

fn native_dependency_section(section: &str, name: &str, closure: &[&str]) -> String {
	let closure = closure
		.iter()
		.map(|dep| format!("\"{dep} 1.0.0\""))
		.collect::<Vec<_>>()
		.join(", ");
	format!(
		"[binary.app.metadata.rust.{section}.{name}]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/{name}.tar.gz\"\nchecksum = \"fixture\"\ndependencies = [{closure}]\n"
	)
}

fn lock_and_sync(dir: &Path) {
	for args in [&["deps", "lock"][..], &["deps", "sync"]] {
		let (ok, log) = run_forge(dir, args);
		assert!(ok, "{args:?}: {log}");
	}
}

fn spec_named<'a>(dag: &'a ActionDag, prefix: &str) -> &'a ActionSpec {
	dag.specs
		.iter()
		.find(|spec| spec.name.starts_with(prefix))
		.unwrap_or_else(|| {
			let names: Vec<&str> = dag.specs.iter().map(|spec| spec.name.as_str()).collect();
			panic!("no action starting with `{prefix}`: {names:?}")
		})
}

fn vendored_lib(name: &str, extra: &str, build: bool) -> String {
	format!(
		"[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n{}\n{extra}",
		if build { "build = \"build.rs\"" } else { "" }
	)
}

#[test]
fn target_dependency_build_script_does_not_need_host_units_of_its_normal_dependencies() {
	let dir = rust_workspace("build-dep-target-crate", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n{}\n", patch_local(&["lib", "shared", "helper"])),
	)
	.unwrap();
	vendored_crate(
		&dir,
		"lib",
		&vendored_lib(
			"lib",
			"[dependencies]\nshared = \"1.0.0\"\n\n[build-dependencies]\nhelper = \"1.0.0\"\n",
			true,
		),
		&[
			("src/lib.rs", "pub fn value() -> u32 { shared::value() }\n"),
			("build.rs", "fn main() { assert_eq!(helper::value(), 1); }\n"),
		],
	);
	vendored_crate(
		&dir,
		"shared",
		&vendored_lib("shared", "", false),
		&[("src/lib.rs", "pub fn value() -> u32 { 42 }\n")],
	);
	vendored_crate(
		&dir,
		"helper",
		&vendored_lib("helper", "", false),
		&[("src/lib.rs", "pub fn value() -> u32 { 1 }\n")],
	);
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n{}{}{}",
			native_dependency_section("dependencies", "lib", &["shared", "helper"]),
			native_dependency_section("dependencies", "shared", &[]),
			native_dependency_section("dependencies", "helper", &[])
		),
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", lib::value()); }\n").unwrap();
	lock_and_sync(&dir);

	let (_, dag) = Engine::open(&dir).plan_dag("release").expect("plan release");
	assert_eq!(
		spec_named(&dag, "rustc dependency lib@1.0.0 (target)").configuration,
		forge_core::ConfigTransition::Target
	);
	assert!(
		!dag.specs
			.iter()
			.any(|spec| spec.name.starts_with("rustc dependency shared@1.0.0 (host)")),
		"a normal dependency of a target crate must not gain a host unit"
	);
	let script = spec_named(&dag, "rustc build script lib@1.0.0");
	assert!(
		!script
			.inputs
			.iter()
			.chain(&script.execution_deps)
			.any(|path| path.to_string_lossy().contains("shared")),
		"a build script consumes its build dependencies, not its normal ones: {:?}",
		script.execution_deps
	);
	assert_eq!(
		extern_of(script, "helper"),
		format!("helper={}", rlib("helper", "release", true))
	);

	build_and_run_rust(&dir, "app", 7, "42\n");
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_dependency_of_a_host_crate_is_a_host_unit_when_a_target_crate_also_uses_it() {
	let dir = rust_workspace("build-dep-hosted", "mode = \"native\"");
	if !install_rust_toolchain(&dir) {
		return;
	}
	let root = std::fs::read_to_string(dir.join("FORGE_ROOT")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!("{root}\n{}\n", patch_local(&["hosted", "shared"])),
	)
	.unwrap();
	vendored_crate(
		&dir,
		"hosted",
		&vendored_lib("hosted", "[build-dependencies]\nshared = \"1.0.0\"\n", true),
		&[
			(
				"src/lib.rs",
				"include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));\npub fn value() -> u32 { FROM_HOSTED }\n",
			),
			(
				"build.rs",
				"fn main() {\n    let generated = std::path::Path::new(&std::env::var(\"OUT_DIR\").unwrap()).join(\"generated.rs\");\n    std::fs::write(generated, format!(\"pub const FROM_HOSTED: u32 = {};\\n\", shared::value())).unwrap();\n}\n",
			),
		],
	);
	vendored_crate(
		&dir,
		"shared",
		&vendored_lib("shared", "", false),
		&[("src/lib.rs", "pub fn value() -> u32 { 42 }\n")],
	);
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n{}\n{}",
			native_dependency_section("build-dependencies", "hosted", &["shared"]),
			native_dependency_section("dependencies", "shared", &[])
		),
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", shared::value()); }\n").unwrap();
	lock_and_sync(&dir);

	let (_, dag) = Engine::open(&dir).plan_dag("release").expect("plan release");
	let host = spec_named(&dag, "rustc dependency shared@1.0.0 (host)");
	let target = spec_named(&dag, "rustc dependency shared@1.0.0 (target)");
	assert_eq!(host.configuration, forge_core::ConfigTransition::Host);
	assert_eq!(target.configuration, forge_core::ConfigTransition::Target);
	let script = spec_named(&dag, "rustc build script hosted@1.0.0");
	assert_eq!(
		extern_of(script, "shared"),
		format!("shared={}", rlib("shared", "release", true))
	);
	assert_eq!(
		extern_of(spec_named(&dag, "rustc //:app"), "shared"),
		format!("shared={}", rlib("shared", "release", false))
	);

	build_and_run_rust(&dir, "app", 6, "42\n");
	assert!(dir.join(rlib("shared", "debug", true)).is_file());
	assert!(dir.join(rlib("shared", "debug", false)).is_file());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_dependency_library_is_a_host_unit_with_the_host_profile() {
	let Some(dir) = build_dependency_workspace("build-dep-host", false) else {
		return;
	};
	let (_, dag) = Engine::open(&dir).plan_dag("release").expect("plan release");

	let host = dependency_unit(&dag, "host");
	assert_eq!(host.configuration, forge_core::ConfigTransition::Host);
	assert!(
		host.args.windows(2).any(|args| args == ["-C", "opt-level=0"]),
		"{:?}",
		host.args
	);
	assert!(
		host.args.windows(2).any(|args| args == ["-C", "debuginfo=1"]),
		"{:?}",
		host.args
	);
	assert!(
		!host.args.windows(2).any(|args| args == ["-C", "strip=symbols"]),
		"{:?}",
		host.args
	);
	assert!(
		host.args.windows(2).any(|args| args == ["-C", "metadata=helper-1.0.0-host"]),
		"{:?}",
		host.args
	);
	assert_eq!(host.output_paths().next(), Some(Path::new(&rlib("helper", "release", true))));
	assert!(
		host.args
			.windows(2)
			.any(|args| args == ["-L", "dependency=forge-out/lib/deps/host/release"]),
		"{:?}",
		host.args
	);
	assert!(
		!dag.specs
			.iter()
			.any(|spec| spec.name == "rustc dependency helper@1.0.0 (target)"),
		"a build-only crate must not be planned for the target"
	);

	let script = dag
		.specs
		.iter()
		.find(|spec| spec.name == "rustc build script app@0.0.0")
		.expect("build script action");
	assert_eq!(script.configuration, forge_core::ConfigTransition::Host);
	assert_eq!(
		extern_of(script, "helper"),
		format!("helper={}", rlib("helper", "release", true))
	);

	let app = dag
		.specs
		.iter()
		.find(|spec| spec.name.starts_with("rustc //:app"))
		.expect("binary action");
	assert_eq!(app.configuration, forge_core::ConfigTransition::Target);
	assert!(
		!app.args.iter().any(|arg| arg.contains("deps/host/")),
		"the target must not link host units: {:?}",
		app.args
	);
	assert!(
		app.args.windows(2).any(|args| args == ["-C", "opt-level=3"]),
		"{:?}",
		app.args
	);

	build_and_run_rust(&dir, "app", 4, "42\n");
	assert!(dir.join(rlib("helper", "debug", true)).is_file());
	assert!(!dir.join(rlib("helper", "debug", false)).exists());
	std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shared_build_and_target_dependency_gets_one_unit_per_configuration() {
	let Some(dir) = build_dependency_workspace("build-dep-shared", true) else {
		return;
	};
	let (_, dag) = Engine::open(&dir).plan_dag("release").expect("plan release");

	let host = dependency_unit(&dag, "host");
	let target = dependency_unit(&dag, "target");
	assert_eq!(host.configuration, forge_core::ConfigTransition::Host);
	assert_eq!(target.configuration, forge_core::ConfigTransition::Target);
	assert!(
		host.args.windows(2).any(|args| args == ["-C", "opt-level=0"]),
		"{:?}",
		host.args
	);
	assert!(
		target.args.windows(2).any(|args| args == ["-C", "opt-level=3"]),
		"{:?}",
		target.args
	);
	assert!(
		host.args.windows(2).any(|args| args == ["-C", "debuginfo=1"]),
		"{:?}",
		host.args
	);
	assert!(
		target.args.windows(2).any(|args| args == ["-C", "debuginfo=0"]),
		"{:?}",
		target.args
	);
	assert!(
		target.args.windows(2).any(|args| args == ["-C", "strip=symbols"]),
		"{:?}",
		target.args
	);
	assert_ne!(host.output_paths().next(), target.output_paths().next());

	let script = dag
		.specs
		.iter()
		.find(|spec| spec.name == "rustc build script app@0.0.0")
		.expect("build script action");
	assert_eq!(
		extern_of(script, "helper"),
		format!("helper={}", rlib("helper", "release", true))
	);
	let app = dag
		.specs
		.iter()
		.find(|spec| spec.name.starts_with("rustc //:app"))
		.expect("binary action");
	assert_eq!(
		extern_of(app, "helper"),
		format!("helper={}", rlib("helper", "release", false))
	);

	build_and_run_rust(&dir, "app", 5, "42:42\n");
	assert!(dir.join(rlib("helper", "debug", true)).is_file());
	assert!(dir.join(rlib("helper", "debug", false)).is_file());
	std::fs::remove_dir_all(dir).unwrap();
}
