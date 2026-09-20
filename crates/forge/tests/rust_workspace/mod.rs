#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::forge_cli::run_forge;

pub fn rust_workspace(name: &str, rust_config: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-rust-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		format!(
			"[project]\nname = \"rust_workspace\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.rust]\nfrom = \"version\"\nversion = \"1.98.0\"\n\n[cell.rust]\n{rust_config}\n"
		),
	)
	.unwrap();
	dir
}

pub fn build_and_run_rust(dir: &Path, binary: &str, executed: usize, expected: &str) {
	let (ok, log) = run_forge(dir, &["build"]);
	assert!(ok, "Rust build failed in {}: {log}", dir.display());
	assert!(
		log.contains(&format!("{executed} executed")),
		"expected actual compilation: {log}"
	);
	let path = dir.join(format!("forge-out/bin/debug/{binary}"));
	let output = Command::new(&path).output().expect("execute the Forge-built Rust binary");
	assert!(output.status.success(), "{} failed: {output:?}", path.display());
	assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}
