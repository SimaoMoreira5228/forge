use std::path::{Path, PathBuf};
use std::process::Command;

mod c_workspace;
mod forge_cli;

use c_workspace::write_workspace;
use forge_cli::run_forge_in_shared_store;

const EXCLUDED: [&str; 8] = [
	"target",
	"forge-out",
	"local",
	".git",
	".cargo",
	".forge",
	".kilo",
	".opencode",
];

fn repository_root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.expect("workspace")
		.parent()
		.expect("repository")
		.to_path_buf()
}

fn copy_tree(source: &Path, destination: &Path) {
	std::fs::create_dir_all(destination).unwrap();
	for entry in std::fs::read_dir(source).unwrap() {
		let entry = entry.unwrap();
		let name = entry.file_name().to_string_lossy().into_owned();
		if EXCLUDED.contains(&name.as_str()) {
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

fn scratch(prefix: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-self-host-{prefix}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	dir
}

fn self_host_workspace() -> PathBuf {
	let dir = scratch("workspace");
	copy_tree(&repository_root(), &dir);
	dir
}

fn produced(dir: &Path, args: &[&str]) -> (bool, String) {
	let out = Command::new(dir.join("forge-out/bin/debug/forge"))
		.args(args)
		.output()
		.expect("launch the Forge-built Forge");
	(
		out.status.success(),
		format!(
			"{}{}",
			String::from_utf8_lossy(&out.stdout),
			String::from_utf8_lossy(&out.stderr)
		),
	)
}

fn build_counts(log: &str) -> (usize, usize, usize) {
	let (head, tail) = log.rsplit_once("actions (").expect("build summary");
	let actions = head
		.split_whitespace()
		.last()
		.expect("action count")
		.parse()
		.expect("action count");
	let counts: Vec<usize> = tail
		.split(|c: char| !c.is_ascii_digit())
		.filter(|part| !part.is_empty())
		.map(|part| part.parse().expect("action count"))
		.collect();
	(actions, counts[0], counts[1])
}

#[test]
fn forge_builds_itself_into_a_working_binary_and_reuses_its_own_cache() {
	if std::env::var_os("FORGE_SELF_HOST").is_none() {
		eprintln!("skipping: set FORGE_SELF_HOST=1 to build Forge with Forge");
		return;
	}
	let dir = self_host_workspace();
	assert!(dir.join("FORGE_ROOT").is_file(), "the copy lost FORGE_ROOT");
	assert!(dir.join("Cargo.lock").is_file(), "the copy lost Cargo.lock");
	assert!(!dir.join("target").exists() && !dir.join("forge-out").exists() && !dir.join("local").exists());

	let (ok, log) = run_forge_in_shared_store(&dir, &["build"]);
	assert!(ok, "self-hosted build failed: {log}");
	let (actions, _, _) = build_counts(&log);
	assert!(actions > 1, "the self-hosted build planned {actions} actions: {log}");
	assert!(dir.join("forge-out/bin/debug/forge").is_file(), "no Forge-built Forge: {log}");

	let (ok, log) = produced(&dir, &["--version"]);
	assert!(ok, "the Forge-built Forge did not run: {log}");
	assert!(
		log.contains(env!("CARGO_PKG_VERSION")),
		"the Forge-built Forge reported the wrong version: {log}"
	);

	let (ok, log) = produced(&dir, &["confine"]);
	assert!(
		ok && log.contains("backend:"),
		"the Forge-built Forge claims no confinement backend: {log}"
	);

	let fixture = scratch("fixture");
	write_workspace(&fixture);
	let out = Command::new(dir.join("forge-out/bin/debug/forge"))
		.args(["run", "app"])
		.current_dir(&fixture)
		.output()
		.expect("the Forge-built Forge must build a program");
	assert!(out.status.success(), "the Forge-built Forge could not run a build: {out:?}");
	assert!(String::from_utf8_lossy(&out.stdout).contains("42"), "{out:?}");
	std::fs::remove_dir_all(&fixture).unwrap();

	let (ok, log) = run_forge_in_shared_store(&dir, &["build"]);
	assert!(ok, "the second self-hosted build failed: {log}");
	let (actions, executed, cache_hits) = build_counts(&log);
	assert_eq!(
		executed, 0,
		"an unchanged self-hosted build executed {executed} of {actions} actions: {log}"
	);
	assert_eq!(
		cache_hits, actions,
		"a self-hosted build must recognise every one of its own inputs: {log}"
	);

	std::fs::remove_dir_all(&dir).unwrap();
}
