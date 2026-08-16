mod common;

use common::*;

fn build_in(dir: &std::path::Path, store: &std::path::Path) -> (bool, String) {
	let out = std::process::Command::new(forge_bin())
		.args(["build"])
		.current_dir(dir)
		.env("FORGE_STORE_DIR", store)
		.output()
		.expect("launch forge");
	(
		out.status.success(),
		format!(
			"{}{}",
			String::from_utf8_lossy(&out.stdout),
			String::from_utf8_lossy(&out.stderr)
		),
	)
}

#[test]
fn identical_workspaces_share_action_results_through_the_store() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let root = std::env::temp_dir().join(format!("forge-ishare-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let store = root.join("store");
	let first = root.join("first");
	let second = root.join("second");
	std::fs::create_dir_all(&first).unwrap();
	std::fs::create_dir_all(&second).unwrap();
	write_workspace(&first);
	write_workspace(&second);

	let (ok, log) = build_in(&first, &store);
	assert!(ok, "first build failed: {log}");
	assert!(log.contains("4 executed"), "first workspace must execute: {log}");

	let (ok, log) = build_in(&second, &store);
	assert!(ok, "second build failed: {log}");
	assert!(
		log.contains("0 executed") && log.contains("4 cache hits"),
		"an identical workspace must reuse global action results: {log}"
	);

	let _ = std::fs::remove_dir_all(&root);
}
