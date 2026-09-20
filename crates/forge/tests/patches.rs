mod forge_cli;
mod rust_toolchain;

use forge_cli::run_forge;
use rust_toolchain::{have_rustc, link_rust_toolchain};

#[test]
fn local_patch_sources_a_dependency_from_the_workspace() {
	if !have_rustc() {
		eprintln!("skipping: no rustc");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ipatch-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::create_dir_all(dir.join("vendor/dep/src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"patch_itest\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.rust]\nfrom = \"version\"\nversion = \"1.98.0\"\n\n[patch.local.dep]\npath = \"vendor/dep\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("vendor/dep/Cargo.toml"),
		"[package]\nname = \"dep\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("vendor/dep/src/lib.rs"),
		"pub fn hello() -> &'static str { \"hello from patch\" }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.dep]\nrange = { min = \"1.0.0\", max = \"2.0.0\" }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("forge.lock"),
		"version = 1\n\n[[packages]]\nname = \"dep\"\nversion = \"1.0.0\"\nsource = \"https://example.invalid/dep.tar\"\nchecksum = \"dep-sha\"\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"{}\", dep::hello()); }\n").unwrap();

	if !link_rust_toolchain(&dir) {
		eprintln!("skipping: Rust 1.98.0 toolchain is not installed");
		return;
	}

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "patched build failed: {log}");
	assert!(
		!dir.join(".forge/sources/dep/1.0.0").exists(),
		"a patched dependency must not be fetched"
	);
	let binary = dir.join("forge-out/bin/debug/app");
	let output = std::process::Command::new(&binary).output().expect("run built binary");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello from patch");

	let _ = std::fs::remove_dir_all(&dir);
}
