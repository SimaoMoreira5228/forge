#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub fn forge_bin() -> &'static str {
	env!("CARGO_BIN_EXE_forge")
}

pub fn diagnostic_text(log: &str) -> String {
	log.split_whitespace()
		.filter(|word| !matches!(*word, "│" | "|"))
		.collect::<Vec<_>>()
		.join(" ")
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

pub fn run_forge_in(dir: &Path, args: &[&str], store: &Path) -> (bool, String) {
	let out = Command::new(forge_bin())
		.args(args)
		.current_dir(dir)
		.env("FORGE_STORE_DIR", store)
		.output()
		.expect("launch forge");
	(out.status.success(), log_of(&out))
}

pub fn action_keys(workspace: &Path) -> BTreeMap<String, String> {
	let proof = std::fs::read_to_string(workspace.join("forge-out/forge.proof")).expect("read forge.proof");
	let recorded: serde_json::Value = serde_json::from_str(&proof).expect("parse forge.proof");
	recorded["entries"]
		.as_array()
		.expect("proof entries")
		.iter()
		.map(|entry| {
			(
				entry["action"].as_str().expect("action name").to_string(),
				entry["key"].as_str().expect("action key").to_string(),
			)
		})
		.collect()
}

fn log_of(out: &std::process::Output) -> String {
	format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}
