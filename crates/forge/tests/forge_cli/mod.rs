#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn forge_bin() -> &'static str {
	env!("CARGO_BIN_EXE_forge")
}

pub fn run_forge(dir: &Path, args: &[&str]) -> (bool, String) {
	let out = Command::new(forge_bin())
		.args(args)
		.current_dir(dir)
		.env("FORGE_STORE_DIR", dir.join(".forge"))
		.output()
		.expect("launch forge");
	(out.status.success(), log_of(&out))
}

pub fn run_forge_in_shared_store(dir: &Path, args: &[&str]) -> (bool, String) {
	let out = Command::new(forge_bin())
		.args(args)
		.current_dir(dir)
		.output()
		.expect("launch forge");
	(out.status.success(), log_of(&out))
}

fn log_of(out: &std::process::Output) -> String {
	format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}
